//! Strongly typed identifiers.
//!
//! Every entity has its own ID type (`#[serde(transparent)] pub struct SlideId(pub Uuid)`), so a
//! `PlaylistItemId` can never be passed where a `PresentationId` is expected. On the wire every ID
//! is a plain lowercase hyphenated UUID string, which TypeScript generators emit as a `string`
//! alias.
//!
//! - [`new`](SlideId::new) mints a random (v4) ID; `Default` does the same, so
//!   `..Default::default()` on a struct yields a *new* object. ID fields are never
//!   `#[serde(default)]`: a stored document without its ID fails to load instead of silently
//!   getting a random one.
//! - [`from_uuid`](SlideId::from_uuid) wraps an imported UUID. The ProPresenter census showed
//!   presentation, cue (slide), element, action and cue-group (section) UUIDs are globally unique
//!   in a real library, so the importer reuses them and re-imports stay idempotent.
//!
//! ## Separate key spaces
//! Each ID type is its own namespace (one directory per document kind). Two known ProPresenter
//! collisions are handled this way rather than by rewriting IDs:
//! - playlist items reuse the UUID of the cue or presentation they hold, so [`PlaylistItemId`]
//!   is never compared with presentation or slide IDs;
//! - byte-identical duplicate themes share every UUID; the importer de-duplicates them.
//!
//! ## Deterministic IDs
//! Watched-folder bins are rescanned, so their items must keep stable IDs across scans and
//! servers: [`MediaBinItemId::for_watched`] and [`MediaAssetId::for_linked`] are UUIDv5 values
//! derived from the owning ID plus the `/`-separated relative path.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

macro_rules! id_types {
    ($($(#[$doc:meta])* $name:ident;)*) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// A fresh random (v4) ID.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            /// Wraps an existing UUID (e.g. one read from an imported document).
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }
            /// The underlying UUID.
            pub const fn uuid(&self) -> Uuid {
                self.0
            }
        }

        /// A fresh random ID (see the module docs).
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.hyphenated().fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }
    )*};
}

id_types! {
    /// A library (sidebar shelf of presentations: "Songs", "Readings").
    LibraryId;
    /// A presentation document (song, sermon deck, announcement loop).
    PresentationId;
    /// A section of a presentation (ProPresenter "group": Verse 1, Chorus).
    SectionId;
    /// An arrangement (ordered section list) inside a presentation.
    ArrangementId;
    /// A slide (presentation cue, theme template body, prop, mask, stage layout).
    SlideId;
    /// An element on a slide.
    ElementId;
    /// An action attached to a slide or macro.
    ActionId;
    /// A global group definition (shared "Verse 1" label with colour and hotkey).
    GlobalGroupId;
    /// A media file record.
    MediaAssetId;
    /// A portable named media root (shared drive / NAS folder).
    MediaRootId;
    /// A media or audio bin.
    MediaBinId;
    /// An item inside a media bin.
    MediaBinItemId;
    /// A playlist tree node (folder or playlist).
    PlaylistId;
    /// An entry inside a playlist. Own key space on purpose (see module docs).
    PlaylistItemId;
    /// A theme.
    ThemeId;
    /// A template slide inside a theme.
    TemplateSlideId;
    /// An editor alignment guide on a template slide.
    GuideId;
    /// A logical output canvas ("screen").
    ScreenId;
    /// A look (per-screen layer routing preset).
    LookId;
    /// A mask (slide composited over a screen).
    MaskId;
    /// A portable logical video input.
    VideoInputId;
    /// A portable logical audio input.
    AudioInputId;
    /// A timer.
    TimerId;
    /// A stage display layout.
    StageLayoutId;
    /// A clear group.
    ClearGroupId;
    /// A prop (independently toggled overlay slide).
    PropId;
    /// A message (templated overlay text).
    MessageId;
    /// A macro (named bundle of actions).
    MacroId;
    /// A Vesper Codex (the site's content store).
    CodexId;
    /// A collection of a Codex (one campus, ministry or room's content).
    CollectionId;
    /// A machine-local output binding.
    OutputId;
}

impl MediaBinItemId {
    /// Deterministic ID of a watched-folder item: UUIDv5 of the bin ID (namespace) and the
    /// item's `/`-separated path relative to the watched folder.
    pub fn for_watched(bin: MediaBinId, relative_path: &str) -> Self {
        Self(Uuid::new_v5(&bin.0, relative_path.as_bytes()))
    }
}

impl MediaAssetId {
    /// Deterministic ID of a linked asset: UUIDv5 of the media root ID (namespace) and the
    /// `/`-separated path relative to that root. A watched-folder scan on any server mints the
    /// same asset ID for the same file.
    pub fn for_linked(root: MediaRootId, relative_path: &str) -> Self {
        Self(Uuid::new_v5(&root.0, relative_path.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialize_as_plain_strings() {
        let id = SlideId::from_uuid(Uuid::nil());
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            "\"00000000-0000-0000-0000-000000000000\""
        );
        let back: SlideId =
            serde_json::from_str("\"00000000-0000-0000-0000-000000000000\"").unwrap();
        assert_eq!(back, id);
        assert_eq!(id.to_string().parse::<SlideId>().unwrap(), id);
    }

    #[test]
    fn deterministic_ids_are_stable() {
        let bin = MediaBinId::from_uuid(Uuid::from_u128(7));
        let a = MediaBinItemId::for_watched(bin, "Loops/Sunrise.mp4");
        assert_eq!(a, MediaBinItemId::for_watched(bin, "Loops/Sunrise.mp4"));
        assert_ne!(a, MediaBinItemId::for_watched(bin, "Loops/Clouds.mp4"));
        let root = MediaRootId::from_uuid(Uuid::from_u128(9));
        assert_eq!(
            MediaAssetId::for_linked(root, "a/b.jpg"),
            MediaAssetId::for_linked(root, "a/b.jpg")
        );
    }
}
