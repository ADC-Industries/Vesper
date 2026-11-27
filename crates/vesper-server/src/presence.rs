//! Presence and edit locks (G8, `docs/PRESENCE-PLAN.md`): who has which document open, and
//! who is editing it.
//!
//! - **Sessions.** A WebSocket connection becomes a session when it first reports what it has
//!   open ([`protocol::ClientMessage::Presence`]); it ends when the socket closes or the
//!   heartbeat gives up on it ([`crate::ws`]). Clients that never report (older clients,
//!   probes) are not listed.
//! - **Locks** are hard and per document: a content write ([`layer`]) from a `clientId` that
//!   has a live session with the document open takes its lock; the same writes from anyone
//!   else answer `423 locked` until the holder closes the document, its last session ends, or
//!   someone takes the lock over (`POST /api/v1/presence/locks`). Writes without a live
//!   session never take a lock (scripts, a show box's forwarded writes) but are refused by
//!   one.
//! - Every change is broadcast as `presenceChanged` with the whole list.
//! - **Per collection.** A lock belongs to the collection the write or take is for (a show
//!   box's forwarded collection on the Codex, else the box's active one), so two collections
//!   that hold the same document IDs (a box's re-upload made a second one) never block each
//!   other (F08-1). A lock or request whose collection isn't known (an older show box) matches
//!   the document in any collection, as before.
//!
//! **Site-wide** (stage 3): on a site with a Codex, a show box reports its sessions to the Codex
//! box over the box-to-Codex link ([`crate::codex::site_presence`]: on every change and every
//! 20 s) and shows the site's merged list it gets back ([`Presence::set_site`]). The Codex box
//! keeps each box's sessions as *remote* ones (dropped [`REMOTE_TTL`] after the box's last
//! report): they count for locks like its own sessions, so a show box's forwarded write takes
//! the lock for its client, and a write from anyone else, through any box, answers `423`. A
//! show box takes, takes over and releases locks on the Codex; its own lock check stands
//! aside while it shows the site's list (its writes all go to the Codex, which checks them).

use crate::api::AppState;
use crate::lock;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use model::{CollectionId, DocumentKind};
use protocol::{
    ApiError, DocumentLock, ErrorCode, MAX_OPEN_DOCUMENTS, OpenDocument, PresenceClient,
    PresenceList, ServerMessage,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// Who a session's client is (from its key and the site).
#[derive(Clone, Debug, Default)]
pub struct Who {
    /// The key's name, or `server machine`.
    pub name: String,
    /// The site user it is signed in as.
    pub user: Option<String>,
    /// The computer name from the WebSocket hello (used when presence doesn't name one).
    pub computer: Option<String>,
}

struct Session {
    who: Who,
    client_id: Option<String>,
    computer: Option<String>,
    open: Vec<OpenDocument>,
}

struct Lock {
    /// The collection it is in (`None`: not known, matches any).
    collection: Option<CollectionId>,
    kind: DocumentKind,
    id: String,
    client_id: String,
    since: String,
}

/// A show box's sessions as the Codex box knows them (its last report).
struct Remote {
    /// The box's name.
    name: String,
    /// When it last reported.
    at: Instant,
    /// Its sessions.
    clients: Vec<PresenceClient>,
}

/// How long a show box's sessions stay listed after its last report (it reports every 20 s).
pub const REMOTE_TTL: Duration = Duration::from_secs(60);

/// Which of the Codex's lists a show box was sent: the Codex's run (`epoch`, new at each
/// start) and its presence generation then. A show box never replaces a list with an older one
/// of the same run (pushes, report and lock answers travel on separate threads and links).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceStamp {
    /// The Codex's run.
    pub epoch: String,
    /// Its presence generation.
    pub generation: u64,
}

#[derive(Default)]
struct Inner {
    sessions: BTreeMap<u64, Session>,
    locks: Vec<Lock>,
    /// On the Codex box: each show box's sessions, by the box's fingerprint.
    remote: BTreeMap<String, Remote>,
    /// On a show box linked to the site's Codex: the site's list, as the Codex last sent it.
    site: Option<PresenceList>,
    /// Which list that was (when the Codex said).
    site_stamp: Option<PresenceStamp>,
}

impl Lock {
    /// The lock of document `kind`/`id` in collection `c` (an unknown collection on either side
    /// matches any).
    fn is(&self, c: Option<CollectionId>, kind: DocumentKind, id: &str) -> bool {
        self.kind == kind
            && self.id == id
            && (self.collection.is_none() || c.is_none() || self.collection == c)
    }
}

/// The collection a lock taken or checked by the request being handled is in: a show box's
/// forwarded collection (on the Codex), else this box's active collection.
pub fn request_collection(sh: &crate::Shared) -> Option<CollectionId> {
    crate::api::forwarded_collection().or(sh.codex().config.collection)
}

impl Inner {
    fn live(&self, client_id: &str) -> impl Iterator<Item = &Session> {
        self.sessions
            .values()
            .filter(move |s| s.client_id.as_deref() == Some(client_id))
    }

    /// A show box's sessions of the client, with the box (fingerprint, name).
    fn remote_live(&self, client_id: &str) -> impl Iterator<Item = (&str, &str, &PresenceClient)> {
        self.remote.iter().flat_map(move |(fp, r)| {
            r.clients
                .iter()
                .filter(move |c| c.client_id.as_deref() == Some(client_id))
                .map(move |c| (fp.as_str(), r.name.as_str(), c))
        })
    }

    /// The client has a session here or on a show box.
    fn is_live(&self, client_id: &str) -> bool {
        self.live(client_id).next().is_some() || self.remote_live(client_id).next().is_some()
    }

    /// The client has the document open for editing in one of its sessions (here or on a show
    /// box): a place that edits it (the Show page only browses), in collection `c` (an unknown
    /// collection on either side matches any).
    fn has_open(
        &self,
        client_id: &str,
        c: Option<CollectionId>,
        kind: DocumentKind,
        id: &str,
    ) -> bool {
        let hit = |o: &OpenDocument| {
            o.kind == kind
                && o.id == id
                && o.place.edits()
                && (o.collection.is_none() || c.is_none() || o.collection == c)
        };
        self.live(client_id).any(|s| s.open.iter().any(hit))
            || self
                .remote_live(client_id)
                .any(|(_, _, c)| c.open.iter().any(hit))
    }

    /// Drops the show boxes that stopped reporting; whether any went.
    fn expire(&mut self) -> bool {
        let before = self.remote.len();
        self.remote.retain(|_, r| r.at.elapsed() < REMOTE_TTL);
        self.remote.len() != before
    }

    /// Drops the locks whose holder no longer has the document open (or no session).
    fn prune(&mut self) -> bool {
        let before = self.locks.len();
        let keep: Vec<bool> = self
            .locks
            .iter()
            .map(|l| self.has_open(&l.client_id, l.collection, l.kind, &l.id))
            .collect();
        let mut i = 0;
        self.locks.retain(|_| {
            i += 1;
            keep[i - 1]
        });
        self.locks.len() != before
    }

    fn holder(&self, c: Option<CollectionId>, kind: DocumentKind, id: &str) -> Option<&Lock> {
        self.locks.iter().find(|l| l.is(c, kind, id))
    }

    fn lock_entry(&self, l: &Lock) -> DocumentLock {
        self.lock_entry_for(l, None, "")
    }

    /// The lock as `viewer` sees it (`None`: this box; a show box's fingerprint: that box, for
    /// which this box is `here`).
    fn lock_entry_for(&self, l: &Lock, viewer: Option<&str>, here: &str) -> DocumentLock {
        let mut e = DocumentLock {
            kind: l.kind,
            id: l.id.clone(),
            client_id: l.client_id.clone(),
            name: String::new(),
            user: None,
            computer: None,
            since: l.since.clone(),
            server: None,
            collection: l.collection,
        };
        // The newest session of the holder names it.
        if let Some(s) = self.live(&l.client_id).last() {
            e.name = s.who.name.clone();
            e.user = s.who.user.clone();
            e.computer = s.computer.clone();
            e.server = viewer.map(|_| here.to_string()).filter(|h| !h.is_empty());
        } else if let Some((fp, name, c)) = self.remote_live(&l.client_id).last() {
            e.name = c.name.clone();
            e.user = c.user.clone();
            e.computer = c.computer.clone();
            e.server = (viewer != Some(fp)).then(|| name.to_string());
        }
        e
    }

    fn list(&self) -> PresenceList {
        self.list_for(None, "")
    }

    /// Every session (this box's, then each show box's) and lock, as `viewer` sees them (see
    /// [`Inner::lock_entry_for`]).
    fn list_for(&self, viewer: Option<&str>, here: &str) -> PresenceList {
        let here_name = viewer.map(|_| here.to_string()).filter(|h| !h.is_empty());
        let mut clients: Vec<PresenceClient> = self
            .sessions
            .iter()
            .map(|(n, s)| PresenceClient {
                session: format!("s{n}"),
                client_id: s.client_id.clone(),
                name: s.who.name.clone(),
                user: s.who.user.clone(),
                computer: s.computer.clone(),
                open: s.open.clone(),
                server: here_name.clone(),
            })
            .collect();
        for (fp, r) in &self.remote {
            let tag: String = fp
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(8)
                .collect();
            let mine = viewer == Some(fp.as_str());
            clients.extend(r.clients.iter().map(|c| PresenceClient {
                session: if mine {
                    c.session.clone()
                } else {
                    format!("{tag}-{}", c.session)
                },
                server: (!mine).then(|| r.name.clone()),
                ..c.clone()
            }));
        }
        PresenceList {
            clients,
            locks: self
                .locks
                .iter()
                .map(|l| self.lock_entry_for(l, viewer, here))
                .collect(),
        }
    }
}

/// How a lock holder is named to people: "Alex on Booth laptop", "Booth laptop", the key's
/// name.
pub fn holder_label(l: &DocumentLock) -> String {
    match (&l.user, &l.computer) {
        (Some(u), Some(c)) => format!("{u} on {c}"),
        (Some(u), None) => u.clone(),
        (None, Some(c)) => c.clone(),
        (None, None) if !l.name.is_empty() => l.name.clone(),
        _ => "another client".into(),
    }
}

/// A document ID as stored (UUIDs lower-case, hyphenated).
fn norm_id(id: &str) -> String {
    id.trim()
        .parse::<uuid::Uuid>()
        .map(|u| u.to_string())
        .unwrap_or_else(|_| id.trim().to_string())
}

fn kind_word(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Presentation => "presentation",
        DocumentKind::Theme => "theme",
        DocumentKind::StageLayout => "stage layout",
        DocumentKind::Prop => "prop",
        DocumentKind::Timer => "timer",
        _ => "document",
    }
}

fn clean(s: Option<String>, max: usize) -> Option<String> {
    s.map(|s| s.trim().chars().take(max).collect::<String>())
        .filter(|s| !s.is_empty())
}

/// The presence list and the edit locks of this box.
pub struct Presence {
    inner: Mutex<Inner>,
    next: std::sync::atomic::AtomicU64,
    events: broadcast::Sender<Arc<ServerMessage>>,
    /// Counts this box's changes (its sessions, its locks, show boxes' reports), for the
    /// site-wide reports and pushes ([`Presence::wait_change`]).
    generation: Mutex<u64>,
    bumped: Condvar,
    /// This run (see [`PresenceStamp`]).
    epoch: String,
    /// On a show box: the site's Codex reads every place (its answers say so); until then
    /// the box reports only places an older Codex reads (E05-1).
    codex_reads_all_places: std::sync::atomic::AtomicBool,
    /// On the Codex box: the show boxes (fingerprints) that read every place; the others get
    /// lists an older box reads (E05-1).
    boxes_reading_all_places: Mutex<std::collections::HashSet<String>>,
}

impl Presence {
    /// Empty; changes are broadcast on `events`.
    pub fn new(events: broadcast::Sender<Arc<ServerMessage>>) -> Presence {
        Presence {
            inner: Mutex::new(Inner::default()),
            next: std::sync::atomic::AtomicU64::new(1),
            events,
            generation: Mutex::new(0),
            bumped: Condvar::new(),
            epoch: uuid::Uuid::new_v4().to_string(),
            codex_reads_all_places: std::sync::atomic::AtomicBool::new(false),
            boxes_reading_all_places: Mutex::new(Default::default()),
        }
    }

    /// On a show box: whether the site's Codex reads every place; [`Presence::set_codex_reads_all_places`]
    /// says whether that changed.
    pub fn codex_reads_all_places(&self) -> bool {
        self.codex_reads_all_places
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// On a show box: the Codex's last answer said whether it reads every place; `true` when
    /// that changed.
    pub fn set_codex_reads_all_places(&self, all: bool) -> bool {
        self.codex_reads_all_places
            .swap(all, std::sync::atomic::Ordering::SeqCst)
            != all
    }

    /// On the Codex box: whether show box `fingerprint` reads every place (as its last report
    /// or lock request said).
    pub fn box_reads_all_places(&self, fingerprint: &str) -> bool {
        lock(&self.boxes_reading_all_places).contains(fingerprint)
    }

    /// On the Codex box: show box `fingerprint` said whether it reads every place.
    pub fn set_box_reads_all_places(&self, fingerprint: &str, all: bool) {
        let mut g = lock(&self.boxes_reading_all_places);
        if all {
            g.insert(fingerprint.to_string());
        } else {
            g.remove(fingerprint);
        }
    }

    /// A number for a new WebSocket connection (it becomes a session when it reports).
    pub fn new_session(&self) -> u64 {
        self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Drops the show boxes that stopped reporting, with their locks, and tells everyone;
    /// whether any went. (The expiry is reported once: whoever sees it does all three.)
    fn expire_in(&self, g: &mut Inner) -> bool {
        if !g.expire() {
            return false;
        }
        g.prune();
        self.changed(g);
        true
    }

    fn changed(&self, inner: &Inner) {
        *lock(&self.generation) += 1;
        self.bumped.notify_all();
        self.announce(inner);
    }

    /// Tells this box's clients the list they see (the site's on a linked show box).
    fn announce(&self, inner: &Inner) {
        let presence = inner.site.clone().unwrap_or_else(|| inner.list());
        let _ = self
            .events
            .send(Arc::new(ServerMessage::PresenceChanged { presence }));
    }

    /// Every session and lock (on a show box linked to the site's Codex: the site's).
    pub fn list(&self) -> PresenceList {
        let g = lock(&self.inner);
        g.site.clone().unwrap_or_else(|| g.list())
    }

    /// The changes so far (see [`Presence::wait_change`]).
    pub fn generation(&self) -> u64 {
        *lock(&self.generation)
    }

    /// Wakes the site-presence thread with nothing else changed (a show box's report is due
    /// again).
    pub fn touch(&self) {
        *lock(&self.generation) += 1;
        self.bumped.notify_all();
    }

    /// Waits until something changed after `seen`, at most `timeout`: the generation now.
    pub fn wait_change(&self, seen: u64, timeout: Duration) -> u64 {
        let g = lock(&self.generation);
        let (g, _) = self
            .bumped
            .wait_timeout_while(g, timeout, |g| *g == seen)
            .unwrap_or_else(|e| e.into_inner());
        *g
    }

    /// This box's own sessions (what a show box reports to the Codex).
    pub fn own_clients(&self) -> Vec<PresenceClient> {
        lock(&self.inner)
            .list_for(None, "")
            .clients
            .into_iter()
            .filter(|c| c.server.is_none())
            .collect()
    }

    /// On a show box: the site's list from the Codex (`None`: not linked; its own list again).
    /// Its clients are told when what they see changes.
    pub fn set_site(&self, site: Option<PresenceList>) {
        self.set_site_stamped(site, None);
    }

    /// [`Presence::set_site`] with the list's stamp: a list older than the one shown (the
    /// same run of the Codex, an older generation) is ignored.
    pub fn set_site_stamped(&self, site: Option<PresenceList>, stamp: Option<PresenceStamp>) {
        let mut g = lock(&self.inner);
        if site.is_none() {
            g.site_stamp = None;
        } else if let (Some(new), Some(shown)) = (&stamp, &g.site_stamp)
            && new.epoch == shown.epoch
            && new.generation < shown.generation
        {
            return;
        }
        if site.is_some() && stamp.is_some() {
            g.site_stamp = stamp;
        }
        if g.site == site {
            return;
        }
        let was_site = g.site.is_some();
        g.site = site;
        if was_site && g.site.is_none() {
            // Back to this box's own list: no locks of its own were kept meanwhile.
            g.prune();
        }
        self.announce(&g);
    }

    /// The box shows the site's list (a show box linked to the site's Codex): locks are the
    /// Codex's.
    pub fn site_wide(&self) -> bool {
        lock(&self.inner).site.is_some()
    }

    /// On the Codex box: a show box's report of its sessions (`fingerprint`, its `name`); the
    /// site's list as that box sees it.
    pub fn report_box(
        &self,
        fingerprint: &str,
        name: &str,
        clients: Vec<PresenceClient>,
        here: &str,
    ) -> PresenceList {
        let mut g = lock(&self.inner);
        self.expire_in(&mut g);
        let clients: Vec<PresenceClient> = clients
            .into_iter()
            .take(256)
            .map(|c| PresenceClient {
                open: c
                    .open
                    .into_iter()
                    .take(MAX_OPEN_DOCUMENTS)
                    .map(|o| OpenDocument {
                        id: norm_id(&o.id),
                        ..o
                    })
                    .collect(),
                server: None,
                ..c
            })
            .collect();
        let same = g
            .remote
            .get(fingerprint)
            .is_some_and(|r| r.name == name && r.clients == clients);
        g.remote.insert(
            fingerprint.to_string(),
            Remote {
                name: name.to_string(),
                at: Instant::now(),
                clients,
            },
        );
        if !same {
            g.prune();
            self.changed(&g);
        }
        g.list_for(Some(fingerprint), here)
    }

    /// On the Codex box: the site's list as the show box `fingerprint` sees it.
    pub fn list_for_box(&self, fingerprint: &str, here: &str) -> PresenceList {
        lock(&self.inner).list_for(Some(fingerprint), here)
    }

    /// [`Presence::list_for_box`] with its stamp (read together: changes bump the generation
    /// under the same lock).
    pub fn stamped_list_for_box(
        &self,
        fingerprint: &str,
        here: &str,
    ) -> (PresenceList, PresenceStamp) {
        let g = lock(&self.inner);
        let list = g.list_for(Some(fingerprint), here);
        let stamp = PresenceStamp {
            epoch: self.epoch.clone(),
            generation: self.generation(),
        };
        (list, stamp)
    }

    /// The show boxes that reported in the last [`REMOTE_TTL`] (fingerprints).
    pub fn boxes(&self) -> Vec<String> {
        lock(&self.inner)
            .remote
            .iter()
            .filter(|(_, r)| r.at.elapsed() < REMOTE_TTL)
            .map(|(f, _)| f.clone())
            .collect()
    }

    /// Drops the show boxes that stopped reporting (and their locks).
    pub fn expire(&self) {
        let mut g = lock(&self.inner);
        self.expire_in(&mut g);
    }

    /// A session's report of what it has open (replaces the previous one).
    pub fn report(
        &self,
        session: u64,
        who: &Who,
        client_id: Option<String>,
        computer: Option<String>,
        open: Vec<OpenDocument>,
    ) -> Result<(), ApiError> {
        if open.len() > MAX_OPEN_DOCUMENTS {
            return Err(ApiError::invalid(
                "open",
                format!("at most {MAX_OPEN_DOCUMENTS} open documents"),
            ));
        }
        let mut open: Vec<OpenDocument> = open
            .into_iter()
            .map(|o| OpenDocument {
                id: norm_id(&o.id),
                ..o
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        open.retain(|o| seen.insert(o.clone()));
        let client_id = clean(client_id, 80);
        let computer = clean(computer, 80).or_else(|| who.computer.clone());
        let mut g = lock(&self.inner);
        let same = g
            .sessions
            .get(&session)
            .is_some_and(|s| s.client_id == client_id && s.computer == computer && s.open == open);
        if same {
            return Ok(());
        }
        g.sessions.insert(
            session,
            Session {
                who: who.clone(),
                client_id,
                computer,
                open,
            },
        );
        g.prune();
        self.changed(&g);
        Ok(())
    }

    /// A WebSocket closed (or the heartbeat gave up on it): its session and, with its last
    /// session, its client's locks go.
    pub fn disconnect(&self, session: u64) {
        let mut g = lock(&self.inner);
        if g.sessions.remove(&session).is_some() {
            g.prune();
            self.changed(&g);
        }
    }

    /// `POST /presence/locks`: takes a document's lock for `client_id` (a client with a live
    /// session), from whoever holds it when `take_over`.
    pub fn take(
        &self,
        collection: Option<CollectionId>,
        kind: DocumentKind,
        id: &str,
        client_id: &str,
        take_over: bool,
    ) -> Result<DocumentLock, ApiError> {
        let id = norm_id(id);
        let mut g = lock(&self.inner);
        self.expire_in(&mut g);
        if !g.is_live(client_id) {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "only a client connected over the WebSocket (that reported its presence with this clientId) can hold an edit lock",
            ));
        }
        if let Some(l) = g.holder(collection, kind, &id) {
            if l.client_id == client_id {
                return Ok(g.lock_entry(l));
            }
            if !take_over {
                return Err(locked_error(&g.lock_entry(l)));
            }
            tracing::info!(?kind, %id, from = %l.client_id, to = %client_id, "edit lock taken over");
        }
        g.locks.retain(|l| !l.is(collection, kind, &id));
        g.locks.push(Lock {
            collection,
            kind,
            id: id.clone(),
            client_id: client_id.to_string(),
            since: crate::auth::rfc3339(std::time::SystemTime::now(), true),
        });
        let entry = g.lock_entry(g.locks.last().expect("just pushed"));
        self.changed(&g);
        Ok(entry)
    }

    /// `DELETE /presence/locks/{kind}/{id}`: releases the caller's lock (nothing when it holds
    /// none).
    pub fn release(
        &self,
        collection: Option<CollectionId>,
        kind: DocumentKind,
        id: &str,
        client_id: &str,
    ) {
        let id = norm_id(id);
        let mut g = lock(&self.inner);
        let before = g.locks.len();
        g.locks
            .retain(|l| !(l.is(collection, kind, &id) && l.client_id == client_id));
        if g.locks.len() != before {
            self.changed(&g);
        }
    }

    /// A content write to a document: refused (`423 locked`) when someone else holds its
    /// lock; takes the lock for a client that has a live session with the document open.
    pub fn check_write(
        &self,
        collection: Option<CollectionId>,
        kind: DocumentKind,
        id: &str,
        client_id: Option<&str>,
    ) -> Result<(), ApiError> {
        let id = norm_id(id);
        let mut g = lock(&self.inner);
        if g.site.is_some() {
            // (The site's Codex checks the write: every content write goes there.)
            return Ok(());
        }
        self.expire_in(&mut g);
        if let Some(l) = g.holder(collection, kind, &id) {
            if Some(l.client_id.as_str()) == client_id {
                return Ok(());
            }
            return Err(locked_error(&g.lock_entry(l)));
        }
        let Some(client_id) = client_id else {
            return Ok(());
        };
        if !g.has_open(client_id, collection, kind, &id) {
            return Ok(());
        }
        g.locks.push(Lock {
            collection,
            kind,
            id,
            client_id: client_id.to_string(),
            since: crate::auth::rfc3339(std::time::SystemTime::now(), true),
        });
        self.changed(&g);
        Ok(())
    }
}

/// `423 locked` naming the holder.
pub fn locked_error(l: &DocumentLock) -> ApiError {
    ApiError::new(
        ErrorCode::Locked,
        format!(
            "{} is editing this {}: take over editing to change it",
            holder_label(l),
            kind_word(l.kind)
        ),
    )
}

/// The document a content write changes, for the lock check: `(kind, id, edits)` (`edits`:
/// an edit-ops route, whose body may carry the `clientId`).
pub fn locked_route(method: &str, path: &str) -> Option<(DocumentKind, String, bool)> {
    let path = path.strip_prefix(protocol::API_PREFIX).unwrap_or(path);
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    let kind = match *parts.first()? {
        "presentations" => DocumentKind::Presentation,
        "themes" => DocumentKind::Theme,
        "stage-layouts" => DocumentKind::StageLayout,
        "props" => DocumentKind::Prop,
        // (A timer's settings; running it is a command, never locked.)
        "timers" => DocumentKind::Timer,
        _ => return None,
    };
    let id = parts.get(1)?.to_string();
    let hit = match (method, &parts[2..]) {
        ("POST", ["edits"]) if kind != DocumentKind::Timer => return Some((kind, id, true)),
        ("PUT" | "DELETE", []) => true,
        ("POST", ["move"]) => kind == DocumentKind::Presentation,
        _ => false,
    };
    hit.then_some((kind, id, false))
}

/// Largest edit body read for its `clientId` (the REST routes take 2 MiB).
const MAX_PEEK: usize = 8 << 20;

/// The lock check of the content writes (a middleware of the REST routes, before
/// write-through: a show box checks its own clients' locks before it forwards).
pub async fn layer(State(app): State<AppState>, req: Request, next: Next) -> Response {
    let Some((kind, id, edits)) = locked_route(req.method().as_str(), req.uri().path()) else {
        return next.run(req).await;
    };
    let header = req
        .headers()
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().chars().take(80).collect::<String>())
        .filter(|s| !s.is_empty());
    let (req, client) = match header {
        Some(c) => (req, Some(c)),
        None if edits => {
            // The edit body's `clientId`.
            let (parts, body) = req.into_parts();
            let bytes = match axum::body::to_bytes(body, MAX_PEEK).await {
                Ok(b) => b,
                Err(e) => {
                    return crate::api::err(ErrorCode::BadRequest, format!("body: {e}"))
                        .into_response();
                }
            };
            let client = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("clientId")?.as_str().map(str::to_owned));
            (
                Request::from_parts(parts, axum::body::Body::from(bytes)),
                client,
            )
        }
        None => (req, None),
    };
    let collection = request_collection(&app.shared);
    match app
        .shared
        .presence
        .check_write(collection, kind, &id, client.as_deref())
    {
        Ok(()) => next.run(req).await,
        Err(e) => crate::api::Error::from(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::PresencePlace;

    const DOC: &str = "6f1c2c1e-0000-4000-8000-000000000001";

    fn p() -> (Presence, broadcast::Receiver<Arc<ServerMessage>>) {
        let (tx, rx) = broadcast::channel(64);
        (Presence::new(tx), rx)
    }

    fn who(name: &str) -> Who {
        Who {
            name: name.into(),
            ..Who::default()
        }
    }

    fn open(id: &str) -> Vec<OpenDocument> {
        vec![OpenDocument {
            kind: DocumentKind::Presentation,
            id: id.into(),
            place: PresencePlace::Edit,
            collection: None,
        }]
    }

    /// F08-1: two collections holding the same document (a box's re-upload made a second
    /// one) have locks of their own; a lock of an unknown collection still matches any.
    #[test]
    fn locks_are_per_collection() {
        let (p, _rx) = p();
        let (a, b) = (
            Some(CollectionId(uuid::Uuid::from_u128(0xa))),
            Some(CollectionId(uuid::Uuid::from_u128(0xb))),
        );
        p.report(1, &who("Booth"), Some("x".into()), None, open(DOC))
            .unwrap();
        p.report(2, &who("Lobby"), Some("y".into()), None, open(DOC))
            .unwrap();
        // X edits the document in A: locked there.
        p.check_write(a, DocumentKind::Presentation, DOC, Some("x"))
            .unwrap();
        assert!(
            p.check_write(a, DocumentKind::Presentation, DOC, Some("y"))
                .is_err()
        );
        // Y edits the same ID in B: its own lock, and X can't write B's copy now.
        p.check_write(b, DocumentKind::Presentation, DOC, Some("y"))
            .unwrap();
        assert!(
            p.check_write(b, DocumentKind::Presentation, DOC, Some("x"))
                .is_err()
        );
        assert_eq!(p.list().locks.len(), 2);
        // Taking over B's lock leaves A's alone.
        let l = p
            .take(b, DocumentKind::Presentation, DOC, "x", true)
            .unwrap();
        assert_eq!(l.client_id, "x");
        assert!(
            p.check_write(a, DocumentKind::Presentation, DOC, Some("y"))
                .is_err(),
            "A's lock is still X's"
        );
        // An unknown collection (an older show box) matches the document anywhere.
        assert!(
            p.check_write(None, DocumentKind::Presentation, DOC, Some("y"))
                .is_err()
        );
        p.release(b, DocumentKind::Presentation, DOC, "x");
        assert_eq!(p.list().locks.len(), 1);
        // The lock names its collection (clients show only their collection's locks).
        assert_eq!(p.list().locks[0].collection, a);
    }

    /// F08-1: an open document names its collection; having the same ID open in another
    /// collection neither takes nor keeps a lock, and an entry without one (an older client)
    /// matches any.
    #[test]
    fn open_documents_are_per_collection() {
        let (p, _rx) = p();
        let (a, b) = (
            Some(CollectionId(uuid::Uuid::from_u128(0xa))),
            Some(CollectionId(uuid::Uuid::from_u128(0xb))),
        );
        let open_in = |c| {
            vec![OpenDocument {
                collection: c,
                ..open(DOC).remove(0)
            }]
        };
        p.report(1, &who("Booth"), Some("x".into()), None, open_in(b))
            .unwrap();
        // X has B's copy open: a write to A's copy takes no lock.
        p.check_write(a, DocumentKind::Presentation, DOC, Some("x"))
            .unwrap();
        assert!(p.list().locks.is_empty());
        // Its write to B's copy does, and moving to A's copy lets it go.
        p.check_write(b, DocumentKind::Presentation, DOC, Some("x"))
            .unwrap();
        assert_eq!(p.list().locks[0].collection, b);
        p.report(1, &who("Booth"), Some("x".into()), None, open_in(a))
            .unwrap();
        assert!(p.list().locks.is_empty());
        // An older client's entry (no collection) matches any.
        p.report(1, &who("Booth"), Some("x".into()), None, open_in(None))
            .unwrap();
        p.check_write(b, DocumentKind::Presentation, DOC, Some("x"))
            .unwrap();
        assert_eq!(p.list().locks.len(), 1);
        assert_eq!(p.list().clients[0].open[0].collection, None);
    }

    #[test]
    fn routes_with_locks() {
        let r = |m, p| locked_route(m, p).map(|(k, _, e)| (k, e));
        assert_eq!(
            r("POST", "/api/v1/presentations/x/edits"),
            Some((DocumentKind::Presentation, true))
        );
        assert_eq!(
            r("DELETE", "/presentations/x"),
            Some((DocumentKind::Presentation, false))
        );
        assert_eq!(
            r("POST", "/presentations/x/move"),
            Some((DocumentKind::Presentation, false))
        );
        assert_eq!(r("POST", "/presentations/x/duplicate"), None);
        assert_eq!(r("GET", "/presentations/x"), None);
        assert_eq!(r("PUT", "/themes/x"), Some((DocumentKind::Theme, false)));
        assert_eq!(
            r("POST", "/stage-layouts/x/edits"),
            Some((DocumentKind::StageLayout, true))
        );
        assert_eq!(r("POST", "/themes/x/move"), None);
        assert_eq!(r("POST", "/playlists/x/edits"), None);
        assert_eq!(
            r("POST", "/props/x/edits"),
            Some((DocumentKind::Prop, true))
        );
        assert_eq!(r("PUT", "/props/x"), Some((DocumentKind::Prop, false)));
        assert_eq!(r("PUT", "/timers/x"), Some((DocumentKind::Timer, false)));
        assert_eq!(r("DELETE", "/timers/x"), Some((DocumentKind::Timer, false)));
        assert_eq!(r("POST", "/timers"), None);
    }

    #[test]
    fn browsing_on_the_show_page_never_takes_a_lock() {
        let (p, _rx) = p();
        let at = |place| {
            vec![OpenDocument {
                kind: DocumentKind::Presentation,
                id: DOC.into(),
                place,
                collection: None,
            }]
        };
        p.report(
            1,
            &who("Booth"),
            Some("a".into()),
            None,
            at(PresencePlace::Show),
        )
        .unwrap();
        p.report(
            2,
            &who("Lobby"),
            Some("b".into()),
            None,
            at(PresencePlace::Edit),
        )
        .unwrap();
        // The Show page's write passes and takes nothing: the editor's write still takes it.
        p.check_write(None, DocumentKind::Presentation, DOC, Some("a"))
            .unwrap();
        assert!(p.list().locks.is_empty());
        p.check_write(None, DocumentKind::Presentation, DOC, Some("b"))
            .unwrap();
        assert_eq!(p.list().locks[0].client_id, "b");
        // The holder keeps it only while a place that edits lists it.
        p.report(
            2,
            &who("Lobby"),
            Some("b".into()),
            None,
            at(PresencePlace::Show),
        )
        .unwrap();
        assert!(p.list().locks.is_empty());
        // A timer's lock is named for people.
        const TIMER: &str = "6f1c2c1e-0000-4000-8000-000000000003";
        let timers = vec![OpenDocument {
            kind: DocumentKind::Timer,
            id: TIMER.into(),
            place: PresencePlace::Timers,
            collection: None,
        }];
        p.report(2, &who("Lobby"), Some("b".into()), None, timers)
            .unwrap();
        p.check_write(None, DocumentKind::Timer, TIMER, Some("b"))
            .unwrap();
        let e = p
            .check_write(None, DocumentKind::Timer, TIMER, Some("a"))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Locked);
        assert!(e.message.contains("this timer"), "{}", e.message);
    }

    #[test]
    fn first_write_takes_the_lock_and_others_are_refused() {
        let (p, _rx) = p();
        // Not open anywhere: no lock.
        p.check_write(None, DocumentKind::Presentation, DOC, Some("a"))
            .unwrap();
        assert!(p.list().locks.is_empty());
        p.report(1, &who("Oriel – A"), Some("a".into()), None, open(DOC))
            .unwrap();
        p.report(2, &who("Oriel – B"), Some("b".into()), None, open(DOC))
            .unwrap();
        p.check_write(
            None,
            DocumentKind::Presentation,
            &DOC.to_uppercase(),
            Some("a"),
        )
        .unwrap();
        assert_eq!(p.list().locks[0].client_id, "a");
        let e = p
            .check_write(None, DocumentKind::Presentation, DOC, Some("b"))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Locked);
        assert!(e.message.contains("Oriel – A"), "{}", e.message);
        // No client id: refused too, never takes one.
        assert!(
            p.check_write(None, DocumentKind::Presentation, DOC, None)
                .is_err()
        );
        // Take over.
        assert!(
            p.take(None, DocumentKind::Presentation, DOC, "b", false)
                .is_err()
        );
        let l = p
            .take(None, DocumentKind::Presentation, DOC, "b", true)
            .unwrap();
        assert_eq!(l.client_id, "b");
        assert!(
            p.check_write(None, DocumentKind::Presentation, DOC, Some("a"))
                .is_err()
        );
        // Closing the document releases it.
        p.report(2, &who("Oriel – B"), Some("b".into()), None, vec![])
            .unwrap();
        assert!(p.list().locks.is_empty());
        p.check_write(None, DocumentKind::Presentation, DOC, Some("a"))
            .unwrap();
        // The last session ending releases it.
        p.disconnect(1);
        assert!(p.list().locks.is_empty());
        assert_eq!(p.list().clients.len(), 1);
    }

    #[test]
    fn changes_are_broadcast_and_repeats_are_not() {
        let (p, mut rx) = p();
        p.report(
            1,
            &who("A"),
            Some("a".into()),
            Some("Booth".into()),
            open(DOC),
        )
        .unwrap();
        assert!(rx.try_recv().is_ok());
        p.report(
            1,
            &who("A"),
            Some("a".into()),
            Some("Booth".into()),
            open(DOC),
        )
        .unwrap();
        assert!(rx.try_recv().is_err());
        p.disconnect(1);
        let m = rx.try_recv().unwrap();
        assert!(
            matches!(m.as_ref(), ServerMessage::PresenceChanged { presence } if presence.clients.is_empty())
        );
        // Without a client id a session can't hold a lock.
        p.report(3, &who("C"), None, None, open(DOC)).unwrap();
        assert!(
            p.take(None, DocumentKind::Presentation, DOC, "c", false)
                .is_err()
        );
    }

    fn remote(
        session: &str,
        client_id: &str,
        computer: &str,
        open: Vec<OpenDocument>,
    ) -> PresenceClient {
        PresenceClient {
            session: session.into(),
            client_id: Some(client_id.into()),
            name: format!("Oriel – {computer}"),
            user: None,
            computer: Some(computer.into()),
            open,
            server: None,
        }
    }

    #[test]
    fn a_show_boxs_sessions_count_for_locks_and_name_their_box() {
        let (p, _rx) = p();
        p.report(1, &who("Oriel – Hall"), Some("a".into()), None, open(DOC))
            .unwrap();
        let seen = p.report_box(
            "fp-b",
            "BOX-B",
            vec![remote("s4", "b", "Booth", open(DOC))],
            "HUB",
        );
        // The box sees its own client without a server, the Codex box's with its name.
        let mine = seen
            .clients
            .iter()
            .find(|c| c.client_id.as_deref() == Some("b"))
            .unwrap();
        assert_eq!(
            (mine.session.as_str(), mine.server.as_deref()),
            ("s4", None)
        );
        let hub = seen
            .clients
            .iter()
            .find(|c| c.client_id.as_deref() == Some("a"))
            .unwrap();
        assert_eq!(hub.server.as_deref(), Some("HUB"));
        // Here: the remote client names its box.
        let here = p.list();
        let r = here
            .clients
            .iter()
            .find(|c| c.client_id.as_deref() == Some("b"))
            .unwrap();
        assert_eq!(r.server.as_deref(), Some("BOX-B"));
        assert_ne!(r.session, "s4");
        // Its (forwarded) write takes the lock; the local client is refused.
        p.check_write(None, DocumentKind::Presentation, DOC, Some("b"))
            .unwrap();
        let e = p
            .check_write(None, DocumentKind::Presentation, DOC, Some("a"))
            .unwrap_err();
        assert!(e.message.contains("Booth"), "{}", e.message);
        assert_eq!(p.list().locks[0].server.as_deref(), Some("BOX-B"));
        // Take over from here; the box's next report without the document ends nothing of
        // a's.
        p.take(None, DocumentKind::Presentation, DOC, "a", true)
            .unwrap();
        p.report_box("fp-b", "BOX-B", vec![], "HUB");
        assert_eq!(p.list().locks[0].client_id, "a");
        // A show box shows the site's list and stands its own check aside.
        let (q, _rx2) = super::tests::p();
        q.set_site(Some(here.clone()));
        assert!(q.site_wide());
        assert_eq!(q.list(), here);
        q.check_write(None, DocumentKind::Presentation, DOC, Some("z"))
            .unwrap();
        q.set_site(None);
        assert!(q.list().clients.is_empty());
    }

    /// Makes the show box `fp` look as if it stopped reporting.
    fn age(p: &Presence, fp: &str) {
        let old = Instant::now()
            .checked_sub(REMOTE_TTL + Duration::from_secs(1))
            .unwrap();
        lock(&p.inner).remote.get_mut(fp).unwrap().at = old;
    }

    #[test]
    fn a_gone_show_boxs_locks_go_whoever_notices_first() {
        const OTHER: &str = "6f1c2c1e-0000-4000-8000-000000000002";
        // (The box's report, a write, a take: each notices the expiry first.)
        for notice in ["report_box", "check_write", "take"] {
            let (p, _rx) = p();
            p.report(1, &who("Oriel – Hall"), Some("a".into()), None, open(OTHER))
                .unwrap();
            p.report_box(
                "fp-b",
                "BOX-B",
                vec![remote("s4", "b", "Booth", open(DOC))],
                "HUB",
            );
            p.report_box("fp-c", "BOX-C", vec![], "HUB");
            p.check_write(None, DocumentKind::Presentation, DOC, Some("b"))
                .unwrap();
            assert_eq!(p.list().locks.len(), 1);
            age(&p, "fp-b");
            let seen = p.generation();
            match notice {
                // Box C's report with nothing new.
                "report_box" => {
                    p.report_box("fp-c", "BOX-C", vec![], "HUB");
                }
                "check_write" => p
                    .check_write(None, DocumentKind::Presentation, OTHER, Some("a"))
                    .unwrap(),
                _ => {
                    p.take(None, DocumentKind::Presentation, OTHER, "a", false)
                        .unwrap();
                }
            }
            assert!(
                p.list().locks.iter().all(|l| l.client_id != "b"),
                "{notice}: b's lock went with its box"
            );
            assert!(p.generation() > seen, "{notice}: the change is told");
            // Nothing is left for the Codex's own expiry.
            let after = p.generation();
            p.expire();
            assert_eq!(p.generation(), after);
            p.check_write(None, DocumentKind::Presentation, DOC, Some("a"))
                .unwrap();
        }
    }

    #[test]
    fn a_show_box_never_goes_back_to_an_older_list() {
        let (hub, _rx) = p();
        hub.report(1, &who("Oriel – Hall"), Some("a".into()), None, open(DOC))
            .unwrap();
        hub.report_box("fp-b", "BOX-B", vec![], "HUB");
        let (old, s1) = hub.stamped_list_for_box("fp-b", "HUB");
        hub.check_write(None, DocumentKind::Presentation, DOC, Some("a"))
            .unwrap();
        let (new, s2) = hub.stamped_list_for_box("fp-b", "HUB");
        assert!(s2.generation > s1.generation);
        assert_eq!(new.locks.len(), 1);
        let (b, _rx2) = p();
        // The newer arrives first; the older is ignored.
        b.set_site_stamped(Some(new.clone()), Some(s2.clone()));
        b.set_site_stamped(Some(old.clone()), Some(s1.clone()));
        assert_eq!(b.list(), new);
        // Another run of the Codex (a restart) starts again.
        let restarted = PresenceStamp {
            epoch: "another".into(),
            generation: 0,
        };
        b.set_site_stamped(Some(old.clone()), Some(restarted));
        assert_eq!(b.list(), old);
        // Unlinked and linked again: whatever comes first.
        b.set_site(None);
        b.set_site_stamped(Some(old.clone()), Some(s1));
        assert_eq!(b.list(), old);
    }

    #[test]
    fn holders_are_named_for_people() {
        let mut l = DocumentLock {
            kind: DocumentKind::Theme,
            id: DOC.into(),
            client_id: "a".into(),
            name: "Oriel – Booth".into(),
            user: Some("alex".into()),
            computer: Some("Booth laptop".into()),
            since: String::new(),
            server: None,
            collection: None,
        };
        assert_eq!(holder_label(&l), "alex on Booth laptop");
        l.user = None;
        assert_eq!(holder_label(&l), "Booth laptop");
        l.computer = None;
        assert_eq!(holder_label(&l), "Oriel – Booth");
        assert!(locked_error(&l).message.contains("this theme"));
    }
}
