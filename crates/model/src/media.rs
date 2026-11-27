//! Media assets, portable media roots, relinking, per-use media triggers and media bins.
//!
//! ## Assets vs. uses
//! Documents never embed file paths. Every slide fill, media action, playlist media item and bin
//! item points at a [`MediaAsset`] by ID. The asset is the **one** place that knows where the
//! file is, whether it is missing and how it was relinked, so relinking `Backgrounds/Sunrise.jpg` once
//! fixes every song that uses it (one missing folder was referenced by 175 songs in the
//! reference library). Per-use settings (layer, scaling, playback, audio routing, transition)
//! live complete at each use site in a [`MediaTrigger`], because ProPresenter stores them per
//! action and the same image is used FILL as a background and FIT as an announcement.
//!
//! ## Missing media is normal
//! The importer never copies media and never mounts shares. Files it cannot find become
//! [`AssetLocation::Linked`] (under a [`MediaRoot`]) or [`AssetLocation::Unresolved`] assets
//! with [`MediaStatus::Missing`], listed in a relink report. The UI shows and relinks them
//! later; triggers on a missing asset render nothing.
//!
//! ## Media roots
//! A [`MediaRoot`] is a portable, named folder ("Shared backgrounds") with [`VolumeHint`]s
//! describing what the source machines called it (drive `X:`, volume "Shared Media", UNC
//! `\\fileserver\media`, path prefix). One root absorbs a NAS mapped under several names. Each
//! server binds a root to a local folder with a machine-local
//! [`MediaRootBinding`](crate::machine::MediaRootBinding). The verbatim import data stays in the
//! immutable [`MediaOrigin`].
//!
//! ## Units
//! Times are seconds (`inPoint`, `outPoint`, fades); `rate` is a speed multiplier; audio gains
//! are dB; crop insets are source pixels.

use crate::common::{
    EdgeInsets, Flip, FrameRate, ImportInfo, Link, SortKey, Timestamp, is_default, is_false,
    is_one, is_true, is_zero, one, yes,
};
use crate::effects::{Effect, Transition};
use crate::ids::{MediaAssetId, MediaBinId, MediaBinItemId, MediaRootId, ThemeId, VideoInputId};
use crate::show::Layer;
use serde::{Deserialize, Serialize};

/// A media file known to the library (stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MediaAsset {
    /// Schema version (see [`crate::migrate()`]).
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Asset ID.
    pub id: MediaAssetId,
    /// Image, video or audio.
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: MediaKind,
    /// Display name (default: file name without extension).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Where the bytes are now.
    #[serde(default, skip_serializing_if = "is_default")]
    pub location: AssetLocation,
    /// Verbatim source reference from the import or upload. Never changes after creation; used
    /// for relink search, bulk re-rooting and the missing-media report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub origin: Option<MediaOrigin>,
    /// Last known availability (persisted so the UI shows missing media without probing).
    #[serde(default, skip_serializing_if = "is_default")]
    pub status: MediaStatus,
    /// When `status` was last checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub checked_at: Option<Timestamp>,
    /// Relink history, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relinks: Vec<RelinkRecord>,
    /// Technical properties (probed, or import hints until probed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub probe: Option<MediaProbe>,
    /// Who owns the asset (the shared library or one theme).
    #[serde(default, skip_serializing_if = "is_default")]
    pub scope: AssetScope,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
    /// Media preparation (`docs/MEDIA-PREP-PLAN.md`): trim, loudness gain, poster frame and
    /// the house-format copy it plays instead of an awkward original.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub prep: Option<MediaPrep>,
}

impl MediaAsset {
    /// A use's playback with this asset's trim as its default range: a use that sets its own
    /// in or out point keeps its whole range (mixing the two could give an in point after the
    /// out point, which players drop).
    pub fn trimmed(&self, pb: &Playback) -> Playback {
        let mut pb = pb.clone();
        if pb.in_point != 0.0 || pb.out_point.is_some() {
            return pb;
        }
        if let Some(p) = &self.prep {
            if let Some(i) = p.trim_in.filter(|i| i.is_finite() && *i > 0.0) {
                pb.in_point = i;
            }
            if let Some(o) = p.trim_out.filter(|o| o.is_finite() && *o > pb.in_point) {
                pb.out_point = Some(o);
            }
        }
        pb
    }

    /// A use's level with this asset's loudness gain added (0 when it has none).
    pub fn levelled(&self, audio: &AudioSettings) -> AudioSettings {
        let mut a = audio.clone();
        if let Some(g) = self.prep.as_ref().and_then(|p| p.gain_db)
            && g.is_finite()
            && a.volume_db > crate::common::SILENCE_DB
        {
            a.volume_db += g;
        }
        a
    }
}

/// What media preparation found and did for an asset. Stored with the asset (so every box
/// that plays it applies the same trim and gain); the trim is the operator's, the rest the
/// preparation job's.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct MediaPrep {
    /// Trim in point, seconds from the file's start (non-destructive: the file is untouched).
    /// A use with its own in point keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub trim_in: Option<f64>,
    /// Trim out point, seconds from the file's start (`None`: the end).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub trim_out: Option<f64>,
    /// Integrated loudness, LUFS (EBU R128), of the file's sound.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub loudness: Option<f64>,
    /// The highest sample, dBFS.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub peak: Option<f64>,
    /// Gain to the house loudness target, dB, applied at playback on top of a use's own
    /// level (never baked into the file).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub gain_db: Option<f64>,
    /// The poster frame's time in the file, seconds (a frame that isn't black).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub poster_at: Option<f64>,
    /// The original file, kept, when the asset now plays a house-format copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub original: Option<PreparedOriginal>,
    /// The SHA-256 of the file last analysed (each file is prepared once).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub analysed: Option<String>,
    /// What the preparation noticed ("Kept as it is: it has an alpha channel").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub notes: Vec<String>,
    /// Why the last preparation failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub error: Option<String>,
    /// When it was last prepared.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub at: Option<Timestamp>,
}

impl MediaPrep {
    /// Forgets what was found about the file (it was relinked to another one); the trim stays.
    pub fn forget_file(&mut self) {
        *self = MediaPrep {
            trim_in: self.trim_in,
            trim_out: self.trim_out,
            ..MediaPrep::default()
        };
    }

    /// Nothing is stored.
    pub fn is_empty(&self) -> bool {
        *self == MediaPrep::default()
    }
}

/// The original file of an asset that plays a house-format copy.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct PreparedOriginal {
    /// Where the original is (it stays in the library).
    #[serde(skip_serializing_if = "is_default")]
    pub location: AssetLocation,
    /// Its facts.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub probe: Option<MediaProbe>,
    /// Why it was converted ("variable frame rate", "interlaced").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub reasons: Vec<String>,
    /// The copy's format ("H.264 1920×1080 29.97 fps, AAC 48 kHz").
    #[serde(skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub format: String,
}

impl Default for MediaAsset {
    fn default() -> Self {
        MediaAsset {
            schema_version: crate::SCHEMA_VERSION,
            id: MediaAssetId::new(),
            kind: MediaKind::default(),
            name: String::new(),
            location: AssetLocation::default(),
            origin: None,
            status: MediaStatus::default(),
            checked_at: None,
            relinks: Vec::new(),
            probe: None,
            scope: AssetScope::default(),
            import: None,
            prep: None,
        }
    }
}

/// Broad media category.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    /// Still image.
    #[default]
    Image,
    /// Video (with optional audio).
    Video,
    /// Audio only.
    Audio,
    /// Unknown value.
    #[serde(other)]
    Unknown,
}

/// Where an asset's bytes live.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AssetLocation {
    /// In the server data folder, content-addressed (`media/<sha256>`). Uploads before the
    /// Codex's media library; still read for old documents (the Codex box moves these files
    /// into its library and rewrites them as [`AssetLocation::Library`]).
    Managed {
        /// Lowercase hex SHA-256 of the file contents.
        sha256: String,
        /// File extension without the dot, lowercase (`"jpg"`).
        #[serde(default, skip_serializing_if = "String::is_empty")]
        extension: String,
    },
    /// An item of the Codex's media library (IDN's site media library, hosted on the box that
    /// holds the Codex; owner 2026-11-22): its lasting item ID and the SHA-256 of its file
    /// (the ID survives renames and moves; the hash finds the same bytes again).
    Library {
        /// The library item's lasting ID (16 hex digits).
        id: String,
        /// Lowercase hex SHA-256 of the file contents.
        sha256: String,
    },
    /// Relative to a portable [`MediaRoot`], resolved per machine through its binding.
    Linked {
        /// The root.
        root: MediaRootId,
        /// Path relative to the root, `/`-separated, no leading slash.
        path: String,
    },
    /// Inside a theme's own asset folder (ProPresenter `ROOT_CURRENT_RESOURCE` `Assets/...`).
    ThemeAsset {
        /// The theme.
        theme: ThemeId,
        /// Path relative to the theme folder, `/`-separated.
        path: String,
    },
    /// Not located yet: only [`MediaAsset::origin`] is known (relink pending).
    #[default]
    Unresolved,
    /// Unknown tag from a newer schema (treated as unresolved).
    #[serde(other)]
    Unknown,
}

/// The verbatim source reference of an imported file. Immutable; every field optional.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct MediaOrigin {
    /// Platform that wrote the reference.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub platform: SourcePlatform,
    /// Original absolute path / URL (`"X:\\Media\\Backgrounds\\Sunrise.jpg"`). Hint only.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub absolute_hint: Option<String>,
    /// Source root kind as written (ProPresenter `"ROOT_SHOW"`, `"ROOT_USER_HOME"`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub root_kind: Option<String>,
    /// Path relative to that root, `/`-separated.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub relative_path: Option<String>,
    /// Windows drive letter without colon (`"X"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub drive_letter: Option<String>,
    /// Volume label (`"Shared Media"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub volume_name: Option<String>,
    /// UNC share (`"\\\\fileserver\\media"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub unc_share: Option<String>,
}

/// The operating system that wrote a path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SourcePlatform {
    /// Windows.
    Windows,
    /// macOS.
    Macos,
    /// Linux.
    Linux,
    /// Not recorded, or a value from a newer schema.
    #[default]
    #[serde(other)]
    Unknown,
}

/// Last known availability of an asset's file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MediaStatus {
    /// Found and readable.
    Online,
    /// Not found: needs relinking (or its volume is not mounted).
    Missing,
    /// Found but cannot be decoded.
    Unsupported,
    /// Not checked yet, or a value from a newer schema.
    #[default]
    #[serde(other)]
    Unknown,
}

/// One relink event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RelinkRecord {
    /// How the file was found.
    #[serde(default, skip_serializing_if = "is_default")]
    pub method: RelinkMethod,
    /// Location before the relink.
    #[serde(default, skip_serializing_if = "is_default")]
    pub previous: AssetLocation,
    /// When it happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub at: Option<Timestamp>,
}

/// How a missing asset was relinked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum RelinkMethod {
    /// The operator picked or uploaded the file.
    #[default]
    Manual,
    /// Same relative path found under a bound media root.
    RootBinding,
    /// Same file name found elsewhere (weak match).
    FileName,
    /// Same content hash found.
    ContentHash,
    /// Unknown value.
    #[serde(other)]
    Unknown,
}

/// Technical properties of a media file. Probed by the server; before the first probe the
/// importer may fill it from source hints (`fromImportHints`), which are often missing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct MediaProbe {
    /// Pixel width (images, video).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub width: Option<u32>,
    /// Pixel height (images, video).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub height: Option<u32>,
    /// Duration in seconds (video, audio).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub duration: Option<f64>,
    /// Frame rate (video).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub frame_rate: Option<FrameRate>,
    /// Has an audio track.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub has_audio: Option<bool>,
    /// Has an alpha channel.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub has_alpha: Option<bool>,
    /// Codec / format name (`"h264"`, `"png"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub codec: Option<String>,
    /// Values came from import hints, not from probing the file.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub from_import_hints: bool,
}

/// Ownership scope of an asset.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AssetScope {
    /// Shared library media.
    #[default]
    Library,
    /// Belongs to a theme; exported and deleted with it.
    Theme {
        /// Owning theme.
        theme: ThemeId,
    },
    /// Unknown tag (library).
    #[serde(other)]
    Unknown,
}

// ---- media roots ----------------------------------------------------------------------------

/// A portable named folder that linked media is relative to (stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MediaRoot {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Root ID.
    pub id: MediaRootId,
    /// Display name ("Shared backgrounds").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Sidebar order.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// What source machines called this location (all names a NAS was mapped under).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<VolumeHint>,
}

impl Default for MediaRoot {
    fn default() -> Self {
        MediaRoot {
            schema_version: crate::SCHEMA_VERSION,
            id: MediaRootId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            hints: Vec::new(),
        }
    }
}

/// A clue about where a root lived on a source machine; any field may be set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct VolumeHint {
    /// Windows drive letter without colon (`"X"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub drive_letter: Option<String>,
    /// Volume label (`"Shared Media"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub volume_name: Option<String>,
    /// UNC prefix (`"\\\\fileserver\\media"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub unc_prefix: Option<String>,
    /// Absolute path prefix on the source machine (`"X:\\Media"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub path_prefix: Option<String>,
}

// ---- per-use settings ---------------------------------------------------------------------------

/// A complete media use: which file, which layer, how it is drawn, played, heard and
/// transitioned. Stored in slide actions, playlist items and bin items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MediaTrigger {
    /// The file.
    pub asset: MediaAssetId,
    /// Target layer.
    #[serde(default, skip_serializing_if = "is_default")]
    pub layer: MediaLayer,
    /// How it is drawn (ignored for audio).
    #[serde(default, skip_serializing_if = "is_default")]
    pub display: MediaDisplay,
    /// How it plays (video / audio).
    #[serde(default, skip_serializing_if = "is_default")]
    pub playback: Playback,
    /// Level and channel routing (video / audio).
    #[serde(default, skip_serializing_if = "is_default")]
    pub audio: AudioSettings,
    /// Transition override for this media (else the layer default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub transition: Option<Transition>,
}

impl MediaTrigger {
    /// A trigger with default settings on `layer`.
    pub fn new(asset: MediaAssetId, layer: MediaLayer) -> Self {
        MediaTrigger {
            asset,
            layer,
            display: MediaDisplay::default(),
            playback: Playback::default(),
            audio: AudioSettings::default(),
            transition: None,
        }
    }
}

/// Layers a media file can target. Each maps to exactly one compositor [`Layer`].
///
/// ProPresenter: `LAYER_TYPE_BACKGROUND` → `background`; `LAYER_TYPE_FOREGROUND` →
/// `foreground`; audio files (written with the background layer) → `audio`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MediaLayer {
    /// Below the slide; stays up across slides until replaced or cleared.
    #[default]
    Background,
    /// Takes over the slide layer (full-screen announcement video); slide elements are hidden
    /// while it is up.
    Foreground,
    /// The live video input layer (e.g. a pre-recorded clip standing in for a camera).
    VideoInput,
    /// Audio only.
    Audio,
    /// The announcements layer.
    Announcements,
    /// Unknown value (treated as background).
    #[serde(other)]
    Unknown,
}

impl MediaLayer {
    /// The compositor layer this media lands on.
    pub fn layer(self) -> Layer {
        match self {
            MediaLayer::Background | MediaLayer::Unknown => Layer::Media,
            MediaLayer::Foreground => Layer::Slide,
            MediaLayer::VideoInput => Layer::VideoInput,
            MediaLayer::Audio => Layer::Audio,
            MediaLayer::Announcements => Layer::Announcements,
        }
    }
}

/// Shows a live video input. Its layer is always [`Layer::VideoInput`], so an input can never
/// sit on an invalid layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VideoInputTrigger {
    /// The logical input (ProPresenter references inputs by index; the importer resolves it).
    pub input: Link<VideoInputId>,
    /// How it is drawn.
    #[serde(default, skip_serializing_if = "is_default")]
    pub display: MediaDisplay,
    /// Level and channel routing of the input's audio for this use.
    #[serde(default, skip_serializing_if = "is_default")]
    pub audio: AudioSettings,
    /// Transition override (else the video-input layer default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub transition: Option<Transition>,
}

impl VideoInputTrigger {
    /// Always [`Layer::VideoInput`].
    pub fn layer(&self) -> Layer {
        Layer::VideoInput
    }
}

/// How a visual media item is fitted and drawn into its target (the whole canvas for a layer,
/// the element bounds for a fill).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct MediaDisplay {
    /// Scaling mode.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub scale: ScaleMode,
    /// Anchor for `fit`, `fill` and `none`.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub anchor: Anchor,
    /// Crop of the source before scaling, in source pixels.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub crop: EdgeInsets,
    /// Corrective rotation of the source.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub rotation: QuarterTurn,
    /// Mirroring.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub flip: Flip,
    /// How to interpret the source alpha channel.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub alpha: SourceAlpha,
    /// Effect stack, applied in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub effects: Vec<Effect>,
}

/// Scaling of media into a target rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ScaleMode {
    /// Fit inside, preserving aspect (letterbox).
    #[default]
    Fit,
    /// Cover the target, preserving aspect (crop overflow).
    Fill,
    /// Stretch exactly (aspect not preserved).
    Stretch,
    /// Native pixel size, positioned by the anchor.
    None,
    /// Unknown value (fit).
    #[serde(other)]
    Unknown,
}

/// A nine-point anchor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Anchor {
    /// Centre.
    #[default]
    Center,
    /// Top-left.
    TopLeft,
    /// Top centre.
    Top,
    /// Top-right.
    TopRight,
    /// Right centre.
    Right,
    /// Bottom-right.
    BottomRight,
    /// Bottom centre.
    Bottom,
    /// Bottom-left.
    BottomLeft,
    /// Left centre.
    Left,
    /// Unknown value (centre).
    #[serde(other)]
    Unknown,
}

/// Rotation in quarter turns, clockwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum QuarterTurn {
    /// None.
    #[default]
    None,
    /// 90° clockwise.
    Cw90,
    /// 180°.
    Cw180,
    /// 270° clockwise.
    Cw270,
    /// Unknown value (none).
    #[serde(other)]
    Unknown,
}

/// How a media source's alpha channel is encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SourceAlpha {
    /// Decide from the file.
    #[default]
    Auto,
    /// Ignore alpha (opaque).
    Opaque,
    /// Straight alpha.
    Straight,
    /// Premultiplied alpha.
    Premultiplied,
    /// Unknown value (auto).
    #[serde(other)]
    Unknown,
}

/// Playback settings of a video / audio use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Playback {
    /// What happens at the out point.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub repeat: Repeat,
    /// What the layer shows after playback ends (when not repeating).
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub end_behavior: EndBehavior,
    /// Start of the played range, seconds from file start.
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub in_point: f64,
    /// End of the played range, seconds (`None` = end of file).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub out_point: Option<f64>,
    /// Speed multiplier.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub rate: f64,
    /// Fade in when starting.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub fade_in: Fade,
    /// Fade out at the end / when cleared.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub fade_out: Fade,
    /// What happens when the same media is triggered while it is already live.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub retrigger: Retrigger,
    /// Cross-fade at the loop point, seconds (seamless loop). `None` = hard loop.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub soft_loop: Option<f64>,
}

impl Default for Playback {
    fn default() -> Self {
        Playback {
            repeat: Repeat::default(),
            end_behavior: EndBehavior::default(),
            in_point: 0.0,
            out_point: None,
            rate: 1.0,
            fade_in: Fade::default(),
            fade_out: Fade::default(),
            retrigger: Retrigger::default(),
            soft_loop: None,
        }
    }
}

/// Repeat behaviour at the out point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Repeat {
    /// Play once.
    #[default]
    Once,
    /// Loop forever (typical for backgrounds).
    Loop,
    /// Play `count` times in total.
    LoopCount {
        /// Total number of plays.
        count: u32,
    },
    /// Keep looping until `seconds` have elapsed.
    LoopFor {
        /// Seconds.
        seconds: f64,
    },
    /// Unknown tag (once).
    #[serde(other)]
    Unknown,
}

/// What a media layer shows after playback ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum EndBehavior {
    /// Hold the last frame.
    #[default]
    HoldLastFrame,
    /// Clear the layer (transparent).
    Clear,
    /// Cut to black.
    Black,
    /// Fade to clear.
    FadeToClear,
    /// Fade to black.
    FadeToBlack,
    /// Unknown value (hold).
    #[serde(other)]
    Unknown,
}

/// A fade-in or fade-out. ProPresenter sets the flag without a duration on almost every video,
/// so the duration is optional: `None` = the show default
/// ([`ShowSettings::default_media_fade`](crate::show::ShowSettings::default_media_fade)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Fade {
    /// The fade is on.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// Fade length in seconds (`None` = show default).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub seconds: Option<f64>,
}

impl Fade {
    /// The effective fade length, or `None` when the fade is off.
    pub fn effective(&self, show_default: f64) -> Option<f64> {
        self.enabled.then(|| self.seconds.unwrap_or(show_default))
    }
}

/// Re-trigger policy when the same media is triggered while it is live.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Retrigger {
    /// Server decides: foreground media restarts, backgrounds keep playing (121 of 140 videos).
    #[default]
    Automatic,
    /// Always restart from the in point.
    Always,
    /// Never restart; keep playing.
    Never,
    /// Unknown value (automatic).
    #[serde(other)]
    Unknown,
}

/// Audio level and channel routing of one use.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct AudioSettings {
    /// No audio at all: this use / input contributes nothing to the audio bus (ProPresenter
    /// `AUDIO_DEVICE_TYPE_NONE`). The level and routing below are kept, so unmuting restores
    /// them. `false` with no routes = pass the source audio through as it is.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub muted: bool,
    /// Gain in dB (0 = unity). Non-finite values are stored as
    /// [`SILENCE_DB`](crate::common::SILENCE_DB); use `muted` for silence.
    #[serde(with = "crate::common::gain_db", skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<f64>", optional))]
    pub volume_db: f64,
    /// Per-channel routing. Empty = default (source channel N → bus channel N).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub channels: Vec<ChannelRoute>,
}

/// Routing of one source audio channel to one bus channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ChannelRoute {
    /// Source channel, 0-based.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub source: u32,
    /// Destination bus channel, 0-based.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub dest: u32,
    /// Gain in dB (0 = unity). Non-finite values are stored as
    /// [`SILENCE_DB`](crate::common::SILENCE_DB).
    #[serde(with = "crate::common::gain_db", skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<f64>", optional))]
    pub gain_db: f64,
    /// Muted.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub muted: bool,
}

// ---- media bins -------------------------------------------------------------------------------

/// A media or audio bin (stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MediaBin {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Bin ID.
    pub id: MediaBinId,
    /// Name ("Backgrounds", "Loops"). Not unique: bins in different folders may share it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Folder path for grouping in the bin sidebar (`["Motion"]`), outermost first
    /// (empty = top level). Like [`Theme::folder`](crate::theme::Theme::folder), folders are
    /// not documents of their own: a folder exists while a bin is in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folder: Vec<String>,
    /// Order among the bins of the same folder.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Media or audio browser.
    #[serde(default, skip_serializing_if = "is_default")]
    pub browser: BinBrowser,
    /// Contents.
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: MediaBinKind,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for MediaBin {
    fn default() -> Self {
        MediaBin {
            schema_version: crate::SCHEMA_VERSION,
            id: MediaBinId::new(),
            name: String::new(),
            folder: Vec::new(),
            sort_key: SortKey::new(),
            browser: BinBrowser::default(),
            kind: MediaBinKind::default(),
            import: None,
        }
    }
}

/// Which browser a bin appears in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum BinBrowser {
    /// Media (video / image).
    #[default]
    Media,
    /// Audio.
    Audio,
    /// Unknown value (media).
    #[serde(other)]
    Unknown,
}

/// Bin contents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MediaBinKind {
    /// A hand-curated list.
    Manual {
        /// Items in order.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<MediaBinItem>,
    },
    /// A watched folder ("smart bin"). The last scan is cached so the bin stays browsable and
    /// triggerable while the folder is offline.
    ///
    /// Scanning: every file under `folder` becomes a [`MediaAsset`] with
    /// [`AssetLocation::Linked`] at ID [`MediaAssetId::for_linked`]`(root, path)` (created if
    /// missing), and a [`MediaBinItem`] at ID [`MediaBinItemId::for_watched`]`(bin, relative
    /// path)` whose trigger is `importBehavior` applied to that asset. IDs are deterministic,
    /// so remote clients can trigger scanned items and rescans keep them stable.
    Watched {
        /// The watched folder.
        folder: WatchedFolder,
        /// Trigger settings applied to newly scanned files.
        #[serde(default, skip_serializing_if = "is_default")]
        import_behavior: TriggerDefaults,
        /// Items found by the last scan, in display order.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        last_scan: Vec<MediaBinItem>,
        /// When the last scan ran.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        scanned_at: Option<Timestamp>,
    },
    /// Unknown tag (empty bin).
    #[serde(other)]
    Unknown,
}

impl Default for MediaBinKind {
    /// An empty manual bin.
    fn default() -> Self {
        MediaBinKind::Manual { items: Vec::new() }
    }
}

/// A watched folder: a media root plus a relative path.
///
/// `root` is required in JSON (ID fields are never defaulted): scanned asset and item IDs are
/// derived from it, so a missing root must fail to load rather than get a random one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WatchedFolder {
    /// The root.
    pub root: MediaRootId,
    /// Folder path relative to the root, `/`-separated (`""` = the root itself).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// Include subfolders.
    #[serde(default, skip_serializing_if = "is_false")]
    pub recursive: bool,
}

/// Default trigger settings for files appearing in a watched folder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct TriggerDefaults {
    /// Target layer.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub layer: MediaLayer,
    /// Drawing.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub display: MediaDisplay,
    /// Playback.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub playback: Playback,
    /// Audio.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub audio: AudioSettings,
}

impl TriggerDefaults {
    /// The trigger a scanned file gets.
    pub fn trigger(&self, asset: MediaAssetId) -> MediaTrigger {
        MediaTrigger {
            asset,
            layer: self.layer,
            display: self.display.clone(),
            playback: self.playback.clone(),
            audio: self.audio.clone(),
            transition: None,
        }
    }
}

/// One item in a media bin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MediaBinItem {
    /// Item ID (deterministic for watched bins).
    pub id: MediaBinItemId,
    /// Display name (`None` = the asset name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<String>,
    /// Disabled items stay in the bin but cannot be triggered.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// What triggering the item does.
    pub trigger: MediaTrigger,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_and_managed_locations_on_the_wire() {
        let lib = AssetLocation::Library {
            id: "0123456789abcdef".into(),
            sha256: "ab".repeat(32),
        };
        let v = serde_json::to_value(&lib).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"type": "library", "id": "0123456789abcdef", "sha256": "ab".repeat(32)})
        );
        assert_eq!(serde_json::from_value::<AssetLocation>(v).unwrap(), lib);
        // Old documents' managed locations still read.
        let old: AssetLocation = serde_json::from_value(
            serde_json::json!({"type": "managed", "sha256": "cd", "extension": "png"}),
        )
        .unwrap();
        assert_eq!(
            old,
            AssetLocation::Managed {
                sha256: "cd".into(),
                extension: "png".into()
            }
        );
    }

    #[test]
    fn prep_trim_and_gain_apply_at_playback_unless_the_use_sets_its_own() {
        let mut a = MediaAsset::default();
        let pb = Playback::default();
        assert_eq!(a.trimmed(&pb), pb);
        a.prep = Some(MediaPrep {
            trim_in: Some(2.0),
            trim_out: Some(9.5),
            gain_db: Some(-4.5),
            ..MediaPrep::default()
        });
        let t = a.trimmed(&pb);
        assert_eq!((t.in_point, t.out_point), (2.0, Some(9.5)));
        // A use with its own range keeps it.
        let own = Playback {
            in_point: 1.0,
            out_point: Some(3.0),
            ..Playback::default()
        };
        assert_eq!(a.trimmed(&own), own);
        // An out point before the use's in point is ignored.
        let late = Playback {
            in_point: 10.0,
            ..Playback::default()
        };
        assert_eq!(a.trimmed(&late).out_point, None);
        // A use with only its own out point (the first 3 s) keeps its range: the trim's in
        // point (after it) would make the player drop the out point and play to the end.
        let first = Playback {
            out_point: Some(1.5),
            ..Playback::default()
        };
        assert_eq!(a.trimmed(&first), first);
        // Applied twice, the same.
        assert_eq!(a.trimmed(&a.trimmed(&pb)), t);
        let au = AudioSettings {
            volume_db: -2.0,
            ..AudioSettings::default()
        };
        assert_eq!(a.levelled(&au).volume_db, -6.5);
        // Silence stays silence.
        let silent = AudioSettings {
            volume_db: crate::common::SILENCE_DB,
            ..AudioSettings::default()
        };
        assert_eq!(a.levelled(&silent).volume_db, crate::common::SILENCE_DB);
        // The prep survives a round trip, and relinking forgets all but the trim.
        let mut p = a.prep.clone().unwrap();
        p.loudness = Some(-20.0);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<MediaPrep>(&json).unwrap(), p);
        p.forget_file();
        assert_eq!(p.trim_in, Some(2.0));
        assert_eq!(p.loudness, None);
    }
}
