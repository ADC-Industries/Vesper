//! # `model` — the shared data model of the presentation server
//!
//! One set of serde types used by the server (storage and show logic), the API (TypeScript
//! types will be generated from these), the ProPresenter importer and the server's JSON
//! document store (one file per document).
//!
//! ```text
//! Library ── Presentation ─┬─ Section ("Verse 1", colour, hotkey, global group link) ── Slide
//!                          ├─ Arrangement (section IDs, repeats allowed)                 ├─ Element
//!                          └─ song info / scripture                                      └─ SlideAction
//! PlaylistNode (folder | playlist ── PlaylistItem: presentation | media | video input | header)
//! MediaAsset (one per file: location, origin, status, relinks)   MediaRoot   MediaBin
//! Theme ── TemplateSlide (a Slide with placeholder slots)
//! Screen · Look · Mask · VideoInput · AudioInput · Timer · StageLayout · ClearGroup · Prop
//! Message · Macro · GlobalGroup · ShowSettings
//! machine-local: OutputBinding · InputBinding · AudioInputBinding · MediaRootBinding
//!                AudioDeviceConfig · FontSubstitutionTable · CaptionsConfig · CodexConfig
//! Vesper Codex: CodexInfo · Collection (a collection holds the content kinds above)
//! ```
//!
//! ## Conventions (every type in this crate)
//! - **JSON:** `camelCase` fields. Enums with data are internally tagged with `"type"`
//!   (`rename_all_fields = "camelCase"`); plain enums are camelCase strings. No untagged enums
//!   and no `#[serde(flatten)]`: variant data sits in nested `kind` fields.
//! - **Compact:** every field is omitted when it equals its default, and Rust `Default` and the
//!   serde defaults agree (tested per document type). `schemaVersion` is always written.
//! - **IDs:** one newtype per entity ([`ids`]), serialized as UUID strings, separate key spaces.
//! - **Units:** geometry `f64` canvas pixels, origin top-left, y down; colours [`Color`]
//!   straight-alpha sRGB `f32` `0..=1`; durations `f64` seconds; frame rates rational
//!   [`FrameRate`]; gains dB. See [`common`].
//! - **Forward compatibility:** growable enums have a `#[serde(other)] Unknown` fallback.
//!   Imported data we cannot model goes into `Unsupported { sourceType, payload }` variants
//!   (actions, fills, effects, transitions, data links, visibility conditions, playlist items)
//!   or into [`ImportInfo::unmapped`], never dropped.
//! - **Versioning:** every stored document has `schemaVersion`; [`migrate::migrate`] upgrades raw
//!   JSON and refuses documents from a newer schema ([`MigrationError::TooNew`]).
//! - **Storage:** fine-grained documents (one per presentation, playlist node, theme, screen,
//!   look, timer, ...), each implementing [`Document`]; ordered siblings carry a fractional
//!   [`SortKey`]. Edit operations and revisions are API concerns, not part of this crate.
//! - **Themes** are copied into slides ([`template::apply_template`]; `templateOrigin`
//!   provenance) or **linked** ([`theme_link`]: the slide follows its template slide, with
//!   per-property overrides). Looks can restyle text at render time
//!   ([`show::TemplateOverride`]).

#![warn(missing_docs)]
// Data-model enums are serialized values, not hot-path state; boxing variants would only
// complicate the API shape.
#![allow(clippy::large_enum_variant)]

pub mod action;
pub mod backup;
pub mod build;
pub mod chordpro;
pub mod chords;
pub mod codex;
pub mod common;
pub mod effects;
pub mod element;
pub mod ids;
pub mod language;
pub mod machine;
pub mod media;
pub mod migrate;
pub mod playlist;
pub mod presentation;
pub mod show;
pub mod slide;
pub mod songselect;
pub mod template;
pub mod text;
pub mod theme;
pub mod theme_link;

pub use action::*;
pub use backup::*;
pub use build::{Build, BuildMode, BuildStart};
pub use chords::{Accidentals, ChordDisplay, ChordMark, ChordNotation, ChordView};
pub use codex::*;
pub use common::*;
pub use effects::*;
pub use element::*;
pub use ids::*;
pub use language::{
    ElementTranslation, LanguageDisplay, MachineTranslation, SecondLanguageStyle, SlideTranslation,
    language_code,
};
pub use machine::*;
pub use media::*;
pub use migrate::{DecodeError, MigrationError, decode, ensure_writable, migrate, version_for};
pub use playlist::*;
pub use presentation::*;
pub use show::*;
pub use slide::*;
pub use template::apply_template;
pub use text::*;
pub use theme::*;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

/// The schema version documents are written with, unless what they hold needs a newer one
/// ([`version_for`]: so older servers keep reading every document that doesn't).
///
/// - **2:** network video is IDN Open Protocol only: `ndi` output transports and input
///   sources became `idn` (same fields).
pub const SCHEMA_VERSION: u32 = 2;

/// The newest schema version this build reads and writes ([`migrate()`] refuses newer
/// documents, and they are never overwritten). Bump it when a stored shape changes
/// incompatibly and add a step to [`migrate()`].
///
/// - **3:** a media asset may be in the Codex's media library (`AssetLocation::Library`).
///   Only documents holding such a location are written as 3 ([`version_for`]): a server
///   before it would read the location as unknown and lose it on its next save, so it must
///   refuse them instead.
pub const NEWEST_SCHEMA_VERSION: u32 = 3;

/// The kinds of stored documents (one directory each). Also usable as the `{kind}`
/// segment of generic REST routes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DocumentKind {
    /// [`Library`].
    Library,
    /// [`Presentation`].
    Presentation,
    /// [`GlobalGroup`].
    GlobalGroup,
    /// [`PlaylistNode`].
    PlaylistNode,
    /// [`MediaBin`].
    MediaBin,
    /// [`MediaAsset`].
    MediaAsset,
    /// [`MediaRoot`].
    MediaRoot,
    /// [`Theme`].
    Theme,
    /// [`Screen`].
    Screen,
    /// [`Look`].
    Look,
    /// [`Mask`].
    Mask,
    /// [`VideoInput`].
    VideoInput,
    /// [`AudioInput`].
    AudioInput,
    /// [`Timer`].
    Timer,
    /// [`StageLayout`].
    StageLayout,
    /// [`ClearGroup`].
    ClearGroup,
    /// [`Prop`].
    Prop,
    /// [`Message`].
    Message,
    /// [`Macro`].
    Macro,
    /// [`ShowSettings`] (singleton).
    ShowSettings,
    /// [`OutputBinding`] (machine-local).
    OutputBinding,
    /// [`InputBinding`] (machine-local).
    InputBinding,
    /// [`AudioInputBinding`] (machine-local).
    AudioInputBinding,
    /// [`MediaRootBinding`] (machine-local).
    MediaRootBinding,
    /// [`AudioDeviceConfig`] (machine-local singleton).
    AudioDeviceConfig,
    /// [`FontSubstitutionTable`] (machine-local singleton).
    FontSubstitutionTable,
    /// [`CaptionsConfig`] (machine-local singleton).
    CaptionsConfig,
    /// [`CodexInfo`] (`codex/codex.json`, singleton).
    CodexInfo,
    /// [`Collection`] (`codex/collections/<id>/collection.json`).
    Collection,
    /// [`CodexConfig`] (machine-local singleton, `machine/codex.json`).
    CodexConfig,
    /// A kind written by a newer server.
    #[serde(other)]
    Unknown,
}

/// Implemented by every stored document.
pub trait Document: Serialize + DeserializeOwned {
    /// Which kind of document this is (its storage directory).
    const KIND: DocumentKind;
    /// Primary key. Singletons use the nil UUID; bindings use the ID of what they bind.
    fn id(&self) -> Uuid;
    /// The document's `schemaVersion`.
    fn schema_version(&self) -> u32;
    /// `false` for machine-local documents, which are never exported or copied to another
    /// server.
    fn portable() -> bool {
        true
    }
}

macro_rules! impl_document {
    ($($ty:ident => $key:ident),* $(,)?) => {$(
        impl Document for $ty {
            const KIND: DocumentKind = DocumentKind::$ty;
            fn id(&self) -> Uuid {
                self.$key.0
            }
            fn schema_version(&self) -> u32 {
                self.schema_version
            }
        }
    )*};
}

impl_document! {
    Library => id,
    Presentation => id,
    GlobalGroup => id,
    PlaylistNode => id,
    MediaBin => id,
    MediaAsset => id,
    MediaRoot => id,
    Theme => id,
    Screen => id,
    Look => id,
    Mask => id,
    VideoInput => id,
    AudioInput => id,
    Timer => id,
    StageLayout => id,
    ClearGroup => id,
    Prop => id,
    Message => id,
    Macro => id,
}

impl Document for ShowSettings {
    const KIND: DocumentKind = DocumentKind::ShowSettings;
    fn id(&self) -> Uuid {
        Uuid::nil()
    }
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

macro_rules! impl_machine_document {
    ($($ty:ident => |$s:ident| $key:expr),* $(,)?) => {$(
        impl Document for $ty {
            const KIND: DocumentKind = DocumentKind::$ty;
            fn id(&self) -> Uuid {
                let $s = self;
                $key
            }
            fn schema_version(&self) -> u32 {
                self.schema_version
            }
            fn portable() -> bool {
                false
            }
        }
    )*};
}

impl_machine_document! {
    OutputBinding => |s| s.id.0,
    InputBinding => |s| s.input.0,
    AudioInputBinding => |s| s.input.0,
    MediaRootBinding => |s| s.root.0,
    AudioDeviceConfig => |_s| Uuid::nil(),
    FontSubstitutionTable => |_s| Uuid::nil(),
    CaptionsConfig => |_s| Uuid::nil(),
}
