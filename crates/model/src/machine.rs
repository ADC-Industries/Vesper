//! Machine-local configuration: how *this* server binds the portable show to physical and
//! network devices and to local folders.
//!
//! These documents are **not portable** ([`Document::portable`](crate::Document::portable)
//! returns `false`): a show moved to another server, or a backup restored on new hardware,
//! carries no stale device names, network source identities or drive letters. A restored show with no
//! bindings renders its screens with no outputs until an operator (or auto-detection using the
//! portable hints) binds them again.
//!
//! Embedding show audio into an output is decided **only** here
//! ([`OutputBinding::embed_audio`]).

use crate::common::{FrameRate, Rect, is_default, is_false, is_true, is_zero, yes};
use crate::ids::{AudioInputId, MediaRootId, OutputId, ScreenId, VideoInputId};
use serde::{Deserialize, Serialize};

/// Sends one screen (or a region of it) to one destination (stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct OutputBinding {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Output ID.
    pub id: OutputId,
    /// Operator-facing name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Screen rendered to this output.
    pub screen: ScreenId,
    /// The output is active.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Destination.
    #[serde(default, skip_serializing_if = "is_default")]
    pub transport: OutputTransport,
    /// Alpha delivery override (`None` = the screen's portable alpha intent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub alpha_override: Option<OutputAlpha>,
    /// Delivery frame rate (`None` = the screen's render rate; frames are repeated / dropped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub frame_rate_override: Option<FrameRate>,
    /// Delivery resolution (`None` = the screen's canvas size).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub resolution_override: Option<Resolution>,
    /// Crop of the screen canvas sent (canvas px; `None` = whole screen). For tiled /
    /// edge-blended setups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub region: Option<Rect>,
    /// Embed the show audio (IDN / OMT / SDI).
    #[serde(default, skip_serializing_if = "is_false")]
    pub embed_audio: bool,
    /// Delay before delivery, in frames of the output's rate (0–30), to line the picture up
    /// with a delayed camera or audio path. Costs up to `videoDelay + 1` buffered frames.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub video_delay: u32,
    /// Which part of the screen is sent.
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub target: OutputTarget,
    /// The tile this output shows when its screen is Grouped or Edge Blend (row-major index
    /// into the screen's [`ScreenArrangement`](crate::show::ScreenArrangement); `None` = the
    /// whole screen). An explicit `region` wins over the tile's crop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub tile: Option<u32>,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// What an output sends of its screen: the picture, or one half of a Fill/Key pair for a
/// hardware keyer (two outputs of the same screen, one `fill` and one `key`, each on its own
/// display or transport). Which part of the screen (a tile or region) is chosen by
/// [`OutputBinding::tile`] and [`OutputBinding::region`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum OutputTarget {
    /// The whole screen.
    #[default]
    Full,
    /// The fill of a Fill/Key pair: the colour over black, opaque. Straight (unshaped) colour,
    /// or premultiplied (shaped) when the output's alpha type is `premultiplied`.
    Fill,
    /// The key of a Fill/Key pair: the alpha as a grey picture (white = opaque), opaque.
    Key,
    /// A target from a newer schema (sent as `full`).
    #[serde(other)]
    Unknown,
}

impl Default for OutputBinding {
    fn default() -> Self {
        OutputBinding {
            schema_version: crate::SCHEMA_VERSION,
            id: OutputId::new(),
            name: String::new(),
            screen: ScreenId::new(),
            enabled: true,
            transport: OutputTransport::default(),
            alpha_override: None,
            frame_rate_override: None,
            resolution_override: None,
            region: None,
            embed_audio: false,
            video_delay: 0,
            target: OutputTarget::Full,
            tile: None,
        }
    }
}

/// Output destinations.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OutputTransport {
    /// IDN Open Protocol sender (compatible with NDI®): the network video transport.
    /// Receivers list it as `"HOST (sourceName)"` together with every other compatible source.
    ///
    /// Schema 1 also had an `ndi` transport (the official runtime, since removed); stored
    /// bindings are migrated to `idn` with the same fields on load
    /// ([`migrate`](crate::migrate::migrate)). The variant no longer exists: an `ndi` value
    /// that bypasses migration decodes to [`OutputTransport::Unknown`] (output off).
    Idn {
        /// Source name advertised on the network (`"Key-Feed"`).
        source_name: String,
        /// Groups (empty = public).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        groups: Vec<String>,
    },
    /// Open Media Transport sender.
    Omt {
        /// Source name.
        name: String,
    },
    /// A window on a local display.
    Display {
        /// Which monitor.
        #[serde(default, skip_serializing_if = "is_default")]
        selector: DisplaySelector,
        /// Borderless full screen (otherwise a window, for testing).
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        fullscreen: bool,
        /// Mode to set (`None` = the display's preferred mode). A mode the display does not
        /// offer falls back to the preferred one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        mode: Option<DisplayMode>,
    },
    /// SDI card output (later).
    Sdi {
        /// Device name or serial.
        device: String,
        /// Fill connector index.
        #[serde(default, skip_serializing_if = "is_default")]
        port: u32,
    },
    /// An output slot without hardware yet (a wall tile whose projector isn't there yet): kept
    /// in the configuration, never runs.
    Placeholder,
    /// Not configured, or a transport from a newer schema (output stays off).
    #[default]
    #[serde(other)]
    Unknown,
}

/// Identifies a monitor robustly across reboots: match by `connector`, then `name`, then
/// `index`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct DisplaySelector {
    /// Connector (`"HDMI-A-1"`, `"\\\\.\\DISPLAY2"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub connector: Option<String>,
    /// Monitor model name from EDID.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<String>,
    /// OS enumeration index.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub index: Option<u32>,
}

/// A display mode: size and refresh rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DisplayMode {
    /// Width, px.
    pub width: u32,
    /// Height, px.
    pub height: u32,
    /// Refresh rate in millihertz (60 Hz = 60000; 59.94 Hz = 59940), as the display's driver
    /// computes it from the mode's timings.
    pub refresh_mhz: u32,
}

/// Pixel resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Machine-local alpha delivery of one output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OutputAlpha {
    /// Flattened; no alpha sent.
    Opaque,
    /// Alpha sent, colour not premultiplied.
    Straight,
    /// Alpha sent, colour premultiplied.
    Premultiplied,
    /// Fill and key on separate device outputs (SDI keyers).
    KeyFill {
        /// Fill device / connector.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        fill_device: Option<String>,
        /// Key (alpha as greyscale) device / connector.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        key_device: Option<String>,
    },
    /// Unknown tag (use the screen's alpha).
    #[serde(other)]
    Unknown,
}

/// Binds a logical video input to a source on this machine (stored document, keyed by input).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InputBinding {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// The logical input (also this document's key).
    pub input: VideoInputId,
    /// Where frames (and embedded audio) come from.
    #[serde(default, skip_serializing_if = "is_default")]
    pub source: InputSource,
}

/// Binds a logical audio input to a source on this machine (stored document, keyed by input).
/// An IDN or OMT source contributes its audio; a device is an audio capture device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AudioInputBinding {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// The logical input (also this document's key).
    pub input: AudioInputId,
    /// Where the audio comes from.
    #[serde(default, skip_serializing_if = "is_default")]
    pub source: InputSource,
}

/// Input sources (of video inputs, and of audio inputs through [`AudioInputBinding`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InputSource {
    /// IDN Open Protocol receiver (compatible with NDI®). Schema 1's `ndi` source is
    /// migrated to this on load.
    Idn {
        /// Full source name, `"HOST (source)"`.
        source_name: String,
    },
    /// OMT receiver.
    Omt {
        /// Source name or address.
        name: String,
    },
    /// Local capture device.
    Device {
        /// Device name or stable identifier.
        name: String,
    },
    /// Not bound, or a source from a newer schema (shows nothing).
    #[default]
    #[serde(other)]
    Unknown,
}

/// Maps a portable media root to a folder on this machine (stored document, keyed by root).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MediaRootBinding {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// The root (also this document's key).
    pub root: MediaRootId,
    /// Absolute local folder (or mounted share).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub local_path: String,
}

/// The local audio device for the show audio bus (singleton stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceConfig {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Device name as the OS reports it (empty = no local audio).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub device: String,
    /// Bus channel → device channel: entry *i* is the 0-based device channel bus channel *i*
    /// plays on. Empty = identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channel_map: Vec<u32>,
    /// Master gain in dB (0 = unity).
    #[serde(
        default,
        with = "crate::common::gain_db",
        skip_serializing_if = "is_zero"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<f64>", optional))]
    pub master_gain_db: f64,
}

/// Font substitutions on this machine (singleton stored document). Rules are tried in order;
/// the first match wins. The importer's missing-font report suggests entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FontSubstitutionTable {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Rules in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<FontSubstitution>,
}

/// "When this font is requested, render with that installed family."
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct FontSubstitution {
    /// Match by PostScript name (`"Calibri-Bold"`); checked before `family`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub postscript_name: Option<String>,
    /// Match by family (`"Calibri"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub family: Option<String>,
    /// Installed family to use instead (`"Carlito"`); weight and italic are kept.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub replacement_family: String,
}

impl FontSubstitutionTable {
    /// The replacement family for a requested font, if any rule matches (case-insensitive).
    pub fn substitute(&self, font: &crate::text::FontRef) -> Option<&str> {
        let eq = |a: &Option<String>, b: Option<&str>| match (a, b) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => false,
        };
        self.rules
            .iter()
            .find(|r| eq(&r.postscript_name, font.postscript_name.as_deref()))
            .or_else(|| {
                self.rules
                    .iter()
                    .find(|r| eq(&r.family, Some(&font.family)))
            })
            .map(|r| r.replacement_family.as_str())
    }
}

impl Default for InputBinding {
    fn default() -> Self {
        InputBinding {
            schema_version: crate::SCHEMA_VERSION,
            input: VideoInputId::new(),
            source: InputSource::default(),
        }
    }
}

impl Default for AudioInputBinding {
    fn default() -> Self {
        AudioInputBinding {
            schema_version: crate::SCHEMA_VERSION,
            input: AudioInputId::new(),
            source: InputSource::default(),
        }
    }
}

impl Default for MediaRootBinding {
    fn default() -> Self {
        MediaRootBinding {
            schema_version: crate::SCHEMA_VERSION,
            root: MediaRootId::new(),
            local_path: String::new(),
        }
    }
}

impl Default for AudioDeviceConfig {
    fn default() -> Self {
        AudioDeviceConfig {
            schema_version: crate::SCHEMA_VERSION,
            device: String::new(),
            channel_map: Vec::new(),
            master_gain_db: 0.0,
        }
    }
}

impl Default for FontSubstitutionTable {
    fn default() -> Self {
        FontSubstitutionTable {
            schema_version: crate::SCHEMA_VERSION,
            rules: Vec::new(),
        }
    }
}

// ---- captions ------------------------------------------------------------------------------

crate::common::default_fns! {
    default_caption_language, is_default_caption_language: String = "en".to_string();
    default_caption_model, is_default_caption_model: String = "auto".to_string();
    default_caption_lines, is_default_caption_lines: u32 = 2;
    default_caption_size, is_default_caption_size: f64 = 56.0;
    default_caption_text, is_default_caption_text: crate::common::Color = crate::common::Color::WHITE;
    default_caption_box, is_default_caption_box: crate::common::Color =
        crate::common::Color::rgba(0.0, 0.0, 0.0, 0.75);
    default_caption_margin, is_default_caption_margin: f64 = 40.0;
}

/// Live captions on this machine (singleton stored document, `docs/CAPTIONS-PLAN.md`):
/// speech from a sound source turned into text (IDN's `idn-captions`, whisper.cpp on this
/// box) and drawn on the [`Layer::Captions`](crate::Layer::Captions) layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CaptionsConfig {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Captions are running.
    #[serde(default, skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// Where the speech comes from.
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub source: CaptionSource,
    /// The language spoken (`en`, `es`, …). Languages other than English need a multilingual
    /// model.
    #[serde(
        default = "default_caption_language",
        skip_serializing_if = "is_default_caption_language"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub language: String,
    /// The speech model: `auto` (measured on this box once) or a model name (`base.en`,
    /// `small.en`, …; `GET /api/v1/captions/models`).
    #[serde(
        default = "default_caption_model",
        skip_serializing_if = "is_default_caption_model"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub model: String,
    /// What speech recognition runs on: the CPU (default) or the GPU. A GPU that can't be used
    /// never stops captions: they run on the CPU instead, and the captions status says why.
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub processor: CaptionProcessor,
    /// Names and words to spell right ("Pastor Example, Ephesians").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub words: String,
    /// Caption sung lines too (whisper invents text over music, so they are left out unless
    /// this is set).
    #[serde(default, skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub keep_music: bool,
    /// Lines shown at once (1–4).
    #[serde(
        default = "default_caption_lines",
        skip_serializing_if = "is_default_caption_lines"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub max_lines: u32,
    /// How the captions box looks.
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub style: CaptionStyle,
    /// Drawn on audience screens (the captions layer; looks can hide it per screen).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub audience: bool,
    /// Drawn on stage screens too (over their layout).
    #[serde(default, skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub stage: bool,
    /// Sent on the network as an IDN text-only source called "Captions".
    #[serde(default, skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub network: bool,
}

impl Default for CaptionsConfig {
    fn default() -> Self {
        CaptionsConfig {
            schema_version: crate::SCHEMA_VERSION,
            enabled: false,
            source: CaptionSource::default(),
            language: default_caption_language(),
            model: default_caption_model(),
            processor: CaptionProcessor::default(),
            words: String::new(),
            keep_music: false,
            max_lines: default_caption_lines(),
            style: CaptionStyle::default(),
            audience: true,
            stage: false,
            network: false,
        }
    }
}

/// Where caption speech comes from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CaptionSource {
    /// The program audio bus (everything Vesper plays).
    #[default]
    Program,
    /// A network source's sound (its full name, `"HOST (Pulpit Mic)"`).
    LiveSource {
        /// The source's full name.
        name: String,
    },
    /// No speech engine: lines come from another captioner (a person typing, a CART
    /// service) through `POST /api/v1/captions/lines`.
    External,
}

/// What speech recognition (whisper) runs on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum CaptionProcessor {
    /// The CPU, at the lowest priority (the default: on a box whose screens render on the
    /// same GPU, whisper on the GPU costs frames).
    #[default]
    Cpu,
    /// The best GPU the speech worker finds (through Vulkan); the CPU (with a note) when there
    /// is none or it fails.
    Gpu,
}

/// Where the captions box sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum CaptionPosition {
    /// Along the bottom edge.
    #[default]
    Bottom,
    /// Along the top edge.
    Top,
}

/// The captions box: text over a band, full width less the margin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct CaptionStyle {
    /// Font family (empty: the server's fallback family).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub font: String,
    /// Text size on a 1080-line screen, px (scaled with the screen).
    #[serde(
        default = "default_caption_size",
        skip_serializing_if = "is_default_caption_size"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub size: f64,
    /// Text colour.
    #[serde(
        default = "default_caption_text",
        skip_serializing_if = "is_default_caption_text"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub color: crate::common::Color,
    /// The band behind the text (alpha 0: none).
    #[serde(
        default = "default_caption_box",
        skip_serializing_if = "is_default_caption_box"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub background: crate::common::Color,
    /// Top or bottom.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub position: CaptionPosition,
    /// Distance from the screen's edges on a 1080-line screen, px.
    #[serde(
        default = "default_caption_margin",
        skip_serializing_if = "is_default_caption_margin"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub margin: f64,
}

impl Default for CaptionStyle {
    fn default() -> Self {
        CaptionStyle {
            font: String::new(),
            size: default_caption_size(),
            color: default_caption_text(),
            background: default_caption_box(),
            position: CaptionPosition::Bottom,
            margin: default_caption_margin(),
        }
    }
}
