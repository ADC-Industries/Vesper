//! Playlists (service orders) and playlist folders.
//!
//! The playlist tree is stored as flat [`PlaylistNode`] documents with a `parent` and a
//! fractional `sortKey`, so a move is a single-document update and two operators editing
//! different playlists never conflict. Whether a node is a folder or a playlist is decided by
//! its `kind`, never by a flag (ProPresenter's own `type` field is unreliable). Folders and
//! playlists share the [`PlaylistId`] key space because `parent` may point at either kind
//! (only folders have children).

use crate::common::{Color, ImportInfo, Link, SortKey, is_default, is_true, yes};
use crate::ids::{ArrangementId, PlaylistId, PlaylistItemId, PresentationId};
use crate::media::{MediaOrigin, MediaTrigger, VideoInputTrigger};
use serde::{Deserialize, Serialize};

/// A node of the playlist tree (stored document): a folder or a playlist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PlaylistNode {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Node ID.
    pub id: PlaylistId,
    /// Parent folder (`None` = top level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub parent: Option<PlaylistId>,
    /// Name ("Sunday Service", "Archive").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order among siblings (folders and playlists share one sequence).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Folder or playlist.
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: PlaylistNodeKind,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for PlaylistNode {
    fn default() -> Self {
        PlaylistNode {
            schema_version: crate::SCHEMA_VERSION,
            id: PlaylistId::new(),
            parent: None,
            name: String::new(),
            sort_key: SortKey::new(),
            kind: PlaylistNodeKind::default(),
            import: None,
        }
    }
}

/// Folder or playlist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlaylistNodeKind {
    /// A folder (children point here through `parent`).
    Folder,
    /// A playlist.
    Playlist {
        /// What the playlist is for (all use the same item type).
        #[serde(default, skip_serializing_if = "is_default")]
        purpose: PlaylistPurpose,
        /// Items in order.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<PlaylistItem>,
    },
    /// Unknown tag (treated as an empty folder).
    #[serde(other)]
    Unknown,
}

impl Default for PlaylistNodeKind {
    /// An empty service playlist.
    fn default() -> Self {
        PlaylistNodeKind::Playlist {
            purpose: PlaylistPurpose::Service,
            items: Vec::new(),
        }
    }
}

/// What a playlist is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PlaylistPurpose {
    /// A service order (presentations, media, headers).
    #[default]
    Service,
    /// A media playlist.
    Media,
    /// An audio playlist (audio media items).
    Audio,
    /// Unknown value (service).
    #[serde(other)]
    Unknown,
}

/// One entry of a playlist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItem {
    /// Item ID (own key space; ProPresenter reuses the cue / presentation UUID here).
    pub id: PlaylistItemId,
    /// Display name override (`None` = the presentation / asset name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<String>,
    /// Disabled (hidden) items stay in the list but are skipped.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// What the item is.
    pub kind: PlaylistItemKind,
    /// Import provenance (e.g. the original arrangement UUID when it did not resolve).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl PlaylistItem {
    /// An enabled item with a fresh ID.
    pub fn new(kind: PlaylistItemKind) -> Self {
        PlaylistItem {
            id: PlaylistItemId::new(),
            name: None,
            enabled: true,
            kind,
            import: None,
        }
    }
}

/// Playlist item variants.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlaylistItemKind {
    /// A presentation from a library.
    Presentation {
        /// The presentation (name hint kept: imported references may not resolve yet).
        presentation: Link<PresentationId>,
        /// Arrangement to play (`None` = the presentation's selected arrangement, else
        /// document order). An imported arrangement that does not resolve becomes `None`, with
        /// the original UUID in the item's `import.originalId`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        arrangement: Option<ArrangementId>,
        /// Source reference of an import that could not be resolved, so a later import can fix
        /// it (`presentation` then holds the source UUID).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        unresolved: Option<MediaOrigin>,
    },
    /// Inline media (walk-in video, announcement still, audio track).
    Media(MediaTrigger),
    /// A live video input.
    VideoInput(VideoInputTrigger),
    /// A section header ("Worship", "Sermon").
    Header {
        /// Header text.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        name: String,
        /// Header colour.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        color: Option<Color>,
    },
    /// A placeholder to be filled later ("Special song TBD"); its text is the item `name`.
    Placeholder,
    /// An imported item kind this build does not model (e.g. Planning Center), kept verbatim.
    Unsupported {
        /// Source kind.
        source_type: String,
        /// Verbatim source JSON.
        #[serde(default, skip_serializing_if = "crate::common::is_null")]
        #[cfg_attr(feature = "ts", ts(type = "unknown", optional = nullable))]
        payload: serde_json::Value,
    },
    /// Unknown tag (skipped).
    #[serde(other)]
    Unknown,
}
