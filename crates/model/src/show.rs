//! Portable show configuration: layers, screens, looks, masks, logical inputs, timers, stage
//! layouts, clear groups, props, messages, macros and the show settings singleton.
//!
//! Everything here is **portable**: it can be copied to another server and still make sense.
//! Anything that names a physical device (network source name, monitor, capture card, audio
//! interface) or a local path lives in [`crate::machine`] and points back here by ID. Each
//! entity is its own small stored document with `schemaVersion` and, where siblings are
//! ordered, a fractional `sortKey`.
//!
//! ## Layers
//! Every screen is composited from the fixed [`Layer`] stack, bottom to top
//! ([`Layer::COMPOSITE_ORDER`]): video input, media (background), slide (incl. foreground
//! media), announcements, props, messages; then the screen's mask. Audio is non-visual.
//! A [`Look`] decides per screen which layers are drawn ([`LayerVisibility`]) and may restyle
//! the slide layer's text through a template ([`TemplateOverride`]).

use crate::action::SlideAction;
use crate::common::{
    Color, FrameRate, ImportInfo, Link, Size, SortKey, default_fns, is_default, is_false, is_true,
    yes,
};
use crate::effects::Transition;
use crate::element::{ClockFormat, TimerFormat};
use crate::ids::{
    AudioInputId, ClearGroupId, LookId, MacroId, MaskId, MediaAssetId, MessageId, PropId, ScreenId,
    StageLayoutId, TemplateSlideId, ThemeId, TimerId, VideoInputId,
};
use crate::media::{AudioSettings, SourceAlpha};
use crate::presentation::ContentDestination;
use crate::slide::{Slide, TemplateRef};
use serde::{Deserialize, Serialize};

default_fns! {
    default_media_fade, is_default_media_fade: f64 = 0.5;
    transparent, is_transparent: Color = Color::TRANSPARENT;
}

// ---- layers --------------------------------------------------------------------------------

/// One layer of the composite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Layer {
    /// Live video from a [`VideoInput`]. Bottom-most.
    VideoInput,
    /// Background media.
    Media,
    /// The live slide; foreground media also plays here.
    Slide,
    /// Announcement loops running independently of the slide layer.
    Announcements,
    /// Props: independent overlay slides.
    Props,
    /// Messages: text overlays.
    Messages,
    /// Live captions (`docs/CAPTIONS-PLAN.md`). Top-most visual layer.
    Captions,
    /// Audio-only playback. Not drawn; exists for routing and clearing.
    Audio,
    /// Value from a newer schema. Never drawn.
    #[serde(other)]
    Unknown,
}

impl Layer {
    /// Visual layers from bottom to top. A screen's mask is drawn after these.
    pub const COMPOSITE_ORDER: [Layer; 7] = [
        Layer::VideoInput,
        Layer::Media,
        Layer::Slide,
        Layer::Announcements,
        Layer::Props,
        Layer::Messages,
        Layer::Captions,
    ];

    /// True for layers that produce pixels.
    pub fn is_visual(self) -> bool {
        !matches!(self, Layer::Audio | Layer::Unknown)
    }
}

/// Which visual layers a screen draws under a look.
///
/// A **missing field means visible** (serde default `true`) and visible fields are omitted, so
/// `{}` = every layer, and layers added in later schema versions appear on old looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct LayerVisibility {
    /// [`Layer::VideoInput`].
    #[serde(skip_serializing_if = "is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub video_input: bool,
    /// [`Layer::Media`] (ProPresenter "presentation background").
    #[serde(skip_serializing_if = "is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub media: bool,
    /// [`Layer::Slide`] (ProPresenter "presentation foreground").
    #[serde(skip_serializing_if = "is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub slide: bool,
    /// [`Layer::Announcements`].
    #[serde(skip_serializing_if = "is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub announcements: bool,
    /// [`Layer::Props`].
    #[serde(skip_serializing_if = "is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub props: bool,
    /// [`Layer::Messages`].
    #[serde(skip_serializing_if = "is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub messages: bool,
    /// [`Layer::Captions`].
    #[serde(skip_serializing_if = "is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub captions: bool,
}

impl Default for LayerVisibility {
    /// Every layer visible.
    fn default() -> Self {
        LayerVisibility::ALL
    }
}

impl LayerVisibility {
    /// Every layer visible.
    pub const ALL: LayerVisibility = LayerVisibility {
        video_input: true,
        media: true,
        slide: true,
        announcements: true,
        props: true,
        messages: true,
        captions: true,
    };
    /// Every layer hidden.
    pub const NONE: LayerVisibility = LayerVisibility {
        video_input: false,
        media: false,
        slide: false,
        announcements: false,
        props: false,
        messages: false,
        captions: false,
    };

    /// Whether `layer` is drawn (non-visual and unknown layers never are).
    pub fn contains(&self, layer: Layer) -> bool {
        match layer {
            Layer::VideoInput => self.video_input,
            Layer::Media => self.media,
            Layer::Slide => self.slide,
            Layer::Announcements => self.announcements,
            Layer::Props => self.props,
            Layer::Messages => self.messages,
            Layer::Captions => self.captions,
            Layer::Audio | Layer::Unknown => false,
        }
    }

    /// Shows or hides `layer` (no-op for non-visual layers).
    pub fn set(&mut self, layer: Layer, visible: bool) {
        match layer {
            Layer::VideoInput => self.video_input = visible,
            Layer::Media => self.media = visible,
            Layer::Slide => self.slide = visible,
            Layer::Announcements => self.announcements = visible,
            Layer::Props => self.props = visible,
            Layer::Messages => self.messages = visible,
            Layer::Captions => self.captions = visible,
            Layer::Audio | Layer::Unknown => {}
        }
    }
}

/// Default transition per layer (part of [`ShowSettings`]). `None` = the built-in default
/// ([`LayerTransitions::BUILT_IN`]). Resolution: see [`Transition::resolve`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct LayerTransitions {
    /// Video-input layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub video_input: Option<Transition>,
    /// Background media layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub media: Option<Transition>,
    /// Slide layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub slide: Option<Transition>,
    /// Announcements layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub announcements: Option<Transition>,
    /// Props layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub props: Option<Transition>,
    /// Messages layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub messages: Option<Transition>,
}

impl LayerTransitions {
    /// Built-in default for every visual layer: a 0.5 s dissolve. Non-visual layers cut.
    pub const BUILT_IN: Transition = Transition {
        kind: crate::effects::TransitionKind::Dissolve,
        duration: 0.5,
        easing: crate::effects::Easing::Linear,
    };

    /// The default transition of `layer`.
    pub fn for_layer(&self, layer: Layer) -> &Transition {
        let configured = match layer {
            Layer::VideoInput => &self.video_input,
            Layer::Media => &self.media,
            Layer::Slide => &self.slide,
            Layer::Announcements => &self.announcements,
            Layer::Props => &self.props,
            Layer::Messages => &self.messages,
            // Caption lines replace each other with a cut.
            Layer::Captions | Layer::Audio | Layer::Unknown => return &Transition::CUT,
        };
        configured.as_ref().unwrap_or(&Self::BUILT_IN)
    }
}

// ---- screens --------------------------------------------------------------------------------

/// A logical output canvas ("Audience", "Key Feed", "Stage") (stored document). Its
/// physical outputs are machine-local [`OutputBinding`](crate::machine::OutputBinding)s.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Screen ID.
    pub id: ScreenId,
    /// Name. Says nothing about transport ("Main HDMI" may be an IDN output).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the screen list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Audience or stage.
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: ScreenKind,
    /// Canvas resolution in px. Slides are scaled from their owner's size to this.
    #[serde(default, skip_serializing_if = "is_default")]
    pub size: Size,
    /// Portable render clock. Outputs may deliver at another rate (machine-local override).
    #[serde(default, skip_serializing_if = "is_default")]
    pub frame_rate: FrameRate,
    /// Alpha intent of this screen's feed (a lower-thirds key feed is `straight`). Outputs may
    /// override it, e.g. with SDI key/fill.
    #[serde(default, skip_serializing_if = "is_default")]
    pub alpha: AlphaMode,
    /// Colour behind all layers (default transparent: opaque outputs flatten over black).
    #[serde(default = "transparent", skip_serializing_if = "is_transparent")]
    pub background: Color,
    /// The screen runs (renders and feeds its outputs). A disabled screen keeps its
    /// configuration and output bindings but renders nothing and runs no outputs.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
    /// Colour correction of the whole picture, applied after compositing and before the
    /// picture goes to any output (default: none).
    #[serde(default, skip_serializing_if = "ColorCorrection::is_identity")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub color: ColorCorrection,
    /// Corner pin (keystone / perspective warp) of the whole picture, applied with the colour
    /// correction (default: none).
    #[serde(default, skip_serializing_if = "CornerPin::is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub corner_pin: CornerPin,
    /// A stage screen's mask, drawn over its layout (stage screens only: an audience screen's
    /// mask comes from the live look).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub mask: Option<MaskId>,
    /// How the screen's outputs share its canvas (default Single: every output shows the
    /// whole screen). See [`ScreenArrangement`].
    #[serde(default, skip_serializing_if = "ScreenArrangement::is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub arrangement: ScreenArrangement,
}

/// How a screen's outputs share its canvas ([`Screen::arrangement`]).
///
/// The canvas ([`Screen::size`]) is the whole wall. In a Grouped or Edge Blend arrangement it
/// is cut into `columns` × `rows` tiles, each shown by the output whose machine-local
/// `tile` names it (row-major). A tile is `(size − (n − 1) · gap) / n` wide (high), where
/// `gap` is the bezel (Grouped: canvas px hidden behind the frames between two displays) or
/// minus the overlap (Edge Blend: canvas px two neighbouring projectors both show, blended).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ScreenArrangement {
    /// Single, Mirror, Grouped or Edge Blend.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub kind: ArrangementKind,
    /// Tiles across (1–8; Grouped and Edge Blend).
    #[serde(skip_serializing_if = "is_one")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub columns: u32,
    /// Tiles down (1–8; Grouped and Edge Blend).
    #[serde(skip_serializing_if = "is_one")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub rows: u32,
    /// Grouped: canvas px hidden between two tiles (bezel compensation), across and down.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub bezel: TileGap,
    /// Edge Blend: canvas px shared by two neighbouring tiles, across and down.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub overlap: TileGap,
    /// Edge Blend: the shape of the blend ramps.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub blend: BlendCurve,
}

impl Default for ScreenArrangement {
    fn default() -> Self {
        ScreenArrangement {
            kind: ArrangementKind::Single,
            columns: 1,
            rows: 1,
            bezel: TileGap::default(),
            overlap: TileGap::default(),
            blend: BlendCurve::default(),
        }
    }
}

fn is_one(v: &u32) -> bool {
    *v == 1
}

/// The arrangements of a screen's outputs ([`ScreenArrangement`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ArrangementKind {
    /// One output shows the whole screen.
    #[default]
    Single,
    /// Every output shows the whole screen.
    Mirror,
    /// The screen spans a grid of displays; each output shows its tile, with bezel
    /// compensation.
    Grouped,
    /// The screen spans overlapping projectors; each output shows its tile and blends the
    /// edges it shares with a neighbour.
    EdgeBlend,
    /// An arrangement from a newer schema (treated as Mirror).
    #[serde(other)]
    Unknown,
}

/// A distance between neighbouring tiles, canvas px ([`ScreenArrangement`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct TileGap {
    /// Between two columns.
    pub x: f64,
    /// Between two rows.
    pub y: f64,
}

/// The blend ramp of an Edge Blend arrangement. Across an overlap of width `w`, at `t` = 0..1
/// from a tile's edge, the tile's light is `f(t) = ½(2t)^curve` below the middle and
/// `1 − ½(2(1 − t))^curve` above, so two neighbours' light adds up to one; the picture value
/// is multiplied by `f^(1/gamma)` (the projectors' gamma).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct BlendCurve {
    /// The projectors' gamma, 1–3 (default 2.2).
    pub gamma: f64,
    /// The ramp's shape, 1–4 (1 = linear, default 2).
    pub curve: f64,
}

impl Default for BlendCurve {
    fn default() -> Self {
        BlendCurve {
            gamma: 2.2,
            curve: 2.0,
        }
    }
}

/// One output's part of its screen: the crop (canvas px, integers) and the blend ramp widths
/// of its four edges (canvas px; 0 = no ramp), as [`ScreenArrangement::tile_layout`] makes
/// them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileLayout {
    /// Left, top, width, height, canvas px.
    pub crop: (u32, u32, u32, u32),
    /// Ramp widths: left, right, top, bottom, canvas px.
    pub blend: [u32; 4],
}

impl ScreenArrangement {
    /// True for the default (Single): not stored.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Grouped or Edge Blend: outputs show tiles.
    pub fn is_tiled(&self) -> bool {
        matches!(
            self.kind,
            ArrangementKind::Grouped | ArrangementKind::EdgeBlend
        )
    }

    /// Number of tiles (1 unless tiled).
    pub fn cells(&self) -> u32 {
        if self.is_tiled() {
            self.columns.max(1) * self.rows.max(1)
        } else {
            1
        }
    }

    /// The distance from one tile's start to the next one's minus the tile size, across and
    /// down: the bezel (Grouped) or minus the overlap (Edge Blend).
    pub fn gap(&self) -> (f64, f64) {
        match self.kind {
            ArrangementKind::Grouped => (self.bezel.x, self.bezel.y),
            ArrangementKind::EdgeBlend => (-self.overlap.x, -self.overlap.y),
            _ => (0.0, 0.0),
        }
    }

    /// A tile's size on a canvas of `size`, px (may be ≤ 0 for an impossible arrangement).
    pub fn tile_size(&self, size: Size) -> (f64, f64) {
        let (c, r) = (self.columns.max(1) as f64, self.rows.max(1) as f64);
        let (gx, gy) = self.gap();
        (
            (size.width - (c - 1.0) * gx) / c,
            (size.height - (r - 1.0) * gy) / r,
        )
    }

    /// The canvas size for tiles of `tile` px (the inverse of [`Self::tile_size`]).
    pub fn canvas_for_tile(&self, tile: (f64, f64)) -> Size {
        let (c, r) = (self.columns.max(1) as f64, self.rows.max(1) as f64);
        let (gx, gy) = self.gap();
        Size::new(c * tile.0 + (c - 1.0) * gx, r * tile.1 + (r - 1.0) * gy)
    }

    /// The crop and blend ramps of tile `index` (row-major) on a canvas of `size`; `None` when
    /// the arrangement isn't tiled or the tile doesn't exist.
    pub fn tile_layout(&self, size: Size, index: u32) -> Option<TileLayout> {
        if !self.is_tiled() || index >= self.cells() {
            return None;
        }
        let (cols, rows) = (self.columns.max(1), self.rows.max(1));
        let (col, row) = (index % cols, index / cols);
        let (tw, th) = self.tile_size(size);
        if tw < 1.0 || th < 1.0 {
            return None;
        }
        let (gx, gy) = self.gap();
        let span = |i: u32, t: f64, g: f64, total: f64| {
            let a = (i as f64 * (t + g)).round().clamp(0.0, total);
            let b = (i as f64 * (t + g) + t).round().clamp(a, total);
            (a as u32, (b - a) as u32)
        };
        let (x, w) = span(col, tw, gx, size.width);
        let (y, h) = span(row, th, gy, size.height);
        let mut blend = [0u32; 4];
        if self.kind == ArrangementKind::EdgeBlend {
            let ox = self.overlap.x.max(0.0).round() as u32;
            let oy = self.overlap.y.max(0.0).round() as u32;
            blend = [
                if col > 0 { ox.min(w) } else { 0 },
                if col + 1 < cols { ox.min(w) } else { 0 },
                if row > 0 { oy.min(h) } else { 0 },
                if row + 1 < rows { oy.min(h) } else { 0 },
            ];
        }
        Some(TileLayout {
            crop: (x, y, w, h),
            blend,
        })
    }
}

impl BlendCurve {
    /// The light of a tile at `t` (0 at its edge, 1 at the far side of the overlap), 0–1.
    pub fn light(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let p = self.curve.max(0.1);
        if t < 0.5 {
            0.5 * (2.0 * t).powf(p)
        } else {
            1.0 - 0.5 * (2.0 * (1.0 - t)).powf(p)
        }
    }

    /// The factor a picture value is multiplied by at `t` (gamma-aware: `light^(1/gamma)`).
    pub fn factor(&self, t: f64) -> f64 {
        self.light(t).powf(1.0 / self.gamma.max(0.1))
    }
}

/// Colour correction of a screen's picture ([`Screen::color`]). Applied by the renderer to
/// the sRGB-encoded, straight (un-premultiplied) colour of every pixel, in this order:
/// channel gains, saturation, contrast (about mid grey), brightness, black level, clamp to
/// 0–1, gamma. Alpha is never changed. The defaults change nothing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ColorCorrection {
    /// Added to every channel, −1 to 1 (default 0).
    pub brightness: f32,
    /// Contrast about mid grey, 0 to 2 (default 1; 0 = flat grey).
    pub contrast: f32,
    /// Gamma, 0.1 to 4 (default 1; above 1 brightens the mid-tones: `c^(1/gamma)`).
    pub gamma: f32,
    /// Saturation, 0 to 2 (default 1; 0 = greyscale, Rec. 709 luma).
    pub saturation: f32,
    /// Red gain, 0 to 2 (default 1).
    pub red: f32,
    /// Green gain, 0 to 2 (default 1).
    pub green: f32,
    /// Blue gain, 0 to 2 (default 1).
    pub blue: f32,
    /// Black level, −0.5 to 0.5 (default 0): lifts (positive) or crushes (negative) the
    /// blacks while white stays white (`black + c · (1 − black)`).
    pub black_level: f32,
}

impl ColorCorrection {
    /// No correction.
    pub const IDENTITY: ColorCorrection = ColorCorrection {
        brightness: 0.0,
        contrast: 1.0,
        gamma: 1.0,
        saturation: 1.0,
        red: 1.0,
        green: 1.0,
        blue: 1.0,
        black_level: 0.0,
    };

    /// True when it changes nothing.
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// Each setting with its name and allowed range (for validation and UIs).
    pub fn fields(&self) -> [(&'static str, f32, std::ops::RangeInclusive<f32>); 8] {
        [
            ("brightness", self.brightness, -1.0..=1.0),
            ("contrast", self.contrast, 0.0..=2.0),
            ("gamma", self.gamma, 0.1..=4.0),
            ("saturation", self.saturation, 0.0..=2.0),
            ("red", self.red, 0.0..=2.0),
            ("green", self.green, 0.0..=2.0),
            ("blue", self.blue, 0.0..=2.0),
            ("blackLevel", self.black_level, -0.5..=0.5),
        ]
    }
}

impl Default for ColorCorrection {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// A corner's offset in screen pixels ([`CornerPin`]). **Positive values move the corner
/// away from the centre of the screen** (outwards), negative ones towards it: `x` is
/// horizontal, `y` vertical, whichever corner it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct CornerOffset {
    /// Horizontal offset, px (positive = outwards).
    pub x: f32,
    /// Vertical offset, px (positive = outwards).
    pub y: f32,
}

impl CornerOffset {
    /// True for no offset.
    pub fn is_zero(&self) -> bool {
        self.x == 0.0 && self.y == 0.0
    }
}

/// Corner pin of a screen's picture ([`Screen::corner_pin`]): the picture's four corners are
/// moved by these offsets and the picture is warped in perspective (a homography) to the
/// quadrilateral they make, e.g. to square up a projector hung off axis. Outside the quad the
/// output is black (opaque screens) or transparent (screens with alpha). Offsets are in
/// screen pixels ([`Screen::size`]), positive outwards; they are kept while `enabled` is off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct CornerPin {
    /// The warp is applied.
    pub enabled: bool,
    /// Top-left corner.
    pub top_left: CornerOffset,
    /// Top-right corner.
    pub top_right: CornerOffset,
    /// Bottom-right corner.
    pub bottom_right: CornerOffset,
    /// Bottom-left corner.
    pub bottom_left: CornerOffset,
}

impl CornerPin {
    /// True for the default (off, no offsets): not stored.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// True when it changes the picture (enabled with an offset).
    pub fn is_active(&self) -> bool {
        self.enabled && !self.corners().iter().all(CornerOffset::is_zero)
    }

    /// The offsets clockwise from the top left.
    pub fn corners(&self) -> [CornerOffset; 4] {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }

    /// Where the picture's corners land on a `width` × `height` screen, in pixels (x right,
    /// y down), clockwise from the top left.
    pub fn quad(&self, width: f32, height: f32) -> [[f32; 2]; 4] {
        let [tl, tr, br, bl] = self.corners();
        [
            [-tl.x, -tl.y],
            [width + tr.x, -tr.y],
            [width + br.x, height + br.y],
            [-bl.x, height + bl.y],
        ]
    }
}

impl Default for Screen {
    fn default() -> Self {
        Screen {
            schema_version: crate::SCHEMA_VERSION,
            id: ScreenId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            kind: ScreenKind::default(),
            size: Size::HD,
            frame_rate: FrameRate::default(),
            alpha: AlphaMode::default(),
            background: Color::TRANSPARENT,
            enabled: true,
            import: None,
            color: ColorCorrection::IDENTITY,
            corner_pin: CornerPin::default(),
            mask: None,
            arrangement: ScreenArrangement::default(),
        }
    }
}

/// A test signal that replaces a screen's composite while it is set (screen setup and
/// troubleshooting; never stored). Drawn by the `render` crate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TestPattern {
    /// SMPTE-style 75 % colour bars with the lower castellations and PLUGE.
    ColorBars,
    /// Grid, centre cross, a circle and the action-safe (93 %) / title-safe (90 %) areas.
    Grid,
    /// Black-to-full ramps of white, red, green and blue.
    Gradient,
    /// One solid colour.
    Solid {
        /// The colour.
        color: Color,
    },
    /// A checkerboard of opaque and transparent squares plus an alpha ramp: shows that a
    /// key output's alpha channel survives downstream.
    AlphaChecker,
    /// A bar sweeping across the screen once a second and a binary frame counter: changes
    /// every frame, so dropped or doubled frames are visible.
    Motion,
    /// Unknown pattern (drawn as colour bars).
    #[serde(other)]
    Unknown,
}

/// Screen role.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ScreenKind {
    /// Shows audience layers, routed by looks.
    #[default]
    Audience,
    /// Shows a stage layout.
    Stage {
        /// The layout it shows (`None` = the default: the first stage layout, else the
        /// built-in "Current / Next"). Set by `setStageLayout`; a stage-layout slide action
        /// overrides it while the show runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        layout: Option<StageLayoutId>,
    },
    /// A signage channel's screen (`docs/SIGNAGE-PLAN.md`): it shows the channel's schedule
    /// and zones, never the show's layers. Made and removed with its channel; not an
    /// operator screen.
    Signage {
        /// The channel's id.
        channel: String,
    },
    /// Unknown tag (audience).
    #[serde(other)]
    Unknown,
}

/// Portable alpha intent of a screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum AlphaMode {
    /// Flattened over the background (black if transparent); no alpha sent.
    #[default]
    Opaque,
    /// Alpha sent, colour not premultiplied.
    Straight,
    /// Alpha sent, colour premultiplied.
    Premultiplied,
    /// Unknown value (opaque).
    #[serde(other)]
    Unknown,
}

// ---- looks -----------------------------------------------------------------------------------

/// A look: per-screen layer routing and optional template restyle (stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Look {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Look ID.
    pub id: LookId,
    /// Name ("Key feed").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the look list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Per-screen settings. Screens not listed draw every layer with no override.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screens: Vec<ScreenLook>,
    /// Transition when this look is made live (`None` = cut).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub transition: Option<Transition>,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            schema_version: crate::SCHEMA_VERSION,
            id: LookId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            screens: Vec::new(),
            transition: None,
            import: None,
        }
    }
}

impl Look {
    /// The settings for `screen` (all layers, no override, when not listed).
    pub fn for_screen(&self, screen: ScreenId) -> ScreenLook {
        self.screens
            .iter()
            .find(|s| s.screen == screen)
            .cloned()
            .unwrap_or_else(|| ScreenLook {
                screen,
                ..ScreenLook::default()
            })
    }
}

/// What one screen shows under a look.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ScreenLook {
    /// The screen.
    pub screen: ScreenId,
    /// Which layers are drawn.
    #[serde(default, skip_serializing_if = "is_default")]
    pub layers: LayerVisibility,
    /// Re-render the slide layer's text through a template slide (lower-thirds feed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub template_override: Option<TemplateOverride>,
    /// Mask composited on top of this screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub mask: Option<MaskId>,
    /// How slides with a second language are shown (see [`crate::language`]).
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub language: crate::language::LanguageDisplay,
}

/// Render-time restyle of the live slide's text through a template slide on one screen (the
/// lower-thirds network output). Implemented by [`TemplateOverride::render`].
///
/// ## Rule
/// 1. **Source:** the live slide's text elements that fill a text placeholder slot, in element
///    order; if it has none, all its text elements. Data-link elements are never sources.
/// 2. **Empty:** if no source has visible text and `hideWhenEmpty` is set, nothing is drawn on
///    this screen's slide layer (image-only and blank slides make the lower third disappear).
/// 3. **Targets:** the template slide's text placeholders (else its text elements), in order.
/// 4. **Mapping:** `byNameThenOrder` pairs a source and a target with the same slot (or element)
///    name first, then the remaining ones in order; `byOrder` pairs strictly in order;
///    `mergeIntoFirst` sends everything to the first target. In every mode, text with no
///    target is **appended to the last target** as extra paragraphs (never dropped). Targets
///    that receive nothing are drawn empty.
/// 5. **Style:** the target's `defaultStyle` / `defaultParagraph` replace font, size, colour and
///    alignment. With `keepInlineFormatting`, runs keep baseline (superscript verse numbers),
///    italic, weight and underline; otherwise all run overrides are dropped.
/// 6. **Everything else** in the template (decoration, background, non-text elements) is drawn
///    as authored; the live slide's own elements, background and actions are not drawn on
///    this screen.
///
/// Foreground media playing on the slide layer is **not** restyled: it is drawn as is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TemplateOverride {
    /// Theme holding the template slide.
    pub theme: Link<ThemeId>,
    /// Template slide.
    pub slide: TemplateSlideId,
    /// How source text maps onto template slots.
    #[serde(default, skip_serializing_if = "is_default")]
    pub mapping: TextMapping,
    /// Keep inline baseline / italic / weight / underline from the source runs.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub keep_inline_formatting: bool,
    /// Draw nothing when the live slide has no visible text.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub hide_when_empty: bool,
}

/// Strategy for pouring source text into template text slots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TextMapping {
    /// Same-named slots first, then the rest in order.
    #[default]
    ByNameThenOrder,
    /// Strictly by order.
    ByOrder,
    /// Everything into the first slot.
    MergeIntoFirst,
    /// Unknown value (by name, then order).
    #[serde(other)]
    Unknown,
}

// ---- masks, inputs ------------------------------------------------------------------------------

/// A mask: a slide composited over a screen, after all its layers (logo bug, corner cut-out,
/// a 4:3 window) (stored document). Its shapes and pictures are ordinary slide elements.
///
/// - [`MaskMode::Overlay`]: the slide is drawn over the screen as authored.
/// - [`MaskMode::Keyhole`] (the "invert"): the screen shows only through the slide's shapes
///   (their alpha); everywhere else is `color`.
///
/// `feather` softens the mask's edges as a whole (a blur of its alpha, in canvas pixels), on
/// top of any feather of its elements.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Mask {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Mask ID.
    pub id: MaskId,
    /// Name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the mask list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Canvas size of `slide`, px.
    #[serde(default, skip_serializing_if = "is_default")]
    pub size: Size,
    /// Content.
    pub slide: Slide,
    /// How the slide masks the screen.
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub mode: MaskMode,
    /// The colour outside the shapes of a keyhole mask (opaque black by default).
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub color: Color,
    /// Edge softness of the whole mask, canvas pixels (0 = hard).
    #[serde(default, skip_serializing_if = "crate::common::is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub feather: f64,
}

/// How a [`Mask`] masks its screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MaskMode {
    /// The slide is drawn over the screen.
    #[default]
    Overlay,
    /// The screen shows only through the slide's shapes; outside them is the mask's colour.
    Keyhole,
    /// Unknown value (drawn as an overlay).
    #[serde(other)]
    Unknown,
}

impl Default for Mask {
    fn default() -> Self {
        Mask {
            schema_version: crate::SCHEMA_VERSION,
            id: MaskId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            size: Size::HD,
            slide: Slide::new(),
            mode: MaskMode::Overlay,
            color: Color::BLACK,
            feather: 0.0,
        }
    }
}

/// A portable logical video input ("Camera 1") (stored document). The source device is
/// bound per machine ([`InputBinding`](crate::machine::InputBinding)).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VideoInput {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Input ID.
    pub id: VideoInputId,
    /// Name shown to operators.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the input list (ProPresenter references inputs by this index).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// UI colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<Color>,
    /// How the source's alpha is interpreted.
    #[serde(default, skip_serializing_if = "is_default")]
    pub alpha: SourceAlpha,
    /// Level and channel routing of the input's embedded audio (empty routing + 0 dB = as is).
    #[serde(default, skip_serializing_if = "is_default")]
    pub audio: AudioSettings,
    /// Placeholder thumbnail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub thumbnail: Option<MediaAssetId>,
    /// Import provenance (ProPresenter list index, device name hint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for VideoInput {
    fn default() -> Self {
        VideoInput {
            schema_version: crate::SCHEMA_VERSION,
            id: VideoInputId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            color: None,
            alpha: SourceAlpha::default(),
            audio: AudioSettings::default(),
            thumbnail: None,
            import: None,
        }
    }
}

/// A portable logical audio input (stored document). The source is bound per machine
/// ([`AudioInputBinding`](crate::machine::AudioInputBinding)).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AudioInput {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Input ID.
    pub id: AudioInputId,
    /// Name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the input list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// When the input is live.
    #[serde(default, skip_serializing_if = "is_default")]
    pub mode: AudioInputMode,
    /// Video inputs that switch this input in the `auto*` modes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_video_inputs: Vec<VideoInputId>,
    /// Level and channel routing.
    #[serde(default, skip_serializing_if = "is_default")]
    pub audio: AudioSettings,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for AudioInput {
    fn default() -> Self {
        AudioInput {
            schema_version: crate::SCHEMA_VERSION,
            id: AudioInputId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            mode: AudioInputMode::default(),
            linked_video_inputs: Vec::new(),
            audio: AudioSettings::default(),
            import: None,
        }
    }
}

/// When an audio input is live.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum AudioInputMode {
    /// Off.
    #[default]
    Off,
    /// Always on.
    On,
    /// On while a linked video input is live.
    AutoOn,
    /// Off while a linked video input is live.
    AutoOff,
    /// Unknown value (off).
    #[serde(other)]
    Unknown,
}

// ---- timers --------------------------------------------------------------------------------------

/// A timer (stored document). Display format lives on each element that shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Timer {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Timer ID.
    pub id: TimerId,
    /// Name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the timer list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Behaviour.
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: TimerKind,
    /// Keep counting past zero (negative time).
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_overrun: bool,
    /// A countdown turns amber with this many seconds (or fewer) left (`None` = never): the
    /// stage display's timer links without colour triggers of their own and the operators'
    /// apps show it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub warning_seconds: Option<f64>,
}

impl Default for Timer {
    fn default() -> Self {
        Timer {
            schema_version: crate::SCHEMA_VERSION,
            id: TimerId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            kind: TimerKind::default(),
            allow_overrun: false,
            warning_seconds: None,
        }
    }
}

/// Timer behaviours (all times in seconds).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TimerKind {
    /// Count down a fixed duration.
    Countdown {
        /// Duration, seconds.
        #[serde(default, skip_serializing_if = "crate::common::is_zero")]
        seconds: f64,
    },
    /// Count down to a time of day (server local time).
    CountdownToTime {
        /// Target, seconds after local midnight (`0..86400`, 24-hour).
        #[serde(default, skip_serializing_if = "crate::common::is_zero")]
        seconds_after_midnight: f64,
        /// How the time was entered / is displayed.
        #[serde(default, skip_serializing_if = "is_default")]
        period: TimePeriod,
    },
    /// Count up from `start` (default 0), stopping at `end` if set.
    Elapsed {
        /// Start value, seconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        start: Option<f64>,
        /// Stop value, seconds (`None` = run until stopped).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        end: Option<f64>,
    },
    /// Unknown tag (shows nothing).
    #[serde(other)]
    Unknown,
}

impl Default for TimerKind {
    /// A zero-length countdown.
    fn default() -> Self {
        TimerKind::Countdown { seconds: 0.0 }
    }
}

/// Clock period of a countdown-to-time target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TimePeriod {
    /// 24-hour clock.
    #[default]
    H24,
    /// AM.
    Am,
    /// PM.
    Pm,
    /// Unknown value (24-hour).
    #[serde(other)]
    Unknown,
}

// ---- stage layouts ------------------------------------------------------------------------------

/// A stage display layout (stored document): a slide whose elements use data links
/// (current / next slide text, clock, timers, video countdown, stage message).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct StageLayout {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Layout ID.
    pub id: StageLayoutId,
    /// Name ("Current and Next").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the layout list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Canvas size of `slide`, px.
    #[serde(default, skip_serializing_if = "is_default")]
    pub size: Size,
    /// Layout content.
    pub slide: Slide,
    /// How this layout's chord charts and key line show the song's chords (transposition, capo,
    /// spelling, Nashville numbers): a guitarist's layout with a capo, a keys player's in
    /// concert pitch. See [`crate::chords`].
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub chords: crate::chords::ChordView,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for StageLayout {
    fn default() -> Self {
        StageLayout {
            schema_version: crate::SCHEMA_VERSION,
            id: StageLayoutId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            size: Size::HD,
            slide: Slide::new(),
            chords: Default::default(),
            import: None,
        }
    }
}

// ---- clear groups -----------------------------------------------------------------------------

/// A named set of clear targets ("Clear All") (stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ClearGroup {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Clear group ID.
    pub id: ClearGroupId,
    /// Name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the clear group list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// What gets cleared, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<ClearTarget>,
    /// Icon name (UI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub icon: Option<String>,
    /// Icon tint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub tint: Option<Color>,
}

impl Default for ClearGroup {
    fn default() -> Self {
        ClearGroup {
            schema_version: crate::SCHEMA_VERSION,
            id: ClearGroupId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            targets: Vec::new(),
            icon: None,
            tint: None,
        }
    }
}

impl ClearGroup {
    /// The default "Clear All" group every new show gets: clears every layer (including
    /// announcements and audio) and stops timelines.
    pub fn clear_all() -> Self {
        ClearGroup {
            name: "Clear All".into(),
            targets: vec![
                ClearTarget {
                    layer: ClearLayer::All,
                    destination: None,
                },
                ClearTarget {
                    layer: ClearLayer::All,
                    destination: Some(ContentDestination::Announcements),
                },
                ClearTarget {
                    layer: ClearLayer::Timelines,
                    destination: None,
                },
            ],
            ..ClearGroup::default()
        }
    }
}

/// One clear target: a layer plus an optional content destination, kept as the pair
/// ProPresenter stores (`{targetLayer, contentDestination}`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ClearTarget {
    /// What to clear.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub layer: ClearLayer,
    /// Restrict to content shown on this destination (`None` = any).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub destination: Option<ContentDestination>,
}

/// Something a clear can target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ClearLayer {
    /// Everything.
    #[default]
    All,
    /// Audio.
    Audio,
    /// Audio effects / sound cues.
    AudioEffects,
    /// Live video input layer.
    VideoInput,
    /// Background media layer.
    Media,
    /// Slide layer.
    Slide,
    /// Announcements layer.
    Announcements,
    /// Props layer.
    Props,
    /// Messages layer.
    Messages,
    /// Mask / logo overlay.
    Mask,
    /// Running timelines and auto-advance.
    Timelines,
    /// Unknown value (ignored).
    #[serde(other)]
    Unknown,
}

// ---- props / messages / macros --------------------------------------------------------------

/// A prop: a slide on the props layer, toggled independently of slides (stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Prop {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Prop ID.
    pub id: PropId,
    /// Name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the prop list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Canvas size of `slide`, px.
    #[serde(default, skip_serializing_if = "is_default")]
    pub size: Size,
    /// Content.
    pub slide: Slide,
    /// Transition in / out (`None` = the props layer default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub transition: Option<Transition>,
}

impl Default for Prop {
    fn default() -> Self {
        Prop {
            schema_version: crate::SCHEMA_VERSION,
            id: PropId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            size: Size::HD,
            slide: Slide::new(),
            transition: None,
        }
    }
}

/// A message: overlay text with `${token}` placeholders rendered through a template slide
/// (stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Message ID.
    pub id: MessageId,
    /// Name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the message list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Text with `${token}` placeholders ("Parent of ${child}, please come to the nursery").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Template slide used to render it; the text goes into its first text slot (`None` =
    /// built-in banner).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub template: Option<TemplateRef>,
    /// Token definitions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tokens: Vec<MessageToken>,
    /// Transition in / out (`None` = the messages layer default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub transition: Option<Transition>,
}

impl Default for Message {
    fn default() -> Self {
        Message {
            schema_version: crate::SCHEMA_VERSION,
            id: MessageId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            text: String::new(),
            template: None,
            tokens: Vec::new(),
            transition: None,
        }
    }
}

/// A message token definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MessageToken {
    /// Name as used in `${name}`.
    pub name: String,
    /// What the token renders.
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: MessageTokenKind,
}

/// What a message token renders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MessageTokenKind {
    /// Operator-entered text.
    Text {
        /// Value used when the show action gives none.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        default_value: String,
    },
    /// A timer's value.
    Timer {
        /// The timer (`None` = most recently started, as for data links).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        timer: Option<Link<TimerId>>,
        /// Format.
        #[serde(default, skip_serializing_if = "is_default")]
        format: TimerFormat,
    },
    /// The wall clock.
    Clock {
        /// Format.
        #[serde(default, skip_serializing_if = "is_default")]
        format: ClockFormat,
    },
    /// Imported token kind this build does not model.
    Unsupported {
        /// Source kind.
        source_type: String,
        /// Verbatim source JSON.
        #[serde(default, skip_serializing_if = "crate::common::is_null")]
        #[cfg_attr(feature = "ts", ts(type = "unknown", optional = nullable))]
        payload: serde_json::Value,
    },
    /// Unknown tag (renders empty).
    #[serde(other)]
    Unknown,
}

impl Default for MessageTokenKind {
    /// Empty text.
    fn default() -> Self {
        MessageTokenKind::Text {
            default_value: String::new(),
        }
    }
}

/// A macro: a named, ordered bundle of actions (stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Macro {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Macro ID.
    pub id: MacroId,
    /// Name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Order in the macro list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// UI colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<Color>,
    /// Actions, run in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<SlideAction>,
    /// Run when the server starts.
    #[serde(default, skip_serializing_if = "is_false")]
    pub run_on_startup: bool,
}

impl Default for Macro {
    fn default() -> Self {
        Macro {
            schema_version: crate::SCHEMA_VERSION,
            id: MacroId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            color: None,
            actions: Vec::new(),
            run_on_startup: false,
        }
    }
}

// ---- show settings ----------------------------------------------------------------------------

/// Show-wide settings (singleton stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ShowSettings {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Default transition per layer.
    #[serde(default, skip_serializing_if = "is_default")]
    pub default_transitions: LayerTransitions,
    /// Canvas size of new presentations, themes and layouts, px.
    #[serde(default, skip_serializing_if = "is_default")]
    pub default_canvas_size: Size,
    /// CCLI licence and copyright display.
    #[serde(default, skip_serializing_if = "is_default")]
    pub ccli: CcliSettings,
    /// Fade length in seconds for media fades that are enabled without a duration (ProPresenter
    /// stores the flag alone on almost every video). Default 0.5 s.
    #[serde(
        default = "default_media_fade",
        skip_serializing_if = "is_default_media_fade"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub default_media_fade: f64,
    /// Look made live at server start (`None` = every layer on every screen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub startup_look: Option<LookId>,
}

impl Default for ShowSettings {
    fn default() -> Self {
        ShowSettings {
            schema_version: crate::SCHEMA_VERSION,
            default_transitions: LayerTransitions::default(),
            default_canvas_size: Size::HD,
            ccli: CcliSettings::default(),
            default_media_fade: default_media_fade(),
            startup_look: None,
        }
    }
}

/// CCLI licence and copyright display settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct CcliSettings {
    /// Church licence number, verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub license_number: Option<String>,
    /// Display the copyright line at all.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// On which slides of a song.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub show_on: CcliDisplay,
    /// Order of the copyright line's parts (empty = the built-in layout).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub layout: Vec<CcliToken>,
    /// The slide the line is drawn on, **inline** (`None` = built-in). Its size is
    /// [`ShowSettings::default_canvas_size`]; the line is rendered by elements with a
    /// [`DataLink::CcliText`](crate::element::DataLink::CcliText) link. When it was copied from
    /// a theme, the slide's own `templateOrigin` records which template slide.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub template: Option<Slide>,
}

/// One part of the CCLI copyright line, filled from the song's
/// [`SongInfo`](crate::presentation::SongInfo) and the licence settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CcliToken {
    /// Literal text (separators, "CCLI Licence #").
    Text {
        /// The text.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        text: String,
    },
    /// Song artist.
    Artist,
    /// Song author(s).
    Author,
    /// Publisher.
    Publisher,
    /// Song title.
    Title,
    /// Copyright year.
    CopyrightYear,
    /// The church's licence number.
    LicenseNumber,
    /// CCLI song number.
    SongNumber,
    /// Token from a newer schema (rendered as nothing).
    #[serde(other)]
    Unknown,
}

/// Which slides of a song show the CCLI line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum CcliDisplay {
    /// First slide.
    #[default]
    FirstSlide,
    /// Last slide.
    LastSlide,
    /// First and last slide.
    FirstAndLast,
    /// Every slide.
    AllSlides,
    /// Unknown value (first slide).
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod arrangement_tests {
    use super::*;

    fn grid(kind: ArrangementKind, columns: u32, rows: u32) -> ScreenArrangement {
        ScreenArrangement {
            kind,
            columns,
            rows,
            ..ScreenArrangement::default()
        }
    }

    #[test]
    fn grouped_tiles_skip_the_bezels() {
        let mut a = grid(ArrangementKind::Grouped, 2, 2);
        a.bezel = TileGap { x: 64.0, y: 40.0 };
        let size = a.canvas_for_tile((1920.0, 1080.0));
        assert_eq!(size, Size::new(3904.0, 2200.0));
        assert_eq!(a.tile_size(size), (1920.0, 1080.0));
        let t = |i| a.tile_layout(size, i).unwrap();
        assert_eq!(t(0).crop, (0, 0, 1920, 1080));
        assert_eq!(t(1).crop, (1984, 0, 1920, 1080));
        assert_eq!(t(3).crop, (1984, 1120, 1920, 1080));
        assert_eq!(t(3).blend, [0; 4], "no ramps when grouped");
        assert!(a.tile_layout(size, 4).is_none());
        assert!(
            grid(ArrangementKind::Mirror, 1, 1)
                .tile_layout(size, 0)
                .is_none()
        );
    }

    #[test]
    fn edge_blend_tiles_overlap_and_ramp_their_inner_edges() {
        let mut a = grid(ArrangementKind::EdgeBlend, 3, 1);
        a.overlap = TileGap { x: 200.0, y: 0.0 };
        let size = a.canvas_for_tile((1920.0, 1080.0));
        assert_eq!(size.width, 3.0 * 1920.0 - 400.0);
        let t = |i| a.tile_layout(size, i).unwrap();
        assert_eq!(t(0).crop, (0, 0, 1920, 1080));
        assert_eq!(t(1).crop, (1720, 0, 1920, 1080));
        assert_eq!(t(0).blend, [0, 200, 0, 0]);
        assert_eq!(t(1).blend, [200, 200, 0, 0]);
        assert_eq!(t(2).blend, [200, 0, 0, 0]);
    }

    #[test]
    fn the_blend_ramp_adds_up_to_full_light() {
        for curve in [1.0, 2.0, 3.0] {
            let b = BlendCurve { gamma: 2.2, curve };
            for i in 0..=20 {
                let t = i as f64 / 20.0;
                assert!((b.light(t) + b.light(1.0 - t) - 1.0).abs() < 1e-12);
                // The value factor is the light through the projector's gamma.
                assert!((b.factor(t).powf(2.2) - b.light(t)).abs() < 1e-9);
            }
            assert_eq!(b.light(0.0), 0.0);
            assert_eq!(b.light(1.0), 1.0);
        }
    }
}
