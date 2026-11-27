//! The Codex's media library: IDN's site media library (`idn_open_protocol::library`), hosted
//! by the Codex box in `<data>/codex/library/` (owner, 2026-11-22: **IDN hosts the media
//! files**). Media assets refer to its items as [`AssetLocation::Library`]`{id, sha256}`: the
//! item's lasting ID (it survives renames and moves) and the SHA-256 of its file.
//!
//! - **Opening** ([`MediaLibrary::ensure_open`], at every load of the Codex on the box that
//!   hosts it): `LibraryServer::open(<data>/codex/library, "Codex")`. IDN refuses a library whose
//!   index can't be read; the Codex then says so in its status (`mediaLibrary.problem`), its
//!   library media are unavailable, and nothing is wiped or recreated. A later load (a reload,
//!   a restart) tries again.
//! - **Serving**: once the Codex is a member of its site ([`super::member`]), the library is
//!   served to the site's machines on the Codex's channel ([`super::link`]), which answers the
//!   library's requests and the box-to-Codex link's.
//! - **Show boxes** (Codex role `client`) have no library of their own: their library items
//!   come from IDN's cache of the site's Codex's library ([`MediaLibrary::set_remote`],
//!   [`super::mirror`]); an item not copied yet is unavailable (never fetched while it plays).
//! - **Paths**: an asset's file is the item's file now. The Codex keeps a copy of IDN's
//!   item → file map ([`MediaLibrary::resolve`]), brought up to date by following the library's
//!   changes ([`follow`]), so the render threads never wait on IDN's index. Items the
//!   collections use that change or vanish are announced (`codexChanged` with `media`).
//! - **Probing**: IDN probes every item once in the background with the Codex's prober (the
//!   media crate's probe); the probe of an asset after an upload or a relink shares it
//!   ([`ProbeMemo`]: one probe per file, whoever asks first).
//! - **In use**: the Codex marks every item a document uses (holder [`HOLDER`],
//!   [`MediaLibrary::set_marks`]); IDN refuses to delete a marked item unless a person types
//!   its name back. The Codex never deletes library items itself.

use crate::blobs;
use idn_open_protocol::library::cache::{Cache, ItemState};
use idn_open_protocol::library::server::LibraryServer;
use model::{CollectionId, MediaKind, MediaProbe};
use protocol::{CodexPart, ServerMessage};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::Duration;

/// `<data>/codex/<DIR>`: the library's folder.
pub const DIR: &str = "library";
/// The library's name in the site.
pub const NAME: &str = "Codex";
/// The Codex's name as the holder of "in use" marks.
pub const HOLDER: &str = "vesper-codex";
/// The most items one holder marks in use (IDN Beta8: 10 000 per holder).
pub const MAX_MARKS: usize = 10_000;
/// How long one wait for the library's changes lasts (the follower looks at `stop` between:
/// a stopping server lets go of the library within it).
const FOLLOW_WAIT: Duration = Duration::from_millis(500);

/// One item as the Codex knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemFile {
    /// Its file now.
    pub path: PathBuf,
    /// Its SHA-256, once IDN worked it out.
    pub sha256: Option<String>,
}

enum State {
    /// Not opened (this box doesn't host the Codex, or nothing loaded yet).
    Closed,
    Open(Arc<LibraryServer>),
    /// IDN refused to open it: why.
    Failed(String),
    /// A show box (Codex role `client`): library items come from the site's Codex through
    /// IDN's cache on this box (`None` until the Codex has been found; see
    /// [`super::mirror`]).
    Remote(Option<Arc<Cache>>),
}

/// The Codex's media library (one per server, in [`crate::Server::library`]; see the module
/// docs).
pub struct MediaLibrary {
    state: RwLock<State>,
    /// Item ID → its file, as last looked at.
    items: RwLock<HashMap<String, ItemFile>>,
    /// Item ID → the collections whose documents use it (live ones; from the last marks).
    used: RwLock<HashMap<String, BTreeSet<CollectionId>>>,
    stop: Arc<AtomicBool>,
    /// The thread following the library's changes.
    follower: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The marks are due again (a write changed media references).
    marks_due: tokio::sync::Notify,
    /// Items in use past [`MAX_MARKS`] at the last marks (said once per change).
    marks_over: std::sync::atomic::AtomicUsize,
    events: OnceLock<tokio::sync::broadcast::Sender<Arc<ServerMessage>>>,
    /// Probes of the library's files, shared by IDN's prober and the media worker.
    pub probes: Arc<ProbeMemo>,
}

fn read<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

fn write<T>(l: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

/// Has the media library take every ending Vesper plays ([`blobs::IMAGE_ENDINGS`] and the
/// others) beside its own (`idn_open_protocol::library::take_endings`; once per program), so
/// what Vesper plays, the library takes.
pub fn take_vesper_endings() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| {
        for (endings, kind) in [
            (blobs::IMAGE_ENDINGS, "picture"),
            (blobs::VIDEO_ENDINGS, "video"),
            (blobs::AUDIO_ENDINGS, "sound"),
        ] {
            if let Err(e) = idn_open_protocol::library::take_endings(endings, kind) {
                tracing::error!("the media library won't take Vesper's {kind} endings: {e}");
            }
        }
    });
}

impl MediaLibrary {
    /// A library not opened yet.
    pub fn new() -> Arc<MediaLibrary> {
        take_vesper_endings();
        Arc::new(MediaLibrary {
            state: RwLock::new(State::Closed),
            items: RwLock::new(HashMap::new()),
            used: RwLock::new(HashMap::new()),
            stop: Arc::new(AtomicBool::new(false)),
            follower: Mutex::new(None),
            marks_due: tokio::sync::Notify::new(),
            marks_over: std::sync::atomic::AtomicUsize::new(0),
            events: OnceLock::new(),
            probes: Arc::new(ProbeMemo::default()),
        })
    }

    /// The library of content loaded outside a Codex (tools, tests): never opened.
    pub fn detached() -> Arc<MediaLibrary> {
        static NONE: OnceLock<Arc<MediaLibrary>> = OnceLock::new();
        NONE.get_or_init(MediaLibrary::new).clone()
    }

    /// Where `codexChanged` goes when items the collections use change or vanish.
    pub fn set_events(&self, events: tokio::sync::broadcast::Sender<Arc<ServerMessage>>) {
        let _ = self.events.set(events);
    }

    /// Opens the library at `dir` unless it is open (see the module docs). Blocking: IDN looks
    /// at the folder. A library IDN refuses stays closed, with the reason
    /// ([`MediaLibrary::problem`]); nothing in its folder is touched.
    pub fn ensure_open(self: &Arc<Self>, dir: &Path) {
        if matches!(*read(&self.state), State::Open(_)) || self.stop.load(Ordering::Relaxed) {
            return;
        }
        // A show box woken (it left the site whose Codex it followed): its cache goes.
        if matches!(*read(&self.state), State::Remote(_)) {
            *write(&self.state) = State::Closed;
        }
        match LibraryServer::open(dir, NAME) {
            Ok(server) => {
                let memo = self.probes.clone();
                server.set_prober(Arc::new(move |p: &Path| memo.facts(p)));
                // Changes from here on are followed; the map is read after, so none is lost.
                let (_, since, _) = server.changes(0, Duration::ZERO);
                let mut items = HashMap::new();
                for it in server.catalog().items {
                    if let Some(path) = server.path_of(&it.id) {
                        items.insert(
                            it.id.clone(),
                            ItemFile {
                                path,
                                sha256: it.sha256.clone(),
                            },
                        );
                    }
                }
                tracing::info!(
                    dir = %dir.display(),
                    items = items.len(),
                    "Codex: media library open"
                );
                *write(&self.items) = items;
                *write(&self.state) = State::Open(server.clone());
                let (me, weak) = (Arc::downgrade(self), Arc::downgrade(&server));
                let spawned = std::thread::Builder::new()
                    .name("codex-library".into())
                    .spawn(move || follow(me, weak, since));
                match spawned {
                    Ok(h) => *crate::lock(&self.follower) = Some(h),
                    Err(e) => tracing::error!("following the media library: {e}"),
                }
            }
            Err(e) => {
                let why = if e.kind() == std::io::ErrorKind::AlreadyExists {
                    format!(
                        "the media library is still open elsewhere in this program (a server stopping); it is opened again at the next load: {e}"
                    )
                } else {
                    format!("the media library can't be opened: {e}")
                };
                tracing::error!(dir = %dir.display(), "Codex: {why} (its media are unavailable; nothing was changed)");
                *write(&self.state) = State::Failed(why);
            }
        }
    }

    /// The library, while open.
    pub fn server(&self) -> Option<Arc<LibraryServer>> {
        match &*read(&self.state) {
            State::Open(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Why the library isn't available (`None` while it is open or this box doesn't host one).
    pub fn problem(&self) -> Option<String> {
        match &*read(&self.state) {
            State::Failed(why) => Some(why.clone()),
            _ => None,
        }
    }

    /// The library is open.
    pub fn is_open(&self) -> bool {
        matches!(*read(&self.state), State::Open(_))
    }

    /// The file of item `id` now (from the Codex's copy of the library's map; on a show box,
    /// its copy in the cache, never the network).
    pub fn resolve(&self, id: &str) -> Result<PathBuf, String> {
        match &*read(&self.state) {
            State::Open(_) => {}
            State::Failed(why) => return Err(why.clone()),
            State::Closed => return Err("this box has no media library".into()),
            State::Remote(None) => {
                return Err("not on this box yet (the site's Codex hasn't been reached)".into());
            }
            State::Remote(Some(cache)) => {
                if let Some(p) = cache.path(id) {
                    return Ok(p);
                }
                return Err(match cache.state(id) {
                    ItemState::Missing => "not in the site's media library".to_string(),
                    ItemState::Failed(e) => {
                        format!("copying it to this box failed ({e}); tried again")
                    }
                    _ => "still copying to this box".to_string(),
                });
            }
        }
        read(&self.items)
            .get(id)
            .map(|f| f.path.clone())
            .ok_or_else(|| format!("item {id} is not in the media library"))
    }

    /// An item with this SHA-256 (the first by ID), if the library has one.
    pub fn by_sha(&self, sha: &str) -> Option<(String, PathBuf)> {
        if !self.is_open() {
            return None;
        }
        read(&self.items)
            .iter()
            .filter(|(_, f)| f.sha256.as_deref() == Some(sha))
            .map(|(id, f)| (id.clone(), f.path.clone()))
            .min_by(|a, b| a.0.cmp(&b.0))
    }

    /// Item `id` as the Codex knows it.
    pub fn item(&self, id: &str) -> Option<ItemFile> {
        read(&self.items).get(id).cloned()
    }

    /// `GET /codex/library`: the items whose path or title holds every word of `q` (any case),
    /// of `kind` (`video`, `picture`, `sound`; `None`: all), by path, at most `limit`.
    /// Blocking (IDN looks at the folder for files copied in by hand). `None` while the
    /// library isn't open.
    pub fn list(
        &self,
        q: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Option<protocol::LibraryItems> {
        let server = self.server()?;
        let words: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
        let mut items: Vec<protocol::LibraryItem> = server
            .catalog()
            .items
            .into_iter()
            .filter(|i| kind.is_none_or(|k| i.kind == k))
            .filter(|i| {
                let hay = format!("{} {}", i.path, i.title).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .map(|i| protocol::LibraryItem {
                id: i.id,
                path: i.path,
                title: i.title,
                kind: i.kind,
                size: i.size,
                sha256: i.sha256,
                duration: i.duration,
            })
            .collect();
        items.sort_by_key(|i| i.path.to_lowercase());
        let total = items.len() as u32;
        items.truncate(limit);
        Some(protocol::LibraryItems { items, total })
    }

    /// Every item: ID → its file (a copy).
    pub fn items(&self) -> HashMap<String, ItemFile> {
        read(&self.items).clone()
    }

    /// Takes the file `path` (on this box, its name the item's name) into the library's
    /// `folder`, moved (IDN's `import`; blocking: IDN reads the file for its SHA-256). The
    /// item, and whether the library had the same bytes already (then nothing was moved: the
    /// file is still at `path`).
    pub fn import(&self, path: &Path, folder: &str) -> std::io::Result<(String, String, bool)> {
        let server = self.server().ok_or_else(|| {
            std::io::Error::other(
                self.problem()
                    .unwrap_or_else(|| "this box has no media library".into()),
            )
        })?;
        let e = server.import(path, folder, false)?;
        let existed = path.exists();
        let sha = e.sha256.clone().unwrap_or_default();
        if let Some(p) = server.path_of(&e.id) {
            write(&self.items).insert(
                e.id.clone(),
                ItemFile {
                    path: p,
                    sha256: e.sha256.clone(),
                },
            );
        }
        Ok((e.id, sha, existed))
    }

    /// Replaces the Codex's "in use" marks with `ids` (`used`: which live collections use
    /// which item, for the change announcements). Blocking (IDN writes its index).
    ///
    /// IDN keeps only marks of items in the library or its trash (Beta8: other ids are
    /// dropped silently), so ids the documents name that the library doesn't have (a missing
    /// item) are left out here: the marks then match, and nothing is written again at every
    /// refresh. At most [`MAX_MARKS`] are marked (the rest said once, as a warning).
    pub fn set_marks(
        &self,
        ids: &BTreeSet<String>,
        used: HashMap<String, BTreeSet<CollectionId>>,
    ) -> Result<usize, String> {
        *write(&self.used) = used;
        let Some(server) = self.server() else {
            return Ok(0);
        };
        let now: BTreeSet<String> = server.marked(HOLDER).into_iter().collect();
        let trash: HashSet<String> = if ids.iter().any(|id| !now.contains(id)) {
            server.trash()["items"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t["id"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            HashSet::new()
        };
        let (want, over) = marks_wanted(ids, &now, |id| {
            server.rel_path(id).is_some() || trash.contains(id)
        });
        if self.marks_over.swap(over, Ordering::Relaxed) != over && over > 0 {
            tracing::warn!(
                over,
                "more media in use than the library marks ({MAX_MARKS}): {over} items aren't marked in use (the library may delete them without asking)"
            );
        }
        // (Nothing to write when they are what they should be.)
        if now == want {
            return Ok(want.len());
        }
        let want: Vec<String> = want.into_iter().collect();
        server.marks(HOLDER, "set", &want)
    }

    /// The items the Codex marks in use now, sorted (IDN's `marked`, read only).
    pub fn marked(&self) -> Vec<String> {
        let mut v = self.server().map(|s| s.marked(HOLDER)).unwrap_or_default();
        v.sort();
        v
    }

    /// Adds `ids` to the Codex's "in use" marks (an upload's new item, a relink).
    pub fn add_marks(&self, ids: &[String]) {
        if let Some(server) = self.server()
            && !ids.is_empty()
            && let Err(e) = server.marks(HOLDER, "add", ids)
        {
            tracing::warn!("marking media in use: {e}");
        }
    }

    /// Asks for the marks to be set again (after writes that change media references).
    pub fn marks_due(&self) {
        self.marks_due.notify_one();
    }

    /// Waits until the marks are due.
    pub async fn wait_marks_due(&self) {
        self.marks_due.notified().await;
    }

    /// This box is a show box (Codex role `client`): its own library (if open) closes, and
    /// library items come from `cache` (IDN's cache of the site's Codex's library; `None`
    /// until the Codex is found).
    pub fn set_remote(&self, cache: Option<Arc<Cache>>) {
        if self.is_open() {
            self.close();
        }
        if self.stop.load(Ordering::Relaxed) {
            return;
        }
        *write(&self.state) = State::Remote(cache);
    }

    /// This box is a show box (Codex role `client`).
    pub fn is_remote(&self) -> bool {
        matches!(*read(&self.state), State::Remote(_))
    }

    /// The show box's cache, once the site's Codex has been found.
    pub fn cache(&self) -> Option<Arc<Cache>> {
        match &*read(&self.state) {
            State::Remote(c) => c.clone(),
            _ => None,
        }
    }

    /// On a show box: item `id` is wanted now (a cue that uses it was triggered); copied
    /// before the pinned items still missing. Whether it is here already.
    pub fn want(&self, id: &str) -> bool {
        let Some(cache) = self.cache() else {
            return false;
        };
        if matches!(cache.state(id), ItemState::Cached(_)) {
            return true;
        }
        cache.want(id);
        false
    }

    /// On a show box: item `id` is in the site's library but not on this box yet (being
    /// copied, waiting, or copied when triggered).
    pub fn copying(&self, id: &str) -> bool {
        match &*read(&self.state) {
            State::Remote(None) => true,
            State::Remote(Some(cache)) => matches!(
                cache.state(id),
                ItemState::Fetching { .. } | ItemState::Waiting | ItemState::NotKept
            ),
            _ => false,
        }
    }

    /// The server stops: no more following, and the library closes (see
    /// [`MediaLibrary::close`]); a show box's cache stops.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.close();
        let state = std::mem::replace(&mut *write(&self.state), State::Closed);
        drop(state);
    }

    /// Closes the library (`LibraryServer::close`: IDN writes what it hadn't yet), so it can
    /// be opened again at once (by a server started next on the same folder, or this one when
    /// the box hosts the Codex again). No more following.
    pub fn close(&self) {
        let state = {
            let mut st = write(&self.state);
            if !matches!(*st, State::Open(_)) {
                return;
            }
            std::mem::replace(&mut *st, State::Closed)
        };
        let follower = crate::lock(&self.follower).take();
        if let Some(h) = follower
            && h.thread().id() != std::thread::current().id()
        {
            let _ = h.join();
        }
        if let State::Open(server) = state {
            // IDN's close: returns once its work has stopped and its index is saved (up to
            // 10 s while a request still holds it; then it ends with that request).
            match server.close() {
                Ok(()) => tracing::info!("Codex: media library closed"),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => tracing::warn!(
                    "Codex: the media library is still in use while the server stops; it closes when that ends: {e}"
                ),
                Err(e) => tracing::warn!("Codex: closing the media library: {e}"),
            }
        }
    }

    fn announce(&self, ids: &HashSet<String>) {
        let Some(events) = self.events.get() else {
            return;
        };
        let used = read(&self.used);
        let mut collections = BTreeSet::new();
        for id in ids {
            if let Some(c) = used.get(id) {
                collections.extend(c.iter().copied());
            }
        }
        for c in collections {
            let _ = events.send(Arc::new(ServerMessage::CodexChanged {
                parts: vec![CodexPart::Media],
                collection: Some(c),
            }));
        }
    }
}

/// Follows the library's changes into the Codex's map (see the module docs), until the server
/// stops or the library closes.
fn follow(
    me: std::sync::Weak<MediaLibrary>,
    server: std::sync::Weak<LibraryServer>,
    mut since: u64,
) {
    loop {
        let (Some(lib), Some(srv)) = (me.upgrade(), server.upgrade()) else {
            return;
        };
        if lib.stop.load(Ordering::Relaxed) || !lib.is_open() {
            return;
        }
        drop(lib);
        let (changes, next, missed) = srv.changes(since, FOLLOW_WAIT);
        let Some(lib) = me.upgrade() else { return };
        let mut touched = HashSet::new();
        if missed {
            // Some changes are no longer kept: the whole map again.
            let mut items = HashMap::new();
            for it in srv.catalog().items {
                if let Some(path) = srv.path_of(&it.id) {
                    items.insert(
                        it.id.clone(),
                        ItemFile {
                            path,
                            sha256: it.sha256.clone(),
                        },
                    );
                }
            }
            let mut map = write(&lib.items);
            for (id, f) in map.iter() {
                if items.get(id) != Some(f) {
                    touched.insert(id.clone());
                }
            }
            for id in items.keys() {
                if !map.contains_key(id) {
                    touched.insert(id.clone());
                }
            }
            *map = items;
        } else {
            let ids: BTreeSet<String> = changes.into_iter().map(|c| c.id).collect();
            for id in ids {
                let now = srv.path_of(&id).map(|path| ItemFile {
                    path,
                    sha256: srv.item(&id).and_then(|i| i.sha256),
                });
                let mut map = write(&lib.items);
                let before = match &now {
                    Some(f) => map.insert(id.clone(), f.clone()),
                    None => map.remove(&id),
                };
                if before != now {
                    touched.insert(id);
                }
            }
        }
        since = next;
        if !touched.is_empty() {
            lib.announce(&touched);
        }
    }
}

/// The library folder an import into a collection goes to: the collection's name, made a
/// plain folder name (no `/`, `\`, `:` or characters Windows refuses; not hidden, not `@`).
pub fn folder_for(name: &str, id: CollectionId) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"/\<>:"|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect();
    let s = s
        .trim()
        .trim_start_matches(['.', '@'])
        .trim_end_matches(['.', ' '])
        .trim()
        .to_string();
    if s.is_empty() {
        format!("Collection {}", &id.to_string()[..8])
    } else {
        s
    }
}

/// A file name the library takes: IDN knows its ending as video, picture or sound (its own
/// endings and Vesper's, [`take_vesper_endings`]).
pub fn takes(name: &str) -> bool {
    take_vesper_endings();
    idn_open_protocol::library::kind_of(name).is_some()
}

type ProbeKey = (PathBuf, u64, u128);

/// Probes of media files, each file probed once (by its path, size and modification time),
/// whoever asks first: IDN's background prober or the media worker. Bounded.
#[derive(Default)]
pub struct ProbeMemo {
    m: Mutex<Memo>,
    cv: Condvar,
}

#[derive(Default)]
struct Memo {
    done: HashMap<ProbeKey, Result<MediaProbe, String>>,
    order: VecDeque<ProbeKey>,
    running: HashSet<ProbeKey>,
    /// Probes run per file (diagnostics and tests).
    runs: HashMap<PathBuf, u32>,
}

/// Probes kept.
const MEMO_SIZE: usize = 4096;

fn key_of(path: &Path) -> Option<ProbeKey> {
    let m = std::fs::metadata(path).ok()?;
    let t = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((path.to_path_buf(), m.len(), t))
}

impl ProbeMemo {
    /// The media facts of `path` (`kind` from its name): probed now unless probed already
    /// (or being probed: then waited for). Run it at low priority (the caller's thread).
    pub fn probe(&self, path: &Path, kind: MediaKind) -> Result<MediaProbe, String> {
        let Some(key) = key_of(path) else {
            return Err(format!("{} can't be read", path.display()));
        };
        {
            let mut m = crate::lock(&self.m);
            loop {
                if let Some(r) = m.done.get(&key) {
                    return r.clone();
                }
                if !m.running.contains(&key) {
                    m.running.insert(key.clone());
                    break;
                }
                m = self
                    .cv
                    .wait_timeout(m, Duration::from_millis(500))
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
        }
        let r = super::worker::probe_file(path, kind);
        let mut m = crate::lock(&self.m);
        m.running.remove(&key);
        *m.runs.entry(key.0.clone()).or_default() += 1;
        if m.done.insert(key.clone(), r.clone()).is_none() {
            m.order.push_back(key);
        }
        while m.order.len() > MEMO_SIZE {
            if let Some(old) = m.order.pop_front() {
                m.done.remove(&old);
            }
        }
        drop(m);
        self.cv.notify_all();
        r
    }

    /// How many times the file at `path` was probed (each change of it once).
    pub fn runs(&self, path: &Path) -> u32 {
        crate::lock(&self.m).runs.get(path).copied().unwrap_or(0)
    }

    /// IDN's prober: the facts the library lists (duration, size in pixels, codec, alpha,
    /// sound tracks), probed on a thread of its own at low priority. `None` when the file
    /// can't be read.
    pub fn facts(&self, path: &Path) -> Option<Value> {
        let name = path.file_name()?.to_string_lossy().to_string();
        let (_, ext) = blobs::split_name(&name);
        let kind = blobs::kind_for(&ext).unwrap_or(MediaKind::Unknown);
        let probed = std::thread::scope(|s| {
            s.spawn(|| {
                super::disk::lower_thread_priority();
                self.probe(path, kind)
            })
            .join()
            .ok()
        })?;
        let p = probed.ok()?;
        Some(json!({
            "duration": p.duration,
            "width": p.width,
            "height": p.height,
            "codec": p.codec,
            "alpha": p.has_alpha,
            // (The media crate tells whether there is sound, not how many tracks.)
            "audio_tracks": p.has_audio.map(u32::from),
        }))
    }
}

/// The marks to hold, from the items the documents use (`ids`): the ones the library has
/// (`known`: in it or its trash), at most [`MAX_MARKS`], the ones marked `now` first (a clipped
/// set doesn't change at every refresh). With how many were left out past the limit.
fn marks_wanted(
    ids: &BTreeSet<String>,
    now: &BTreeSet<String>,
    known: impl Fn(&str) -> bool,
) -> (BTreeSet<String>, usize) {
    let (kept, new): (Vec<&String>, Vec<&String>) = ids
        .iter()
        .filter(|id| now.contains(*id) || known(id))
        .partition(|id| now.contains(*id));
    let all = kept.len() + new.len();
    let want: BTreeSet<String> = kept
        .into_iter()
        .chain(new)
        .take(MAX_MARKS)
        .cloned()
        .collect();
    (want, all.saturating_sub(MAX_MARKS))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn marks_leave_out_items_the_library_doesnt_have() {
        let known = |id: &str| id != "gone";
        // An id the library doesn't have: not asked for, so the marks match.
        let (want, over) = marks_wanted(&set(&["a", "b", "gone"]), &set(&["a", "b"]), known);
        assert_eq!((want, over), (set(&["a", "b"]), 0));
        // A new one it has is added; one no longer used goes.
        let (want, _) = marks_wanted(&set(&["a", "c", "gone"]), &set(&["a", "b"]), known);
        assert_eq!(want, set(&["a", "c"]));
    }

    #[test]
    fn marks_stay_inside_the_limit() {
        let ids: BTreeSet<String> = (0..MAX_MARKS + 5).map(|i| format!("i{i:05}")).collect();
        // The ones marked now are kept first: the set doesn't change at every refresh.
        let now: BTreeSet<String> = ids.iter().rev().take(10).cloned().collect();
        let (want, over) = marks_wanted(&ids, &now, |_| true);
        assert_eq!((want.len(), over), (MAX_MARKS, 5));
        assert!(now.is_subset(&want));
        let (again, _) = marks_wanted(&ids, &want, |_| true);
        assert_eq!(again, want);
    }

    #[test]
    fn folder_names() {
        let id = CollectionId::new();
        assert_eq!(folder_for("Youth", id), "Youth");
        assert_eq!(folder_for(" a/b:c ", id), "a_b_c");
        assert_eq!(folder_for("@NAS", id), "NAS");
        assert_eq!(
            folder_for("..", id),
            format!("Collection {}", &id.to_string()[..8])
        );
        assert!(takes("Clip.MP4") && !takes("notes.txt"));
    }
}
