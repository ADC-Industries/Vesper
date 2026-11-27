//! Importing a Codex export: the file is uploaded (`<data>/codex-import/<id>.upload`, kept an
//! hour), previewed with its passphrase, then applied with explicit choices.
//!
//! **Defaults (conservative):** collections the Codex doesn't have are added; a collection it
//! has is kept unless the admin asks to replace it; settings backup sets it lacks (by ID and by
//! product and machine) are added, its own are kept; the site is never touched unless asked.
//!
//! - **Replacing a collection**: not the one this box shows (`409`: assign another first).
//!   Its current copy goes to the collection trash first, as a collection of its own (a new ID,
//!   named "… (before import)"): restoring it from there is the undo. The collection keeps its
//!   ID and its change log, which gets an entry for every document the import added, changed
//!   or removed, so the show boxes following it bring their copies up to date as after any
//!   edit (`documentChanged`, `codexChanged`).
//! - **New collections** come with their own change log as exported. The Codex keeps its own
//!   ID: show boxes that followed the exported Codex compare everything once (a resync).
//! - **Collections in this Codex's collection trash** aren't imported (restore or delete them
//!   for good first).
//! - **The site** (`site: true`, the site's server only): the Site Manager's folder from the
//!   file is staged with IDN's `backup::stage`; the Site Manager puts it in place at the
//!   server's next start (IDN's `apply_staged`, which refuses it when machines joined since or
//!   a week went by). A restart is needed.
//! - **Media** aren't in the file: once the media folder is restored (or its files are added
//!   to the library, or it is bound as a media root), the relink search finds them by
//!   SHA-256.

use super::{Exports, Opened, Paths, api, is_document, join_err, newest_change};
use crate::Shared;
use crate::backups::store::{Store, sha256, valid_sha};
use crate::collection_trash::{DELETION, DeletedState};
use crate::log::NewChange;
use crate::writer::{codex_changed, diff};
use crate::{LoadedCollection, write_doc};
use futures_util::StreamExt as _;
use model::{
    BackupSet, ChangeBy, Collection, CollectionDeletion, CollectionId, DocChange, SettingsSnapshot,
};
use protocol::{
    ApiError, CodexPart, DocumentChange, ErrorCode, ExportedCollection, ImportBackupSet,
    ImportCollection, ImportComparison, ImportPreview, ImportRequest, ImportResult, ImportUpload,
    MEDIA_NOTE, ServerMessage,
};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::io::AsyncWriteExt as _;

/// Uploaded imports waiting at most.
const MAX_PENDING: usize = 4;
/// Largest upload: IDN's largest backup plus room for its index.
pub const MAX_UPLOAD: u64 = idn_open_protocol::backup::MAX_SIZE + (64 << 20);

/// An uploaded import waiting for its preview and apply.
pub(crate) struct Pending {
    path: PathBuf,
    bytes: u64,
    expires: SystemTime,
}

fn io(e: impl std::fmt::Display) -> ApiError {
    api(ErrorCode::Internal, format!("importing: {e}"))
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type()?.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// A collection of the file, as the preview and the apply see it.
struct FileCollection {
    exported: ExportedCollection,
    too_new: bool,
    documents: BTreeMap<String, String>,
}

fn file_collection(
    opened: &Opened,
    id: CollectionId,
    files: &[(String, Vec<u8>)],
) -> Option<FileCollection> {
    let (collection, too_new) = match super::parse_collection(files) {
        Ok(Some(c)) if c.id == id => (Some(c), false),
        Ok(_) => return None,
        Err(()) => (None, true),
    };
    let log = files
        .iter()
        .find(|(r, _)| r == "changes.jsonl")
        .map(|(_, b)| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    let listed = opened.manifest.collections.iter().find(|c| c.id == id);
    let documents: BTreeMap<String, String> = files
        .iter()
        .filter(|(r, _)| is_document(r))
        .map(|(r, b)| (r.clone(), sha256(b)))
        .collect();
    let name = collection
        .as_ref()
        .map(|c| c.name.clone())
        .or_else(|| listed.map(|c| c.name.clone()))
        .unwrap_or_default();
    Some(FileCollection {
        exported: ExportedCollection {
            id,
            name,
            documents: documents.len() as u32,
            seq: listed.map(|c| c.seq).unwrap_or(0),
            last_change_at: newest_change(&log).or_else(|| {
                collection
                    .as_ref()
                    .map(|c| c.created_at.clone())
                    .filter(|s| !s.is_empty())
            }),
        },
        too_new,
        documents,
    })
}

/// How each collection of the file stands against this Codex.
fn compare(sh: &Shared, opened: &Opened) -> Vec<(ImportCollection, FileCollection)> {
    let codex = sh.codex();
    opened
        .collections
        .iter()
        .filter_map(|(id, files)| {
            let fc = file_collection(opened, *id, files)?;
            let here = super::collection_here(sh, *id);
            let comparison = match &here {
                Some(h) => {
                    if super::documents_on_disk(&codex.layout.collection(*id)) == fc.documents {
                        ImportComparison::Same
                    } else if fc.exported.last_change_at > h.last_change_at {
                        ImportComparison::Newer
                    } else {
                        ImportComparison::Older
                    }
                }
                None if codex.is_deleted(*id) => ImportComparison::InTrash,
                None => ImportComparison::New,
            };
            Some((
                ImportCollection {
                    collection: fc.exported.clone(),
                    comparison,
                    here,
                    active: codex.active == *id,
                    too_new: fc.too_new,
                },
                fc,
            ))
        })
        .collect()
}

/// The file's backup sets and their snapshots.
fn file_sets(opened: &Opened) -> Vec<(BackupSet, Vec<SettingsSnapshot>)> {
    let mut sets: Vec<(BackupSet, Vec<SettingsSnapshot>)> = opened
        .backups
        .iter()
        .filter_map(|(rel, bytes)| {
            let name = rel.strip_prefix("sets/")?.strip_suffix(".json")?;
            let set: BackupSet = serde_json::from_slice(bytes).ok()?;
            (set.id.to_string() == name).then_some((set, Vec::new()))
        })
        .collect();
    for (set, snaps) in &mut sets {
        let prefix = format!("snapshots/{}/", set.id);
        snaps.extend(
            opened
                .backups
                .iter()
                .filter(|(rel, _)| rel.starts_with(&prefix) && rel.ends_with(".json"))
                .filter_map(|(_, b)| serde_json::from_slice::<SettingsSnapshot>(b).ok())
                .filter(|s| s.set == set.id),
        );
    }
    sets
}

/// This Codex has a set for the same product and machine (or the same ID).
fn set_present(here: &[BackupSet], set: &BackupSet) -> bool {
    here.iter()
        .any(|h| h.id == set.id || (h.product == set.product && h.machine == set.machine))
}

impl Exports {
    /// `POST /codex/export/imports`: stores the uploaded file (streamed to disk, at most
    /// [`MAX_UPLOAD`]).
    pub async fn upload(
        &self,
        sh: &Arc<Shared>,
        body: axum::body::Body,
    ) -> Result<ImportUpload, ApiError> {
        super::host(sh)?;
        self.drop_expired_imports(sh).await;
        if crate::lock(&self.state).imports.len() >= MAX_PENDING {
            return Err(api(
                ErrorCode::RateLimited,
                format!("{MAX_PENDING} imports wait already: apply or delete one first"),
            ));
        }
        let dir = Paths::new(&sh.data().root).imports();
        let id = uuid::Uuid::new_v4().to_string();
        let path = dir.join(format!("{id}.upload"));
        tokio::fs::create_dir_all(&dir).await.map_err(io)?;
        let mut f = tokio::fs::File::create(&path).await.map_err(io)?;
        let mut bytes = 0u64;
        let mut stream = body.into_data_stream();
        let r: Result<(), ApiError> = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| {
                    api(
                        ErrorCode::BadRequest,
                        format!("the upload was cut short: {e}"),
                    )
                })?;
                bytes += chunk.len() as u64;
                if bytes > MAX_UPLOAD {
                    return Err(api(
                        ErrorCode::BadRequest,
                        format!(
                            "the file is larger than a Codex export may be ({} MB)",
                            MAX_UPLOAD >> 20
                        ),
                    ));
                }
                f.write_all(&chunk).await.map_err(io)?;
            }
            if bytes == 0 {
                return Err(ApiError::invalid("file", "the request has no file"));
            }
            f.flush().await.map_err(io)?;
            Ok(())
        }
        .await;
        drop(f);
        if let Err(e) = r {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(e);
        }
        let expires = SystemTime::now() + self.opts.import_expiry;
        crate::lock(&self.state).imports.insert(
            id.clone(),
            Pending {
                path,
                bytes,
                expires,
            },
        );
        Ok(ImportUpload {
            id,
            bytes,
            expires_at: crate::auth::rfc3339(expires, true),
        })
    }

    /// `DELETE /codex/export/imports/{id}`.
    pub async fn drop_import(&self, sh: &Arc<Shared>, id: &str) -> Result<(), ApiError> {
        super::host(sh)?;
        let p = crate::lock(&self.state)
            .imports
            .remove(id)
            .ok_or_else(|| not_found(id))?;
        let _ = tokio::fs::remove_file(&p.path).await;
        Ok(())
    }

    /// Uploads that waited too long go (and files no upload of this run names).
    pub(crate) async fn drop_expired_imports(&self, sh: &Arc<Shared>) {
        let now = SystemTime::now();
        let (gone, keep): (Vec<PathBuf>, HashSet<PathBuf>) = {
            let mut st = crate::lock(&self.state);
            let gone = st
                .imports
                .iter()
                .filter(|(_, p)| p.expires <= now)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            let gone = gone
                .into_iter()
                .filter_map(|id| st.imports.remove(&id).map(|p| p.path))
                .collect();
            (gone, st.imports.values().map(|p| p.path.clone()).collect())
        };
        let dir = Paths::new(&sh.data().root).imports();
        let _ = tokio::task::spawn_blocking(move || {
            for p in gone {
                let _ = std::fs::remove_file(p);
            }
            // Uploads of an earlier run of the server, and staging an apply cut short left.
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if keep.contains(&p) {
                        continue;
                    }
                    let old = e
                        .metadata()
                        .and_then(|m| m.modified())
                        .is_ok_and(|t| t.elapsed().is_ok_and(|d| d.as_secs() > 3600));
                    let upload = p.extension().is_some_and(|x| x == "upload");
                    if upload && old {
                        let _ = std::fs::remove_file(&p);
                    } else if p.extension().is_some_and(|x| x == "d") && old {
                        let _ = std::fs::remove_dir_all(&p);
                    }
                }
            }
        })
        .await;
    }

    async fn read_upload(&self, id: &str) -> Result<Vec<u8>, ApiError> {
        let path = crate::lock(&self.state)
            .imports
            .get(id)
            .map(|p| (p.path.clone(), p.bytes))
            .ok_or_else(|| not_found(id))?
            .0;
        tokio::fs::read(&path).await.map_err(io)
    }

    async fn open_upload(&self, id: &str, passphrase: String) -> Result<Opened, ApiError> {
        let bytes = self.read_upload(id).await?;
        tokio::task::spawn_blocking(move || super::open(&bytes, &passphrase))
            .await
            .map_err(join_err)?
    }

    /// `POST /codex/export/imports/{id}/preview`.
    pub async fn preview(
        &self,
        sh: &Arc<Shared>,
        id: &str,
        passphrase: String,
    ) -> Result<ImportPreview, ApiError> {
        super::host(sh)?;
        let opened = self.open_upload(id, passphrase).await?;
        let (sh2, id2) = (sh.clone(), id.to_string());
        tokio::task::spawn_blocking(move || {
            let here = Store::new(&sh2.codex().layout.codex()).sets();
            ImportPreview {
                id: id2,
                sealed_at: opened.sealed.created.clone(),
                collections: compare(&sh2, &opened).into_iter().map(|(c, _)| c).collect(),
                backup_sets: file_sets(&opened)
                    .into_iter()
                    .map(|(s, snaps)| ImportBackupSet {
                        present: set_present(&here, &s),
                        id: s.id,
                        product: s.product,
                        machine: s.machine,
                        name: s.name,
                        snapshots: snaps.len() as u32,
                    })
                    .collect(),
                site: opened.site.is_some(),
                media_included: false,
                note: MEDIA_NOTE.into(),
                manifest: opened.manifest,
            }
        })
        .await
        .map_err(join_err)
    }

    /// `POST /codex/export/imports/{id}/apply`: see the module docs. The upload is used up.
    pub async fn apply(
        &self,
        sh: &Arc<Shared>,
        id: &str,
        req: ImportRequest,
        by: ChangeBy,
    ) -> Result<ImportResult, ApiError> {
        super::host(sh)?;
        crate::writer::writable(sh)?;
        let _r = self.running.lock().await;
        let opened = Arc::new(self.open_upload(id, req.passphrase.clone()).await?);
        let (sh2, o2) = (sh.clone(), opened.clone());
        let compared = tokio::task::spawn_blocking(move || compare(&sh2, &o2))
            .await
            .map_err(join_err)?;

        // Check the choices before anything changes.
        for r in &req.replace {
            let Some((c, _)) = compared.iter().find(|(c, _)| c.collection.id == *r) else {
                return Err(ApiError::invalid(
                    "replace",
                    format!("collection {r} isn't in the file"),
                ));
            };
            if c.comparison == ImportComparison::InTrash {
                return Err(api(
                    ErrorCode::Conflict,
                    format!(
                        "collection {r} is in this Codex's collection trash: restore it or delete it for good first"
                    ),
                ));
            }
            if c.here.is_none() {
                return Err(ApiError::invalid(
                    "replace",
                    format!("collection {r} isn't in this Codex: it is added, not replaced"),
                ));
            }
            if c.active {
                return Err(api(
                    ErrorCode::Conflict,
                    format!(
                        "this box shows the collection “{}”: show another collection on this box first, then replace it",
                        c.collection.name
                    ),
                ));
            }
            if c.too_new {
                return Err(api(
                    ErrorCode::Conflict,
                    format!(
                        "collection {r} was written by a newer server: it can't be imported here"
                    ),
                ));
            }
            if sh.codex().collection(*r).is_some_and(|l| l.too_new) {
                return Err(api(
                    ErrorCode::Conflict,
                    format!(
                        "collection {r} here was written by a newer server: it can't be replaced"
                    ),
                ));
            }
        }
        let site_files = if req.site {
            let Some(inner) = opened.site.clone() else {
                return Err(ApiError::invalid(
                    "site",
                    "the file holds no site (it was made on a box that isn't the site's server)",
                ));
            };
            if sh.site().handle().is_none() {
                return Err(api(
                    ErrorCode::Conflict,
                    "the site is restored on the site's server: this box doesn't run its site's Site Manager now",
                ));
            }
            let pass = req.passphrase.clone();
            let files =
                tokio::task::spawn_blocking(move || idn_open_protocol::backup::open(&inner, &pass))
                    .await
                    .map_err(join_err)?
                    .map_err(|e| {
                        api(
                            ErrorCode::BadRequest,
                            format!("the file's site part can't be opened: {e}"),
                        )
                    })?;
            if files.0.kind != super::SITE_KIND {
                return Err(api(
                    ErrorCode::BadRequest,
                    format!(
                        "the file's site part is an IDN backup of kind “{}”",
                        files.0.kind
                    ),
                ));
            }
            Some(files.1)
        } else {
            None
        };

        let mut result = ImportResult {
            media_included: false,
            note: MEDIA_NOTE.into(),
            ..ImportResult::default()
        };
        let mut add = Vec::new();
        let mut replace = Vec::new();
        for (c, fc) in compared {
            let cid = c.collection.id;
            if req.replace.contains(&cid) {
                replace.push(fc);
            } else if c.comparison == ImportComparison::New
                && !c.too_new
                && !req.skip.contains(&cid)
            {
                add.push(fc);
            } else {
                result.skipped.push(cid);
            }
        }

        // The collections, staged next to the Codex (same filesystem), then moved in by the
        // writer.
        let paths = Paths::new(&sh.data().root);
        let stage = paths.imports().join(format!("{}.d", uuid::Uuid::new_v4()));
        {
            let stage = stage.clone();
            let files: Vec<(CollectionId, super::Files)> = add
                .iter()
                .chain(replace.iter())
                .filter_map(|fc| {
                    let id = fc.exported.id;
                    Some((id, opened.collections.get(&id)?.clone()))
                })
                .collect();
            tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                for (id, files) in files {
                    let dir = stage.join(id.to_string());
                    // Only what a collection's folder holds: a file can't add anything else
                    // (a `deleted.json`, say).
                    for (rel, bytes) in files {
                        if !(rel == "collection.json"
                            || rel == "changes.jsonl"
                            || is_document(&rel))
                        {
                            continue;
                        }
                        let p = dir.join(&rel);
                        std::fs::create_dir_all(p.parent().expect("a folder"))?;
                        std::fs::write(p, bytes)?;
                    }
                }
                Ok(())
            })
            .await
            .map_err(join_err)?
            .map_err(io)?;
        }
        let r = self
            .move_in(sh, &stage, &add, &replace, &by, &mut result)
            .await;
        let s = stage.clone();
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(s)).await;
        r?;

        if req.backups {
            let (sets, snaps) = import_backups(sh, &opened).await?;
            result.backup_sets = sets;
            result.snapshots = snaps;
        }
        if let Some(files) = site_files {
            let dir = sh.site().idn_dir().to_path_buf();
            tokio::task::spawn_blocking(move || idn_open_protocol::backup::stage(&files, &dir))
                .await
                .map_err(join_err)?
                .map_err(|e| {
                    api(
                        ErrorCode::Internal,
                        format!("staging the site's restore: {e}"),
                    )
                })?;
            result.site_staged = true;
            result.restart_needed = true;
            tracing::warn!(
                "the site's Site Manager folder from a Codex export is staged: it takes the site's place at the server's next start"
            );
        }
        if let Some(p) = crate::lock(&self.state).imports.remove(id) {
            let _ = std::fs::remove_file(p.path);
        }
        tracing::info!(
            added = result.added.len(),
            replaced = result.replaced.len(),
            skipped = result.skipped.len(),
            backup_sets = result.backup_sets,
            site = result.site_staged,
            "Codex export imported"
        );
        Ok(result)
    }

    /// The writer's part: new collections moved in, replaced ones swapped (their current copy
    /// into the collection trash first), the change logs and events.
    async fn move_in(
        &self,
        sh: &Arc<Shared>,
        stage: &Path,
        add: &[FileCollection],
        replace: &[FileCollection],
        by: &ChangeBy,
        result: &mut ImportResult,
    ) -> Result<(), ApiError> {
        if add.is_empty() && replace.is_empty() {
            return Ok(());
        }
        let _w = sh.codex_writer().exclusive().await;
        crate::writer::writable(sh)?;
        let codex = sh.codex();
        let layout = codex.layout.clone();
        let roots = sh.data().content.media_roots.clone();
        let lib = sh.library().clone();
        let at = super::now();
        let before: BTreeMap<CollectionId, crate::data::DocHashes> = replace
            .iter()
            .filter_map(|fc| {
                let id = fc.exported.id;
                Some((id, sh.collection_content(id)?.hashes.clone()))
            })
            .collect();
        let previous: BTreeMap<CollectionId, Collection> = replace
            .iter()
            .filter_map(|fc| {
                let id = fc.exported.id;
                Some((id, codex.collection(id)?.collection.clone()))
            })
            .collect();
        let add_ids: Vec<CollectionId> = add.iter().map(|f| f.exported.id).collect();
        let replace_ids: Vec<CollectionId> = replace.iter().map(|f| f.exported.id).collect();
        let (stage, b, a) = (stage.to_path_buf(), by.clone(), at.clone());
        type Moved = (Vec<LoadedCollection>, Vec<DeletedState>, Vec<String>);
        let moved = tokio::task::spawn_blocking(move || -> anyhow::Result<Moved> {
            let mut loaded = Vec::new();
            let mut deleted = Vec::new();
            let mut issues = Vec::new();
            for id in &add_ids {
                let to = layout.collection(*id);
                anyhow::ensure!(!to.exists(), "collection {id} appeared meanwhile");
                std::fs::create_dir_all(layout.collections())?;
                std::fs::rename(stage.join(id.to_string()), &to)?;
            }
            for id in &replace_ids {
                let cur = layout.collection(*id);
                let prev = previous
                    .get(id)
                    .ok_or_else(|| anyhow::anyhow!("collection {id} is gone"))?;
                // 1. The current copy into the collection trash, as a collection of its own.
                let copy = Collection {
                    id: CollectionId::new(),
                    name: format!("{} (before import)", prev.name),
                    ..prev.clone()
                };
                let partial = layout
                    .collection_trash()
                    .join(format!("{}.partial", copy.id));
                copy_dir(&cur, &partial)?;
                write_doc(&partial.join("collection.json"), &copy)?;
                let deletion = CollectionDeletion {
                    schema_version: model::SCHEMA_VERSION,
                    deleted_at: a.clone(),
                    by: b.clone(),
                };
                crate::store::write_atomic(
                    &partial.join(DELETION),
                    &serde_json::to_vec_pretty(&deletion)?,
                )?;
                std::fs::rename(&partial, layout.deleted_collection(copy.id))?;
                deleted.push(DeletedState {
                    collection: copy,
                    deletion,
                });
                // 2. The collection's change log goes on in the new copy.
                let staged = stage.join(id.to_string());
                let log = crate::log::path(&staged);
                match std::fs::copy(crate::log::path(&cur), &log) {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        let _ = std::fs::remove_file(&log);
                    }
                    Err(e) => return Err(e.into()),
                }
                // 3. The swap.
                let old = stage.join(format!("old-{id}"));
                std::fs::rename(&cur, &old)?;
                if let Err(e) = std::fs::rename(&staged, &cur) {
                    let _ = std::fs::rename(&old, &cur);
                    return Err(e.into());
                }
                let _ = std::fs::remove_dir_all(&old);
            }
            for id in add_ids.iter().chain(replace_ids.iter()) {
                let mut found = Vec::new();
                match crate::load_collection(
                    &layout,
                    &layout.collection(*id),
                    &lib,
                    &roots,
                    &mut found,
                ) {
                    Some(c) => loaded.push(c),
                    None => issues.push(format!("collection {id} can't be read after the import")),
                }
                issues.extend(
                    found
                        .into_iter()
                        .map(|i| format!("{}: {}", i.file, i.error)),
                );
            }
            Ok((loaded, deleted, issues))
        })
        .await
        .map_err(join_err)?;
        let (loaded, deleted, issues) = match moved {
            Ok(m) => m,
            Err(e) => {
                // Whatever moved in is loaded at the next reload; say what happened.
                return Err(api(ErrorCode::Internal, format!("importing failed: {e:#}")));
            }
        };
        for i in &issues {
            tracing::warn!("Codex import: {i}");
        }
        let mut next = (*sh.codex()).clone();
        for c in &loaded {
            next = next.with_collection(c.clone());
        }
        for d in deleted.into_iter().rev() {
            next.deleted.insert(0, d);
        }
        sh.set_codex(Arc::new(next));
        sh.library().marks_due();
        // The replaced collections' change logs and events.
        for c in &loaded {
            let id = c.collection.id;
            let Some(old) = before.get(&id) else {
                result.added.push(id);
                continue;
            };
            result.replaced.push(id);
            let new = c
                .content
                .as_ref()
                .map(|c| c.hashes.clone())
                .unwrap_or_default();
            let changes = diff(old, &new);
            sh.codex_writer().forget_log(id);
            let entries: Vec<NewChange> = changes
                .iter()
                .map(|(kind, doc, change)| NewChange {
                    kind: *kind,
                    id: *doc,
                    change: match change {
                        DocumentChange::Added => DocChange::Added,
                        DocumentChange::Changed => DocChange::Changed,
                        DocumentChange::Removed => DocChange::Removed,
                    },
                    revision: None,
                })
                .collect();
            sh.codex_writer().log(sh, id, entries, by).await;
            for (kind, doc, change) in changes {
                let _ = sh.events().send(Arc::new(ServerMessage::DocumentChanged {
                    kind,
                    id: doc.to_string(),
                    change,
                    revision: None,
                    client_id: None,
                    ops: None,
                    collection: Some(id),
                }));
            }
        }
        let mut parts = vec![CodexPart::Status, CodexPart::Collections];
        if !replace.is_empty() {
            parts.push(CodexPart::CollectionTrash);
        }
        let one = (loaded.len() == 1).then(|| loaded[0].collection.id);
        codex_changed(sh, &parts, one);
        Ok(())
    }
}

/// The settings backup sets this Codex lacks, with their snapshots and blobs (a snapshot
/// whose blobs aren't all in the file, or don't match their SHA-256, is left out).
async fn import_backups(sh: &Arc<Shared>, opened: &Opened) -> Result<(u32, u32), ApiError> {
    let sets = file_sets(opened);
    let blobs: BTreeMap<String, Vec<u8>> = opened
        .backups
        .iter()
        .filter_map(|(rel, b)| {
            let sha = rel.strip_prefix("blobs/")?;
            (valid_sha(sha) && sha256(b) == sha).then(|| (sha.to_string(), b.clone()))
        })
        .collect();
    let _b = sh.backups().hold().await;
    let store = Store::new(&sh.codex().layout.codex());
    let r = tokio::task::spawn_blocking(move || -> anyhow::Result<(u32, u32)> {
        let mut here = store.sets();
        let (mut n_sets, mut n_snaps) = (0, 0);
        for (set, snaps) in sets {
            if set_present(&here, &set) {
                continue;
            }
            for s in snaps {
                if !s.items.iter().all(|i| blobs.contains_key(&i.sha256)) {
                    tracing::warn!(snapshot = %s.id, "Codex import: a snapshot without all its blobs left out");
                    continue;
                }
                for i in &s.items {
                    store.put_blob(&i.sha256, &blobs[&i.sha256])?;
                }
                store.put_snapshot(&s)?;
                n_snaps += 1;
            }
            store.put_set(&set)?;
            here.push(set);
            n_sets += 1;
        }
        Ok((n_sets, n_snaps))
    })
    .await
    .map_err(join_err)?
    .map_err(|e| api(ErrorCode::Internal, format!("importing the settings backups: {e:#}")))?;
    if r.0 > 0 {
        codex_changed(sh, &[CodexPart::Backups], None);
    }
    Ok(r)
}

fn not_found(id: &str) -> ApiError {
    api(
        ErrorCode::NotFound,
        format!("no uploaded import {id} (uploads are dropped after an hour, and once applied)"),
    )
}
