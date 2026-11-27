//! WebSocket messages and errors.
//!
//! One WebSocket per client at [`WS_PATH`](crate::WS_PATH). Text frames carry JSON
//! [`ClientMessage`]s / [`ServerMessage`]s; binary frames (server → client only) carry live
//! preview images ([`crate::preview`]).
//!
//! ```text
//! client                                   server
//!   ── connect /api/v1/ws?protocolVersion=1&token=… ──►
//!   ◄── {"type":"hello","server":{…}}
//!   ◄── {"type":"state","revision":7,"state":{…}}
//!   ── {"type":"command","id":1,"command":{"type":"next"}} ──►
//!   ◄── {"type":"state","revision":8,"state":{…}}        (to every client)
//!   ◄── {"type":"result","id":1,"ok":true,"revision":8}  (to the sender)
//! ```
//!
//! The state for a command is always sent before its result, so a client that receives
//! `result` with `revision: n` already has state `n` (or newer).

use crate::command::Command;
use crate::state::ShowState;
use model::DocumentKind;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A message from a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ClientMessage {
    /// Optional first message (instead of the `protocolVersion` query parameter).
    Hello {
        /// The client's protocol major version.
        protocol_version: u32,
        /// Display name of the client (logs).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        client_name: Option<String>,
    },
    /// Run a command. The server answers with a `result` carrying the same `id`.
    Command {
        /// Chosen by the client; echoed in the result.
        id: u32,
        /// The command.
        command: Command,
    },
    /// Application-level heartbeat: the server answers `pong` with the same `id` right away.
    /// Browsers cannot see WebSocket ping frames, so clients use this to detect a dead link.
    Ping {
        /// Chosen by the client; echoed in the pong.
        id: u32,
    },
    /// What this client has open (presence, see [`crate::presence`]): the whole list, which
    /// replaces the one sent before. Sent after every change and every (re)connect.
    Presence {
        /// The client's `clientId` (also its `X-Client-Id`): ties its REST writes to this
        /// session, for edit locks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        client_id: Option<String>,
        /// The computer's name, shown to others.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        computer: Option<String>,
        /// The documents open (at most [`crate::MAX_OPEN_DOCUMENTS`]).
        open: Vec<crate::presence::OpenDocument>,
    },
}

/// A message from the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ServerMessage {
    /// First message on every connection.
    Hello {
        /// Who the server is.
        server: ServerHello,
    },
    /// The full show state. Sent on connect and after every change; `revision` increases by at
    /// least one each time (a slow client may skip revisions: only the newest state matters).
    State {
        /// Monotonic revision of this state.
        revision: u64,
        /// The state.
        state: ShowState,
    },
    /// The outcome of a client's command.
    Result(CommandResult),
    /// A stored document changed (refetch it over REST).
    DocumentChanged {
        /// Document kind.
        kind: DocumentKind,
        /// Document ID (UUID).
        id: String,
        /// What happened.
        change: DocumentChange,
        /// The document's revision after the change (edits and CRUD; absent for removals and
        /// reloads).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        revision: Option<String>,
        /// The client that made the change (its `clientId` / `X-Client-Id`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        client_id: Option<String>,
        /// The edit ops as applied (edits only): a client holding the previous revision can
        /// patch its copy instead of refetching.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        ops: Option<crate::edit::EditOps>,
        /// The Codex collection the document belongs to (collection kinds: presentations,
        /// libraries, playlists, timers, …; absent for this box's own documents such as
        /// screens).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        collection: Option<model::CollectionId>,
    },
    /// Parts of the Codex changed (collections, this box's active collection): refetch them
    /// over REST. `collection` names the collection concerned, when there is one.
    CodexChanged {
        /// What changed.
        parts: Vec<crate::codex::CodexPart>,
        /// The collection concerned.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        collection: Option<model::CollectionId>,
    },
    /// Something an operator should see (missing media, font substitutions, output errors).
    Notice {
        /// Severity.
        level: NoticeLevel,
        /// Human-readable text.
        message: String,
        /// Stable key of a rolling notice (`fonts`, `media`, `missingMedia:<presentation>`,
        /// ...). A notice with the same key *replaces* the previous one instead of stacking.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        key: Option<String>,
    },
    /// Live peak meters of the master bus: dBFS per channel, rounded to whole dB (`-60` =
    /// silence or below). At most 10 a second, and only while a level changes; the newest
    /// values are also in the next `state` (`audio.levels`).
    AudioLevels {
        /// Peak level per channel, dBFS.
        levels: Vec<f64>,
        /// Output buffer underruns since the server started.
        xruns: u64,
    },
    /// Live timer values: sent at most once a second while a timer runs (only the running,
    /// overrun or just-expired timers). The full values are also in every `state`
    /// (`timers`); a state change of a timer (started, expired, …) always comes as a `state`.
    TimerTick {
        /// The timers.
        timers: Vec<crate::state::TimerTick>,
    },
    /// A typed site event (joins, join codes, devices; see [`crate::site`]). Sent while this
    /// server runs the site's Site Manager.
    SiteEvent {
        /// The event.
        event: crate::site::SiteEvent,
    },
    /// Parts of the site's state changed: refetch them over REST.
    SiteChanged {
        /// What changed.
        parts: Vec<crate::site::SitePart>,
    },
    /// Who has what open, and the edit locks (see [`crate::presence`]): the whole list, after
    /// every change.
    PresenceChanged {
        /// Every session and lock.
        presence: crate::presence::PresenceList,
    },
    /// The live network video sources changed (listed, gone, or how one is received): the
    /// whole list, as `GET /api/v1/sources` answers it (see [`crate::sources`]).
    SourcesChanged {
        /// Every source.
        sources: Vec<crate::sources::LiveSource>,
    },
    /// The answer to a client's `ping`.
    Pong {
        /// The ping's `id`.
        id: u32,
    },
    /// A protocol-level error that is not tied to a command (malformed message, incompatible
    /// protocol version). Incompatible versions are followed by a close frame.
    Error {
        /// What went wrong.
        error: ApiError,
    },
}

/// Server identity, as sent in `hello`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServerHello {
    /// Stable server ID (UUID, survives restarts).
    pub id: String,
    /// Display name (default: the host name).
    pub name: String,
    /// Server software version.
    pub version: String,
    /// Protocol major version ([`PROTOCOL_VERSION`](crate::PROTOCOL_VERSION)).
    pub protocol_version: u32,
}

/// The result of one command (`{"type":"result", ...}` on the WebSocket; the same fields
/// without `id` are the body of `POST /api/v1/commands`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    /// The command's `id` (0 over REST).
    pub id: u32,
    /// Whether the command was applied.
    pub ok: bool,
    /// Revision of the state that includes the command's effect (also set when the command
    /// changed nothing: then it is the current revision).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revision: Option<u64>,
    /// Why it failed (`ok: false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<ApiError>,
}

impl CommandResult {
    /// A success.
    pub fn ok(id: u32, revision: u64) -> Self {
        CommandResult {
            id,
            ok: true,
            revision: Some(revision),
            error: None,
        }
    }

    /// A failure.
    pub fn err(id: u32, error: ApiError) -> Self {
        CommandResult {
            id,
            ok: false,
            revision: None,
            error: Some(error),
        }
    }
}

/// How a document changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum DocumentChange {
    /// New document.
    Added,
    /// Changed content.
    Changed,
    /// Deleted.
    Removed,
}

/// Severity of a notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum NoticeLevel {
    /// For information.
    Info,
    /// Something is degraded (a font was substituted, a media file is missing).
    Warning,
    /// Something failed (an output stopped).
    Error,
}

/// An error: a stable machine-readable `code` plus a human-readable `message`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    /// Stable code.
    pub code: ErrorCode,
    /// Human-readable explanation (English, for logs and tooltips).
    pub message: String,
    /// The request field the error is about, as a dotted camelCase path (`name`, `size`,
    /// `transport.sourceName`), so a form can show it next to that field. Validation errors
    /// only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub field: Option<String>,
    /// On `409 liveShow`: what is live on the box, each named for people (`a slide of
    /// “Song”`, `media “Loop”`, `the message “Parking”`), so a client can ask before it
    /// confirms. (Boxed: an error stays small.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<Vec<String>>")]
    #[allow(clippy::box_collection)]
    pub live: Option<Box<Vec<String>>>,
}

impl ApiError {
    /// An error with a code and message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        ApiError {
            code,
            message: message.into(),
            field: None,
            live: None,
        }
    }

    /// A `badRequest` validation error about one request field.
    pub fn invalid(field: impl Into<String>, message: impl Into<String>) -> Self {
        ApiError {
            code: ErrorCode::BadRequest,
            message: message.into(),
            field: Some(field.into()),
            live: None,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

/// Error codes. REST responses use the HTTP status in brackets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// Malformed request or message, or invalid arguments (400).
    BadRequest,
    /// No valid token (401). Pair first.
    Unauthorized,
    /// Valid client, but the route is reserved for the server machine itself, or the site
    /// role it acts as may not do this (403).
    Forbidden,
    /// Unknown presentation, slide, look, playlist, … (404).
    NotFound,
    /// The command does not make sense in the current state, e.g. `next` before anything was
    /// triggered, or a document changed since the client read it (`If-Match` mismatch) (409).
    Conflict,
    /// Someone else holds the document's edit lock (423): look it up in `GET
    /// /api/v1/presence`, or take it over (`POST /api/v1/presence/locks` with `takeOver`).
    Locked,
    /// Something is live on this box that the request would take off the screens (409): send
    /// it again with `confirm: true` to go ahead (`PUT /api/v1/codex/assignment`).
    LiveShow,
    /// Not enough disk space on the server for this (507): a media upload would leave less free
    /// than the server keeps (5 % of the disk plus 2 GiB).
    InsufficientStorage,
    /// Defined in the protocol but not implemented by this server yet (501).
    NotImplemented,
    /// Too many attempts; try again later (429).
    RateLimited,
    /// The client's protocol major version is not the server's (400).
    IncompatibleProtocol,
    /// The command only works over the WebSocket (e.g. preview subscriptions) (400).
    WsOnly,
    /// Temporarily unavailable, e.g. the renderer is gone (503).
    Unavailable,
    /// A bug or an I/O failure on the server (500).
    Internal,
    /// This box is a show box: the request belongs to the box that holds the site's Codex
    /// (uploads, collection administration). The body's `codex` names that box (409).
    CodexElsewhere,
    /// A code from a newer server.
    #[serde(other)]
    Unknown,
}

impl ErrorCode {
    /// The HTTP status used for this code on REST routes.
    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::BadRequest | ErrorCode::IncompatibleProtocol | ErrorCode::WsOnly => 400,
            ErrorCode::Unauthorized => 401,
            ErrorCode::Forbidden => 403,
            ErrorCode::NotFound => 404,
            ErrorCode::Conflict | ErrorCode::LiveShow | ErrorCode::CodexElsewhere => 409,
            ErrorCode::Locked => 423,
            ErrorCode::RateLimited => 429,
            ErrorCode::NotImplemented => 501,
            ErrorCode::Unavailable => 503,
            ErrorCode::InsufficientStorage => 507,
            ErrorCode::Internal | ErrorCode::Unknown => 500,
        }
    }
}

/// Body of every REST error response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    /// The error.
    pub error: ApiError,
    /// On an edit conflict (`409` after an `If-Match` mismatch): the document as it is now
    /// (a [`ScreenEntry`](crate::ScreenEntry) or [`OutputEntry`](crate::OutputEntry)), or
    /// `null` when it was deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "unknown", optional)]
    pub current: Option<serde_json::Value>,
    /// On a show box (`409 codexElsewhere`, or `503` while it doesn't know): the box that
    /// holds the site's Codex, where uploads and Codex management go.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub codex: Option<crate::CodexBox>,
}
