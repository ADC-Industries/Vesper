//! Site-wide presence and edit locks over the box-to-Codex link (`docs/PRESENCE-PLAN.md`,
//! stage 3; see [`crate::presence`]).
//!
//! | Kind | Request | Answer |
//! |---|---|---|
//! | [`PRESENCE`] (show box → Codex) | [`Report`] `{name, clients, stamped?}`: the box's own sessions, on every change and every [`REPORT_EVERY`] | `PresenceList`: the site's, as that box sees it (its own clients without `server`); with `stamped`, [`ReportAnswer`] `{presence, stamp}` |
//! | [`LOCK`] (show box → Codex) | [`LockRequest`] `{name, clients, op, kind, id, clientId, takeOver?}`: the box's sessions first, then the lock taken (over) or released | [`LockAnswer`] `{lock?, presence, stamp?}`; `423 locked` as the `ApiError`'s JSON |
//! | [`PRESENCE_CHANGED`] (Codex → show box) | [`Pushed`] `{presence, stamp?}`: the site's list changed | `{}` |
//!
//! **Places across versions** (E05-1): a box or Codex from before the places `show` and
//! `timers` reads only `edit`, `reflow` and `library`, and one unknown place makes it refuse the
//! whole message. So each side says it reads every place (`allPlaces` in [`Report`],
//! [`LockRequest`], [`ReportAnswer`] and [`LockAnswer`]), and a peer that didn't say so gets
//! clients cut down to what it reads ([`for_older_peer`]).
//!
//! A `stamp` ([`PresenceStamp`] `{epoch, generation}`) says which of the Codex's lists it is: a
//! show box never replaces its list with an older one of the same run of the Codex (pushes,
//! report and lock answers arrive on separate threads and links, in any order).
//!
//! One thread per server ([`start`]): on a show box linked to the site's Codex it reports and
//! shows the site's list; on the Codex box it drops boxes that stopped reporting and pushes the
//! list to the boxes that report whenever it changes.

use super::link::VERSION;
use crate::Shared;
use crate::presence::PresenceStamp;
use idn_open_protocol::channel::{Channel, Reply};
use model::{CodexRole, DocumentKind};
use protocol::{ApiError, DocumentLock, ErrorCode, PresenceClient, PresenceList, PresencePlace};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// The link's message kinds (`vesper.codex.presence`, `vesper.codex.lock`,
/// `vesper.codex.presence_changed`).
pub use super::link::{LOCK, PRESENCE, PRESENCE_CHANGED};
/// A show box reports at least this often (the Codex drops it after
/// [`crate::presence::REMOTE_TTL`]).
pub const REPORT_EVERY: Duration = Duration::from_secs(20);
/// How long a report or a lock request waits for the Codex.
const ASK: Duration = Duration::from_secs(5);
/// How long a push waits for the box.
const PUSH: Duration = Duration::from_secs(3);

fn v1() -> u32 {
    VERSION
}

/// [`PRESENCE`]'s request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The show box's name.
    pub name: String,
    /// Its own sessions.
    pub clients: Vec<PresenceClient>,
    /// Answer a [`ReportAnswer`] (with the list's stamp), not a bare list.
    #[serde(default)]
    pub stamped: bool,
    /// The box reads every place (an unknown one as `other`): lists for it aren't cut down.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_places: bool,
    /// The fingerprint of the IDN Node on the box's machine (batch 15: which Node is which
    /// Vesper, so Mosaic never offers that machine as a display box). Absent: no Node there, or
    /// an older box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

/// What [`LOCK`] does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LockOp {
    /// Take (over).
    Take,
    /// Release the caller's.
    Release,
}

/// [`LOCK`]'s request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The show box's name.
    pub name: String,
    /// Its own sessions (taken in first, so the caller's is known).
    pub clients: Vec<PresenceClient>,
    /// Take or release.
    pub op: LockOp,
    /// The document's kind.
    pub kind: DocumentKind,
    /// Its ID.
    pub id: String,
    /// The caller's `clientId`.
    pub client_id: String,
    /// Take it from whoever holds it.
    #[serde(default)]
    pub take_over: bool,
    /// The box reads every place (as in [`Report`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_places: bool,
    /// The collection the box shows (the lock is in it; F08-1). Missing from older boxes:
    /// their locks match the document in any collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<model::CollectionId>,
}

/// [`LOCK`]'s answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockAnswer {
    /// The link's version.
    pub v: u32,
    /// The lock taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<DocumentLock>,
    /// The site's list after it.
    pub presence: PresenceList,
    /// Which list (none from an older Codex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<PresenceStamp>,
    /// The Codex reads every place (from an older one: not said).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_places: bool,
}

/// [`PRESENCE`]'s answer to a report that asks for it stamped (`stamped`); a bare
/// `PresenceList` otherwise (and from an older Codex).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportAnswer {
    /// The link's version.
    pub v: u32,
    /// The site's list, as the box sees it.
    pub presence: PresenceList,
    /// Which list.
    pub stamp: PresenceStamp,
    /// The Codex reads every place (from an older one: not said).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_places: bool,
}

/// What a show box reads from [`PRESENCE`].
#[derive(Deserialize)]
#[serde(untagged)]
enum ReportReply {
    Stamped(ReportAnswer),
    Bare(PresenceList),
}

/// [`PRESENCE_CHANGED`]'s request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pushed {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The site's list, as the box sees it.
    pub presence: PresenceList,
    /// Which list (none from an older Codex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<PresenceStamp>,
}

fn json<T: Serialize>(v: &T) -> Reply {
    match serde_json::to_vec(v) {
        Ok(b) => Reply::Ok(b),
        Err(e) => Reply::Error(e.to_string()),
    }
}

/// Clients as a peer from before the places `show` and `timers` reads them (E05-1): only
/// `edit`, `reflow` and `library`. A timer's settings read as `library` (an edit place there
/// too, so the timer's lock lasts on an older Codex); `show` and unknown places are left out
/// (browsing never holds a lock).
pub fn for_older_peer(mut clients: Vec<PresenceClient>) -> Vec<PresenceClient> {
    for c in &mut clients {
        c.open.retain_mut(|d| match d.place {
            PresencePlace::Edit | PresencePlace::Reflow | PresencePlace::Library => true,
            PresencePlace::Timers => {
                d.place = PresencePlace::Library;
                true
            }
            PresencePlace::Show | PresencePlace::Other => false,
        });
    }
    clients
}

/// The site's list for show box `fingerprint`, cut down unless it reads every place.
fn list_for(sh: &Shared, fingerprint: &str, mut list: PresenceList) -> PresenceList {
    if !sh.presence.box_reads_all_places(fingerprint) {
        list.clients = for_older_peer(list.clients);
    }
    list
}

/// This box's own clients for the site's Codex, cut down unless it reads every place.
fn own_for_codex(sh: &Shared) -> Vec<PresenceClient> {
    let own = sh.presence.own_clients();
    if sh.presence.codex_reads_all_places() {
        own
    } else {
        for_older_peer(own)
    }
}

/// This box's name as other boxes show it.
fn here(sh: &Shared) -> String {
    sh.auth.identity().name
}

/// The Codex's side: a show box's [`PRESENCE`] or [`LOCK`] (the link checked it is a machine
/// of the site).
pub fn serve(sh: &Shared, fingerprint: &str, kind: &str, payload: &[u8]) -> Reply {
    let here = here(sh);
    match kind {
        PRESENCE => match serde_json::from_slice::<Report>(payload).inspect(|r| {
            // (A report that parsed: keyed by the sending box, so a box only ever names its
            // own machine's Node, and it lapses when the box stops reporting.)
            if let Some(node) = &r.node {
                sh.mosaic.note_vesper_node(fingerprint, node, &r.name);
            }
        }) {
            Ok(r) if r.stamped => {
                sh.presence
                    .set_box_reads_all_places(fingerprint, r.all_places);
                sh.presence
                    .report_box(fingerprint, &r.name, r.clients, &here);
                let (presence, stamp) = sh.presence.stamped_list_for_box(fingerprint, &here);
                json(&ReportAnswer {
                    v: VERSION,
                    presence: list_for(sh, fingerprint, presence),
                    stamp,
                    all_places: true,
                })
            }
            Ok(r) => {
                sh.presence
                    .set_box_reads_all_places(fingerprint, r.all_places);
                let list = sh
                    .presence
                    .report_box(fingerprint, &r.name, r.clients, &here);
                json(&list_for(sh, fingerprint, list))
            }
            Err(e) => Reply::Error(format!("bad request: {e}")),
        },
        LOCK => {
            let r: LockRequest = match serde_json::from_slice(payload) {
                Ok(r) => r,
                Err(e) => return Reply::Error(format!("bad request: {e}")),
            };
            sh.presence
                .set_box_reads_all_places(fingerprint, r.all_places);
            sh.presence
                .report_box(fingerprint, &r.name, r.clients, &here);
            let lock = match r.op {
                LockOp::Take => {
                    match sh
                        .presence
                        .take(r.collection, r.kind, &r.id, &r.client_id, r.take_over)
                    {
                        Ok(l) => Some(l),
                        Err(e) => return super::backups::remote::error_reply(&e),
                    }
                }
                LockOp::Release => {
                    sh.presence
                        .release(r.collection, r.kind, &r.id, &r.client_id);
                    None
                }
            };
            let (presence, stamp) = sh.presence.stamped_list_for_box(fingerprint, &here);
            let lock = lock.map(|l| {
                presence
                    .locks
                    .iter()
                    .find(|x| x.kind == l.kind && x.id == l.id && x.client_id == l.client_id)
                    .cloned()
                    .unwrap_or(l)
            });
            json(&LockAnswer {
                v: VERSION,
                lock,
                presence: list_for(sh, fingerprint, presence),
                stamp: Some(stamp),
                all_places: true,
            })
        }
        _ => Reply::Unknown,
    }
}

fn ask<T: serde::de::DeserializeOwned>(
    channel: &Channel,
    codex: &str,
    kind: &str,
    body: &impl Serialize,
) -> Result<T, ApiError> {
    let payload =
        serde_json::to_vec(body).map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?;
    match channel.request(codex, kind, &payload, ASK) {
        Ok(Reply::Ok(b)) => serde_json::from_slice(&b).map_err(|e| {
            ApiError::new(
                ErrorCode::Internal,
                format!("{kind}: unreadable answer: {e}"),
            )
        }),
        Ok(Reply::Error(e)) => Err(super::backups::remote::error_of(kind, &e)),
        Ok(other) => Err(ApiError::new(
            ErrorCode::Unavailable,
            format!("the site's Codex didn't take {kind}: {other:?}"),
        )),
        Err(e) => Err(ApiError::new(
            ErrorCode::Unavailable,
            format!("the site's Codex can't be reached from this box now: {e}"),
        )),
    }
}

/// On a show box: takes (over) or releases a lock on the site's Codex (blocking). `None` when
/// the box isn't a show box linked to the Codex (the lock is this box's own then).
pub fn forward_lock(
    sh: &Shared,
    op: LockOp,
    kind: DocumentKind,
    id: &str,
    client_id: &str,
    take_over: bool,
) -> Option<Result<Option<DocumentLock>, ApiError>> {
    if sh.codex().config.role != CodexRole::Client {
        return None;
    }
    let (channel, codex) = match sh.mirror.channel() {
        Ok(c) => c,
        Err(e) => return Some(Err(e)),
    };
    let req = LockRequest {
        v: VERSION,
        name: here(sh),
        clients: own_for_codex(sh),
        op,
        kind,
        id: id.to_string(),
        client_id: client_id.to_string(),
        take_over,
        all_places: true,
        collection: sh.codex().config.collection,
    };
    Some(ask::<LockAnswer>(&channel, &codex, LOCK, &req).map(|a| {
        if sh.presence.set_codex_reads_all_places(a.all_places) {
            // Report again with every place (the presence thread wakes on the change).
            sh.presence.touch();
        }
        sh.presence.set_site_stamped(Some(a.presence), a.stamp);
        a.lock
    }))
}

/// On a show box: the Codex pushed the site's list.
pub fn pushed(sh: &Shared, payload: &[u8]) -> Reply {
    match serde_json::from_slice::<Pushed>(payload) {
        Ok(p) => {
            if sh.codex().config.role == CodexRole::Client {
                sh.presence.set_site_stamped(Some(p.presence), p.stamp);
            }
            Reply::Ok(b"{}".to_vec())
        }
        Err(e) => Reply::Error(format!("bad request: {e}")),
    }
}

/// Starts the server's site-presence thread (it ends with the server).
pub fn start(sh: &Arc<Shared>) {
    let weak = Arc::downgrade(sh);
    if let Err(e) = std::thread::Builder::new()
        .name("site-presence".into())
        .spawn(move || run(weak))
    {
        tracing::error!("the site presence thread didn't start: {e}");
    }
}

fn run(weak: Weak<Shared>) {
    let mut seen = 0u64;
    let mut reported: Option<Instant> = None;
    let mut failing = false;
    loop {
        let Some(sh) = weak.upgrade() else { return };
        if sh.codex_link.stopping() {
            sh.presence.set_site(None);
            return;
        }
        let now = sh.presence.generation();
        let changed = now != seen;
        seen = now;
        if sh.codex().config.role == CodexRole::Client {
            let due = changed || reported.is_none_or(|t| t.elapsed() >= REPORT_EVERY);
            match sh.mirror.channel() {
                Ok((channel, codex)) if due => {
                    let r = Report {
                        v: VERSION,
                        name: here(&sh),
                        clients: own_for_codex(&sh),
                        stamped: true,
                        all_places: true,
                        node: sh.mosaic.own_node(),
                    };
                    match ask::<ReportReply>(&channel, &codex, PRESENCE, &r) {
                        Ok(reply) => {
                            let (list, stamp, all) = match reply {
                                ReportReply::Stamped(a) => {
                                    (a.presence, Some(a.stamp), a.all_places)
                                }
                                ReportReply::Bare(l) => (l, None, false),
                            };
                            if sh.presence.set_codex_reads_all_places(all) {
                                // Report again at once, with the places it reads now.
                                sh.presence.touch();
                            }
                            if failing {
                                tracing::info!("site presence: the site's Codex answers again");
                            }
                            failing = false;
                            reported = Some(Instant::now());
                            sh.presence.set_site_stamped(Some(list), stamp);
                        }
                        Err(e) => {
                            if !failing {
                                tracing::info!("site presence: {}", e.message);
                            }
                            failing = true;
                            reported = Some(Instant::now());
                            // (It may be an older Codex now: the next report says only what
                            // that reads, and its answer tells again.)
                            sh.presence.set_codex_reads_all_places(false);
                            // (This box's own list meanwhile; writes need the Codex anyway.)
                            sh.presence.set_site(None);
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => sh.presence.set_site(None),
            }
        } else {
            sh.presence.set_site(None);
            sh.presence.expire();
            if changed {
                push(&sh);
            }
        }
        drop(sh);
        // Woken by a change, or at least every second (to notice stops, expiries and the
        // reports due).
        let Some(sh) = weak.upgrade() else { return };
        let p = &sh.presence;
        let _ = p.wait_change(seen, Duration::from_secs(1));
    }
}

/// On the Codex box: the site's list to every box that reports (each as it sees it; a thread
/// each: a box that doesn't answer reports anyway).
fn push(sh: &Shared) {
    let boxes = sh.presence.boxes();
    if boxes.is_empty() {
        return;
    }
    let Some(channel) = sh.codex_link.channel() else {
        return;
    };
    let here = here(sh);
    for fp in boxes {
        let (presence, stamp) = sh.presence.stamped_list_for_box(&fp, &here);
        let body = serde_json::to_vec(&Pushed {
            v: VERSION,
            presence: list_for(sh, &fp, presence),
            stamp: Some(stamp),
        })
        .unwrap_or_default();
        let ch = channel.clone();
        let _ = std::thread::Builder::new()
            .name("presence-push".into())
            .spawn(move || {
                if let Err(e) = ch.request(&fp, PRESENCE_CHANGED, &body, PUSH) {
                    tracing::debug!(box_fp = %fp, "site presence: pushing: {e}");
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::OpenDocument;

    /// What a peer from before the places `show` and `timers` reads (no `other` either).
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    enum OldPlace {
        Edit,
        Reflow,
        Library,
    }
    #[derive(Deserialize)]
    struct OldOpen {
        place: OldPlace,
    }
    #[derive(Deserialize)]
    struct OldClient {
        open: Vec<OldOpen>,
    }
    #[derive(Deserialize)]
    struct OldList {
        clients: Vec<OldClient>,
    }

    fn clients() -> Vec<PresenceClient> {
        use PresencePlace::*;
        vec![PresenceClient {
            session: "s1".into(),
            client_id: Some("k1".into()),
            name: "Oriel".into(),
            user: None,
            computer: None,
            open: [Edit, Reflow, Library, Show, Timers, Other]
                .into_iter()
                .map(|place| OpenDocument {
                    kind: DocumentKind::Presentation,
                    id: "p1".into(),
                    place,
                    collection: None,
                })
                .collect(),
            server: None,
        }]
    }

    /// E05-1: an older peer can read what a newer one sends it once cut down, and the
    /// `allPlaces` marks default to "not said" for messages of an older peer.
    #[test]
    fn an_older_peer_reads_the_cut_down_places() {
        let full = serde_json::to_vec(&PresenceList {
            clients: clients(),
            locks: Vec::new(),
        })
        .unwrap();
        assert!(serde_json::from_slice::<OldList>(&full).is_err(), "the bug");
        let cut = for_older_peer(clients());
        let old: OldList = serde_json::from_slice(
            &serde_json::to_vec(&PresenceList {
                clients: cut.clone(),
                locks: Vec::new(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            old.clients[0]
                .open
                .iter()
                .map(|d| &d.place)
                .collect::<Vec<_>>()[..],
            [
                OldPlace::Edit,
                OldPlace::Reflow,
                OldPlace::Library,
                OldPlace::Library
            ]
        ));
        assert_eq!(cut[0].open[3].place, PresencePlace::Library, "the timer");
        // An older box's report and lock request: not said.
        let r: Report = serde_json::from_value(
            serde_json::json!({"name": "OLD", "clients": [], "stamped": true}),
        )
        .unwrap();
        assert!(!r.all_places);
        // An older Codex's answer: not said; a newer one's: said.
        let stamp = serde_json::json!({"epoch": "e", "generation": 1});
        let older =
            serde_json::json!({"v": 1, "presence": {"clients": [], "locks": []}, "stamp": stamp});
        let Ok(ReportReply::Stamped(a)) = serde_json::from_value::<ReportReply>(older.clone())
        else {
            panic!()
        };
        assert!(!a.all_places);
        let mut newer = older;
        newer["allPlaces"] = true.into();
        let Ok(ReportReply::Stamped(a)) = serde_json::from_value::<ReportReply>(newer) else {
            panic!()
        };
        assert!(a.all_places);
    }
}
