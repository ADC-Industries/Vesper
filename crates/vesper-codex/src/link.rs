//! The box-to-Codex link over IDN's `channel/1` (M8): the requests a show box sends the site's
//! Codex, and the Codex's side of them.
//!
//! **One channel on the Codex box.** The Codex serves its site with its own membership
//! ([`super::member`]) on one channel whose handler ([`CodexHandler`]) answers Vesper's request
//! kinds (`vesper.codex.*`, below) and hands every other request, file and upload to the media
//! library (IDN's `LibraryServer`, when open). One fingerprint for documents and media. Only
//! machines of the site reach the channel (IDN checks the device certificate); Vesper's kinds
//! are refused to a device that isn't listed (or is revoked) in the site's Site Manager.
//!
//! **Requests** (JSON, versioned by `v`, [`VERSION`]; documents travel as their files' bytes,
//! so a show box keeps exactly what the Codex has):
//!
//! | Kind | Request | Answer |
//! |---|---|---|
//! | [`HELLO`] | [`HelloRequest`] `{v, collection?}` | [`Hello`]: the Codex's ID, its server's ID and name, the schema versions it reads and writes, every collection with its change-log position |
//! | [`CHANGES`] | [`ChangesRequest`] `{v, collection, since, limit?}` | `CollectionChanges` (the change log after `since`; `resync` below its floor) |
//! | [`INDEX`] | [`IndexRequest`] `{v, collection}` | [`Index`]: the change-log position, then every document's kind, ID and SHA-256 (the bulk snapshot's list) |
//! | [`DOCS`] | [`DocsRequest`] `{v, collection, kind, ids}` | [`Docs`]: the documents' files with their SHA-256; `missing` ones aren't there; ids in neither list didn't fit in this answer (ask again) |
//! | [`CHANGED`] | [`Changed`] `{v, collection, seq}` | the Codex tells each show box following `collection` that it changed (sent by the Codex; a box polls too) |
//! | `backup_offer`, `pending_restore`, `restore_begin`, `restore_result` | the show box's settings backups and restores ([`super::backups::remote`]) | |
//! | [`RESTORE_QUEUED`] | `{v}` | the Codex tells a show box a restore waits for it (the box asks for it at once; its poll is the fallback) |
//! | `adopt`, `adopt_docs`, `adopt_finish`, `upload_begin`, `upload_chunk`, `upload_finish` | a show box uploads its own, dormant collection ([`super::adopt`]) | |
//! | `presence`, `lock`, `presence_changed` | site-wide presence and edit locks (the server's `codex::site_presence`) | |
//! | `write`, `search` | a show box's forwarded REST write, or a search beyond its collection ([`super::write_through`]) | the Codex's REST answer; `SearchResults` |
//!
//! Requests that run async Codex code (backups, restores) are answered from the server's
//! runtime ([`Link::run`]); errors come back as the `ApiError`'s JSON.
//!
//! The bulk snapshot of a collection is [`INDEX`] then [`DOCS`] in pages (a few thousand JSON
//! files: no channel file needed). The index's position is read before the files are listed,
//! so following the change log from there misses nothing.
//!
//! **Pushes.** After a write is in a collection's change log the Codex sends [`CHANGED`] to the
//! show boxes that asked about that collection in the last [`MIRROR_TTL`] (one request each, on
//! a thread of their own, short timeouts: a box that doesn't answer polls anyway).

use crate::Shared;
use crate::backups::remote;
use crate::{log, write_through};
use idn_open_protocol::channel::{
    Channel, ChannelConfig, FileInfo, FileSink, Handler, Peer, Reply,
};
use idn_open_protocol::trust::Enrollment;
use model::{CodexId, Collection, CollectionId, DocumentKind};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

/// `vesper.codex.hello`.
pub const HELLO: &str = "vesper.codex.hello";
/// `vesper.codex.changes`.
pub const CHANGES: &str = "vesper.codex.changes";
/// `vesper.codex.index`.
pub const INDEX: &str = "vesper.codex.index";
/// `vesper.codex.docs`.
pub const DOCS: &str = "vesper.codex.docs";
/// `vesper.codex.changed` (Codex → show box).
pub const CHANGED: &str = "vesper.codex.changed";
/// `vesper.codex.presence`: a show box's presence report (the server's presence, see
/// `site_presence` in the server).
pub const PRESENCE: &str = "vesper.codex.presence";
/// `vesper.codex.lock`: a show box's edit lock request.
pub const LOCK: &str = "vesper.codex.lock";
/// `vesper.codex.presence_changed` (Codex → show box): the site's presence list changed.
pub const PRESENCE_CHANGED: &str = "vesper.codex.presence_changed";
/// `vesper.codex.restore_queued` (Codex → show box): a settings restore waits for the box,
/// which asks for it at once (its poll is the fallback).
pub const RESTORE_QUEUED: &str = "vesper.codex.restore_queued";
/// The link's version (`v` in every request and answer).
pub const VERSION: u32 = 1;
/// A [`DOCS`] answer stops adding documents past this many bytes (the channel's payload limit
/// is 16 MiB).
pub const DOCS_BUDGET: usize = 8 << 20;
/// A show box that hasn't asked anything for this long gets no more pushes.
pub const MIRROR_TTL: Duration = Duration::from_secs(600);
/// How long a push waits for the box's answer.
const PUSH_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a request answered from the runtime may take on the Codex.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

fn v1() -> u32 {
    VERSION
}

/// [`HELLO`]'s request.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The collection the box follows (for the Codex's pushes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<CollectionId>,
}

/// [`HELLO`]'s answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    /// The link's version.
    pub v: u32,
    /// The Codex's ID.
    pub codex_id: CodexId,
    /// The server ID of the box holding it.
    pub server_id: String,
    /// That server's name.
    #[serde(default)]
    pub server_name: String,
    /// The port of its REST API (where uploads and Codex management go).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_port: Option<u16>,
    /// Base URLs of its REST API it knows of itself (its listening address when it isn't a
    /// wildcard).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<String>,
    /// The newest document schema it reads.
    pub reads: u32,
    /// The newest document schema it writes.
    pub writes: u32,
    /// Its collections, in its order.
    pub collections: Vec<HelloCollection>,
}

/// One collection in [`Hello`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloCollection {
    /// The collection document.
    pub collection: Collection,
    /// Its change log's newest position.
    pub seq: u64,
}

/// [`CHANGES`]'s request (the answer is `protocol::CollectionChanges`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The collection.
    pub collection: CollectionId,
    /// Changes after this position.
    pub since: u64,
    /// At most this many (default and most: the change log's page, 1000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// [`INDEX`]'s request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The collection.
    pub collection: CollectionId,
}

/// [`INDEX`]'s answer.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    /// The link's version.
    pub v: u32,
    /// The change log's newest position, read before the documents were listed.
    pub seq: u64,
    /// Every document of the collection.
    pub docs: Vec<IndexEntry>,
}

/// One document in [`Index`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    /// Its kind's folder name (`presentation`, `mediaAsset`, …).
    pub kind: String,
    /// Its ID.
    pub id: uuid::Uuid,
    /// The SHA-256 of its file.
    pub sha256: String,
}

/// [`DOCS`]'s request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocsRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The collection.
    pub collection: CollectionId,
    /// The documents' kind (its folder name).
    pub kind: String,
    /// Their IDs.
    pub ids: Vec<uuid::Uuid>,
}

/// [`DOCS`]'s answer.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Docs {
    /// The link's version.
    pub v: u32,
    /// The documents found, in the order asked.
    pub docs: Vec<DocBody>,
    /// The documents that aren't there (deleted since).
    #[serde(default)]
    pub missing: Vec<uuid::Uuid>,
}

/// One document in [`Docs`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocBody {
    /// Its ID.
    pub id: uuid::Uuid,
    /// The SHA-256 of `body`.
    pub sha256: String,
    /// The document's file, as the Codex has it.
    pub body: String,
}

/// [`CHANGED`]: the collection's change log moved on.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Changed {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The collection.
    pub collection: CollectionId,
    /// Its change log's newest position.
    pub seq: u64,
}

/// The SHA-256 of `bytes`, hex.
pub fn sha_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The collection kind whose folder is `name` (only the collection kinds).
pub fn collection_kind(name: &str) -> Option<DocumentKind> {
    crate::data::COLLECTION_KINDS
        .iter()
        .copied()
        .find(|k| crate::data::kind_dir(*k) == name)
}

/// An answer as JSON.
fn ok<T: Serialize>(v: &T) -> Reply {
    match serde_json::to_vec(v) {
        Ok(b) => Reply::Ok(b),
        Err(e) => Reply::Error(e.to_string()),
    }
}

fn parse<'a, T: Deserialize<'a>>(payload: &'a [u8]) -> Result<T, Reply> {
    serde_json::from_slice(payload).map_err(|e| Reply::Error(format!("bad request: {e}")))
}

/// A served file's SHA-256 as last worked out: (size, modification time, SHA-256).
type Hashed = (u64, Option<SystemTime>, String);

/// A show box following a collection, as the Codex knows it from the box's requests (G14).
#[derive(Clone, Debug)]
pub struct MirrorBox {
    /// The collection it follows.
    pub collection: CollectionId,
    /// The change-log position it has been sent (its `changes` answer's `to`, or the index's
    /// position); `None` until it asked for changes or the index.
    pub seq: Option<u64>,
    /// Its name in the site (the channel peer's).
    pub name: String,
    /// When it last asked (for pushes).
    pub at: Instant,
    /// The same, as a time (for the status).
    pub seen: SystemTime,
}

/// How long a show box that stopped asking stays listed in the status (as not online).
pub const LISTED_FOR: Duration = Duration::from_secs(24 * 3600);

/// The Codex's side of the link (one per server, in [`crate::Server::codex_link`]; see the module
/// docs). Runs while the Codex is a member of its site.
pub struct Link {
    me: Weak<Link>,
    /// The channel, and the membership's fingerprint it runs with.
    serving: Mutex<Option<(String, Arc<Channel>)>>,
    serve_failed: AtomicBool,
    /// A show box started following, switched collection or came back ([`Link::take_boxes_changed`]).
    boxes_changed: AtomicBool,
    /// Show boxes by fingerprint: the collection each follows, how far it has the change log,
    /// and when it last asked.
    mirrors: Mutex<HashMap<String, MirrorBox>>,
    /// Collections changed since the last pushes, with their newest position.
    pending: Mutex<BTreeMap<CollectionId, u64>>,
    wake: Condvar,
    stop: AtomicBool,
    pusher: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// SHA-256s of documents' files, by path (worked out once per change of a file).
    hashes: Mutex<HashMap<PathBuf, Hashed>>,
    /// Push changes to the show boxes ([`super::CodexOptions::push_changes`]).
    push: bool,
    /// The channel's UDP port ([`super::CodexOptions::channel_port`]).
    port: u16,
    /// The server's runtime (the requests that run async Codex code).
    rt: Mutex<Option<tokio::runtime::Handle>>,
    /// The server's REST routes (show boxes' forwarded writes run through them) and the
    /// address they listen on.
    router: Mutex<Option<(axum::Router, std::net::SocketAddr)>>,
}

impl Link {
    /// The link, not serving yet.
    pub fn new(options: &super::CodexOptions) -> Arc<Link> {
        Arc::new_cyclic(|me| Link {
            me: me.clone(),
            serving: Mutex::new(None),
            serve_failed: AtomicBool::new(false),
            boxes_changed: AtomicBool::new(false),
            mirrors: Mutex::new(HashMap::new()),
            pending: Mutex::new(BTreeMap::new()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            pusher: Mutex::new(None),
            hashes: Mutex::new(HashMap::new()),
            push: options.push_changes,
            port: options.channel_port,
            rt: Mutex::new(None),
            router: Mutex::new(None),
        })
    }

    /// Serves the site with the Codex's membership `member` (one channel: Vesper's requests
    /// and the media library's), or stops serving without one (every site tick).
    pub fn follow(&self, sh: &Arc<Shared>, member: Option<Arc<Enrollment>>) {
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            crate::lock(&self.rt).get_or_insert(rt);
        }
        let mut s = crate::lock(&self.serving);
        let Some(member) = member.filter(|_| !self.stop.load(Ordering::Relaxed)) else {
            if s.take().is_some() {
                tracing::info!("Codex: no longer served to the site");
            }
            return;
        };
        let fp = member.fingerprint();
        if s.as_ref().is_some_and(|(f, _)| *f == fp) {
            return;
        }
        drop(s.take());
        let handler = Arc::new(CodexHandler {
            sh: Arc::downgrade(sh),
            link: self.me.clone(),
        });
        let cfg = ChannelConfig {
            port: self.port,
            ..ChannelConfig::new(member)
        };
        match Channel::start(cfg, handler) {
            Ok(ch) => {
                tracing::info!(fingerprint = %fp, port = ch.port(), "Codex: served to the site (documents and media library)");
                self.serve_failed.store(false, Ordering::Relaxed);
                *s = Some((fp, ch));
                drop(s);
                self.start_pusher();
            }
            Err(e) => {
                if !self.serve_failed.swap(true, Ordering::Relaxed) {
                    tracing::warn!("Codex: serving the site: {e}");
                }
            }
        }
    }

    /// The Codex is served to the site now (its channel runs).
    pub fn serving(&self) -> bool {
        crate::lock(&self.serving).is_some()
    }

    /// The channel's UDP port, while serving.
    pub fn port(&self) -> Option<u16> {
        crate::lock(&self.serving).as_ref().map(|(_, c)| c.port())
    }

    /// The show boxes following `collection` now (fingerprints).
    pub fn mirrors_of(&self, collection: CollectionId) -> Vec<String> {
        crate::lock(&self.mirrors)
            .iter()
            .filter(|(_, b)| b.collection == collection && b.at.elapsed() < MIRROR_TTL)
            .map(|(fp, _)| fp.clone())
            .collect()
    }

    /// A collection's change log moved on to `seq`: the show boxes following it are told.
    pub fn changed(&self, collection: CollectionId, seq: u64) {
        if !self.push {
            return;
        }
        let mut p = crate::lock(&self.pending);
        let e = p.entry(collection).or_default();
        *e = (*e).max(seq);
        self.wake.notify_all();
    }

    /// Runs `fut` on the server's runtime and waits for it (on the channel's thread): its
    /// answer as JSON, or its error as the `ApiError`'s JSON.
    pub fn run<T, F>(&self, fut: F) -> Reply
    where
        T: Serialize + Send + 'static,
        F: std::future::Future<Output = Result<T, protocol::ApiError>> + Send + 'static,
    {
        let Some(rt) = crate::lock(&self.rt).clone() else {
            return Reply::Error("the Codex isn't running".into());
        };
        if self.stop.load(Ordering::Relaxed) {
            return Reply::Error("the Codex's server is stopping".into());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        rt.spawn(async move {
            let _ = tx.send(fut.await);
        });
        match rx.recv_timeout(RUN_TIMEOUT) {
            Ok(Ok(v)) => ok(&v),
            Ok(Err(e)) => super::backups::remote::error_reply(&e),
            Err(_) => Reply::Error("the Codex took too long to answer".into()),
        }
    }

    /// The server is stopping.
    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// The channel, while serving.
    pub fn channel(&self) -> Option<Arc<Channel>> {
        crate::lock(&self.serving).as_ref().map(|(_, c)| c.clone())
    }

    /// The REST routes (once the server listens).
    pub fn router(&self) -> Option<axum::Router> {
        crate::lock(&self.router).as_ref().map(|(r, _)| r.clone())
    }

    /// The REST routes and their address (set once the server listens).
    pub fn set_router(&self, router: axum::Router, addr: std::net::SocketAddr) {
        *crate::lock(&self.router) = Some((router, addr));
    }

    /// Tells the show box `fingerprint` that a settings restore waits for it (on a thread of
    /// its own; a box that doesn't answer takes it at its next poll).
    pub fn restore_queued(&self, fingerprint: &str) {
        let Some(channel) = crate::lock(&self.serving).as_ref().map(|(_, c)| c.clone()) else {
            return;
        };
        let fp = fingerprint.to_string();
        let body = serde_json::to_vec(&serde_json::json!({ "v": VERSION })).unwrap_or_default();
        let _ = std::thread::Builder::new()
            .name("codex-push-one".into())
            .spawn(move || {
                if let Err(e) = channel.request(&fp, RESTORE_QUEUED, &body, PUSH_TIMEOUT) {
                    tracing::debug!(box_fp = %fp, "Codex: telling a show box of its restore: {e}");
                }
            });
    }

    /// The server stops: no more serving or pushing.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        drop(crate::lock(&self.rt).take());
        drop(crate::lock(&self.router).take());
        drop(crate::lock(&self.serving).take());
        self.wake.notify_all();
        let h = crate::lock(&self.pusher).take();
        if let Some(h) = h
            && h.thread().id() != std::thread::current().id()
        {
            let _ = h.join();
        }
    }

    /// The show boxes that asked in the last [`LISTED_FOR`], by fingerprint (G14).
    pub fn mirror_boxes(&self) -> Vec<(String, MirrorBox)> {
        let mut v: Vec<(String, MirrorBox)> = crate::lock(&self.mirrors)
            .iter()
            .filter(|(_, b)| b.at.elapsed() < LISTED_FOR)
            .map(|(f, b)| (f.clone(), b.clone()))
            .collect();
        v.sort_by(|a, b| {
            a.1.name
                .to_lowercase()
                .cmp(&b.1.name.to_lowercase())
                .then_with(|| a.0.cmp(&b.0))
        });
        v
    }

    /// A show box asked about `collection` (`seq`: the change-log position it has been sent
    /// now, if this request says). Oriels are told (`codexChanged {status, collections}`, see
    /// [`Link::take_boxes_changed`]) when the box starts following, switches collection, comes
    /// back or moves on in the change log.
    fn note_mirror(&self, from: &Peer, collection: CollectionId, seq: Option<u64>) {
        let mut m = crate::lock(&self.mirrors);
        m.retain(|_, b| b.at.elapsed() < LISTED_FOR);
        let old = m.get(&from.fingerprint).cloned();
        let news = old.as_ref().is_none_or(|o| {
            o.collection != collection
                || o.at.elapsed() >= MIRROR_TTL
                || (seq.is_some() && o.seq != seq)
        });
        let seq = match (&old, seq) {
            (_, Some(s)) => Some(s),
            (Some(o), None) if o.collection == collection => o.seq,
            _ => None,
        };
        m.insert(
            from.fingerprint.clone(),
            MirrorBox {
                collection,
                seq,
                name: from.name.clone(),
                at: Instant::now(),
                seen: SystemTime::now(),
            },
        );
        drop(m);
        if news {
            self.boxes_changed.store(true, Ordering::Relaxed);
        }
    }

    /// Whether a box started following, switched or came back since the last call.
    pub fn take_boxes_changed(&self) -> bool {
        self.boxes_changed.swap(false, Ordering::Relaxed)
    }

    fn start_pusher(&self) {
        let mut p = crate::lock(&self.pusher);
        if p.is_some() || !self.push {
            return;
        }
        let me = self.me.clone();
        match std::thread::Builder::new()
            .name("codex-push".into())
            .spawn(move || push_loop(me))
        {
            Ok(h) => *p = Some(h),
            Err(e) => tracing::warn!("Codex: the push thread didn't start: {e}"),
        }
    }

    /// The SHA-256 of a document's file (worked out again only when it changed).
    fn file_sha(&self, path: &Path) -> std::io::Result<String> {
        let meta = std::fs::metadata(path)?;
        let key = (meta.len(), meta.modified().ok());
        if let Some((len, at, sha)) = crate::lock(&self.hashes).get(path)
            && (*len, *at) == key
        {
            return Ok(sha.clone());
        }
        let sha = sha_hex(&std::fs::read(path)?);
        let mut h = crate::lock(&self.hashes);
        if h.len() > 100_000 {
            h.clear();
        }
        h.insert(path.to_path_buf(), (key.0, key.1, sha.clone()));
        Ok(sha)
    }
}

/// Sends [`CHANGED`] to the show boxes following each changed collection.
fn push_loop(link: Weak<Link>) {
    loop {
        let Some(l) = link.upgrade() else { return };
        let pending = {
            let mut p = crate::lock(&l.pending);
            while p.is_empty() && !l.stop.load(Ordering::Relaxed) {
                p = l
                    .wake
                    .wait_timeout(p, Duration::from_secs(1))
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
            std::mem::take(&mut *p)
        };
        if l.stop.load(Ordering::Relaxed) {
            return;
        }
        let Some(channel) = crate::lock(&l.serving).as_ref().map(|(_, c)| c.clone()) else {
            continue;
        };
        for (collection, seq) in pending {
            for fp in l.mirrors_of(collection) {
                let ch = channel.clone();
                let body = serde_json::to_vec(&Changed {
                    v: VERSION,
                    collection,
                    seq,
                })
                .unwrap_or_default();
                // (A box that doesn't answer polls anyway: each push on its own thread.)
                let _ = std::thread::Builder::new()
                    .name("codex-push-one".into())
                    .spawn(move || match ch.request(&fp, CHANGED, &body, PUSH_TIMEOUT) {
                        Ok(Reply::Ok(_)) => {}
                        Ok(r) => tracing::debug!(box_fp = %fp, "Codex: a show box answered a push: {r:?}"),
                        Err(e) => tracing::debug!(box_fp = %fp, "Codex: pushing a change: {e}"),
                    });
            }
        }
        drop(l);
    }
}

/// The Codex's channel handler: Vesper's requests, and the media library's (see the module
/// docs).
pub struct CodexHandler {
    sh: Weak<Shared>,
    link: Weak<Link>,
}

impl CodexHandler {
    /// `from` is a machine of this site, as the Site Manager lists it now.
    fn member(&self, sh: &Shared, from: &Peer) -> bool {
        sh.site().handle().is_some_and(|h| {
            h.authority()
                .devices()
                .iter()
                .any(|d| d.fingerprint == from.fingerprint && !d.revoked)
        })
    }

    fn vesper(&self, sh: &Shared, link: &Link, from: &Peer, kind: &str, payload: &[u8]) -> Reply {
        if !self.member(sh, from) {
            tracing::warn!(fingerprint = %from.fingerprint, name = %from.name, "Codex: {kind} from a device the site doesn't list: refused");
            return Reply::Refused;
        }
        if !sh.codex_is_host() {
            return Reply::Error("this box doesn't host the site's Codex now".into());
        }
        let r = match kind {
            HELLO => self.hello(sh, link, from, payload),
            CHANGES => self.changes(sh, link, from, payload),
            INDEX => self.index(sh, link, from, payload),
            DOCS => self.docs(sh, link, payload),
            remote::BACKUP_OFFER
            | remote::PENDING_RESTORE
            | remote::RESTORE_BEGIN
            | remote::RESTORE_RESULT => self.backups(link, from, kind, payload),
            write_through::WRITE => self.write(link, payload),
            PRESENCE | LOCK => Ok(sh.presence_serve(&from.fingerprint, kind, payload)),
            k if super::adopt::KINDS.contains(&k) => match self.sh.upgrade() {
                Some(sh) => Ok(super::adopt::serve(sh, link, k, payload)),
                None => Err(Reply::Unknown),
            },
            write_through::SEARCH => self.search(link, payload),
            _ => return Reply::Unknown,
        };
        if link.take_boxes_changed() {
            super::writer::codex_changed(
                sh,
                &[
                    protocol::CodexPart::Status,
                    protocol::CodexPart::Collections,
                ],
                None,
            );
        }
        r.unwrap_or_else(|e| e)
    }

    /// A show box's forwarded REST write ([`write_through::execute`]).
    fn write(&self, link: &Link, payload: &[u8]) -> Result<Reply, Reply> {
        let sh = self.sh.upgrade().ok_or(Reply::Unknown)?;
        let req: write_through::WriteRequest = parse(payload)?;
        let Some((router, _)) = crate::lock(&link.router).clone() else {
            return Err(Reply::Error("the Codex's routes aren't up yet".into()));
        };
        Ok(link.run(write_through::execute(sh, router, req)))
    }

    /// A show box's search beyond its own collection.
    fn search(&self, link: &Link, payload: &[u8]) -> Result<Reply, Reply> {
        let sh = self.sh.upgrade().ok_or(Reply::Unknown)?;
        let req: write_through::SearchRequest = parse(payload)?;
        let params = super::search::Params {
            q: req.q,
            collection: req.collection,
            kinds: req.kinds,
            limit: req.limit,
        };
        Ok(link.run(async move {
            tokio::task::spawn_blocking(move || super::search::run(&sh, &params))
                .await
                .map_err(|e| {
                    protocol::ApiError::new(protocol::ErrorCode::Internal, e.to_string())
                })?
        }))
    }

    /// A show box's settings backups and restores ([`remote`]).
    fn backups(
        &self,
        link: &Link,
        from: &Peer,
        kind: &str,
        payload: &[u8],
    ) -> Result<Reply, Reply> {
        let sh = self.sh.upgrade().ok_or(Reply::Unknown)?;
        let fp = from.fingerprint.clone();
        Ok(match kind {
            remote::BACKUP_OFFER => {
                let offer = parse::<remote::OfferWire>(payload)?.into_offer();
                link.run(async move { sh.backups().accept_from_box(&sh, &fp, offer, None).await })
            }
            remote::PENDING_RESTORE => link.run(async move {
                let restore = sh.backups().pending_for(&sh, &fp).await?;
                Ok(remote::PendingAnswer {
                    v: VERSION,
                    restore,
                })
            }),
            remote::RESTORE_BEGIN => {
                let req: remote::BeginRequest = parse(payload)?;
                link.run(async move {
                    let (package, before) = sh
                        .backups()
                        .begin_restore(&sh, &fp, &req.id, req.offer.into_offer())
                        .await?;
                    remote::BeginAnswer::new(&package, before)
                })
            }
            _ => {
                let req: remote::ResultRequest = parse(payload)?;
                link.run(async move {
                    sh.backups().finish_restore(&sh, &fp, req).await?;
                    Ok(serde_json::json!({}))
                })
            }
        })
    }

    fn hello(&self, sh: &Shared, link: &Link, from: &Peer, payload: &[u8]) -> Result<Reply, Reply> {
        let req: HelloRequest = if payload.is_empty() {
            HelloRequest::default()
        } else {
            parse(payload)?
        };
        let codex = sh.codex();
        if let Some(c) = req.collection.filter(|c| codex.collection(*c).is_some()) {
            link.note_mirror(from, c, None);
        }
        let identity = sh.auth().identity();
        let listening = crate::lock(&link.router).as_ref().map(|(_, a)| *a);
        let collections = codex
            .collections
            .iter()
            .filter(|c| !c.too_new)
            .map(|c| HelloCollection {
                collection: c.collection.clone(),
                seq: sh.codex_writer().head(sh, c.collection.id),
            })
            .collect();
        Ok(ok(&Hello {
            v: VERSION,
            codex_id: codex.info.id,
            server_id: identity.id.to_string(),
            server_name: identity.name,
            api_port: listening.map(|a| a.port()),
            addresses: listening
                .filter(|a| !a.ip().is_unspecified())
                .map(|a| vec![format!("http://{a}")])
                .unwrap_or_default(),
            // The newest it writes: documents holding what needs a newer schema are written
            // with it (`model::version_for`).
            reads: model::NEWEST_SCHEMA_VERSION,
            writes: model::NEWEST_SCHEMA_VERSION,
            collections,
        }))
    }

    /// The collection's folder, if it is a live collection of the Codex.
    fn collection_dir(sh: &Shared, id: CollectionId) -> Result<PathBuf, Reply> {
        let codex = sh.codex();
        if codex.collection(id).is_none() {
            return Err(Reply::Error(format!("collection {id} is not in the Codex")));
        }
        Ok(codex.layout.collection(id))
    }

    fn changes(
        &self,
        sh: &Shared,
        link: &Link,
        from: &Peer,
        payload: &[u8],
    ) -> Result<Reply, Reply> {
        let req: ChangesRequest = parse(payload)?;
        let dir = Self::collection_dir(sh, req.collection)?;
        let limit = req.limit.unwrap_or(log::MAX_LIMIT);
        let answer = log::since(&log::path(&dir), req.since, limit)
            .map_err(|e| Reply::Error(format!("reading the change log: {e}")))?;
        // A resync answer sends nothing: the box goes on with the index.
        let sent = (!answer.resync).then_some(answer.to.max(req.since));
        link.note_mirror(from, req.collection, sent);
        Ok(ok(&answer))
    }

    fn index(&self, sh: &Shared, link: &Link, from: &Peer, payload: &[u8]) -> Result<Reply, Reply> {
        let req: IndexRequest = parse(payload)?;
        let dir = Self::collection_dir(sh, req.collection)?;
        // The position first: every write is on disk before it is logged, so the files listed
        // next are at least this new.
        let seq = sh.codex_writer().head(sh, req.collection);
        link.note_mirror(from, req.collection, Some(seq));
        let mut docs = Vec::new();
        for kind in crate::data::COLLECTION_KINDS {
            let name = crate::data::kind_dir(*kind);
            let Ok(rd) = std::fs::read_dir(dir.join("portable").join(&name)) else {
                continue;
            };
            let mut files: Vec<(uuid::Uuid, PathBuf)> = rd
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    let id = p
                        .file_name()?
                        .to_str()?
                        .strip_suffix(".json")?
                        .parse()
                        .ok()?;
                    Some((id, p))
                })
                .collect();
            files.sort();
            for (id, p) in files {
                // (A file gone meanwhile: the change log says so.)
                if let Ok(sha256) = link.file_sha(&p) {
                    docs.push(IndexEntry {
                        kind: name.clone(),
                        id,
                        sha256,
                    });
                }
            }
        }
        Ok(ok(&Index {
            v: VERSION,
            seq,
            docs,
        }))
    }

    fn docs(&self, sh: &Shared, _link: &Link, payload: &[u8]) -> Result<Reply, Reply> {
        let req: DocsRequest = parse(payload)?;
        let dir = Self::collection_dir(sh, req.collection)?;
        if collection_kind(&req.kind).is_none() {
            return Err(Reply::Error(format!("{}: not a collection kind", req.kind)));
        }
        let kind_dir = dir.join("portable").join(&req.kind);
        let mut out = Docs {
            v: VERSION,
            ..Docs::default()
        };
        let mut bytes = 0;
        for id in req.ids {
            match std::fs::read(kind_dir.join(format!("{id}.json"))) {
                Ok(b) => {
                    if bytes > 0 && bytes + b.len() > DOCS_BUDGET {
                        // The rest in the next answer.
                        break;
                    }
                    let Ok(body) = String::from_utf8(b) else {
                        out.missing.push(id);
                        continue;
                    };
                    bytes += body.len();
                    out.docs.push(DocBody {
                        id,
                        sha256: sha_hex(body.as_bytes()),
                        body,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => out.missing.push(id),
                Err(e) => return Err(Reply::Error(format!("reading {id}: {e}"))),
            }
        }
        Ok(ok(&out))
    }
}

impl Handler for CodexHandler {
    fn on_request(&self, from: &Peer, kind: &str, payload: Vec<u8>) -> Reply {
        let (Some(sh), Some(link)) = (self.sh.upgrade(), self.link.upgrade()) else {
            return Reply::Unknown;
        };
        if kind.starts_with("vesper.codex.") {
            return self.vesper(&sh, &link, from, kind, &payload);
        }
        match sh.library().server() {
            Some(lib) => lib.request(from, kind, &payload).unwrap_or(Reply::Unknown),
            None => Reply::Unknown,
        }
    }

    fn file(&self, _from: &Peer, id: &str) -> Option<PathBuf> {
        self.sh.upgrade()?.library().server()?.file(id)
    }

    fn on_push(&self, from: &Peer, offer: &FileInfo) -> Option<Box<dyn FileSink>> {
        self.sh.upgrade()?.library().server()?.on_push(from, offer)
    }

    fn on_received(&self, _from: &Peer, offer: &FileInfo, result: std::io::Result<()>) {
        if let Some(lib) = self.sh.upgrade().and_then(|sh| sh.library().server()) {
            lib.on_received(offer, result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_collection_kinds_are_served() {
        assert_eq!(
            collection_kind("presentation"),
            Some(DocumentKind::Presentation)
        );
        assert_eq!(
            collection_kind("mediaAsset"),
            Some(DocumentKind::MediaAsset)
        );
        for bad in [
            "screen",
            "look",
            "outputBinding",
            "../presentation",
            "",
            "codexConfig",
        ] {
            assert_eq!(collection_kind(bad), None, "{bad}");
        }
    }

    #[test]
    fn requests_read_without_a_version_and_answers_are_camel_case() {
        let r: ChangesRequest = serde_json::from_str(
            r#"{"collection":"6a1d1c8e-0000-4000-8000-0000000000b0","since":3}"#,
        )
        .unwrap();
        assert_eq!((r.v, r.since, r.limit), (VERSION, 3, None));
        let i = serde_json::to_value(Index {
            v: VERSION,
            seq: 7,
            docs: vec![IndexEntry {
                kind: "presentation".into(),
                id: uuid::Uuid::nil(),
                sha256: sha_hex(b"{}"),
            }],
        })
        .unwrap();
        assert_eq!(i["docs"][0]["sha256"].as_str().unwrap().len(), 64);
        assert_eq!(i["seq"], 7);
        let h: HelloRequest = serde_json::from_str("{}").unwrap();
        assert_eq!((h.v, h.collection), (VERSION, None));
    }
}
