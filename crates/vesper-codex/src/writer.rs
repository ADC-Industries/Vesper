//! The Codex writer: the one serialized path of every collection write.
//!
//! Each write holds the writer's lock from start to end: **plan** (against the collection's
//! current documents, no side effects: the server's content and show-document plans),
//! **persist** ([`crate::store::apply`], atomic, never over a newer schema), **swap** the new
//! documents in, then **notify** (`documentChanged` with the collection, `codexChanged`). For
//! this box's active collection the swap is the show actor's
//! ([`super::CollectionUpdate`]: the show follows the edit, as content edits always
//! did); for any other collection it is the Codex state's. Collection CRUD and this box's
//! assignment go through the same lock, and so does a reload from disk ([`Writer::reload`]), so
//! nothing changes a collection between a plan and its swap.
//!
//! Every document added, changed or removed is appended to its collection's change log
//! ([`super::log`], with who did it) once it is on disk. A deleted document goes to the trash
//! ([`super::trash`]) before it is deleted; restoring recreates it with its ID, through the
//! same path as any other write.

use super::log::{self, LogState, NewChange};
use super::trash::{self, Trash};
use crate::Shared;
use crate::data::{ChangeEvent, CollectionContent, ContentChange, DocHashes};
use crate::store::{DocWrite, StoreError};
use crate::{CollectionUpdate, Show};
use crate::{LoadedCollection, write_doc};
use model::{
    AssetLocation, ChangeBy, Collection, CollectionId, DocChange, Document, DocumentKind,
    TrashEntry, sort_key_between,
};
use protocol::{
    ApiError, CodexPart, CodexStatus, CollectionChanges, CollectionCounts, CollectionSummary,
    CreateCollection, DeletedCollection, DocumentChange, ErrorCode, ServerMessage, TrashItem,
    TrashPurged, TrashRestored, UpdateCollection,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use vesper_core::change::{ConfigError, check_revision, revision};

/// Longest collection name, characters.
pub const MAX_NAME: usize = 200;
/// Longest collection description, characters.
pub const MAX_DESCRIPTION: usize = 2000;

/// The Codex writer (one per server, [`super::Server::codex_writer`]).
pub struct Writer {
    lock: tokio::sync::Mutex<()>,
    /// Each collection's change log, once used (changed only under `lock`).
    logs: Mutex<HashMap<CollectionId, LogState>>,
    /// The trash's index, oldest first, once read (changed only under `lock`).
    trash: Mutex<Option<Vec<TrashEntry>>>,
    /// Changes on disk that neither reached a collection's change log nor could be noted as
    /// pending (a full disk): appended first at its next append.
    unlogged: Mutex<HashMap<CollectionId, Vec<(NewChange, ChangeBy)>>>,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

/// Media documents to write in one collection ([`Writer::media`]): each replaces the document
/// with its ID, or adds it.
#[derive(Clone, Debug, Default)]
pub struct MediaPlan {
    /// Media assets.
    pub assets: Vec<model::MediaAsset>,
    /// Media bins.
    pub bins: Vec<model::MediaBin>,
}

/// A planned change of one collection's documents.
pub struct Planned {
    /// Files to write / delete, in order.
    pub writes: Vec<DocWrite>,
    /// The change, for the show (the active collection) or the Codex state.
    pub update: CollectionUpdate,
    /// `documentChanged` events once applied.
    pub events: Vec<ChangeEvent>,
    /// Documents it deletes, for the trash (the main one first, then a folder's subtree).
    pub trashed: Option<(DocumentKind, Vec<Value>)>,
    /// A document it creates with a given ID (a restore or an undo): out of the trash.
    pub recreated: Option<uuid::Uuid>,
}

impl Planned {
    /// A change that trashes and recreates nothing.
    pub fn new(writes: Vec<DocWrite>, update: CollectionUpdate, events: Vec<ChangeEvent>) -> Self {
        Planned {
            writes,
            update,
            events,
            trashed: None,
            recreated: None,
        }
    }
}

fn doc_change(c: DocumentChange) -> DocChange {
    match c {
        DocumentChange::Added => DocChange::Added,
        DocumentChange::Changed => DocChange::Changed,
        DocumentChange::Removed => DocChange::Removed,
    }
}

fn join_err(e: tokio::task::JoinError) -> ApiError {
    ApiError::new(ErrorCode::Internal, e.to_string())
}

fn err(code: ErrorCode, msg: impl Into<String>) -> ConfigError {
    ApiError::new(code, msg).into()
}

fn now() -> String {
    crate::auth::rfc3339(std::time::SystemTime::now(), true)
}

fn store_error(e: StoreError) -> ApiError {
    match e {
        StoreError::TooNew(m) => ApiError::new(ErrorCode::Conflict, m),
        StoreError::Io(e) => ApiError::new(ErrorCode::Internal, format!("saving failed: {e:#}")),
    }
}

/// `content` with a change applied (for a collection that isn't active).
fn applied(content: &CollectionContent, update: &CollectionUpdate) -> CollectionContent {
    match update {
        CollectionUpdate::Content(change) => content.with_change(change),
        CollectionUpdate::ShowDocs {
            timers,
            props,
            messages,
            stage_layouts,
        } => content.with_show_docs(
            timers.clone(),
            props.clone(),
            messages.clone(),
            stage_layouts.clone(),
        ),
        CollectionUpdate::Switch(_) => content.clone(),
        CollectionUpdate::Mirror(c) => (**c).clone(),
    }
}

/// Content writes are taken on this box: it hosts the Codex. A show box forwards its content
/// writes to the site's Codex ([`super::write_through`]); anything else that would write here
/// answers `503 unavailable`; its own Codex, dormant, is read only.
pub fn writable(sh: &Shared) -> Result<(), ApiError> {
    match sh.codex().config.role {
        model::CodexRole::Host => Ok(()),
        model::CodexRole::Client => Err(ApiError::new(
            ErrorCode::Unavailable,
            "this box shows a copy of the site's Codex: this change is made on the site's server (the box that holds the Codex)",
        )),
        model::CodexRole::Dormant => Err(ApiError::new(
            ErrorCode::Unavailable,
            "this box's own Codex is dormant while the site's collection is copied: edit on the site's server",
        )),
        model::CodexRole::Unknown => Err(ApiError::new(
            ErrorCode::Unavailable,
            "this box's Codex role comes from a newer server: edits are refused",
        )),
    }
}

tokio::task_local! {
    /// A show box's forwarded write ([`super::write_through::execute`]): by when the route
    /// must have the writer, or it is refused. The box stops waiting soon after, and a write
    /// it gave up on must never be applied later; a long job that queued meanwhile (an import,
    /// an export) must not slip in ahead and hold the write past that.
    pub(crate) static WRITER_DEADLINE: tokio::time::Instant;
}

/// A forwarded write didn't get the writer in time.
pub fn busy() -> ApiError {
    ApiError::new(
        ErrorCode::Unavailable,
        "the site's server is busy with a long job (an import or export): nothing was changed, try again in a minute",
    )
}

impl Writer {
    /// Holds the writer (no write runs meanwhile): a show box swaps its copy in.
    pub async fn exclusive(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.lock.lock().await
    }

    /// Takes the writer for a write: within a forwarded write's [`WRITER_DEADLINE`] (else
    /// [`busy`]), otherwise as long as it takes.
    async fn hold(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, ApiError> {
        match WRITER_DEADLINE.try_with(|d| *d) {
            // Past the deadline the box has stopped waiting: refused even with the writer free
            // (`timeout_at` would still hand out a free lock), after slow work before this (E04-3).
            Ok(d) if tokio::time::Instant::now() >= d => Err(busy()),
            Ok(d) => tokio::time::timeout_at(d, self.lock.lock())
                .await
                .map_err(|_| busy()),
            Err(_) => Ok(self.lock.lock().await),
        }
    }

    /// A collection's change log's newest position (from the writer's state, else its file).
    pub fn head(&self, sh: &Shared, collection: CollectionId) -> u64 {
        if let Some(st) = crate::lock(&self.logs).get(&collection) {
            return st.head;
        }
        log::state(&log::path(&sh.codex().layout.collection(collection)))
            .map(|s| s.head)
            .unwrap_or(0)
    }

    /// A writer.
    pub fn new() -> Writer {
        Writer {
            lock: tokio::sync::Mutex::new(()),
            logs: Mutex::new(HashMap::new()),
            trash: Mutex::new(None),
            unlogged: Mutex::new(HashMap::new()),
        }
    }

    /// `reloadData`: reloads the data folder (the box's documents and every collection) while
    /// holding the writer, and logs what changed on disk in each collection (`by`: `reload`).
    pub async fn reload(&self, sh: &Arc<Shared>, show: &dyn Show) -> Result<u64, ApiError> {
        let _w = self.lock.lock().await;
        self.reload_locked(sh, show).await
    }

    async fn reload_locked(&self, sh: &Arc<Shared>, show: &dyn Show) -> Result<u64, ApiError> {
        let host_before = sh.codex_is_host();
        let hashes = |sh: &Shared| -> HashMap<CollectionId, DocHashes> {
            sh.codex()
                .collections
                .iter()
                .filter_map(|c| {
                    let id = c.collection.id;
                    Some((id, sh.collection_content(id)?.hashes.clone()))
                })
                .collect()
        };
        let before = hashes(sh);
        let revision = show.reload_data().await?;
        sh.library().marks_due();
        if !host_before || !sh.codex_is_host() {
            // (A show box, or the role changed: the changes aren't this Codex's.)
            return Ok(revision);
        }
        let after = hashes(sh);
        let by = ChangeBy {
            client: "reload".into(),
            client_id: None,
            machine: sh.auth().identity().id.to_string(),
        };
        for (id, new) in &after {
            let Some(old) = before.get(id) else { continue };
            let changes: Vec<NewChange> = diff(old, new)
                .into_iter()
                .map(|(kind, doc, change)| NewChange {
                    kind,
                    id: doc,
                    change: doc_change(change),
                    revision: None,
                })
                .collect();
            self.log(sh, *id, changes, &by).await;
        }
        Ok(revision)
    }

    /// A change of this box's active collection's documents (a show box's forwarded request:
    /// of the collection it shows), planned by `plan` against the collection's current
    /// documents under the writer's lock: content edits and timers, messages, props and stage
    /// layouts, planned by the server. `client_id` is echoed in `documentChanged`.
    pub async fn change<R>(
        &self,
        sh: &Arc<Shared>,
        show: &dyn Show,
        plan: impl FnOnce(&Arc<CollectionContent>) -> Result<(Planned, R), ConfigError>,
        client_id: Option<String>,
        by: ChangeBy,
    ) -> Result<R, ConfigError> {
        let _w = self.hold().await?;
        writable(sh)?;
        let collection = sh.forwarded_collection().unwrap_or(sh.codex().active);
        let content = self.content_of(sh, collection)?;
        let (planned, result) = plan(&content)?;
        self.commit(sh, show, collection, &content, planned, client_id, &by)
            .await?;
        Ok(result)
    }

    /// Writes media assets and bins of a collection (uploads, probes, relinks). `plan` runs
    /// under the writer's lock against the collection's current documents; documents equal to
    /// the stored ones are left alone. Returns the revisions of the written assets, in order.
    pub async fn media(
        &self,
        sh: &Arc<Shared>,
        show: &dyn Show,
        collection: CollectionId,
        plan: impl FnOnce(&CollectionContent) -> Result<MediaPlan, ConfigError>,
        client_id: Option<String>,
        by: ChangeBy,
    ) -> Result<Vec<String>, ConfigError> {
        let _w = self.hold().await?;
        writable(sh)?;
        let content = self.content_of(sh, collection)?;
        let p = plan(&content)?;
        let mut writes = Vec::new();
        let mut events = Vec::new();
        let mut change = ContentChange::default();
        let mut revisions = Vec::new();
        for mut a in p.assets {
            // The schema version it is written with, kept in memory too (its revision stays
            // the same after a reload).
            if let Ok(v) = serde_json::to_value(&a) {
                a.schema_version = model::version_for(&v);
            }
            let old = content.assets.get(&a.id);
            if old == Some(&a) {
                continue;
            }
            let rev = revision(&a);
            writes.push(DocWrite::put(&a).map_err(|e| err(ErrorCode::Internal, format!("{e:#}")))?);
            events.push(ChangeEvent {
                kind: DocumentKind::MediaAsset,
                id: a.id.0,
                change: if old.is_some() {
                    DocumentChange::Changed
                } else {
                    DocumentChange::Added
                },
                revision: Some(rev.clone()),
                ops: None,
            });
            revisions.push(rev);
            change.assets.push((a.id, Some(a)));
        }
        for b in p.bins {
            let old = content.media_bins.iter().find(|x| x.id == b.id);
            if old == Some(&b) {
                continue;
            }
            writes.push(DocWrite::put(&b).map_err(|e| err(ErrorCode::Internal, format!("{e:#}")))?);
            events.push(ChangeEvent {
                kind: DocumentKind::MediaBin,
                id: b.id.0,
                change: if old.is_some() {
                    DocumentChange::Changed
                } else {
                    DocumentChange::Added
                },
                revision: Some(revision(&b)),
                ops: None,
            });
            change.media_bins.push((b.id, Some(b)));
        }
        let planned = Planned::new(writes, CollectionUpdate::Content(change), events);
        self.commit(sh, show, collection, &content, planned, client_id, &by)
            .await?;
        Ok(revisions)
    }

    fn content_of(
        &self,
        sh: &Shared,
        collection: CollectionId,
    ) -> Result<Arc<CollectionContent>, ConfigError> {
        sh.collection_content(collection).ok_or_else(|| {
            err(
                ErrorCode::NotFound,
                format!("collection {collection} is not in the Codex"),
            )
        })
    }

    /// Persists a planned change, swaps it in, logs it and notifies. Holds the lock (the
    /// caller's).
    #[allow(clippy::too_many_arguments)]
    async fn commit(
        &self,
        sh: &Arc<Shared>,
        show: &dyn Show,
        collection: CollectionId,
        content: &Arc<CollectionContent>,
        planned: Planned,
        client_id: Option<String>,
        by: &ChangeBy,
    ) -> Result<(), ApiError> {
        if planned.writes.is_empty() {
            return Ok(());
        }
        // Into the trash first: a document is never deleted without its copy there.
        let entry = planned
            .trashed
            .and_then(|(kind, docs)| trash::entry(collection, kind, docs, by, SystemTime::now()));
        if let Some(e) = &entry {
            let (t, put) = (self.trash_dir(sh), e.clone());
            tokio::task::spawn_blocking(move || t.put(&put))
                .await
                .map_err(join_err)?
                .map_err(|e| {
                    ApiError::new(
                        ErrorCode::Internal,
                        format!("moving to the trash failed: {e:#}"),
                    )
                })?;
            self.trash_index(sh)
                .get_or_insert_with(Vec::new)
                .push(trash::indexed(e));
        }
        let root = content.root.clone();
        let writes = planned.writes;
        let media_touched = writes.iter().any(|w| w.kind == DocumentKind::MediaAsset);
        let deleted: Vec<PathBuf> = writes
            .iter()
            .filter(|w| w.doc.is_none())
            .map(|w| w.path(&root))
            .collect();
        let changes: Vec<NewChange> = planned
            .events
            .iter()
            .map(|e| NewChange {
                kind: e.kind,
                id: e.id,
                change: doc_change(e.change),
                revision: e.revision.clone(),
            })
            .collect();
        // The changes are noted as pending before the documents are written: a stop before
        // they reach the log leaves them noted, and they are logged at the next start.
        let lp = log::path(&sh.codex().layout.collection(collection));
        let (note, nb) = (changes.clone(), by.clone());
        let (stored, noted) = tokio::task::spawn_blocking(move || {
            let noted = log::note_pending(&lp, &note, &nb)
                .inspect_err(|e| tracing::warn!("noting pending changes: {e}"))
                .ok();
            let r = crate::store::apply_batch(&root, &writes);
            if let (Err((_, false)), Some(len)) = (&r, noted) {
                // Nothing of the batch is on disk.
                log::unnote(&lp, len);
            }
            (r, noted.is_some())
        })
        .await
        .map_err(join_err)?;
        let logged = if noted { Vec::new() } else { changes };
        if let Err((e, partial)) = stored {
            if partial {
                // Part of the batch stayed on disk (undoing it failed too): it is logged, and
                // a document it deleted keeps its copy in the trash.
                self.log(sh, collection, logged, by).await;
            }
            if let Some(t) = &entry
                && !(partial && deleted.iter().any(|p| !p.exists()))
            {
                self.drop_from_trash(sh, &[t.id]).await;
            }
            return Err(store_error(e));
        }
        self.log(sh, collection, logged, by).await;
        if sh.codex().active == collection {
            show.collection(planned.update).await?;
        } else {
            let new = Arc::new(applied(content, &planned.update));
            sh.set_codex(Arc::new(sh.codex().with_content(collection, new)));
        }
        let counts_changed = planned
            .events
            .iter()
            .any(|e| e.change != DocumentChange::Changed);
        for ev in planned.events {
            let _ = sh.events().send(Arc::new(ServerMessage::DocumentChanged {
                kind: ev.kind,
                id: ev.id.to_string(),
                change: ev.change,
                revision: ev.revision,
                client_id: client_id.clone(),
                ops: ev.ops,
                collection: Some(collection),
            }));
        }
        if counts_changed {
            codex_changed(sh, &[CodexPart::Collections], Some(collection));
        }
        let mut trash_changed = entry.is_some();
        if let Some(doc) = planned.recreated {
            // Back by its ID (an undo): no longer in the trash.
            let gone: Vec<uuid::Uuid> = self
                .trash_index(sh)
                .iter()
                .flatten()
                .filter(|e| e.collection == collection && e.doc_id == doc)
                .map(|e| e.id)
                .collect();
            if !gone.is_empty() {
                self.drop_from_trash(sh, &gone).await;
                trash_changed = true;
            }
        }
        if trash_changed {
            codex_changed(sh, &[CodexPart::Status, CodexPart::Trash], Some(collection));
        }
        if trash_changed || media_touched {
            sh.library().marks_due();
        }
        Ok(())
    }

    /// A collection's change log changed on disk (an import): read again at the next append.
    pub(crate) fn forget_log(&self, collection: CollectionId) {
        crate::lock(&self.logs).remove(&collection);
    }

    /// Appends to a collection's change log (the writer held) and tells the show boxes: the
    /// changes noted as pending first ([`log::append_with_pending`]), then `changes`. A
    /// failure is logged, not returned (the change is on disk already): the changes are
    /// noted as pending (else kept here) and appended at the next append.
    pub(crate) async fn log(
        &self,
        sh: &Shared,
        collection: CollectionId,
        changes: Vec<NewChange>,
        by: &ChangeBy,
    ) {
        let path = log::path(&sh.codex().layout.collection(collection));
        let held = crate::lock(&self.unlogged)
            .remove(&collection)
            .unwrap_or_default();
        if changes.is_empty() && held.is_empty() && !log::pending_path(&path).exists() {
            return;
        }
        let state = crate::lock(&self.logs).get(&collection).cloned();
        let by = by.clone();
        let r = tokio::task::spawn_blocking(move || {
            // How many of `held` are in the log: only the rest is kept for the next append
            // (E04-1: one already appended must not be appended again).
            let mut done = 0;
            let appended = (|| {
                let mut st = match state {
                    Some(s) => s,
                    None => log::state(&path)?,
                };
                let now = SystemTime::now();
                for (c, b) in &held {
                    log::append(&path, &mut st, std::slice::from_ref(c), b, now)?;
                    done += 1;
                }
                // The pending changes, then `changes` on their own: when the first part fails,
                // `changes` are certainly not in the log yet.
                log::append_with_pending(&path, &mut st, &[], &by, now)?;
                log::append(&path, &mut st, &changes, &by, now)?;
                Ok::<_, std::io::Error>(st)
            })();
            appended.map_err(|e| {
                let mut left: Vec<_> = held.into_iter().skip(done).collect();
                if log::note_pending(&path, &changes, &by).is_err() {
                    left.extend(changes.into_iter().map(|c| (c, by.clone())));
                }
                (e, left)
            })
        })
        .await;
        match r {
            Ok(Ok(st)) => {
                let head = st.head;
                crate::lock(&self.logs).insert(collection, st);
                // The show boxes following it are told.
                sh.codex_link().changed(collection, head);
            }
            Ok(Err((e, left))) => {
                crate::lock(&self.logs).remove(&collection);
                if !left.is_empty() {
                    crate::lock(&self.unlogged)
                        .entry(collection)
                        .or_default()
                        .extend(left);
                }
                tracing::error!(%collection, "appending to the change log failed (appended at the next change): {e}");
            }
            Err(e) => tracing::error!(%collection, "appending to the change log failed: {e}"),
        }
    }

    /// At the start: appends what each collection has noted as pending (a stop between a
    /// write and its log entry; the start's migrations), so the show boxes fetch it.
    pub async fn log_pending(&self, sh: &Arc<Shared>) {
        if !sh.codex_is_host() {
            return;
        }
        let _w = self.lock.lock().await;
        let ids: Vec<CollectionId> = sh
            .codex()
            .collections
            .iter()
            .map(|c| c.collection.id)
            .collect();
        for id in ids {
            self.log(sh, id, Vec::new(), &ChangeBy::default()).await;
        }
    }

    fn trash_dir(&self, sh: &Shared) -> Trash {
        Trash::new(&sh.codex().layout.codex())
    }

    /// The trash's index (read on first use).
    fn trash_index(&self, sh: &Shared) -> std::sync::MutexGuard<'_, Option<Vec<TrashEntry>>> {
        let mut g = crate::lock(&self.trash);
        if g.is_none() {
            *g = Some(self.trash_dir(sh).load());
        }
        g
    }

    /// Takes entries out of the trash for good (index first, then files).
    async fn drop_from_trash(&self, sh: &Shared, ids: &[uuid::Uuid]) {
        let keep: Vec<TrashEntry> = {
            let mut g = self.trash_index(sh);
            let list = g.get_or_insert_with(Vec::new);
            list.retain(|e| !ids.contains(&e.id));
            list.clone()
        };
        let (t, ids) = (self.trash_dir(sh), ids.to_vec());
        match tokio::task::spawn_blocking(move || t.remove(&keep, &ids)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::error!("updating the trash failed: {e:#}"),
            Err(e) => tracing::error!("updating the trash failed: {e}"),
        }
    }

    /// The trash, newest first (entries deleted in the same second: in reverse order of
    /// deletion, which is the index's order).
    pub fn trash_items(&self, sh: &Shared) -> Vec<TrashItem> {
        let codex = sh.codex();
        let g = self.trash_index(sh);
        let mut items: Vec<TrashItem> = g
            .iter()
            .flatten()
            .rev()
            .filter(|e| !codex.is_deleted(e.collection))
            .map(trash::item)
            .collect();
        items.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
        items
    }

    /// Every item trash entry with its documents (reads the trash's files).
    async fn trash_entries(&self, sh: &Shared) -> Vec<TrashEntry> {
        let ids: Vec<uuid::Uuid> = self
            .trash_index(sh)
            .iter()
            .flatten()
            .map(|e| e.id)
            .collect();
        let t = self.trash_dir(sh);
        tokio::task::spawn_blocking(move || {
            ids.into_iter().filter_map(|id| t.get(id).ok()).collect()
        })
        .await
        .unwrap_or_default()
    }

    /// `GET /codex/storage`.
    pub async fn storage(&self, sh: &Arc<Shared>) -> Result<protocol::CodexStorage, ApiError> {
        let entries = self.trash_entries(sh).await;
        let s = sh.clone();
        tokio::task::spawn_blocking(move || super::storage::report(&s, &entries))
            .await
            .map_err(join_err)
    }

    /// Entries in the trash (those of collections in the collection trash aside: they come
    /// back with their collection).
    pub fn trash_count(&self, sh: &Shared) -> u32 {
        let codex = sh.codex();
        self.trash_index(sh)
            .iter()
            .flatten()
            .filter(|e| !codex.is_deleted(e.collection))
            .count() as u32
    }

    /// Item trash entries of one collection.
    fn trash_count_of(&self, sh: &Shared, collection: CollectionId) -> u32 {
        self.trash_index(sh)
            .iter()
            .flatten()
            .filter(|e| e.collection == collection)
            .count() as u32
    }

    /// `POST /codex/trash/{id}/restore`: recreates the entry's documents (a folder with its
    /// subtree, parents first) with their IDs in the collection they were deleted from, through
    /// the usual path (validation, log, events), then takes the entry out of the trash.
    pub async fn restore(
        &self,
        sh: &Arc<Shared>,
        show: &dyn Show,
        id: uuid::Uuid,
        client_id: Option<String>,
        by: ChangeBy,
    ) -> Result<TrashRestored, ConfigError> {
        let _w = self.hold().await?;
        writable(sh)?;
        if !self.trash_index(sh).iter().flatten().any(|e| e.id == id) {
            return Err(err(
                ErrorCode::NotFound,
                format!("trash entry {id} not found"),
            ));
        }
        let t = self.trash_dir(sh);
        let deleted = {
            let codex = sh.codex();
            self.trash_index(sh)
                .iter()
                .flatten()
                .find(|e| e.id == id)
                .is_some_and(|e| codex.is_deleted(e.collection))
        };
        if deleted {
            return Err(err(
                ErrorCode::Conflict,
                "its collection is in the collection trash: restore the collection first",
            ));
        }
        let entry = tokio::task::spawn_blocking(move || t.get(id))
            .await
            .map_err(join_err)?
            .map_err(|e| {
                err(
                    ErrorCode::Internal,
                    format!("reading the trash entry failed: {e:#}"),
                )
            })?;
        let collection = entry.collection;
        let content = self.content_of(sh, collection).map_err(|_| {
            err(
                ErrorCode::Conflict,
                format!("the collection “{}” was deleted from is gone", entry.name),
            )
        })?;
        let docs: Vec<Value> = entry
            .document
            .iter()
            .cloned()
            .chain(entry.subtree.iter().cloned())
            .collect();
        let ids: Vec<String> = docs
            .iter()
            .filter_map(|d| d.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();
        let mut planned = restore_plan(sh, &content, entry.kind, docs).map_err(|e| {
            let mut e = e;
            if e.error.code == ErrorCode::BadRequest {
                e.error = ApiError::new(
                    ErrorCode::Conflict,
                    format!("“{}” can't be restored: {}", entry.name, e.error.message),
                );
            }
            e
        })?;
        planned.recreated = Some(entry.doc_id);
        self.commit(sh, show, collection, &content, planned, client_id, &by)
            .await?;
        // (`recreated` took the entry out of the trash.)
        if self.trash_index(sh).iter().flatten().any(|e| e.id == id) {
            self.drop_from_trash(sh, &[id]).await;
        }
        Ok(TrashRestored {
            collection,
            kind: entry.kind,
            documents: ids,
        })
    }

    /// `DELETE /codex/trash`: empties the trash for good. Returns how many entries went.
    pub async fn purge(&self, sh: &Arc<Shared>) -> TrashPurged {
        let _w = self.lock.lock().await;
        if let Err(e) = writable(sh) {
            tracing::debug!("{}", e.message);
            return TrashPurged { purged: 0 };
        }
        let codex = sh.codex();
        // Entries of collections in the collection trash stay with them.
        let index: Vec<TrashEntry> = self.trash_index(sh).iter().flatten().cloned().collect();
        let ids: Vec<uuid::Uuid> = index
            .iter()
            .filter(|e| !codex.is_deleted(e.collection))
            .map(|e| e.id)
            .collect();
        let t = self.trash_dir(sh);
        let orphans = tokio::task::spawn_blocking(move || t.orphans(&index))
            .await
            .unwrap_or_default();
        let mut all = ids.clone();
        all.extend(orphans.into_iter().filter(|o| !ids.contains(o)));
        self.drop_from_trash(sh, &all).await;
        sh.library().marks_due();
        if !ids.is_empty() {
            codex_changed(sh, &[CodexPart::Status, CodexPart::Trash], None);
        }
        TrashPurged {
            purged: ids.len() as u32,
        }
    }

    /// The periodic upkeep: expired trash entries (and files no entry names) go, change logs
    /// due for compaction are compacted, thumbnails of media the Codex no longer has go
    /// ([`super::storage::clean_thumbs`]); the media library's marks follow
    /// ([`Writer::refresh_marks`]).
    pub async fn tick(&self, sh: &Arc<Shared>, now: SystemTime) {
        let _w = self.lock.lock().await;
        if !sh.codex_is_host() {
            // (A show box: its own Codex is dormant, kept as it is.)
            return;
        }
        let index: Vec<TrashEntry> = self.trash_index(sh).iter().flatten().cloned().collect();
        let mut gone = trash::expired(&index, now);
        let t = self.trash_dir(sh);
        let idx = index.clone();
        let orphans = tokio::task::spawn_blocking(move || t.orphans(&idx))
            .await
            .unwrap_or_default();
        let expired = gone.len();
        gone.extend(orphans);
        if !gone.is_empty() {
            self.drop_from_trash(sh, &gone).await;
            tracing::info!(expired, removed = gone.len(), "Codex: trash pruned");
        }
        if expired > 0 {
            codex_changed(sh, &[CodexPart::Status, CodexPart::Trash], None);
        }
        let codex = sh.codex();
        for c in &codex.collections {
            let id = c.collection.id;
            let path = log::path(&codex.layout.collection(id));
            let state = crate::lock(&self.logs).get(&id).cloned();
            let r = tokio::task::spawn_blocking(move || {
                let mut st = match state {
                    Some(s) => s,
                    None => log::state(&path)?,
                };
                log::compact_if_due(&path, &mut st, now)?;
                Ok::<_, std::io::Error>(st)
            })
            .await;
            match r {
                Ok(Ok(st)) => {
                    crate::lock(&self.logs).insert(id, st);
                }
                Ok(Err(e)) => {
                    tracing::error!(collection = %id, "compacting the change log failed: {e}")
                }
                Err(e) => {
                    tracing::error!(collection = %id, "compacting the change log failed: {e}")
                }
            }
        }
        let s = sh.clone();
        match tokio::task::spawn_blocking(move || super::storage::clean_thumbs(&s)).await {
            Ok(n) if n > 0 => tracing::info!(thumbs = n, "Codex: thumbnails of gone media removed"),
            Ok(_) => {}
            Err(e) => tracing::error!("removing thumbnails failed: {e}"),
        }
        if expired > 0 {
            sh.library().marks_due();
        }
    }

    /// Marks every media library item a document uses as in use (and nothing else; see
    /// [`super::storage`]), holding the writer so no write changes the references meanwhile.
    pub async fn refresh_marks(&self, sh: &Arc<Shared>) {
        if !sh.library().is_open() {
            return;
        }
        let _w = self.lock.lock().await;
        let entries = self.trash_entries(sh).await;
        let s = sh.clone();
        match tokio::task::spawn_blocking(move || super::storage::refresh_marks(&s, &entries)).await
        {
            Ok(Ok(n)) => tracing::debug!(items = n, "Codex: media in use marked in the library"),
            Ok(Err(e)) => tracing::warn!("marking media in use: {e}"),
            Err(e) => tracing::error!("marking media in use: {e}"),
        }
    }

    /// `POST /codex/collections`: a new, empty collection.
    pub async fn create_collection(
        &self,
        sh: &Arc<Shared>,
        body: CreateCollection,
    ) -> Result<CollectionSummary, ConfigError> {
        let _w = self.hold().await?;
        writable(sh)?;
        let codex = sh.codex();
        let last = codex
            .collections
            .iter()
            .map(|c| c.collection.sort_key.as_str())
            .filter(|k| !k.is_empty())
            .max();
        let mut c = Collection {
            name: clean_name(&body.name, "name")?,
            description: clean_description(body.description, "description")?,
            color: body.color,
            purpose: body.purpose.unwrap_or_default(),
            sort_key: match body.sort_key {
                Some(k) => clean_sort_key(k, "sortKey")?,
                None => sort_key_between(last, None),
            },
            created_at: now(),
            ..Collection::default()
        };
        c.schema_version = model::SCHEMA_VERSION;
        let layout = codex.layout.clone();
        let (doc, roots, lib) = (
            c.clone(),
            sh.data().content.media_roots.clone(),
            sh.library().clone(),
        );
        let content = tokio::task::spawn_blocking(move || {
            write_doc(&layout.collection_json(doc.id), &doc)?;
            Ok::<_, anyhow::Error>(CollectionContent::load(
                &layout.collection(doc.id),
                layout.media(),
                lib,
                roots,
            ))
        })
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
        .map_err(|e| err(ErrorCode::Internal, format!("saving failed: {e:#}")))?;
        sh.set_codex(Arc::new(codex.with_collection(LoadedCollection {
            collection: c.clone(),
            too_new: false,
            content: Some(Arc::new(content)),
        })));
        tracing::info!(collection = %c.id, name = %c.name, "Codex: collection created");
        codex_changed(sh, &[CodexPart::Status, CodexPart::Collections], Some(c.id));
        Ok(summary_of(sh, c.id).await?)
    }

    /// `PUT /codex/collections/{id}` (`If-Match`).
    pub async fn update_collection(
        &self,
        sh: &Arc<Shared>,
        id: CollectionId,
        body: UpdateCollection,
        if_match: Option<String>,
    ) -> Result<CollectionSummary, ConfigError> {
        let _w = self.hold().await?;
        writable(sh)?;
        let codex = sh.codex();
        let loaded = existing(&codex, id)?;
        check(sh, &loaded.collection, &if_match, "this collection").await?;
        if loaded.too_new {
            return Err(err(
                ErrorCode::Conflict,
                "this collection was written by a newer server version: it can't be changed here",
            ));
        }
        let cur = &loaded.collection;
        let mut c = cur.clone();
        if let Some(n) = &body.name {
            c.name = clean_name(n, "name")?;
        }
        if let Some(d) = body.description {
            c.description = clean_description(d, "description")?;
        }
        if let Some(col) = body.color {
            c.color = col;
        }
        if let Some(p) = body.purpose {
            c.purpose = p;
        }
        if let Some(k) = body.sort_key {
            c.sort_key = clean_sort_key(k, "sortKey")?;
        }
        if c != *cur {
            c.schema_version = model::SCHEMA_VERSION;
            let (path, doc) = (codex.layout.collection_json(id), c.clone());
            tokio::task::spawn_blocking(move || write_doc(&path, &doc))
                .await
                .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
                .map_err(|e| err(ErrorCode::Internal, format!("saving failed: {e:#}")))?;
            let mut l = loaded.clone();
            l.collection = c;
            // The writer holds every change of the Codex state but a switch, which takes the
            // writer too: the state read above is still current.
            sh.set_codex(Arc::new(sh.codex().with_collection(l)));
            codex_changed(sh, &[CodexPart::Status, CodexPart::Collections], Some(id));
        }
        Ok(summary_of(sh, id).await?)
    }

    /// `DELETE /codex/collections/{id}` (`If-Match`): moves the collection into the
    /// collection trash, intact ([`super::collection_trash`]). Not the collection this box
    /// shows (`409`).
    pub async fn delete_collection(
        &self,
        sh: &Arc<Shared>,
        id: CollectionId,
        if_match: Option<String>,
        by: ChangeBy,
    ) -> Result<(), ConfigError> {
        let _w = self.hold().await?;
        writable(sh)?;
        let codex = sh.codex();
        let loaded = existing(&codex, id)?;
        check(sh, &loaded.collection, &if_match, "this collection").await?;
        if codex.active == id {
            return Err(err(
                ErrorCode::Conflict,
                format!(
                    "this box shows the collection “{}”: show another collection on this box first",
                    loaded.collection.name
                ),
            ));
        }
        let (layout, at) = (codex.layout.clone(), now());
        let (b, a) = (by.clone(), at.clone());
        tokio::task::spawn_blocking(move || super::collection_trash::delete(&layout, id, &b, &a))
            .await
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
            .map_err(|e| err(ErrorCode::Internal, format!("deleting failed: {e:#}")))?;
        let mut next = sh.codex().without_collection(id);
        next.deleted.insert(
            0,
            super::collection_trash::DeletedState {
                collection: loaded.collection.clone(),
                deletion: model::CollectionDeletion {
                    schema_version: model::SCHEMA_VERSION,
                    deleted_at: at,
                    by,
                },
            },
        );
        sh.set_codex(Arc::new(next));
        crate::lock(&self.logs).remove(&id);
        sh.library().marks_due();
        tracing::info!(collection = %id, "Codex: collection moved to the collection trash");
        codex_changed(
            sh,
            &[
                CodexPart::Status,
                CodexPart::Collections,
                CodexPart::Trash,
                CodexPart::CollectionTrash,
            ],
            Some(id),
        );
        Ok(())
    }

    /// `POST /codex/collection-trash/{id}/restore`: the collection comes back as it was.
    pub async fn restore_collection(
        &self,
        sh: &Arc<Shared>,
        id: CollectionId,
    ) -> Result<CollectionSummary, ConfigError> {
        let _w = self.hold().await?;
        writable(sh)?;
        let codex = sh.codex();
        if !codex.is_deleted(id) {
            return Err(err(
                ErrorCode::NotFound,
                format!("collection {id} is not in the collection trash"),
            ));
        }
        let (layout, roots, lib) = (
            codex.layout.clone(),
            sh.data().content.media_roots.clone(),
            sh.library().clone(),
        );
        let loaded = tokio::task::spawn_blocking(move || {
            super::collection_trash::restore(&layout, id)?;
            let mut issues = Vec::new();
            let c =
                super::load_collection(&layout, &layout.collection(id), &lib, &roots, &mut issues);
            c.ok_or_else(|| anyhow::anyhow!("its collection.json can't be read"))
        })
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
        .map_err(|e| err(ErrorCode::Conflict, format!("restoring failed: {e:#}")))?;
        let mut next = sh.codex().with_collection(loaded);
        next.deleted.retain(|d| d.collection.id != id);
        sh.set_codex(Arc::new(next));
        sh.library().marks_due();
        tracing::info!(collection = %id, "Codex: collection restored");
        codex_changed(
            sh,
            &[
                CodexPart::Status,
                CodexPart::Collections,
                CodexPart::Trash,
                CodexPart::CollectionTrash,
            ],
            Some(id),
        );
        Ok(summary_of(sh, id).await?)
    }

    /// `DELETE /codex/collection-trash/{id}` (`If-Match`): deletes a collection of the
    /// collection trash for good, with its documents, change log and its entries in the item
    /// trash.
    pub async fn purge_collection(
        &self,
        sh: &Arc<Shared>,
        id: CollectionId,
        if_match: Option<String>,
    ) -> Result<u32, ConfigError> {
        let _w = self.hold().await?;
        writable(sh)?;
        let codex = sh.codex();
        let Some(d) = codex.deleted(id) else {
            return Err(err(
                ErrorCode::NotFound,
                format!("collection {id} is not in the collection trash"),
            ));
        };
        let want = if_match.as_deref().filter(|w| *w != "*");
        if want.is_some_and(|w| w != revision(&d.collection)) {
            let current = deleted_of(sh, id)
                .await
                .ok()
                .and_then(|s| serde_json::to_value(s).ok());
            return check_revision(
                Some(&d.collection),
                |_| current.clone().unwrap_or_default(),
                &if_match,
                "this collection",
            )
            .map(|_| 0);
        }
        let items: Vec<uuid::Uuid> = self
            .trash_index(sh)
            .iter()
            .flatten()
            .filter(|e| e.collection == id)
            .map(|e| e.id)
            .collect();
        if !items.is_empty() {
            self.drop_from_trash(sh, &items).await;
        }
        let layout = codex.layout.clone();
        tokio::task::spawn_blocking(move || super::collection_trash::purge(&layout, id))
            .await
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
            .map_err(|e| err(ErrorCode::Internal, format!("deleting failed: {e}")))?;
        let mut next = (*sh.codex()).clone();
        next.deleted.retain(|d| d.collection.id != id);
        sh.set_codex(Arc::new(next));
        sh.library().marks_due();
        tracing::info!(collection = %id, items = items.len(), "Codex: collection deleted for good");
        codex_changed(
            sh,
            &[CodexPart::Status, CodexPart::CollectionTrash],
            Some(id),
        );
        Ok(items.len() as u32)
    }

    /// `PUT /codex/assignment`: this box shows `id` from now on. `machine/codex.json` records
    /// it; the show actor swaps the content in (the show keeps running, like after a reload);
    /// every document that differs is announced (`documentChanged`).
    ///
    /// While something is live on this box (a slide, media, a prop, a message), the switch needs
    /// `confirm`: without it the answer is `409 liveShow`, naming what is live.
    pub async fn assign(
        &self,
        sh: &Arc<Shared>,
        show: &dyn Show,
        id: CollectionId,
        confirm: bool,
    ) -> Result<CodexStatus, ConfigError> {
        let _w = self.hold().await?;
        let codex = sh.codex();
        match codex.config.role {
            model::CodexRole::Host => {}
            model::CodexRole::Client => {
                return self.assign_client(sh, show, id, confirm).await;
            }
            _ => writable(sh)?,
        }
        if codex.collection(id).is_none() {
            return Err(ApiError::invalid(
                "collection",
                format!("collection {id} is not in the Codex"),
            )
            .into());
        }
        if codex.active == id {
            return Ok(status(sh));
        }
        let live = sh.live_items();
        if !live.is_empty() && !confirm {
            return Err(live_show(live).into());
        }
        let was = codex.active;
        let mut config = codex.config.clone();
        config.collection = Some(id);
        config.schema_version = model::SCHEMA_VERSION;
        let path = codex.layout.config_json();
        let (p, c) = (path.clone(), config);
        tokio::task::spawn_blocking(move || {
            crate::store::check_writable(&p).map_err(store_error)?;
            write_doc(&p, &c).map_err(|e| {
                ApiError::new(
                    ErrorCode::Internal,
                    format!("saving the assignment failed: {e:#}"),
                )
            })
        })
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))??;
        let before = sh.data();
        if let Err(e) = show.collection(CollectionUpdate::Switch(id)).await {
            // Back to what the box shows.
            let (p, c) = (path, codex.config.clone());
            let _ = tokio::task::spawn_blocking(move || write_doc(&p, &c)).await;
            return Err(e.into());
        }
        let after = sh.data();
        tracing::info!(from = %was, to = %id, "Codex: this box shows another collection");
        let changes = diff(&before.content.hashes, &after.content.hashes);
        for (kind, doc, change) in changes {
            let _ = sh.events().send(Arc::new(ServerMessage::DocumentChanged {
                kind,
                id: doc.to_string(),
                change,
                revision: None,
                client_id: None,
                ops: None,
                collection: Some(if change == DocumentChange::Removed {
                    was
                } else {
                    id
                }),
            }));
        }
        codex_changed(sh, &[CodexPart::Status, CodexPart::Collections], None);
        Ok(status(sh))
    }
}

impl Writer {
    /// `PUT /codex/assignment` on a show box: the site's Codex's collection `id` is copied
    /// here, then shown (as after a reload). Holds the writer (the caller's lock).
    async fn assign_client(
        &self,
        sh: &Arc<Shared>,
        show: &dyn Show,
        id: CollectionId,
        confirm: bool,
    ) -> Result<CodexStatus, ConfigError> {
        if sh.codex().active == id {
            return Ok(status(sh));
        }
        let live = sh.live_items();
        if !live.is_empty() && !confirm {
            return Err(live_show(live).into());
        }
        let s = sh.clone();
        tokio::task::spawn_blocking(move || s.mirror().prepare_assignment(&s, id))
            .await
            .map_err(join_err)??;
        let codex = sh.codex();
        let mut config = codex.config.clone();
        config.collection = Some(id);
        config.schema_version = model::SCHEMA_VERSION;
        let path = codex.layout.config_json();
        tokio::task::spawn_blocking(move || {
            crate::store::check_writable(&path).map_err(store_error)?;
            write_doc(&path, &config).map_err(|e| {
                ApiError::new(
                    ErrorCode::Internal,
                    format!("saving the assignment failed: {e:#}"),
                )
            })
        })
        .await
        .map_err(join_err)??;
        tracing::info!(from = %codex.active, to = %id, "Codex: this show box shows another collection of the site's Codex");
        self.reload_locked(sh, show).await?;
        sh.mirror().poke();
        Ok(status(sh))
    }
}

/// The `409 liveShow` refusal of a collection switch, naming what is live (also in `live`).
fn live_show(live: Vec<String>) -> ApiError {
    let mut e = ApiError::new(
        ErrorCode::LiveShow,
        format!(
            "live on this box: {}. Switching collections takes it off the screens: confirm to go ahead",
            live.join(", ")
        ),
    );
    e.live = Some(Box::new(live));
    e
}

/// What differs between two sets of document fingerprints.
pub fn diff(old: &DocHashes, new: &DocHashes) -> Vec<(DocumentKind, uuid::Uuid, DocumentChange)> {
    let mut out = Vec::new();
    for (key, h) in new {
        match old.get(key) {
            None => out.push((key.0, key.1, DocumentChange::Added)),
            Some(o) if o != h => out.push((key.0, key.1, DocumentChange::Changed)),
            _ => {}
        }
    }
    for key in old.keys() {
        if !new.contains_key(key) {
            out.push((key.0, key.1, DocumentChange::Removed));
        }
    }
    out
}

/// Sends `codexChanged`.
pub fn codex_changed(sh: &Shared, parts: &[CodexPart], collection: Option<CollectionId>) {
    let _ = sh.events().send(Arc::new(ServerMessage::CodexChanged {
        parts: parts.to_vec(),
        collection,
    }));
}

fn existing(codex: &super::CodexState, id: CollectionId) -> Result<LoadedCollection, ConfigError> {
    codex
        .collection(id)
        .cloned()
        .ok_or_else(|| err(ErrorCode::NotFound, format!("collection {id} not found")))
}

/// The `If-Match` check of a collection: `409` with the current summary on a mismatch.
async fn check(
    sh: &Arc<Shared>,
    c: &Collection,
    if_match: &Option<String>,
    what: &str,
) -> Result<(), ConfigError> {
    let want = if_match.as_deref().filter(|w| *w != "*");
    if want.is_none_or(|w| w == revision(c)) {
        return Ok(());
    }
    let current = summary_of(sh, c.id)
        .await
        .ok()
        .and_then(|s| serde_json::to_value(s).ok());
    check_revision(
        Some(c),
        |_| current.clone().unwrap_or_default(),
        if_match,
        what,
    )
}

fn clean_name(name: &str, field: &str) -> Result<String, ConfigError> {
    let t = name.trim();
    if t.is_empty() {
        return Err(ApiError::invalid(field, "a name is needed").into());
    }
    if t.chars().count() > MAX_NAME || t.chars().any(char::is_control) {
        return Err(ApiError::invalid(
            field,
            format!("names are at most {MAX_NAME} characters, without control characters"),
        )
        .into());
    }
    Ok(t.to_string())
}

fn clean_description(d: Option<String>, field: &str) -> Result<Option<String>, ConfigError> {
    let Some(d) = d else { return Ok(None) };
    let t = d.trim();
    if t.chars().count() > MAX_DESCRIPTION {
        return Err(ApiError::invalid(
            field,
            format!("a description is at most {MAX_DESCRIPTION} characters"),
        )
        .into());
    }
    Ok((!t.is_empty()).then(|| t.to_string()))
}

fn clean_sort_key(k: String, field: &str) -> Result<String, ConfigError> {
    if k.len() > 64 || !k.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::invalid(field, "a sort key is up to 64 letters and digits").into());
    }
    Ok(k)
}

/// Bytes of the files below `dir` (recursively).
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                dir_bytes(&p)
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

/// A collection's documents, per kind.
pub fn counts(c: &CollectionContent) -> CollectionCounts {
    let n = |x: usize| x as u32;
    CollectionCounts {
        presentations: n(c.presentations.len()),
        libraries: n(c.libraries.len()),
        playlists: n(c.playlists.len()),
        themes: n(c.themes.len()),
        media_assets: n(c.assets.len()),
        media_bins: n(c.media_bins.len()),
        global_groups: n(c.global_groups.len()),
        timers: n(c.timers.len()),
        messages: n(c.messages.len()),
        props: n(c.props.len()),
        macros: n(c.macros.len()),
        clear_groups: n(c.clear_groups.len()),
        stage_layouts: n(c.stage_layouts.len()),
    }
}

/// `GET /api/v1/codex`.
pub fn status(sh: &Shared) -> CodexStatus {
    let codex = sh.codex();
    let documents = codex
        .collections
        .iter()
        .filter_map(|c| sh.collection_content(c.collection.id))
        .map(|c| counts(&c).total())
        .sum();
    CodexStatus {
        id: codex.info.id,
        role: codex.config.role,
        collection: codex.active,
        collection_name: codex
            .collection(codex.active)
            .map(|c| c.collection.name.clone())
            .unwrap_or_default(),
        collections: codex.collections.len() as u32,
        documents,
        trash: sh.codex_writer().trash_count(sh),
        deleted_collections: codex.deleted.len() as u32,
        media_library: protocol::MediaLibraryStatus {
            available: sh.library().is_open(),
            problem: sh.library().problem(),
            served: sh.library().is_open() && sh.codex_link().serving(),
        },
        sync: sh.mirror().status(sh),
        dormant_collection: codex
            .config
            .own_collection
            .filter(|_| codex.config.role == model::CodexRole::Client),
        boxes: boxes(sh),
    }
}

/// The show boxes following this Codex's collections (G14): empty unless this box hosts it.
pub fn boxes(sh: &Shared) -> Vec<protocol::CodexBoxSync> {
    if !sh.codex_is_host() {
        return Vec::new();
    }
    sh.codex_link()
        .mirror_boxes()
        .into_iter()
        .map(|(fingerprint, b)| {
            let head = sh.codex_writer().head(sh, b.collection);
            protocol::CodexBoxSync {
                fingerprint,
                name: b.name,
                collection: b.collection,
                seq: b.seq,
                behind: b.seq.map(|s| head.saturating_sub(s)),
                last_seen: crate::auth::rfc3339(b.seen, true),
                online: b.at.elapsed() < super::link::MIRROR_TTL,
            }
        })
        .collect()
}

/// The Vespers showing `id` and how many are up to date (G14): the online show boxes
/// following it, and this box when it hosts the Codex and shows it.
pub fn collection_boxes(
    sh: &Shared,
    id: CollectionId,
    all: &[protocol::CodexBoxSync],
) -> protocol::CollectionBoxes {
    let mut out = protocol::CollectionBoxes::default();
    if !sh.codex_is_host() {
        return out;
    }
    if sh.codex().active == id {
        out.showing += 1;
        out.synced += 1;
    }
    for b in all.iter().filter(|b| b.online && b.collection == id) {
        out.showing += 1;
        if b.behind == Some(0) {
            out.synced += 1;
        }
    }
    out
}

/// The Codex's media files a collection's assets use (library items, managed files of the
/// old store), once each.
fn media_files(c: &CollectionContent) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    c.assets
        .values()
        .filter_map(|a| match &a.location {
            AssetLocation::Managed { sha256, extension } => {
                Some(c.media_dir.join(format!("{sha256}.{extension}")))
            }
            AssetLocation::Library { id, .. } => c.library.resolve(id).ok(),
            _ => None,
        })
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

/// A collection's summary (reads sizes from disk: call off the async threads, or through
/// [`summary_of`]).
pub fn summary(sh: &Shared, id: CollectionId) -> Option<CollectionSummary> {
    let codex = sh.codex();
    let loaded = codex.collection(id)?;
    let content = sh.collection_content(id)?;
    let media_bytes = media_files(&content)
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    Some(CollectionSummary {
        revision: revision(&loaded.collection),
        collection: loaded.collection.clone(),
        active: codex.active == id,
        read_only: loaded.too_new,
        counts: counts(&content),
        document_bytes: dir_bytes(&codex.layout.collection(id).join("portable")),
        media_bytes,
        boxes: collection_boxes(sh, id, &boxes(sh)),
    })
}

/// [`summary`] on the blocking pool; `404` for a collection that isn't there.
pub async fn summary_of(sh: &Arc<Shared>, id: CollectionId) -> Result<CollectionSummary, ApiError> {
    let s = sh.clone();
    tokio::task::spawn_blocking(move || summary(&s, id))
        .await
        .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, format!("collection {id} not found")))
}

/// Every collection's summary, in the Codex's order.
pub async fn summaries(sh: &Arc<Shared>) -> Result<Vec<CollectionSummary>, ApiError> {
    let s = sh.clone();
    tokio::task::spawn_blocking(move || {
        let codex = s.codex();
        codex
            .collections
            .iter()
            .filter_map(|c| summary(&s, c.collection.id))
            .collect()
    })
    .await
    .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))
}

/// A deleted collection's listing (loads its documents from disk: call off the async threads).
pub fn deleted_summary(sh: &Shared, id: CollectionId) -> Option<DeletedCollection> {
    let codex = sh.codex();
    let d = codex.deleted(id)?;
    let dir = codex.layout.deleted_collection(id);
    let content = CollectionContent::load(
        &dir,
        codex.layout.media(),
        sh.library().clone(),
        sh.data().content.media_roots.clone(),
    );
    let media_bytes = media_files(&content)
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    Some(DeletedCollection {
        revision: revision(&d.collection),
        collection: d.collection.clone(),
        counts: counts(&content),
        document_bytes: dir_bytes(&dir.join("portable")),
        media_bytes,
        trash_items: sh.codex_writer().trash_count_of(sh, id),
        deleted_at: d.deletion.deleted_at.clone(),
        by: d.deletion.by.clone(),
    })
}

/// [`deleted_summary`] on the blocking pool; `404` when it isn't in the collection trash.
pub async fn deleted_of(sh: &Arc<Shared>, id: CollectionId) -> Result<DeletedCollection, ApiError> {
    let s = sh.clone();
    tokio::task::spawn_blocking(move || deleted_summary(&s, id))
        .await
        .map_err(join_err)?
        .ok_or_else(|| {
            ApiError::new(
                ErrorCode::NotFound,
                format!("collection {id} is not in the collection trash"),
            )
        })
}

/// `GET /codex/collection-trash`: every deleted collection, newest first.
pub async fn deleted_collections(sh: &Arc<Shared>) -> Result<Vec<DeletedCollection>, ApiError> {
    let s = sh.clone();
    tokio::task::spawn_blocking(move || {
        let codex = s.codex();
        codex
            .deleted
            .iter()
            .filter_map(|d| deleted_summary(&s, d.collection.id))
            .collect()
    })
    .await
    .map_err(join_err)
}

/// `GET /codex/collections/{id}/changes?since=`: the collection's changes after `since`.
pub async fn changes(
    sh: &Arc<Shared>,
    collection: CollectionId,
    since: u64,
    limit: usize,
) -> Result<CollectionChanges, ApiError> {
    let codex = sh.codex();
    if codex.collection(collection).is_none() {
        return Err(ApiError::new(
            ErrorCode::NotFound,
            format!("collection {collection} not found"),
        ));
    }
    let path = log::path(&codex.layout.collection(collection));
    tokio::task::spawn_blocking(move || log::since(&path, since, limit))
        .await
        .map_err(join_err)?
        .map_err(|e| {
            ApiError::new(
                ErrorCode::Internal,
                format!("reading the change log failed: {e}"),
            )
        })
}

/// A stored document read back as `T` (`409` when it can't be).
pub fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ConfigError> {
    serde_json::from_value(v).map_err(|e| {
        err(
            ErrorCode::Conflict,
            format!("the trash entry can't be read: {e}"),
        )
    })
}

/// Re-adds one show document as it was stored (keeping every field, its import data too).
fn restore_show_doc<T>(
    list: &[T],
    v: Value,
    make: impl FnOnce(Vec<T>) -> CollectionUpdate,
) -> Result<Planned, ConfigError>
where
    T: Document + Clone + serde::Serialize + serde::de::DeserializeOwned,
{
    let doc: T = decode(v)?;
    if list.iter().any(|d| d.id() == doc.id()) {
        return Err(err(
            ErrorCode::Conflict,
            format!("{} already exists", doc.id()),
        ));
    }
    let write = DocWrite::put(&doc).map_err(|e| err(ErrorCode::Internal, format!("{e:#}")))?;
    let event = ChangeEvent {
        kind: T::KIND,
        id: doc.id(),
        change: DocumentChange::Added,
        revision: Some(revision(&doc)),
        ops: None,
    };
    let mut new = list.to_vec();
    new.push(doc);
    Ok(Planned::new(vec![write], make(new), vec![event]))
}

/// The plan of a restore: the documents recreated in order (parents first), each validated
/// against the content as the ones before it left it.
fn restore_plan(
    sh: &Shared,
    content: &CollectionContent,
    kind: DocumentKind,
    docs: Vec<Value>,
) -> Result<Planned, ConfigError> {
    let one = || {
        docs.first()
            .cloned()
            .ok_or_else(|| err(ErrorCode::Conflict, "the trash entry is empty"))
    };
    let show = |timers, props, messages, stage_layouts| CollectionUpdate::ShowDocs {
        timers,
        props,
        messages,
        stage_layouts,
    };
    match kind {
        DocumentKind::Timer => {
            restore_show_doc(&content.timers, one()?, |l| show(Some(l), None, None, None))
        }
        DocumentKind::Prop => {
            restore_show_doc(&content.props, one()?, |l| show(None, Some(l), None, None))
        }
        DocumentKind::Message => restore_show_doc(&content.messages, one()?, |l| {
            show(None, None, Some(l), None)
        }),
        DocumentKind::StageLayout => restore_show_doc(&content.stage_layouts, one()?, |l| {
            show(None, None, None, Some(l))
        }),
        DocumentKind::Library
        | DocumentKind::Presentation
        | DocumentKind::PlaylistNode
        | DocumentKind::Theme
        | DocumentKind::MediaBin
        | DocumentKind::GlobalGroup => {
            let mut cur = content.clone();
            let mut change = ContentChange::default();
            let (mut writes, mut events) = (Vec::new(), Vec::new());
            for v in docs {
                let plan = sh.recreate(&cur, kind, v)?;
                cur = cur.with_change(&plan.change);
                change.presentations.extend(plan.change.presentations);
                change.libraries.extend(plan.change.libraries);
                change.playlists.extend(plan.change.playlists);
                change.themes.extend(plan.change.themes);
                change.media_bins.extend(plan.change.media_bins);
                change.global_groups.extend(plan.change.global_groups);
                writes.extend(plan.writes);
                events.extend(plan.events);
            }
            Ok(Planned::new(
                writes,
                CollectionUpdate::Content(change),
                events,
            ))
        }
        other => Err(err(
            ErrorCode::Conflict,
            format!("{other:?} documents can't be restored"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E04-3: a forwarded write that reaches the writer after its deadline is refused, even
    /// with the writer free; before the deadline it gets it.
    #[tokio::test]
    async fn a_write_past_its_deadline_is_refused_even_with_the_writer_free() {
        let w = Writer::new();
        let late = tokio::time::Instant::now() - std::time::Duration::from_millis(1);
        let r = WRITER_DEADLINE.scope(late, w.hold()).await;
        assert!(r.is_err(), "refused after the deadline");
        let soon = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        assert!(WRITER_DEADLINE.scope(soon, w.hold()).await.is_ok());
        assert!(
            w.hold().await.is_ok(),
            "no deadline: waits as long as it takes"
        );
    }
}
