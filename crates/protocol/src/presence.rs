//! Presence and edit locks (G8): who has which document open, where, and who is editing it.
//!
//! Each WebSocket connection is a *session*. A client reports what it has open with a
//! [`crate::ClientMessage::Presence`] message (the whole list, after every change and every
//! reconnect); the server keeps one [`PresenceClient`] per session and broadcasts the whole
//! [`PresenceList`] as [`crate::ServerMessage::PresenceChanged`] after every change. A session
//! ends when its socket closes or the heartbeat gives up on it.
//!
//! Edit locks are hard: a content write to a locked document from another client answers
//! `423 locked` ([`crate::ErrorCode::Locked`]). The first write from a client with a live
//! session takes the lock; anyone may take it over (`POST /api/v1/presence/locks` with
//! `takeOver`). A lock ends when its holder closes the document or its last session ends.
//!
//! On a site with a Codex, presence and locks are site-wide: show boxes report their sessions
//! to the Codex box, which holds every lock and sends each box the merged list (clients of
//! other boxes name their box in `server`).
//!
//! Open documents and locks name the collection they are in (`collection`), so a client shows
//! only those of the collection it shows; an entry without one (an older peer) matches any.

use model::{CollectionId, DocumentKind};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Most documents one presence message may list.
pub const MAX_OPEN_DOCUMENTS: usize = 32;

/// Where a client has a document open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PresencePlace {
    /// The slide editor (Edit).
    Edit,
    /// Reflow (lyrics as text).
    Reflow,
    /// The Library page (viewing, or editing in place). With kind `collection`: the
    /// collection the Library page shows ("2 others have this collection open").
    Library,
    /// The Show page, browsing: the playlist and the presentation it shows. Never takes an
    /// edit lock (the operator runs the service from here).
    Show,
    /// The Timers page: the timer whose settings it shows.
    Timers,
    /// A place a newer client reports (shown, never takes a lock).
    #[serde(other)]
    Other,
}

impl PresencePlace {
    /// A place where the document is edited: a write from a client that has it open here
    /// takes its edit lock, and the lock lasts while one of these lists it.
    pub fn edits(self) -> bool {
        matches!(
            self,
            Self::Edit | Self::Reflow | Self::Library | Self::Timers
        )
    }
}

/// A document a client has open.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenDocument {
    /// Its kind (`presentation`, `theme`, `stageLayout`, `prop`, `timer`, `playlistNode`,
    /// `collection`, …).
    pub kind: DocumentKind,
    /// Its ID.
    pub id: String,
    /// Where it is open.
    pub place: PresencePlace,
    /// The collection it is in: the one the client shows (its box's active collection).
    /// Absent from an older client: it matches the document in any collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub collection: Option<CollectionId>,
}

/// One connected client (a WebSocket session) and what it has open.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PresenceClient {
    /// The session (one per WebSocket connection, made by the server: `s12`).
    pub session: String,
    /// The `clientId` the client reported (Oriel's per-run id, also its `X-Client-Id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub client_id: Option<String>,
    /// The client's key name (`Oriel – Booth laptop`), or `server machine`.
    pub name: String,
    /// The site user it is signed in as (access level `users`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub user: Option<String>,
    /// The computer it runs on, as the client reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub computer: Option<String>,
    /// What it has open.
    pub open: Vec<OpenDocument>,
    /// The Vesper it is connected to, when that is another box of the site (site-wide
    /// presence over the box-to-Codex link); absent for this box's own clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub server: Option<String>,
}

/// An edit lock: who is editing a document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentLock {
    /// The document's kind.
    pub kind: DocumentKind,
    /// The document's ID.
    pub id: String,
    /// The holder's `clientId`.
    pub client_id: String,
    /// The holder's key name (`Oriel – Booth laptop`), or `server machine`.
    pub name: String,
    /// The site user the holder is signed in as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub user: Option<String>,
    /// The holder's computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub computer: Option<String>,
    /// When the holder took it (RFC 3339 UTC).
    pub since: String,
    /// The Vesper the holder is connected to, when that is another box of the site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub server: Option<String>,
    /// The collection the lock is in (the collection its write or take was for). Absent when
    /// not known (an older show box's lock, an older server): it matches the document in any
    /// collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub collection: Option<CollectionId>,
}

/// `GET /api/v1/presence`, and the `presenceChanged` event: every session and every lock.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PresenceList {
    /// The connected clients, oldest session first.
    pub clients: Vec<PresenceClient>,
    /// The edit locks held.
    pub locks: Vec<DocumentLock>,
}

/// `POST /api/v1/presence/locks`: take a document's edit lock (with `X-Client-Id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TakeLock {
    /// The document's kind.
    pub kind: DocumentKind,
    /// The document's ID.
    pub id: String,
    /// Take it from whoever holds it (they are told by `presenceChanged`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub take_over: Option<bool>,
}
