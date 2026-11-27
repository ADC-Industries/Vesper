//! # `protocol` — the control protocol (REST + WebSocket)
//!
//! Every type that crosses the wire between the server and its clients (the control app, a
//! browser remote, a Stream Deck/Companion script). The server serializes these types and the
//! TypeScript bindings in `crates/protocol/bindings/` are generated from them with `ts-rs`, so
//! the UI and the server cannot drift apart. See `crates/protocol/README.md` for the protocol
//! as a client author sees it (message examples, auth flow, preview frame layout).
//!
//! - [`messages`]: WebSocket messages ([`ClientMessage`], [`ServerMessage`]) and errors.
//! - [`command`]: every [`Command`] (sent over the WebSocket or `POST /api/v1/commands`).
//! - [`state`]: the full operator state ([`ShowState`]) the server broadcasts after each change.
//! - [`edit`]: content editing (presentation / playlist edit ops, library CRUD bodies).
//! - [`rest`]: REST request / response bodies (documents, pairing, server info).
//! - [`preview`]: the binary live-preview frame header.
//! - [`site`]: the site (the embedded IDN Site Manager): joins, devices, users, tokens, audit.
//! - [`codex`]: Vesper Codex, the site's content store: status, collections, assignment.
//! - [`signage`]: signage channels, schedules, playlists, feeds and the override.
//! - [`slides_import`]: PowerPoint / Keynote / Google Slides import into the Codex.
//! - [`updates`]: the site's update channel and the Vesper Launchers' reports (G10).
//! - [`media_prep`]: media preparation (house-format conversion, loudness, poster, trim).
//! - [`reports`]: reports (CCLI song usage).
//! - [`discover`] (feature `discover`): find servers on the LAN over mDNS.
//! - [`bindings`]: TypeScript export (used by the bindings test).
//!
//! ## Conventions
//! JSON text frames, `camelCase` fields, enums with data internally tagged with `"type"` (the
//! same conventions as `model`). Optional fields are omitted rather than `null`, except in
//! [`ShowState`], whose shape is fixed (`null` = nothing there).
//!
//! ## Versioning
//! [`PROTOCOL_VERSION`] is a major version: it changes only for incompatible changes. Adding
//! fields, commands, messages or enum values is compatible; clients ignore what they do not
//! know.

#![warn(missing_docs)]
// Wire enums are built once per message and serialized; boxing the big variants would only
// complicate the API.
#![allow(clippy::large_enum_variant)]

pub mod backup;
pub mod bindings;
pub mod codex;
pub mod command;
#[cfg(feature = "discover")]
pub mod discover;
pub mod edit;
pub mod export;
pub mod media_prep;
pub mod messages;
pub mod mosaic;
pub mod presence;
pub mod preview;
pub mod remote;
pub mod reports;
pub mod rest;
pub mod scripture;
pub mod signage;
pub mod site;
pub mod slides_import;
pub mod sources;
pub mod state;
pub mod streaming;
pub mod translate;
pub mod updates;

pub use backup::*;
pub use codex::*;
pub use command::*;
pub use edit::*;
pub use export::*;
pub use media_prep::*;
pub use messages::*;
pub use mosaic::*;
pub use presence::*;
pub use preview::{PREVIEW_HEADER_LEN, PreviewFrameHeader};
pub use remote::*;
pub use reports::*;
pub use rest::*;
pub use scripture::*;
pub use signage::*;
pub use site::*;
pub use slides_import::*;
pub use sources::*;
pub use state::*;
pub use streaming::*;
pub use translate::*;
pub use updates::*;

/// The server's lowercase product identifier (Vesper): the one place it is spelled for the mDNS
/// service type and the advertised host label.
#[macro_export]
macro_rules! product_id {
    () => {
        "vesper"
    };
}

/// Lowercase server product identifier (see [`product_id!`]).
pub const PRODUCT_ID: &str = product_id!();

/// The server's product name, for display (default server name, logs).
pub const PRODUCT_NAME: &str = "Vesper";

/// mDNS / DNS-SD service type the server advertises (`_vesper._tcp.local.`).
pub const MDNS_SERVICE_TYPE: &str = concat!("_", product_id!(), "._tcp.local.");

/// Protocol major version. A client and a server talk only when their majors are equal.
pub const PROTOCOL_VERSION: u32 = 1;

/// Default TCP port of the server (HTTP + WebSocket).
pub const DEFAULT_PORT: u16 = 5555;

/// Path prefix of every route.
pub const API_PREFIX: &str = "/api/v1";

/// WebSocket path.
pub const WS_PATH: &str = "/api/v1/ws";

/// Highest live-preview rate a client may ask for (frames per second).
pub const MAX_PREVIEW_FPS: f64 = 15.0;

/// Largest live-preview / thumbnail width, px.
pub const MAX_IMAGE_WIDTH: u32 = 1920;

/// Thumbnail width when the request does not say.
pub const DEFAULT_THUMBNAIL_WIDTH: u32 = 320;

/// mDNS TXT record keys.
pub mod txt {
    /// Server ID (stable UUID).
    pub const ID: &str = "id";
    /// Display name.
    pub const NAME: &str = "name";
    /// Server software version.
    pub const VERSION: &str = "version";
    /// Protocol major version.
    pub const PROTOCOL: &str = "proto";
}
