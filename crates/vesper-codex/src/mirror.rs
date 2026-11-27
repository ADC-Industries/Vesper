//! A show box's mirror of the site's Codex (M8–M9): the box is a machine of another server's
//! site whose server hosts the Codex, shows a local copy of one of its collections, follows its
//! changes, and keeps the media its playlists and bins use on its own disk. Live output always
//! plays from local disk and never waits on the network.
//!
//! ```text
//! <data>/codex-cache/collections/<id>/collection.json      the collection (from the Codex)
//! <data>/codex-cache/collections/<id>/portable/<kind>/…    its documents, byte for byte
//! <data>/codex-cache/collections/<id>/state.json           MirrorState {codexId, seq, held?}
//! <data>/codex-cache/media/                                IDN's library cache (size-capped)
//! ```
//!
//! **Roles** (`machine/codex.json`): a box that joins a site whose server answers as a Codex
//! (the library named `Codex`, then [`super::link::HELLO`]) puts its own Codex to sleep:
//! `dormant` (kept intact, read only) while the first copy of the site's collection arrives,
//! then `client` (the own Codex's active collection is kept in `ownCollection`, the box shows
//! the mirror). Leaving the site wakes the own Codex (`host`, its collection shown again).
//! Uploading the dormant collection to the site's Codex comes later.
//!
//! **The link** ([`super::link`]): the box's own channel with its site membership (a fixed UDP
//! port with [`super::CodexOptions::channel_port`], for firewalls), the Codex found by its
//! library's name over that channel (`Library::find_named_via`, or `find_on_via` the
//! fingerprint saved in `machine/codex.json`): one endpoint toward the Codex box for Vesper's
//! requests, the Codex's pushes and the media cache.
//! The Codex pushes `changed`; the box also asks every [`super::CodexOptions::mirror_poll`].
//!
//! **Syncing** a collection: the first time (or when the Codex says `resync`, a position is
//! missing, or the Codex is another one) the Codex's index is compared with the local files
//! (SHA-256) and only what differs is fetched or removed; otherwise the change log is followed
//! from the saved position, in order. The position is saved after each applied batch, so a
//! restart resumes there. The documents are written atomically; the shown content is then
//! rebuilt and swapped in like an edit (the show follows; `documentChanged` with the
//! collection). The Codex down: the box keeps showing what it has and says so in its status.
//!
//! **Media** through IDN's library cache (`library::cache::Cache`): every library item the
//! box's playlists and media bins use is pinned (recomputed after changes); anything else is
//! wanted when a cue that uses it is triggered. An item not here yet is `copying`: the cue
//! behaves like missing media (the render never blocks). IDN evicts unpinned copies over the
//! cap ([`super::CodexOptions::cache_bytes`]).
//!
//! **Writes** on a show box go through to the Codex ([`super::write_through`]): the box
//! forwards them ([`Mirror::forward`]) and brings its copy up to date at once after one
//! ([`Mirror::refresh`]).
//!
//! **Settings backups and restores** ([`super::backups::remote`]): the box offers its
//! settings to the Codex on the backup schedule ([`Mirror::offer_backup`]), and at each poll
//! asks whether a restore is queued for it (at once when the Codex says one is,
//! [`link::RESTORE_QUEUED`]); if so it offers its `beforeRestore` snapshot, applies the
//! package it gets and reports the result.

use crate::Shared;
use crate::link::{self, Docs, DocsRequest, Hello, HelloRequest, Index, IndexRequest};
use crate::{CollectionUpdate, Show};
use crate::{read_doc, write_doc};
use idn_open_protocol::channel::{Channel, ChannelConfig, Handler, Peer, Reply};
use idn_open_protocol::library::Library;
use idn_open_protocol::library::cache::{Cache, ItemState};
use idn_open_protocol::trust::Enrollment;
use model::{
    AssetLocation, CodexConfig, CodexId, CodexLink, CodexRole, Collection, CollectionId,
    PlaylistItemKind, PlaylistNodeKind,
};
use protocol::{
    CodexCacheStatus, CodexPart, CodexSync, CollectionChanges, DocumentChange, ServerMessage,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

/// `<data>/codex-cache/collections/<id>/state.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorState {
    /// The Codex this copy comes from.
    pub codex_id: CodexId,
    /// The change-log position it is at.
    pub seq: u64,
    /// Documents of this schema, newer than this build reads, were not copied (the older
    /// copy stays shown): once a build reads it, the index is compared again to fetch them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<u32>,
}

/// How long one request to the Codex may take.
const ASK: Duration = Duration::from_secs(15);
/// How long finding the site's Codex may take (each try).
const FIND: Duration = Duration::from_secs(10);
/// IDs asked for in one `docs` request.
const DOCS_PAGE: usize = 200;
/// Failed HELLOs in a row after which the site's Codex is searched for by its name again.
const REFIND_AFTER: u32 = 3;
/// The loop looks at the site this often (joins and leaves), and at the media cache.
const TICK: Duration = Duration::from_secs(1);

/// The state file of a mirrored collection's folder.
pub fn state_path(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

/// A mirrored collection's state, if it has one.
pub fn read_state(dir: &Path) -> Option<MirrorState> {
    let b = std::fs::read(state_path(dir)).ok()?;
    serde_json::from_slice(&b).ok()
}

fn write_state(dir: &Path, st: &MirrorState) -> Result<(), String> {
    let b = serde_json::to_vec_pretty(st).map_err(|e| e.to_string())?;
    crate::store::write_atomic(&state_path(dir), &b).map_err(|e| format!("{e:#}"))
}

fn now() -> String {
    crate::auth::rfc3339(std::time::SystemTime::now(), true)
}

/// What a sync did.
#[derive(Clone, Debug, Default)]
pub struct Synced {
    /// Documents written or removed (kind folder, ID).
    pub touched: Vec<(String, uuid::Uuid)>,
    /// It compared the whole index (a first copy or a resync).
    pub full: bool,
    /// The newest schema of documents it didn't copy because this build can't read them.
    pub held: Option<u32>,
    /// The collection document, when it changed (renamed, recoloured).
    pub collection: Option<Collection>,
}

/// The schema version of a document's bytes, when it is newer than this build reads.
fn too_new(body: &str) -> Option<u32> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Head {
        #[serde(default)]
        schema_version: u32,
    }
    let v = serde_json::from_str::<Head>(body).ok()?.schema_version;
    (v > model::NEWEST_SCHEMA_VERSION).then_some(v)
}

/// Counters (diagnostics and tests).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Syncs by the Codex's index (a first copy, a resync).
    pub index_syncs: u64,
    /// Batches of the change log applied.
    pub change_batches: u64,
    /// Pushes received from the Codex.
    pub pushes: u64,
    /// Swaps of the shown content after changes.
    pub swaps: u64,
}

/// The link to the site's Codex, once there is one.
#[derive(Default)]
struct Linked {
    /// The membership's fingerprint the channel runs with.
    member: Option<String>,
    channel: Option<Arc<Channel>>,
    /// The Codex's fingerprint.
    codex: Option<String>,
    cache: Option<Arc<Cache>>,
    hello: Option<Hello>,
    /// No new search before this.
    find_at: Option<Instant>,
    /// When the Codex was last asked (the poll).
    polled: Option<Instant>,
    /// HELLOs in a row the Codex didn't answer (every [`REFIND_AFTER`], the site is searched
    /// for its Codex again: a reinstalled one answers with another fingerprint).
    failed_hellos: u32,
    /// The items pinned last.
    pins: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct StatusKey {
    reachable: bool,
    seq: u64,
    codex_seq: u64,
    problem: Option<String>,
    media: (u32, u32, u32, u32, u32),
}

/// A show box's mirror (one per server, in [`crate::Server::mirror`]; see the module docs).
pub struct Mirror {
    me: Weak<Mirror>,
    options: super::CodexOptions,
    linked: Mutex<Linked>,
    status: Mutex<CodexSync>,
    announced: Mutex<Option<StatusKey>>,
    /// One sync at a time (the loop and an assignment).
    sync_lock: Mutex<()>,
    /// One poll at a time, its sync and the swap that follows together: a refresh after a
    /// write-through returns only once the copy it brought is shown.
    poll_lock: Mutex<()>,
    wake: Condvar,
    woken: Mutex<bool>,
    stop: AtomicBool,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    counters: Mutex<Counters>,
    /// The show actor (taken at stop: it holds the shared state).
    show: Mutex<Option<(Arc<dyn Show>, tokio::runtime::Handle)>>,
    /// The server's state (the site's presence pushed by the Codex goes there).
    shared: Mutex<Option<Weak<Shared>>>,
}

/// Answers the Codex's pushes on the show box's channel.
struct MirrorHandler {
    mirror: Weak<Mirror>,
}

impl Handler for MirrorHandler {
    fn on_request(&self, from: &Peer, kind: &str, payload: Vec<u8>) -> Reply {
        if kind != link::CHANGED && kind != link::RESTORE_QUEUED && kind != link::PRESENCE_CHANGED {
            return Reply::Unknown;
        }
        let Some(m) = self.mirror.upgrade() else {
            return Reply::Unknown;
        };
        let codex = crate::lock(&m.linked).codex.clone();
        if codex.as_deref() != Some(from.fingerprint.as_str()) {
            return Reply::Refused;
        }
        if kind == link::PRESENCE_CHANGED {
            let sh = crate::lock(&m.shared).as_ref().and_then(Weak::upgrade);
            return match sh {
                Some(sh) => sh.presence_pushed(&payload),
                None => Reply::Unknown,
            };
        }
        if kind == link::CHANGED {
            if serde_json::from_slice::<link::Changed>(&payload).is_err() {
                return Reply::Error("bad request".into());
            }
            crate::lock(&m.counters).pushes += 1;
        }
        m.poke();
        Reply::Ok(b"{}".to_vec())
    }
}

impl Mirror {
    /// A mirror, not started.
    pub fn new(options: &super::CodexOptions) -> Arc<Mirror> {
        Arc::new_cyclic(|me| Mirror {
            me: me.clone(),
            options: options.clone(),
            linked: Mutex::new(Linked::default()),
            status: Mutex::new(CodexSync::default()),
            announced: Mutex::new(None),
            sync_lock: Mutex::new(()),
            poll_lock: Mutex::new(()),
            wake: Condvar::new(),
            woken: Mutex::new(false),
            stop: AtomicBool::new(false),
            thread: Mutex::new(None),
            counters: Mutex::new(Counters::default()),
            show: Mutex::new(None),
            shared: Mutex::new(None),
        })
    }

    /// Starts following the site (a thread of its own).
    pub fn start(&self, sh: &Arc<Shared>, show: Arc<dyn Show>) {
        *crate::lock(&self.show) = Some((show, tokio::runtime::Handle::current()));
        *crate::lock(&self.shared) = Some(Arc::downgrade(sh));
        let (me, weak) = (self.me.clone(), Arc::downgrade(sh));
        match std::thread::Builder::new()
            .name("codex-mirror".into())
            .spawn(move || run(me, weak))
        {
            Ok(h) => *crate::lock(&self.thread) = Some(h),
            Err(e) => tracing::error!("the Codex mirror didn't start: {e}"),
        }
    }

    /// Stops it: no more syncing; the link and the media cache go.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        drop(crate::lock(&self.show).take());
        self.poke();
        let old = std::mem::take(&mut *crate::lock(&self.linked));
        drop(old);
        // (Not waited for: a search for the Codex may take a few seconds; the thread ends
        // at its next look.)
        drop(crate::lock(&self.thread).take());
    }

    /// Wakes the loop now (a push, a join).
    pub fn poke(&self) {
        *crate::lock(&self.woken) = true;
        self.wake.notify_all();
    }

    /// The counters.
    pub fn counters(&self) -> Counters {
        *crate::lock(&self.counters)
    }

    /// The box is linked to the site's Codex (it found it and its channel runs): it offers
    /// its settings there.
    pub fn linked(&self) -> bool {
        let l = crate::lock(&self.linked);
        l.channel.is_some() && l.codex.is_some()
    }

    fn link(&self) -> Result<(Arc<Channel>, String), protocol::ApiError> {
        let l = crate::lock(&self.linked);
        match (l.channel.clone(), l.codex.clone()) {
            (Some(c), Some(f)) => Ok((c, f)),
            _ => Err(protocol::ApiError::new(
                protocol::ErrorCode::Unavailable,
                "the site's Codex can't be reached from this box now",
            )),
        }
    }

    /// The channel to the site's Codex and its fingerprint (`503` while it can't be reached).
    pub fn channel(&self) -> Result<(Arc<Channel>, String), protocol::ApiError> {
        self.link()
    }

    /// Forwards a REST request to the site's Codex (write-through; blocking): its answer.
    pub fn forward(
        &self,
        req: &super::write_through::WriteRequest,
    ) -> Result<super::write_through::WriteAnswer, protocol::ApiError> {
        let (channel, codex) = self.link()?;
        ask_api(
            &channel,
            &codex,
            super::write_through::WRITE,
            req,
            super::write_through::TIMEOUT,
        )
    }

    /// A search beyond this box's collection, asked of the site's Codex (blocking).
    pub fn forward_search(
        &self,
        req: &super::write_through::SearchRequest,
    ) -> Result<protocol::SearchResults, protocol::ApiError> {
        let (channel, codex) = self.link()?;
        ask_api(&channel, &codex, super::write_through::SEARCH, req, ASK)
    }

    /// Brings the copy of the shown collection up to date now (after a write went through;
    /// blocking).
    pub fn refresh(&self, sh: &Arc<Shared>) {
        let Ok((channel, _)) = self.link() else {
            return;
        };
        self.poll(sh, &channel, false);
    }

    /// The box that holds the site's Codex, as this box knows it (for a client to send
    /// uploads and Codex management there).
    pub fn codex_box(&self, sh: &Shared) -> Option<protocol::CodexBox> {
        let config = sh.codex().config.clone();
        let (hello, fingerprint) = {
            let l = crate::lock(&self.linked);
            (l.hello.clone(), l.codex.clone())
        };
        let server_id = hello
            .as_ref()
            .map(|h| h.server_id.clone())
            .or_else(|| config.codex.as_ref().map(|c| c.server_id.clone()))
            .filter(|s| !s.is_empty())?;
        let mut addresses = Vec::new();
        if let Some(h) = &hello {
            if let (Some(port), Some(host)) = (
                h.api_port,
                sh.site().membership().server.as_deref().and_then(host_of),
            ) {
                addresses.push(base_url(&host, port));
            }
            for a in &h.addresses {
                if !addresses.contains(a) {
                    addresses.push(a.clone());
                }
            }
        }
        Some(protocol::CodexBox {
            server_id,
            name: hello.map(|h| h.server_name).filter(|n| !n.is_empty()),
            fingerprint: fingerprint.or_else(|| config.codex.map(|c| c.fingerprint)),
            addresses,
        })
    }

    /// Offers this box's settings to the site's Codex (blocking): its answer.
    pub fn offer_backup(
        &self,
        offer: super::backups::SnapshotOffer,
    ) -> Result<protocol::SnapshotTaken, protocol::ApiError> {
        let (channel, codex) = self.link()?;
        let wire = super::backups::remote::OfferWire::from_offer(&offer).map_err(|e| {
            protocol::ApiError::new(
                protocol::ErrorCode::Internal,
                format!("this box's settings can't be sent: {e}"),
            )
        })?;
        ask_api(
            &channel,
            &codex,
            super::backups::remote::BACKUP_OFFER,
            &wire,
            ASK,
        )
    }

    /// The items pinned now.
    pub fn pins(&self) -> BTreeSet<String> {
        crate::lock(&self.linked).pins.clone()
    }

    /// `GET /codex`'s `sync`: on a show box (or one finding its site's Codex).
    pub fn status(&self, sh: &Shared) -> Option<CodexSync> {
        let role = sh.codex().config.role;
        let linked = crate::lock(&self.linked);
        if role == CodexRole::Host && linked.member.is_none() {
            return None;
        }
        let mut s = crate::lock(&self.status).clone();
        if let Some(h) = linked.hello.as_ref().filter(|h| !h.server_name.is_empty()) {
            s.name = Some(h.server_name.clone());
        }
        let cache = linked.cache.clone();
        let pins = linked.pins.len() as u32;
        drop(linked);
        if let Some(c) = cache {
            s.media = cache_status(&c, pins);
        }
        s.behind = s.codex_seq.saturating_sub(s.seq);
        Some(s)
    }

    fn set_status(&self, f: impl FnOnce(&mut CodexSync)) {
        f(&mut crate::lock(&self.status));
    }

    /// `PUT /codex/assignment` on a show box: copies collection `id` of the site's Codex
    /// (blocking; the caller holds the Codex writer and switches after).
    pub fn prepare_assignment(
        &self,
        sh: &Shared,
        id: CollectionId,
    ) -> Result<(), protocol::ApiError> {
        use protocol::{ApiError, ErrorCode};
        let (channel, codex) = {
            let l = crate::lock(&self.linked);
            (l.channel.clone(), l.codex.clone())
        };
        let (Some(channel), Some(codex)) = (channel, codex) else {
            return Err(ApiError::new(
                ErrorCode::Unavailable,
                "the site's Codex can't be reached from this box now: the collection it shows can't change",
            ));
        };
        let hello = ask::<Hello>(
            &channel,
            &codex,
            link::HELLO,
            &HelloRequest {
                v: link::VERSION,
                collection: Some(id),
            },
        )
        .map_err(|e| ApiError::new(ErrorCode::Unavailable, format!("the site's Codex: {e}")))?;
        if !hello.collections.iter().any(|c| c.collection.id == id) {
            return Err(ApiError::invalid(
                "collection",
                format!("collection {id} is not in the site's Codex"),
            ));
        }
        self.sync(sh, &channel, &codex, &hello, id).map_err(|e| {
            ApiError::new(
                ErrorCode::Unavailable,
                format!("copying the collection: {e}"),
            )
        })?;
        crate::lock(&self.linked).hello = Some(hello);
        Ok(())
    }

    /// Brings the copy of collection `col` up to date (see the module docs). Blocking.
    fn sync(
        &self,
        sh: &Shared,
        channel: &Channel,
        codex: &str,
        hello: &Hello,
        col: CollectionId,
    ) -> Result<Synced, String> {
        let _g = crate::lock(&self.sync_lock);
        let layout = sh.codex().layout.clone();
        let dir = layout.mirror_collection(col);
        let Some(entry) = hello.collections.iter().find(|c| c.collection.id == col) else {
            return Err(format!("collection {col} is not in the site's Codex"));
        };
        // The collection document, as the Codex has it.
        let cj = dir.join("collection.json");
        let mut out = Synced::default();
        if read_doc::<Collection>(&cj).ok().flatten().as_ref() != Some(&entry.collection) {
            write_doc(&cj, &entry.collection).map_err(|e| format!("{e:#}"))?;
            out.collection = Some(entry.collection.clone());
        }
        let mut seq = match read_state(&dir) {
            // (Documents held back for a newer schema are fetched once this build reads it.)
            Some(st)
                if st.codex_id == hello.codex_id
                    && st.held.is_none_or(|v| v > model::NEWEST_SCHEMA_VERSION) =>
            {
                out.held = st.held;
                st.seq
            }
            _ => {
                let seq = self.index_sync(channel, codex, col, &dir, hello.codex_id, &mut out)?;
                out.full = true;
                seq
            }
        };
        self.set_status(|s| s.seq = seq);
        if seq == entry.seq {
            // (Nothing new by the Codex's hello.)
            return Ok(out);
        }
        let mut resyncs = 0;
        loop {
            let ch: CollectionChanges = ask(
                channel,
                codex,
                link::CHANGES,
                &link::ChangesRequest {
                    v: link::VERSION,
                    collection: col,
                    since: seq,
                    limit: None,
                },
            )?;
            let gap = ch.changes.first().is_some_and(|c| c.seq != seq + 1);
            if ch.resync || gap {
                resyncs += 1;
                if resyncs > 3 {
                    return Err("the Codex's change log keeps asking for a resync".into());
                }
                tracing::info!(collection = %col, since = seq, "Codex mirror: resync by the Codex's index");
                seq = self.index_sync(channel, codex, col, &dir, hello.codex_id, &mut out)?;
                out.full = true;
                self.set_status(|s| s.seq = seq);
                continue;
            }
            if ch.changes.is_empty() {
                break;
            }
            // The last word on each document, in order.
            let mut last: BTreeMap<(String, uuid::Uuid), bool> = BTreeMap::new();
            for c in &ch.changes {
                let kind = crate::data::kind_dir(c.kind);
                if link::collection_kind(&kind).is_none() {
                    continue;
                }
                last.insert((kind, c.id), c.change == model::DocChange::Removed);
            }
            let mut fetch: BTreeMap<String, Vec<uuid::Uuid>> = BTreeMap::new();
            for ((kind, id), removed) in last {
                if removed {
                    remove_doc(&dir, &kind, id)?;
                    out.touched.push((kind, id));
                } else {
                    fetch.entry(kind).or_default().push(id);
                }
            }
            for (kind, ids) in fetch {
                self.fetch(channel, codex, col, &dir, &kind, &ids, &mut out)?;
            }
            seq = ch.to;
            write_state(
                &dir,
                &MirrorState {
                    codex_id: hello.codex_id,
                    seq,
                    held: out.held,
                },
            )?;
            crate::lock(&self.counters).change_batches += 1;
            self.set_status(|s| s.seq = seq);
        }
        Ok(out)
    }

    /// Compares the Codex's index with the local files and fetches or removes what differs.
    /// The position the copy is at.
    #[allow(clippy::too_many_arguments)]
    fn index_sync(
        &self,
        channel: &Channel,
        codex: &str,
        col: CollectionId,
        dir: &Path,
        codex_id: CodexId,
        out: &mut Synced,
    ) -> Result<u64, String> {
        let idx: Index = ask(
            channel,
            codex,
            link::INDEX,
            &IndexRequest {
                v: link::VERSION,
                collection: col,
            },
        )?;
        crate::lock(&self.counters).index_syncs += 1;
        let local = local_files(dir);
        // The whole index is compared: only what this sync holds back stays held.
        out.held = None;
        let mut theirs: HashSet<(String, uuid::Uuid)> = HashSet::new();
        let mut fetch: BTreeMap<String, Vec<uuid::Uuid>> = BTreeMap::new();
        for e in idx.docs {
            if link::collection_kind(&e.kind).is_none() {
                continue;
            }
            let key = (e.kind.clone(), e.id);
            if local.get(&key) != Some(&e.sha256) {
                fetch.entry(e.kind).or_default().push(e.id);
            }
            theirs.insert(key);
        }
        for key in local.keys() {
            if !theirs.contains(key) {
                remove_doc(dir, &key.0, key.1)?;
                out.touched.push(key.clone());
            }
        }
        for (kind, ids) in fetch {
            self.fetch(channel, codex, col, dir, &kind, &ids, out)?;
        }
        write_state(
            dir,
            &MirrorState {
                codex_id,
                seq: idx.seq,
                held: out.held,
            },
        )?;
        Ok(idx.seq)
    }

    /// Fetches documents of one kind into the copy (removing those the Codex no longer has).
    #[allow(clippy::too_many_arguments)]
    fn fetch(
        &self,
        channel: &Channel,
        codex: &str,
        col: CollectionId,
        dir: &Path,
        kind: &str,
        ids: &[uuid::Uuid],
        out: &mut Synced,
    ) -> Result<(), String> {
        let mut left: Vec<uuid::Uuid> = ids.to_vec();
        while !left.is_empty() {
            let page: Vec<uuid::Uuid> = left.iter().take(DOCS_PAGE).copied().collect();
            let docs: Docs = ask(
                channel,
                codex,
                link::DOCS,
                &DocsRequest {
                    v: link::VERSION,
                    collection: col,
                    kind: kind.to_string(),
                    ids: page.clone(),
                },
            )?;
            let mut done: HashSet<uuid::Uuid> = HashSet::new();
            for d in docs.docs {
                if !page.contains(&d.id) {
                    continue;
                }
                if link::sha_hex(d.body.as_bytes()) != d.sha256 {
                    return Err(format!("{kind} {}: its SHA-256 doesn't match", d.id));
                }
                if let Some(v) = too_new(&d.body) {
                    // This build can't read it: keep the copy it has (still shown) rather
                    // than replace it with one that would drop out of the content.
                    tracing::warn!(kind, id = %d.id, schema = v, "Codex mirror: a document of a newer schema kept at its older copy: update this server");
                    out.held = out.held.max(Some(v));
                    done.insert(d.id);
                    continue;
                }
                let path = doc_path(dir, kind, d.id);
                crate::store::write_atomic(&path, d.body.as_bytes())
                    .map_err(|e| format!("{e:#}"))?;
                out.touched.push((kind.to_string(), d.id));
                done.insert(d.id);
            }
            for id in docs.missing {
                if page.contains(&id) {
                    remove_doc(dir, kind, id)?;
                    out.touched.push((kind.to_string(), id));
                    done.insert(id);
                }
            }
            if done.is_empty() {
                return Err(format!(
                    "the Codex answered none of {} {kind} documents",
                    page.len()
                ));
            }
            left.retain(|id| !done.contains(id));
        }
        Ok(())
    }
}

/// The host of a site server's address (`host`, `host:port`, `[v6]:port`).
fn host_of(server: &str) -> Option<String> {
    let s = server.trim();
    if let Ok(a) = s.parse::<std::net::SocketAddr>() {
        return Some(a.ip().to_string());
    }
    if let Some(rest) = s.strip_prefix('[') {
        return rest.split(']').next().map(str::to_string);
    }
    let host = match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => s,
    };
    (!host.is_empty()).then(|| host.to_string())
}

/// `http://host:port` (IPv6 in brackets).
fn base_url(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("http://[{host}]:{port}")
    } else {
        format!("http://{host}:{port}")
    }
}

fn doc_path(dir: &Path, kind: &str, id: uuid::Uuid) -> PathBuf {
    dir.join("portable").join(kind).join(format!("{id}.json"))
}

fn remove_doc(dir: &Path, kind: &str, id: uuid::Uuid) -> Result<(), String> {
    match std::fs::remove_file(doc_path(dir, kind, id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("removing {kind} {id}: {e}")),
    }
}

/// The copy's documents: (kind folder, ID) → SHA-256 of the file.
fn local_files(dir: &Path) -> HashMap<(String, uuid::Uuid), String> {
    let mut out = HashMap::new();
    for kind in crate::data::COLLECTION_KINDS {
        let name = crate::data::kind_dir(*kind);
        let Ok(rd) = std::fs::read_dir(dir.join("portable").join(&name)) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Some(id) = p
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".json"))
                .and_then(|n| n.parse::<uuid::Uuid>().ok())
            else {
                continue;
            };
            if let Ok(b) = std::fs::read(&p) {
                out.insert((name.clone(), id), link::sha_hex(&b));
            }
        }
    }
    out
}

/// A request to the Codex and its answer, its errors as `ApiError`s (the Codex's own, or
/// `unavailable` when it can't be reached).
pub(crate) fn ask_api<T: serde::de::DeserializeOwned>(
    channel: &Channel,
    codex: &str,
    kind: &str,
    body: &impl Serialize,
    timeout: Duration,
) -> Result<T, protocol::ApiError> {
    use protocol::{ApiError, ErrorCode};
    let payload =
        serde_json::to_vec(body).map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?;
    if payload.len() > idn_open_protocol::channel::MAX_PAYLOAD {
        return Err(ApiError::new(
            ErrorCode::BadRequest,
            format!(
                "too large to send to the site's Codex ({} bytes; at most {})",
                payload.len(),
                idn_open_protocol::channel::MAX_PAYLOAD
            ),
        ));
    }
    let unavailable = |m: String| ApiError::new(ErrorCode::Unavailable, m);
    match channel.request(codex, kind, &payload, timeout) {
        Ok(Reply::Ok(b)) => serde_json::from_slice(&b).map_err(|e| {
            ApiError::new(
                ErrorCode::Internal,
                format!("{kind}: unreadable answer from the site's Codex: {e}"),
            )
        }),
        Ok(Reply::Error(e)) => Err(super::backups::remote::error_of(kind, &e)),
        Ok(Reply::Refused) => Err(unavailable(format!(
            "the site's Codex refused this box ({kind}): is it still a machine of the site?"
        ))),
        Ok(Reply::Unknown) => Err(unavailable(format!(
            "the site's Codex doesn't answer {kind} (an older server?)"
        ))),
        Err(e) => Err(unavailable(format!(
            "the site's Codex can't be reached from this box now: {e}"
        ))),
    }
}

/// A request to the Codex and its answer.
fn ask<T: serde::de::DeserializeOwned>(
    channel: &Channel,
    codex: &str,
    kind: &str,
    body: &impl Serialize,
) -> Result<T, String> {
    let payload = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    match channel.request(codex, kind, &payload, ASK) {
        Ok(Reply::Ok(b)) => {
            serde_json::from_slice(&b).map_err(|e| format!("{kind}: unreadable answer: {e}"))
        }
        Ok(Reply::Error(e)) => Err(format!("{kind}: {e}")),
        Ok(Reply::Refused) => Err(format!("{kind}: refused by the Codex")),
        Ok(Reply::Unknown) => Err(format!(
            "{kind}: the Codex doesn't answer it (an older server?)"
        )),
        Err(e) => Err(format!("{kind}: {e}")),
    }
}

fn cache_status(c: &Cache, pinned: u32) -> CodexCacheStatus {
    let mut s = CodexCacheStatus {
        pinned,
        ..CodexCacheStatus::default()
    };
    for st in c.states().values() {
        match st {
            ItemState::Cached(_) => s.cached += 1,
            ItemState::Fetching { .. } | ItemState::Waiting => s.copying += 1,
            ItemState::Missing => s.missing += 1,
            ItemState::Failed(_) => s.failed += 1,
            ItemState::NotKept => {}
        }
    }
    (s.bytes, s.cap_bytes) = c.usage();
    s
}

/// The library items the box's playlists and media bins use: the media they trigger, and the
/// media of the presentations they hold.
pub fn scheduled_items(c: &crate::data::CollectionContent) -> BTreeSet<String> {
    fn walk(v: &serde_json::Value, out: &mut HashSet<model::MediaAssetId>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    if k == "asset"
                        && let Some(id) = v.as_str().and_then(|s| s.parse::<uuid::Uuid>().ok())
                    {
                        out.insert(model::MediaAssetId::from(id));
                    } else {
                        walk(v, out);
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    let mut assets = HashSet::new();
    let mut presentations = HashSet::new();
    for node in c.playlists.values() {
        if let PlaylistNodeKind::Playlist { items, .. } = &node.kind {
            for it in items {
                match &it.kind {
                    PlaylistItemKind::Presentation { presentation, .. } => {
                        presentations.insert(presentation.id);
                    }
                    other => {
                        if let Ok(v) = serde_json::to_value(other) {
                            walk(&v, &mut assets);
                        }
                    }
                }
            }
        }
    }
    for b in c.media_bins.iter() {
        if let Ok(v) = serde_json::to_value(b) {
            walk(&v, &mut assets);
        }
    }
    for p in presentations {
        if let Some(p) = c.presentation(p) {
            assets.extend(c.presentation_assets(p));
        }
    }
    assets
        .into_iter()
        .filter_map(|a| match &c.assets.get(&a)?.location {
            AssetLocation::Library { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}

/// The mirror's loop (see the module docs).
fn run(me: Weak<Mirror>, sh: Weak<Shared>) {
    loop {
        let Some(m) = me.upgrade() else { return };
        {
            let mut w = crate::lock(&m.woken);
            if !*w {
                w = m
                    .wake
                    .wait_timeout(w, TICK)
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
            let pushed = std::mem::take(&mut *w);
            drop(w);
            if pushed {
                crate::lock(&m.linked).polled = None;
            }
        }
        if m.stop.load(Ordering::Relaxed) {
            return;
        }
        let Some(sh) = sh.upgrade() else { return };
        m.step(&sh);
        drop(sh);
    }
}

impl Mirror {
    fn show(&self) -> Option<(Arc<dyn Show>, tokio::runtime::Handle)> {
        crate::lock(&self.show).clone()
    }

    /// One look at the site and the Codex.
    fn step(&self, sh: &Arc<Shared>) {
        let member = sh.site().member_enrollment();
        let role = sh.codex().config.role;
        let Some(member) = member else {
            if sh.site().has_member() {
                // In another site, its membership not ready (a start): wait for it.
                return;
            }
            // Not (or no longer) a machine of another site.
            let had = {
                let mut l = crate::lock(&self.linked);
                let had = l.member.is_some();
                *l = Linked::default();
                had
            };
            if had {
                *crate::lock(&self.status) = CodexSync::default();
            }
            if matches!(role, CodexRole::Client | CodexRole::Dormant) {
                self.wake_own(sh);
            }
            return;
        };
        let fp = member.fingerprint();
        let ch = {
            let mut l = crate::lock(&self.linked);
            if l.member.as_deref() != Some(fp.as_str()) {
                // Another membership (or the first): everything anew.
                *l = Linked {
                    member: Some(fp.clone()),
                    ..Linked::default()
                };
            }
            l.channel.clone()
        };
        let channel = match ch {
            Some(c) => c,
            None => match self.start_channel(&member) {
                Ok(c) => c,
                Err(e) => {
                    self.problem(Some(format!("the box's channel didn't start: {e}")), false);
                    return;
                }
            },
        };
        if !self.ensure_codex(sh, &channel) {
            self.announce(sh);
            return;
        }
        let due = {
            let l = crate::lock(&self.linked);
            l.polled
                .is_none_or(|t| t.elapsed() >= self.options.mirror_poll)
                || role == CodexRole::Dormant
        };
        if due {
            self.poll(sh, &channel, true);
        }
        self.pin(sh);
        self.announce(sh);
    }

    fn start_channel(&self, member: &Arc<Enrollment>) -> std::io::Result<Arc<Channel>> {
        let cfg = ChannelConfig {
            port: self.options.channel_port,
            ..ChannelConfig::new(member.clone())
        };
        let ch = Channel::start(
            cfg,
            Arc::new(MirrorHandler {
                mirror: self.me.clone(),
            }),
        )?;
        tracing::info!(
            port = ch.port(),
            "Codex mirror: this box's channel in its site"
        );
        crate::lock(&self.linked).channel = Some(ch.clone());
        Ok(ch)
    }

    /// Finds the site's Codex once (and opens the media cache on it), over the box's own
    /// channel: one endpoint toward the Codex box, for Vesper's requests and the media
    /// library's. (No IDN Node runs on a show box, so the channel is a full endpoint: the
    /// Codex's pushes reach it.) Whether it is known.
    fn ensure_codex(&self, sh: &Shared, channel: &Arc<Channel>) -> bool {
        {
            let l = crate::lock(&self.linked);
            if l.codex.is_some() && l.cache.is_some() {
                return true;
            }
            if l.find_at.is_some_and(|t| Instant::now() < t) {
                return false;
            }
        }
        let codex = sh.codex();
        let saved = codex
            .config
            .codex
            .as_ref()
            .map(|c| c.fingerprint.clone())
            .filter(|f| !f.is_empty());
        let found = saved
            .as_ref()
            .and_then(|fp| Library::find_on_via(channel.clone(), fp, FIND).ok())
            .map(Ok)
            .unwrap_or_else(|| {
                Library::find_named_via(channel.clone(), super::library::NAME, FIND)
            });
        let lib = match found {
            Ok(l) => Arc::new(l),
            Err(e) => {
                crate::lock(&self.linked).find_at =
                    Some(Instant::now() + self.options.mirror_poll.min(Duration::from_secs(5)));
                self.problem(Some(format!("the site's Codex wasn't found: {e}")), false);
                return false;
            }
        };
        self.adopt(sh, lib)
    }

    /// Follows the site's Codex found as `lib` from now on (and opens the media cache on it).
    fn adopt(&self, sh: &Shared, lib: Arc<Library>) -> bool {
        let codex = sh.codex();
        let codex_fp = lib.host_fingerprint().to_string();
        let dir = codex.layout.cache_media();
        match Cache::open(&dir, self.options.cache_bytes, lib) {
            Ok(cache) => {
                tracing::info!(codex = %codex_fp, dir = %dir.display(), cap = self.options.cache_bytes, "Codex mirror: the site's Codex found; media cache open");
                let mut l = crate::lock(&self.linked);
                l.codex = Some(codex_fp.clone());
                l.cache = Some(cache.clone());
                drop(l);
                self.set_status(|s| s.fingerprint = Some(codex_fp));
                if sh.codex().config.role == CodexRole::Client {
                    sh.library().set_remote(Some(cache));
                }
                true
            }
            Err(e) => {
                crate::lock(&self.linked).find_at = Some(Instant::now() + self.options.mirror_poll);
                self.problem(Some(format!("the media cache didn't open: {e}")), false);
                false
            }
        }
    }

    /// After [`REFIND_AFTER`] failed HELLOs in a row, searches the site for its Codex by name:
    /// one answering with another fingerprint (reinstalled, a new device identity) is followed
    /// from now on. (A Codex that is just down isn't found: everything stays as it is.)
    fn refind(&self, sh: &Shared, channel: &Arc<Channel>, codex_fp: &str) {
        let failed = {
            let mut l = crate::lock(&self.linked);
            l.failed_hellos += 1;
            l.failed_hellos
        };
        if failed % REFIND_AFTER != 0 {
            return;
        }
        let Ok(lib) = Library::find_named_via(channel.clone(), super::library::NAME, FIND) else {
            return;
        };
        if lib.host_fingerprint() == codex_fp {
            return;
        }
        tracing::info!(old = %codex_fp, new = %lib.host_fingerprint(), "Codex mirror: the site's Codex answers with another fingerprint now: following it");
        if self.adopt(sh, Arc::new(lib)) {
            crate::lock(&self.linked).failed_hellos = 0;
        }
    }

    fn problem(&self, problem: Option<String>, reachable: bool) {
        if let Some(p) = &problem {
            tracing::debug!("Codex mirror: {p}");
        }
        self.set_status(|s| {
            s.problem = problem;
            s.reachable = reachable;
        });
    }

    /// Asks the Codex what is new and follows it (`restores`: and whether a settings restore
    /// waits for this box).
    fn poll(&self, sh: &Arc<Shared>, channel: &Arc<Channel>, restores: bool) {
        let _p = crate::lock(&self.poll_lock);
        let (codex_fp, cache) = {
            let mut l = crate::lock(&self.linked);
            l.polled = Some(Instant::now());
            (l.codex.clone(), l.cache.clone())
        };
        let Some(codex_fp) = codex_fp else { return };
        let config = sh.codex().config.clone();
        let following = (config.role == CodexRole::Client)
            .then_some(config.collection)
            .flatten();
        let hello: Hello = match ask(
            channel,
            &codex_fp,
            link::HELLO,
            &HelloRequest {
                v: link::VERSION,
                collection: following,
            },
        ) {
            Ok(h) => h,
            Err(e) => {
                self.problem(
                    Some(format!("the site's Codex can't be reached: {e}")),
                    false,
                );
                self.refind(sh, channel, &codex_fp);
                return;
            }
        };
        crate::lock(&self.linked).failed_hellos = 0;
        self.set_status(|s| {
            s.reachable = true;
            s.problem = None;
            s.last_sync = Some(now());
            s.server_id = Some(hello.server_id.clone());
        });
        crate::lock(&self.linked).hello = Some(hello.clone());
        let link = CodexLink {
            server_id: hello.server_id.clone(),
            fingerprint: codex_fp.clone(),
        };
        let mut role = config.role;
        if role == CodexRole::Host {
            // The site's server hosts a Codex: this box's own goes to sleep.
            if !self.save_config(sh, |c| {
                c.role = CodexRole::Dormant;
                c.codex = Some(link.clone());
            }) {
                return;
            }
            tracing::info!(codex = %hello.codex_id, server = %hello.server_name, "Codex: this box is a machine of a site with a Codex: its own Codex is dormant while the site's collection is copied");
            super::writer::codex_changed(sh, &[CodexPart::Status], None);
            role = CodexRole::Dormant;
        } else if config.codex.as_ref() != Some(&link) {
            self.save_config(sh, |c| c.codex = Some(link.clone()));
        }
        // The collection to follow: the one assigned, or the Codex's first.
        let target = match (role, following) {
            (CodexRole::Client, Some(c)) => {
                if !hello.collections.iter().any(|x| x.collection.id == c) {
                    self.problem(
                        Some(format!(
                            "collection {c} is no longer in the site's Codex: assign another"
                        )),
                        true,
                    );
                    return;
                }
                c
            }
            _ => match hello.collections.first() {
                Some(c) => c.collection.id,
                None => {
                    self.problem(Some("the site's Codex has no collection".into()), true);
                    return;
                }
            },
        };
        if let Some(e) = hello.collections.iter().find(|c| c.collection.id == target) {
            let seq = e.seq;
            self.set_status(|s| s.codex_seq = seq);
        }
        let hello_problem = (hello.writes > model::NEWEST_SCHEMA_VERSION).then_some(hello.writes);
        if hello_problem.is_some() {
            self.set_status(|s| {
                s.problem = Some(format!(
                    "the site's Codex writes documents of schema {}; this box reads up to {}: update this server",
                    hello.writes,
                    model::NEWEST_SCHEMA_VERSION
                ))
            });
        }
        let synced = match self.sync(sh, channel, &codex_fp, &hello, target) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(collection = %target, "Codex mirror: {e}");
                self.problem(Some(e), true);
                return;
            }
        };
        if let (Some(v), None) = (synced.held, hello_problem) {
            self.set_status(|s| {
                s.problem = Some(format!(
                    "some of the site's documents are of schema {v}; this box reads up to {}: update this server",
                    model::NEWEST_SCHEMA_VERSION
                ))
            });
        }
        if role == CodexRole::Dormant {
            self.become_client(sh, target, cache);
        } else {
            if !synced.touched.is_empty() {
                self.swap(sh, target);
            }
            if let Some(c) = synced.collection {
                self.collection_changed(sh, c);
            }
            if restores {
                self.take_restore(sh, channel, &codex_fp);
            }
        }
    }

    /// Asks the Codex whether a settings restore is queued for this box; takes it when there
    /// is one: offers its settings now (the `beforeRestore` snapshot, the undo), applies the
    /// package it gets (files, then `reloadData`) and reports how it went.
    fn take_restore(&self, sh: &Arc<Shared>, channel: &Channel, codex: &str) {
        use super::backups::remote;
        let pending: remote::PendingAnswer = match ask_api(
            channel,
            codex,
            remote::PENDING_RESTORE,
            &remote::PendingRequest { v: link::VERSION },
            ASK,
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("Codex mirror: asking for a settings restore: {}", e.message);
                return;
            }
        };
        let Some(job) = pending.restore else { return };
        let Some((show, rt)) = self.show() else {
            return;
        };
        tracing::info!(restore = %job.id, "settings restore: this box takes the one the Codex queued for it");
        let offer = match super::backups::this_box_offer(
            sh,
            model::SnapshotReason::BeforeRestore,
            job.by.clone(),
        )
        .map_err(|e| format!("{e:#}"))
        .and_then(|o| remote::OfferWire::from_offer(&o))
        {
            Ok(o) => o,
            Err(e) => {
                tracing::error!("settings restore: collecting this box's settings: {e}");
                self.report_restore(channel, codex, &job.id, Err(e));
                return;
            }
        };
        let begin: remote::BeginAnswer = match ask_api(
            channel,
            codex,
            remote::RESTORE_BEGIN,
            &remote::BeginRequest {
                v: link::VERSION,
                id: job.id.clone(),
                offer,
            },
            ASK.max(Duration::from_secs(60)),
        ) {
            Ok(b) => b,
            Err(e) => {
                // (Refused by the Codex: it marked the restore failed with why.)
                tracing::warn!(restore = %job.id, "settings restore: {}", e.message);
                return;
            }
        };
        let package = begin.package();
        let sh2 = sh.clone();
        let applied =
            rt.block_on(async move { super::backups::apply_here(&sh2, &show, package).await });
        match &applied {
            Ok(a) => {
                tracing::info!(restore = %job.id, files = a.files, restart = ?a.restart, "settings restored from the Codex")
            }
            Err(e) => tracing::error!(restore = %job.id, "settings restore: {}", e.message),
        }
        self.report_restore(
            channel,
            codex,
            &job.id,
            applied.map(|a| a.restart).map_err(|e| e.message),
        );
    }

    fn report_restore(
        &self,
        channel: &Channel,
        codex: &str,
        id: &str,
        r: Result<Vec<String>, String>,
    ) {
        use super::backups::remote;
        let (ok, error, restart) = match r {
            Ok(restart) => (true, None, restart),
            Err(e) => (false, Some(e), Vec::new()),
        };
        let report = remote::ResultRequest {
            v: link::VERSION,
            id: id.to_string(),
            ok,
            error,
            restart,
        };
        if let Err(e) =
            ask_api::<serde_json::Value>(channel, codex, remote::RESTORE_RESULT, &report, ASK)
        {
            tracing::warn!(restore = %id, "settings restore: reporting the result: {}", e.message);
        }
    }

    /// Writes `machine/codex.json` changed by `f` (and uses it at once). Whether it was saved.
    fn save_config(&self, sh: &Shared, f: impl FnOnce(&mut CodexConfig)) -> bool {
        self.write_config(sh, f, true)
    }

    /// Writes `machine/codex.json` changed by `f`; `publish`: used at once (else by the reload
    /// that follows, so the status never shows a role with the other role's collection).
    fn write_config(&self, sh: &Shared, f: impl FnOnce(&mut CodexConfig), publish: bool) -> bool {
        let codex = sh.codex();
        let mut config = codex.config.clone();
        f(&mut config);
        config.schema_version = model::SCHEMA_VERSION;
        let path = codex.layout.config_json();
        if let Err(e) = crate::store::check_writable(&path)
            .map_err(|e| anyhow::anyhow!("{e:?}"))
            .and_then(|_| write_doc(&path, &config))
        {
            tracing::error!("Codex mirror: saving {}: {e:#}", path.display());
            self.problem(
                Some(format!("saving this box's Codex settings: {e:#}")),
                true,
            );
            return false;
        }
        if publish {
            let mut s = (*codex).clone();
            s.config = config;
            sh.set_codex(Arc::new(s));
        }
        true
    }

    /// The first copy is here: the box shows it (role `client`); its own Codex stays dormant.
    fn become_client(&self, sh: &Arc<Shared>, target: CollectionId, cache: Option<Arc<Cache>>) {
        let Some((show, rt)) = self.show() else {
            return;
        };
        let own = sh.codex().config.collection;
        if !self.write_config(
            sh,
            |c| {
                c.role = CodexRole::Client;
                c.own_collection = own;
                c.collection = Some(target);
            },
            false,
        ) {
            return;
        }
        sh.library().set_remote(cache);
        let sh2 = sh.clone();
        let r = rt.block_on(async move { sh2.codex_writer().reload(&sh2, &show).await });
        match r {
            Ok(_) => {
                tracing::info!(collection = %target, "Codex: this box shows the site's collection (its own Codex is dormant)")
            }
            Err(e) => tracing::error!("Codex: showing the site's collection: {}", e.message),
        }
        super::writer::codex_changed(sh, &[CodexPart::Status, CodexPart::Collections], None);
    }

    /// Not in another site any more: the own Codex wakes (role `host`, its collection shown).
    fn wake_own(&self, sh: &Arc<Shared>) {
        let Some((show, rt)) = self.show() else {
            return;
        };
        let own = sh.codex().config.own_collection;
        let role = sh.codex().config.role;
        if !self.write_config(
            sh,
            |c| {
                c.role = CodexRole::Host;
                if role == CodexRole::Client {
                    c.collection = own;
                }
                c.own_collection = None;
                c.codex = None;
            },
            false,
        ) {
            return;
        }
        // An upload of the dormant collection was for that site.
        sh.adopt().forget(sh);
        let sh2 = sh.clone();
        let r = rt.block_on(async move { sh2.codex_writer().reload(&sh2, &show).await });
        match r {
            Ok(_) => tracing::info!("Codex: this box left the site: its own Codex is awake again"),
            Err(e) => tracing::error!("Codex: waking this box's own Codex: {}", e.message),
        }
        super::writer::codex_changed(sh, &[CodexPart::Status, CodexPart::Collections], None);
    }

    /// The copy of the shown collection changed: its content is rebuilt from disk and swapped
    /// in like an edit (the show follows); `documentChanged` for what differs.
    fn swap(&self, sh: &Arc<Shared>, target: CollectionId) {
        let Some((show, rt)) = self.show() else {
            return;
        };
        let sh2 = sh.clone();
        rt.block_on(async move {
            let _w = sh2.codex_writer().exclusive().await;
            let codex = sh2.codex();
            if codex.config.role != CodexRole::Client || codex.active != target {
                return;
            }
            let old = sh2.data();
            let (root, media, lib, roots) = (
                codex.layout.mirror_collection(target),
                codex.layout.media(),
                sh2.library().clone(),
                old.content.media_roots.clone(),
            );
            let Ok(content) = tokio::task::spawn_blocking(move || {
                crate::data::CollectionContent::load(&root, media, lib, roots)
            })
            .await
            else {
                return;
            };
            let content = Arc::new(content);
            let changes = super::writer::diff(&old.content.hashes, &content.hashes);
            if let Err(e) = show.collection(CollectionUpdate::Mirror(content)).await {
                tracing::error!("Codex mirror: showing the changes: {}", e.message);
                return;
            }
            let counts = changes.iter().any(|c| c.2 != DocumentChange::Changed);
            for (kind, id, change) in changes {
                let _ = sh2.events().send(Arc::new(ServerMessage::DocumentChanged {
                    kind,
                    id: id.to_string(),
                    change,
                    revision: None,
                    client_id: None,
                    ops: None,
                    collection: Some(target),
                }));
            }
            if counts {
                super::writer::codex_changed(&sh2, &[CodexPart::Collections], Some(target));
            }
        });
        crate::lock(&self.counters).swaps += 1;
    }

    /// The shown collection's document changed on the Codex (a rename, a new colour): the
    /// box's status and collection list say so.
    fn collection_changed(&self, sh: &Arc<Shared>, collection: Collection) {
        let Some((_, rt)) = self.show() else {
            return;
        };
        let sh2 = sh.clone();
        rt.block_on(async move {
            let _w = sh2.codex_writer().exclusive().await;
            let codex = sh2.codex();
            if codex.config.role != CodexRole::Client || codex.active != collection.id {
                return;
            }
            let mut s = (*codex).clone();
            let Some(c) = s
                .collections
                .iter_mut()
                .find(|c| c.collection.id == collection.id)
            else {
                return;
            };
            if c.collection == collection {
                return;
            }
            c.collection = collection.clone();
            sh2.set_codex(Arc::new(s));
            super::writer::codex_changed(
                &sh2,
                &[CodexPart::Status, CodexPart::Collections],
                Some(collection.id),
            );
        });
    }

    /// Pins the items the shown collection's playlists and bins use (when they changed).
    fn pin(&self, sh: &Shared) {
        if sh.codex().config.role != CodexRole::Client {
            return;
        }
        let Some(cache) = crate::lock(&self.linked).cache.clone() else {
            return;
        };
        let items = scheduled_items(&sh.data().content);
        let mut l = crate::lock(&self.linked);
        if l.pins == items {
            return;
        }
        let ids: Vec<String> = items.iter().cloned().collect();
        cache.pin(&ids);
        tracing::debug!(items = ids.len(), "Codex mirror: scheduled media pinned");
        l.pins = items;
    }

    /// `codexChanged` (status) when the status changed meaningfully (not on every byte).
    fn announce(&self, sh: &Shared) {
        let Some(s) = self.status(sh) else { return };
        let key = StatusKey {
            reachable: s.reachable,
            seq: s.seq,
            codex_seq: s.codex_seq,
            problem: s.problem.clone(),
            media: (
                s.media.pinned,
                s.media.cached,
                s.media.copying,
                s.media.missing,
                s.media.failed,
            ),
        };
        let mut a = crate::lock(&self.announced);
        if a.as_ref() != Some(&key) {
            *a = Some(key);
            drop(a);
            super::writer::codex_changed(sh, &[CodexPart::Status], None);
        }
    }
}

/// Media of the shown collection a cue uses: the library items not on this box yet are
/// wanted now (copied before the pinned ones still missing). Their names, for a notice. (An
/// item the cache doesn't list yet, just added to the Codex, counts: it is copied once the
/// cache has caught up with the library.)
pub fn want(sh: &Shared, assets: &[model::MediaAssetId]) -> Vec<String> {
    if !sh.library().is_remote() {
        return Vec::new();
    }
    let data = sh.data();
    let mut copying = Vec::new();
    for a in assets {
        let Some(asset) = data.content.assets.get(a) else {
            continue;
        };
        // (A prepared asset whose copy is still copying plays its original meanwhile.)
        if let AssetLocation::Library { id, .. } = &asset.location
            && !sh.library().want(id)
            && data.content.asset_path(*a).is_err()
        {
            copying.push(asset.name.clone());
        }
    }
    copying
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_codex_box_addresses() {
        assert_eq!(host_of("10.0.0.5:5959").as_deref(), Some("10.0.0.5"));
        assert_eq!(host_of("site-server").as_deref(), Some("site-server"));
        assert_eq!(host_of("site-server:5959").as_deref(), Some("site-server"));
        assert_eq!(host_of("[fe80::1]:5959").as_deref(), Some("fe80::1"));
        assert_eq!(host_of("fe80::1").as_deref(), Some("fe80::1"));
        assert_eq!(host_of(""), None);
        assert_eq!(base_url("10.0.0.5", 7878), "http://10.0.0.5:7878");
        assert_eq!(base_url("fe80::1", 7878), "http://[fe80::1]:7878");
    }

    #[test]
    fn documents_of_a_newer_schema_are_held_back() {
        let newer = model::NEWEST_SCHEMA_VERSION + 1;
        assert_eq!(
            too_new(&format!(r#"{{"schemaVersion": {newer}, "id": "x"}}"#)),
            Some(newer)
        );
        let current = model::NEWEST_SCHEMA_VERSION;
        assert_eq!(too_new(&format!(r#"{{"schemaVersion": {current}}}"#)), None);
        assert_eq!(too_new(r#"{"name": "no version"}"#), None);
        // The held schema survives in the state file; an old state file reads without it.
        let st = MirrorState {
            codex_id: CodexId::new(),
            seq: 7,
            held: Some(newer),
        };
        let b = serde_json::to_vec(&st).unwrap();
        assert_eq!(serde_json::from_slice::<MirrorState>(&b).unwrap(), st);
        let old: MirrorState =
            serde_json::from_str(&format!(r#"{{"codexId": "{}", "seq": 3}}"#, st.codex_id.0))
                .unwrap();
        assert_eq!(old.held, None);
    }
}
