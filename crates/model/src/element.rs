//! Slide elements: geometry, kind (shape or text), decorations (fill, stroke, shadow, feather),
//! template placeholders, live data links and visibility conditions.
//!
//! ## Decorations: one representation of "off"
//! [`Fill`], [`Stroke`], [`Shadow`], [`Feather`] (and the text box's
//! [`LineMask`](crate::text::LineMask) / [`TextOutline`](crate::text::TextOutline)) are always
//! present with an `enabled` flag (default `false`). A disabled decoration keeps its tuned
//! settings, so toggling it back on in the editor restores them, as in ProPresenter. Each struct
//! is omitted from JSON when it equals its default, and the defaults are ProPresenter's own
//! (census): a disabled blue colour fill, a disabled 3 px white stroke, a disabled
//! 315°/5/5/0.75 black shadow, a disabled 0.05 inside feather. Never `Option<T>` + `enabled`.
//!
//! ## Units
//! `bounds` and every length are canvas px (origin top-left, y down); `rotation` is degrees
//! clockwise about the centre of `bounds`; shapes and paths use unit coordinates of `bounds`.

use crate::common::{
    Color, ImportInfo, Link, Point, Rect, default_fns, is_default, is_false, is_one, is_zero, one,
};
use crate::ids::{ElementId, MediaAssetId, MessageId, TimerId, VideoInputId};
use crate::media::{MediaDisplay, Playback};
use crate::text::TextBox;
use serde::{Deserialize, Serialize};

default_fns! {
    default_stroke_width, is_default_stroke_width: f64 = 3.0;
    default_stroke_color, is_default_stroke_color: Color = Color::WHITE;
    default_shadow_angle, is_default_shadow_angle: f64 = 315.0;
    default_shadow_offset, is_default_shadow_offset: f64 = 5.0;
    default_shadow_radius, is_default_shadow_radius: f64 = 5.0;
    default_shadow_opacity, is_default_shadow_opacity: f64 = 0.75;
    default_feather_radius, is_default_feather_radius: f64 = 0.05;
    default_midpoint, is_default_midpoint: f64 = 0.5;
    default_scroll_speed, is_default_scroll_speed: f64 = 120.0;
    default_scroll_gap, is_default_scroll_gap: f64 = 120.0;
}

/// ProPresenter's default (disabled) fill colour, rgb(30, 144, 255).
pub const DEFAULT_FILL_COLOR: Color = Color::rgb(30.0 / 255.0, 144.0 / 255.0, 1.0);

/// One object on a slide. Z-order is the order in `Slide::elements` (first = bottom).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Element {
    /// Stable ID (selection, data links, visibility conditions). Imported UUIDs are reused.
    pub id: ElementId,
    /// Name shown in the layer list ("Verse", "Reference", "Background").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Position and size in canvas px, before rotation.
    #[serde(default, skip_serializing_if = "is_default")]
    pub bounds: Rect,
    /// Rotation about the centre of `bounds`, degrees clockwise.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: f64,
    /// Whole-element opacity `0..=1` (multiplies fill, stroke, text and shadow).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f64,
    /// Mirroring of the rendered element.
    #[serde(default, skip_serializing_if = "is_default")]
    pub flip: crate::common::Flip,
    /// Not rendered (still editable; data-link visibility may also hide elements).
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Editor: cannot be selected / moved on the canvas.
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// Editor: resizing keeps the aspect ratio.
    #[serde(default, skip_serializing_if = "is_false")]
    pub aspect_locked: bool,
    /// What the element is: a shape, or a shape holding text.
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: ElementKind,
    /// Interior fill (colour, gradient, image/video, backdrop effect).
    #[serde(default, skip_serializing_if = "is_default")]
    pub fill: Fill,
    /// Outline stroke along the shape.
    #[serde(default, skip_serializing_if = "is_default")]
    pub stroke: Stroke,
    /// Drop shadow of the whole element.
    #[serde(default, skip_serializing_if = "is_default")]
    pub shadow: Shadow,
    /// Soft edge.
    #[serde(default, skip_serializing_if = "is_default")]
    pub feather: Feather,
    /// Template slot this element fills (see [`Placeholder`]). Imported from ProPresenter
    /// `info` bit 2 ("text element"); the importer derives the slot name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub placeholder: Option<Placeholder>,
    /// The element belongs to a theme / template layout (ProPresenter `info` bit 1). Applying
    /// another theme replaces themed decoration; non-themed elements are user content.
    #[serde(default, skip_serializing_if = "is_false")]
    pub themed: bool,
    /// The template element this element follows on a slide linked to its theme (see
    /// [`crate::theme_link`]); `None` = the slide's own element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub from_theme: Option<ThemeElementLink>,
    /// The element is a scrolling ticker (ProPresenter `info` bit 4; provenance: what scrolls
    /// is [`Element::scroll`], which the importer fills from the scroller's settings).
    #[serde(default, skip_serializing_if = "is_false")]
    pub ticker: bool,
    /// Scrolling text: the text moves through its box on the screen clock (a text element's;
    /// the fill, stroke and shadow stay put).
    #[serde(default, skip_serializing_if = "is_default")]
    pub scroll: TextScroll,
    /// Live content rendered into the element's text (stage displays, countdowns).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub data_link: Option<DataLink>,
    /// Conditions that must **all** hold for the element to be drawn. Empty = always.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visibility: Vec<VisibilityCondition>,
    /// Import provenance (raw `info` bits, `originalFont`, build animations, scroller, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for Element {
    fn default() -> Self {
        Element::new(Rect::default())
    }
}

impl Element {
    /// A plain rectangle with nothing in it.
    pub fn new(bounds: Rect) -> Self {
        Element {
            id: ElementId::new(),
            name: String::new(),
            bounds,
            rotation: 0.0,
            opacity: 1.0,
            flip: crate::common::Flip::default(),
            hidden: false,
            locked: false,
            aspect_locked: false,
            kind: ElementKind::default(),
            fill: Fill::default(),
            stroke: Stroke::default(),
            shadow: Shadow::default(),
            feather: Feather::default(),
            placeholder: None,
            themed: false,
            from_theme: None,
            ticker: false,
            scroll: TextScroll::default(),
            data_link: None,
            visibility: Vec::new(),
            import: None,
        }
    }

    /// A rectangular text element.
    pub fn text(bounds: Rect, text: TextBox) -> Self {
        Element {
            kind: ElementKind::Text {
                shape: Shape::Rectangle,
                text,
            },
            ..Element::new(bounds)
        }
    }

    /// A rectangle filled with an image / video (PowerPoint slide images, picture slides).
    pub fn media(bounds: Rect, asset: MediaAssetId) -> Self {
        Element {
            fill: Fill::enabled(FillKind::Media {
                asset,
                display: MediaDisplay::default(),
                playback: Playback::default(),
                audio: Default::default(),
            }),
            ..Element::new(bounds)
        }
    }

    /// The text box, if this is a text element.
    pub fn text_box(&self) -> Option<&TextBox> {
        match &self.kind {
            ElementKind::Text { text, .. } => Some(text),
            _ => None,
        }
    }

    /// The text box, mutably.
    pub fn text_box_mut(&mut self) -> Option<&mut TextBox> {
        match &mut self.kind {
            ElementKind::Text { text, .. } => Some(text),
            _ => None,
        }
    }

    /// True when the element shows an image / video fill (enabled or not).
    pub fn has_media_fill(&self) -> bool {
        matches!(self.fill.kind, FillKind::Media { .. })
    }

    /// Copy with a fresh ID (duplicate / paste / template copy).
    pub fn duplicate(&self) -> Self {
        Element {
            id: ElementId::new(),
            ..self.clone()
        }
    }
}

/// How an element follows a template element of its slide's theme.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ThemeElementLink {
    /// The template element.
    pub element: ElementId,
    /// Properties this element sets itself ([`crate::theme_link::PROPERTIES`]). Kept by the
    /// server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub overrides: Vec<String>,
}

/// What an element is. Media is not a kind: an image element is a shape whose [`Fill`] is
/// media, so "image with caption" is a text element with a media fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ElementKind {
    /// A shape (drawn by its fill, stroke, shadow and feather).
    Shape {
        /// Outline geometry.
        #[serde(default, skip_serializing_if = "is_default")]
        shape: Shape,
    },
    /// A shape holding a text box (it may be empty but styled).
    Text {
        /// Outline geometry of the box (clips the fill).
        #[serde(default, skip_serializing_if = "is_default")]
        shape: Shape,
        /// The text.
        #[serde(default, skip_serializing_if = "is_default")]
        text: TextBox,
    },
    /// Unknown tag from a newer schema (rendered as nothing).
    #[serde(other)]
    Unknown,
}

impl Default for ElementKind {
    /// A rectangle shape.
    fn default() -> Self {
        ElementKind::Shape {
            shape: Shape::Rectangle,
        }
    }
}

/// Element outline shape. Parametric shapes are generated to fit `bounds`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Shape {
    /// Rectangle.
    #[default]
    Rectangle,
    /// Rounded rectangle.
    RoundedRectangle {
        /// Corner radius as a fraction `0..=0.5` of the shorter side.
        roundness: f64,
    },
    /// Ellipse inscribed in the bounds.
    Ellipse,
    /// Isosceles triangle, apex at the top centre.
    IsoscelesTriangle,
    /// Right triangle, right angle at the bottom-left.
    RightTriangle,
    /// Rhombus (diamond).
    Rhombus,
    /// Star.
    Star {
        /// Number of points.
        points: u32,
        /// Inner radius as a fraction `0..=1` of the outer radius.
        inner_radius: f64,
    },
    /// Regular polygon.
    Polygon {
        /// Number of sides (≥ 3).
        sides: u32,
    },
    /// Right-pointing arrow.
    RightArrow {
        /// Arrow-head corner in unit coordinates of the bounds.
        corner: Point,
    },
    /// Double-headed arrow.
    DoubleArrow {
        /// Arrow-head corner in unit coordinates of the bounds.
        corner: Point,
    },
    /// Free-form Bézier path.
    Path {
        /// The path.
        path: ShapePath,
    },
    /// Unknown shape (rendered as its bounds rectangle).
    #[serde(other)]
    Unknown,
}

/// A Bézier path in *unit* coordinates (0..=1 of the element bounds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ShapePath {
    /// The last point connects back to the first.
    #[serde(skip_serializing_if = "crate::common::is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub closed: bool,
    /// Points in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub points: Vec<PathPoint>,
}

impl Default for ShapePath {
    /// An empty closed path.
    fn default() -> Self {
        ShapePath {
            closed: true,
            points: Vec::new(),
        }
    }
}

/// One path point with its control handles (unit coordinates, absolute).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct PathPoint {
    /// Anchor point.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub point: Point,
    /// Incoming control handle.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub handle_in: Point,
    /// Outgoing control handle.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub handle_out: Point,
    /// The segment through this point is curved (handles used); otherwise a corner.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub curved: bool,
}

// ---- fill ------------------------------------------------------------------------------------

/// Interior fill. ProPresenter writes a disabled colour fill on most elements; it is kept
/// (and omitted from JSON while it equals the default).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Fill {
    /// The fill is drawn.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// What fills the shape.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub kind: FillKind,
}

impl Default for Fill {
    /// Disabled, ProPresenter's default blue colour.
    fn default() -> Self {
        Fill {
            enabled: false,
            kind: FillKind::default(),
        }
    }
}

impl Fill {
    /// An enabled fill.
    pub fn enabled(kind: FillKind) -> Self {
        Fill {
            enabled: true,
            kind,
        }
    }
    /// An enabled solid colour fill.
    pub fn color(color: Color) -> Self {
        Fill::enabled(FillKind::Color { color })
    }
}

/// Fill content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FillKind {
    /// Solid colour.
    Color {
        /// The colour.
        color: Color,
    },
    /// Gradient.
    Gradient {
        /// The gradient.
        gradient: Gradient,
    },
    /// Image / video drawn into the bounds and clipped to the shape (PowerPoint slide images,
    /// picture placeholders). The asset may be missing; the renderer then draws nothing.
    Media {
        /// The media file.
        asset: MediaAssetId,
        /// Scaling, crop, alpha and effects.
        #[serde(default, skip_serializing_if = "is_default")]
        display: MediaDisplay,
        /// Video playback settings (ignored for images).
        #[serde(default, skip_serializing_if = "is_default")]
        playback: Playback,
        /// The video's sound: level, mute and channel routing (ignored for images; omitted =
        /// full level, unmuted). Plays on the slide layer's group, so the slide layer's
        /// level and mute apply too.
        #[serde(default, skip_serializing_if = "is_default")]
        audio: crate::media::AudioSettings,
    },
    /// A live network video source (IDN Open Protocol, compatible with NDI®) drawn into the
    /// bounds and clipped to the shape, like a video fill: a camera box on a slide. Draws
    /// nothing until the source sends pictures.
    LiveSource {
        /// The source: its full name `HOST (name)` or `ip:port`. Used when `input` is not set
        /// (or not bound on this machine).
        #[serde(default, skip_serializing_if = "String::is_empty")]
        source: String,
        /// A logical video input, resolved through this machine's
        /// [`InputBinding`](crate::machine::InputBinding); wins over `source`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        input: Option<VideoInputId>,
        /// Scaling, crop, alpha and effects.
        #[serde(default, skip_serializing_if = "is_default")]
        display: MediaDisplay,
    },
    /// An effect applied to whatever is *behind* the element (frosted glass, invert).
    BackgroundEffect {
        /// The effect.
        effect: BackgroundEffect,
    },
    /// Glyphs punch through the element fill ("cut-out" text; text boxes only).
    CutOut,
    /// Imported fill this build cannot represent; rendered as nothing, kept verbatim.
    Unsupported {
        /// Source type name (e.g. `"propresenter.fill.<kind>"`).
        source_type: String,
        /// Verbatim source JSON.
        #[serde(default, skip_serializing_if = "crate::common::is_null")]
        #[cfg_attr(feature = "ts", ts(type = "unknown", optional = nullable))]
        payload: serde_json::Value,
    },
    /// Unknown tag from a newer schema (rendered as nothing).
    #[serde(other)]
    Unknown,
}

impl Default for FillKind {
    /// [`DEFAULT_FILL_COLOR`].
    fn default() -> Self {
        FillKind::Color {
            color: DEFAULT_FILL_COLOR,
        }
    }
}

/// A colour gradient.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Gradient {
    /// Gradient geometry.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub kind: GradientKind,
    /// Direction in degrees, clockwise from "left to right" (linear / angle gradients).
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub angle: f64,
    /// Length as a fraction of the element diagonal (1 = full).
    #[serde(skip_serializing_if = "is_one")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub length: f64,
    /// Colour stops ordered by position.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub stops: Vec<GradientStop>,
}

impl Default for Gradient {
    /// Linear, 0°, full length, no stops.
    fn default() -> Self {
        Gradient {
            kind: GradientKind::Linear,
            angle: 0.0,
            length: 1.0,
            stops: Vec::new(),
        }
    }
}

/// Gradient geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum GradientKind {
    /// Linear.
    #[default]
    Linear,
    /// Radial from the centre.
    Radial,
    /// Conic / angular sweep.
    Angle,
    /// Unknown value (linear).
    #[serde(other)]
    Unknown,
}

/// One gradient stop.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GradientStop {
    /// Stop colour.
    pub color: Color,
    /// Position `0..=1` along the gradient.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub position: f64,
    /// Blend mid-point to the next stop, `0..=1` (0.5 = linear).
    #[serde(
        default = "default_midpoint",
        skip_serializing_if = "is_default_midpoint"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub midpoint: f64,
}

/// An effect applied to what is behind an element.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BackgroundEffect {
    /// Blur (frosted glass).
    Blur {
        /// Blur amount `0..=1` (fraction of the maximum radius).
        amount: f64,
        /// Saturation multiplier of the blurred content (1 = unchanged).
        #[serde(default = "one", skip_serializing_if = "is_one")]
        saturation: f64,
    },
    /// Colour inversion.
    Invert,
    /// Unknown tag (no effect).
    #[serde(other)]
    Unknown,
}

// ---- stroke / shadow / feather -----------------------------------------------------------------

/// Border along the element outline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Stroke {
    /// The stroke is drawn.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// Width in px, centred on the outline.
    #[serde(skip_serializing_if = "is_default_stroke_width")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub width: f64,
    /// Colour.
    #[serde(skip_serializing_if = "is_default_stroke_color")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub color: Color,
    /// Dash style.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub style: StrokeStyle,
    /// Custom dash pattern (alternating on / off lengths in px) for `style: custom`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub pattern: Vec<f64>,
}

impl Default for Stroke {
    /// Disabled, 3 px white solid (ProPresenter's default).
    fn default() -> Self {
        Stroke {
            enabled: false,
            width: default_stroke_width(),
            color: default_stroke_color(),
            style: StrokeStyle::Solid,
            pattern: Vec::new(),
        }
    }
}

/// Stroke dash style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum StrokeStyle {
    /// Solid line.
    #[default]
    Solid,
    /// Square dots.
    SquareDash,
    /// Short dashes.
    ShortDash,
    /// Long dashes.
    LongDash,
    /// Dash pattern from `pattern`.
    Custom,
    /// Two parallel lines, each a third of the width, a third apart (rectangles, rounded
    /// rectangles and ellipses; other shapes draw it solid). Older servers read it as solid.
    Double,
    /// Unknown value (solid).
    #[serde(other)]
    Unknown,
}

/// Drop shadow (element shadow or text shadow), stored in the polar form the editor shows.
///
/// `angle` is the direction the shadow is cast, in degrees counter-clockwise from +x in a
/// **y-up** sense (ProPresenter convention): 315 = down-right, 270 = straight down. The
/// renderer converts once: `dx = offset·cos(angle)`, `dy = −offset·sin(angle)` in canvas px
/// (y down), blur radius `radius` px, colour `color` with alpha multiplied by `opacity`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Shadow {
    /// The shadow is drawn.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// Direction, degrees CCW from +x, y-up (315 = down-right).
    #[serde(skip_serializing_if = "is_default_shadow_angle")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub angle: f64,
    /// Distance from the caster, px.
    #[serde(skip_serializing_if = "is_default_shadow_offset")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub offset: f64,
    /// Blur radius, px.
    #[serde(skip_serializing_if = "is_default_shadow_radius")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub radius: f64,
    /// Shadow colour.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub color: Color,
    /// Opacity `0..=1`, multiplied with `color.a`.
    #[serde(skip_serializing_if = "is_default_shadow_opacity")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub opacity: f64,
}

impl Default for Shadow {
    /// Disabled, 315°, offset 5, radius 5, black, opacity 0.75 (ProPresenter's default).
    fn default() -> Self {
        Shadow {
            enabled: false,
            angle: default_shadow_angle(),
            offset: default_shadow_offset(),
            radius: default_shadow_radius(),
            color: Color::BLACK,
            opacity: default_shadow_opacity(),
        }
    }
}

impl Shadow {
    /// The shadow's offset vector in canvas px (y down), derived from `angle` and `offset`.
    pub fn offset_vector(&self) -> Point {
        let rad = self.angle.to_radians();
        Point::new(self.offset * rad.cos(), -self.offset * rad.sin())
    }
}

/// Scrolling text (a decoration: disabled by default, keeps its settings when switched off).
///
/// The text is laid out once as a strip (on one line per paragraph for `left` / `right`,
/// wrapped to the box width for `up` / `down`) and moves through the box at `speed`, clipped
/// to it, timed from when the slide went live. With `repeat`, a new copy follows `gap` px
/// behind the last one (a seamless loop); without, it scrolls through once.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct TextScroll {
    /// The text scrolls.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// Which way the text moves.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub direction: ScrollDirection,
    /// Canvas px per second.
    #[serde(skip_serializing_if = "is_default_scroll_speed")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub speed: f64,
    /// Where the text is when the slide goes live.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub start: ScrollStart,
    /// Loop: copies follow each other for as long as the slide is live.
    #[serde(
        default = "crate::common::yes",
        skip_serializing_if = "crate::common::is_true"
    )]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub repeat: bool,
    /// Canvas px between the end of one copy and the start of the next.
    #[serde(skip_serializing_if = "is_default_scroll_gap")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub gap: f64,
}

impl Default for TextScroll {
    /// Disabled; left at 120 px/s from the automatic start, looping 120 px apart.
    fn default() -> Self {
        TextScroll {
            enabled: false,
            direction: ScrollDirection::Left,
            speed: default_scroll_speed(),
            start: ScrollStart::Auto,
            repeat: true,
            gap: default_scroll_gap(),
        }
    }
}

/// Which way scrolling text moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ScrollDirection {
    /// Right to left (a ticker).
    #[default]
    Left,
    /// Left to right.
    Right,
    /// Bottom to top (credits).
    Up,
    /// Top to bottom.
    Down,
    /// Unknown value (left).
    #[serde(other)]
    Unknown,
}

/// Where scrolling text starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ScrollStart {
    /// On screen where it is laid out when it fits the box, else just off screen.
    #[default]
    Auto,
    /// Just outside the box, on the side it moves in from.
    Offscreen,
    /// Where it is laid out in the box.
    Onscreen,
    /// Unknown value (automatic).
    #[serde(other)]
    Unknown,
}

/// Soft edge (feather).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Feather {
    /// The feather is applied.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// Where the soft edge sits relative to the outline.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub style: FeatherStyle,
    /// Width of the soft edge as a fraction `0..=1` of half the shorter side of the bounds.
    #[serde(skip_serializing_if = "is_default_feather_radius")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub radius: f64,
}

impl Default for Feather {
    /// Disabled, inside, 0.05 (ProPresenter's default).
    fn default() -> Self {
        Feather {
            enabled: false,
            style: FeatherStyle::Inside,
            radius: default_feather_radius(),
        }
    }
}

/// Feather placement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum FeatherStyle {
    /// Inside the outline.
    #[default]
    Inside,
    /// Centred on the outline.
    Center,
    /// Outside the outline.
    Outside,
    /// Unknown value (inside).
    #[serde(other)]
    Unknown,
}

// ---- placeholders ----------------------------------------------------------------------------

/// Marks an element as a named template slot.
///
/// Theme application ([`crate::template::apply_template`]) and look restyling
/// ([`crate::show::TemplateOverride`]) match content to template elements by `slot`, then by
/// order. ProPresenter has no slot names: the importer derives them from the element name
/// ("Verse", "Reference", "Title"), else `"text1"`, `"text2"`, ... by order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Placeholder {
    /// Slot name, e.g. `"text1"`, `"title"`, `"reference"`, `"picture"`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub slot: String,
    /// What the slot accepts.
    #[serde(default, skip_serializing_if = "is_default")]
    pub accepts: SlotAccepts,
}

impl Placeholder {
    /// A text slot.
    pub fn text(slot: impl Into<String>) -> Self {
        Placeholder {
            slot: slot.into(),
            accepts: SlotAccepts::Text,
        }
    }
    /// A media slot.
    pub fn media(slot: impl Into<String>) -> Self {
        Placeholder {
            slot: slot.into(),
            accepts: SlotAccepts::Media,
        }
    }
    /// True when the slot takes text.
    pub fn takes_text(&self) -> bool {
        matches!(
            self.accepts,
            SlotAccepts::Text | SlotAccepts::Any | SlotAccepts::Unknown
        )
    }
    /// True when the slot takes a media fill.
    pub fn takes_media(&self) -> bool {
        matches!(self.accepts, SlotAccepts::Media | SlotAccepts::Any)
    }
}

/// Content a placeholder accepts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SlotAccepts {
    /// Text (keeps the slide's words, takes the template's style).
    #[default]
    Text,
    /// A media fill (keeps the slide's picture, takes the template's frame).
    Media,
    /// Either.
    Any,
    /// Unknown value (treated as text).
    #[serde(other)]
    Unknown,
}

// ---- data links ---------------------------------------------------------------------------------

/// Live data rendered into an element's text at render time (stage layouts, countdown slides).
/// The element's text box styles the generated text; its own paragraphs are a design-time
/// preview.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DataLink {
    /// Text of the current / next slide.
    SlideText {
        /// Which slide.
        #[serde(default, skip_serializing_if = "is_default")]
        which: SlideWhich,
        /// Which text of that slide.
        #[serde(default, skip_serializing_if = "is_default")]
        source: SlideTextSource,
        /// Keep the source run formatting (superscript, italic) instead of plain text.
        #[serde(default, skip_serializing_if = "is_false")]
        keep_formatting: bool,
    },
    /// Speaker notes of the current / next slide.
    SlideNotes {
        /// Which slide.
        #[serde(default, skip_serializing_if = "is_default")]
        which: SlideWhich,
    },
    /// Thumbnail image of the current / next slide (fills the element).
    SlideImage {
        /// Which slide.
        #[serde(default, skip_serializing_if = "is_default")]
        which: SlideWhich,
    },
    /// Label of the current / next slide.
    SlideLabel {
        /// Which slide.
        #[serde(default, skip_serializing_if = "is_default")]
        which: SlideWhich,
    },
    /// Section (group) name of the current / next slide or next section.
    SectionName {
        /// Which section.
        #[serde(default, skip_serializing_if = "is_default")]
        which: SectionWhich,
    },
    /// Slide number / count.
    SlideCount {
        /// Which number.
        #[serde(default, skip_serializing_if = "is_default")]
        count: SlideCountKind,
    },
    /// Name of a playlist item relative to the live one.
    PlaylistItemName {
        /// Which item.
        #[serde(default, skip_serializing_if = "is_default")]
        which: PlaylistItemWhich,
    },
    /// Wall clock.
    Clock {
        /// Formatting.
        #[serde(default, skip_serializing_if = "is_default")]
        format: ClockFormat,
    },
    /// A timer's value.
    Timer {
        /// The timer. `None` (ProPresenter links without a timer UUID) has exactly one meaning:
        /// render the **most recently started** timer, else render empty.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        timer: Option<Link<TimerId>>,
        /// Units and template.
        #[serde(default, skip_serializing_if = "is_default")]
        format: TimerFormat,
        /// Text colour changes as the remaining time falls.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        color_triggers: Vec<ColorTrigger>,
        /// Which time: the timer's value (default), the time elapsed or the time remaining.
        #[serde(default, skip_serializing_if = "is_default")]
        show: TimerShow,
    },
    /// Remaining time of the playing video.
    VideoCountdown {
        /// Units and template.
        #[serde(default, skip_serializing_if = "is_default")]
        format: TimerFormat,
        /// Which layer's video.
        #[serde(default, skip_serializing_if = "is_default")]
        source: CountdownSource,
        /// Ignore looping videos.
        #[serde(default, skip_serializing_if = "is_false")]
        ignore_looping: bool,
    },
    /// Remaining time of the playing audio.
    AudioCountdown {
        /// Units and template.
        #[serde(default, skip_serializing_if = "is_default")]
        format: TimerFormat,
        /// Ignore looping audio.
        #[serde(default, skip_serializing_if = "is_false")]
        ignore_looping: bool,
    },
    /// Operator-to-stage message text.
    Message {
        /// Flash when a new message arrives.
        #[serde(default, skip_serializing_if = "is_false")]
        flash: bool,
        /// Flash colour (`None` = theme default).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        flash_color: Option<Color>,
    },
    /// CCLI copyright text of the current song: the copyright line built from its song info
    /// and the show's CCLI settings (empty when the live presentation has none).
    CcliText,
    /// Name of the live presentation (slide layer, or the announcements layer).
    PresentationName {
        /// Which layer's presentation.
        #[serde(default, skip_serializing_if = "is_default")]
        source: CountdownSource,
    },
    /// Notes of the live presentation (slide layer).
    PresentationNotes,
    /// The live song's artwork (a picture link: the element shows the image, fitted).
    SongArtwork,
    /// Chord chart: the current / next slide's lyrics with their chords drawn above the words,
    /// in the stage layout's chord view (transposition, capo, Nashville numbers). The element's
    /// text style is kept; the chords take [`crate::ChordDisplay`] from the element (shown even
    /// when it is off). See [`crate::chords`].
    ChordChart {
        /// Which slide.
        #[serde(default, skip_serializing_if = "is_default")]
        which: SlideWhich,
    },
    /// The live song's key for the stage layout's chord view (`"Key A · Capo 2 (G shapes)"`;
    /// empty without a song key or capo).
    SongKey,
    /// Imported link kind this build does not model (e.g. RSS); renders the element's own text.
    Unsupported {
        /// Source kind (ProPresenter oneof name, e.g. `"rssFeed"`).
        source_type: String,
        /// Verbatim source JSON.
        #[serde(default, skip_serializing_if = "crate::common::is_null")]
        #[cfg_attr(feature = "ts", ts(type = "unknown", optional = nullable))]
        payload: serde_json::Value,
    },
    /// Unknown tag from a newer schema (renders the element's own text).
    #[serde(other)]
    Unknown,
}

/// Which time a [`DataLink::Timer`] shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TimerShow {
    /// The timer's own value (a countdown's time left, a count-up timer's time).
    #[default]
    Value,
    /// Time elapsed: a countdown's duration minus its value; a count-up timer's value.
    Elapsed,
    /// Time remaining: a countdown's value; a count-up timer's end minus its value.
    Remaining,
    /// Unknown value (the timer's value).
    #[serde(other)]
    Unknown,
}

/// Which slide a data link reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SlideWhich {
    /// The live slide.
    #[default]
    Current,
    /// The slide after the live one (respecting the arrangement).
    Next,
    /// Unknown value (current).
    #[serde(other)]
    Unknown,
}

/// Which text of a slide a [`DataLink::SlideText`] shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SlideTextSource {
    /// All text: the slide's text slots, or every text element when it has none.
    #[default]
    AllText,
    /// Only the element filling this placeholder slot.
    Slot {
        /// Slot name.
        slot: String,
    },
    /// Only the element with this name.
    ElementNamed {
        /// Element name.
        name: String,
    },
    /// Unknown tag (all text).
    #[serde(other)]
    Unknown,
}

/// Which section a data link reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SectionWhich {
    /// Section of the live slide.
    #[default]
    CurrentSlide,
    /// Section of the next slide.
    NextSlide,
    /// The next section.
    NextSection,
    /// Unknown value.
    #[serde(other)]
    Unknown,
}

/// Slide counter kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SlideCountKind {
    /// 1-based index of the live slide.
    #[default]
    Current,
    /// Slides remaining.
    Remaining,
    /// Total slides.
    Total,
    /// Unknown value.
    #[serde(other)]
    Unknown,
}

/// Which playlist item a data link reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PlaylistItemWhich {
    /// The live item.
    #[default]
    Current,
    /// The next item.
    Next,
    /// The header above the live item.
    CurrentHeader,
    /// The next header.
    NextHeader,
    /// The playlist's own name.
    Playlist,
    /// Unknown value.
    #[serde(other)]
    Unknown,
}

/// Which layer a video countdown follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum CountdownSource {
    /// Presentation media (background / foreground).
    #[default]
    Presentation,
    /// Announcements layer.
    Announcements,
    /// Unknown value.
    #[serde(other)]
    Unknown,
}

/// Clock formatting (locale-aware).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ClockFormat {
    /// Text around the value with a `${clock}` token (`"Now: ${clock}"`); `None` = value only.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub template: Option<String>,
    /// Time part style.
    #[serde(skip_serializing_if = "is_short_style")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub time: DateTimeStyle,
    /// Date part style.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub date: DateTimeStyle,
    /// Force 24-hour time (`None` = locale default).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub use_24_hour: Option<bool>,
}

fn is_short_style(v: &DateTimeStyle) -> bool {
    *v == DateTimeStyle::Short
}

impl Default for ClockFormat {
    /// Short time, no date.
    fn default() -> Self {
        ClockFormat {
            template: None,
            time: DateTimeStyle::Short,
            date: DateTimeStyle::None,
            use_24_hour: None,
        }
    }
}

/// Date / time verbosity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DateTimeStyle {
    /// Omit this part.
    #[default]
    None,
    /// `9:30 AM`, `10/19/26`.
    Short,
    /// Medium.
    Medium,
    /// Long.
    Long,
    /// Full.
    Full,
    /// Unknown value (short).
    #[serde(other)]
    Unknown,
}

/// How a duration (timer, countdown) is rendered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct TimerFormat {
    /// Text around the value with a `${timer}` token (`"We begin in ${timer}"`);
    /// `None` = value only.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub template: Option<String>,
    /// Hours.
    #[serde(skip_serializing_if = "is_short_omit_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub hours: TimeUnitStyle,
    /// Minutes.
    #[serde(skip_serializing_if = "is_long_unit")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub minutes: TimeUnitStyle,
    /// Seconds.
    #[serde(skip_serializing_if = "is_long_unit")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub seconds: TimeUnitStyle,
    /// Milliseconds.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub milliseconds: TimeUnitStyle,
}

fn is_short_omit_zero(v: &TimeUnitStyle) -> bool {
    *v == TimeUnitStyle::ShortOmitZero
}
fn is_long_unit(v: &TimeUnitStyle) -> bool {
    *v == TimeUnitStyle::Long
}

impl Default for TimerFormat {
    /// `h:mm:ss` with the hours hidden while zero.
    fn default() -> Self {
        TimerFormat {
            template: None,
            hours: TimeUnitStyle::ShortOmitZero,
            minutes: TimeUnitStyle::Long,
            seconds: TimeUnitStyle::Long,
            milliseconds: TimeUnitStyle::Hidden,
        }
    }
}

/// How one time unit is rendered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TimeUnitStyle {
    /// Not shown.
    #[default]
    Hidden,
    /// No zero padding (`5`).
    Short,
    /// Zero padded (`05`).
    Long,
    /// Short, omitted while zero and leading.
    ShortOmitZero,
    /// Long, omitted while zero and leading.
    LongOmitZero,
    /// Unknown value (long).
    #[serde(other)]
    Unknown,
}

/// Change the text colour when a countdown reaches a remaining time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ColorTrigger {
    /// Remaining seconds at or below which the colour applies.
    pub remaining: f64,
    /// Text colour.
    pub color: Color,
}

// ---- visibility -------------------------------------------------------------------------------

/// One condition for drawing an element. An element's `visibility` list must hold entirely.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum VisibilityCondition {
    /// Another element on the same slide has (or lacks) visible text
    /// ("bullet N visible only if 'Point N' has text").
    ElementHasText {
        /// The other element (same slide).
        element: ElementId,
        /// `true` = visible when it has text; `false` = when it is empty.
        #[serde(
            default = "crate::common::yes",
            skip_serializing_if = "crate::common::is_true"
        )]
        #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
        has_text: bool,
    },
    /// A timer state.
    Timer {
        /// The timer (`None` = the most recently started timer, as for data links).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        timer: Option<Link<TimerId>>,
        /// Required state.
        #[serde(default, skip_serializing_if = "is_default")]
        state: TimerState,
    },
    /// A live video input is (not) showing.
    VideoInput {
        /// The input.
        input: Link<VideoInputId>,
        /// `true` = visible while the input is live.
        #[serde(
            default = "crate::common::yes",
            skip_serializing_if = "crate::common::is_true"
        )]
        #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
        active: bool,
    },
    /// A message is (not) on the messages layer.
    Message {
        /// The message (`None` = any message).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        message: Option<Link<MessageId>>,
        /// `true` = visible while it shows.
        #[serde(
            default = "crate::common::yes",
            skip_serializing_if = "crate::common::is_true"
        )]
        #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
        active: bool,
    },
    /// A stage message is (not) showing.
    StageMessage {
        /// `true` = visible while one shows.
        #[serde(
            default = "crate::common::yes",
            skip_serializing_if = "crate::common::is_true"
        )]
        #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
        active: bool,
    },
    /// A show layer has (no) content.
    Layer {
        /// The layer.
        layer: crate::show::Layer,
        /// `true` = visible while the layer has content.
        #[serde(
            default = "crate::common::yes",
            skip_serializing_if = "crate::common::is_true"
        )]
        #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
        active: bool,
    },
    /// The playlist around the live item has (no) such item: "Up next" only when there is
    /// a next item. `playlist` = the live item is in a playlist.
    PlaylistItem {
        /// Which item.
        #[serde(default, skip_serializing_if = "is_default")]
        which: PlaylistItemWhich,
        /// `true` = visible when it exists.
        #[serde(
            default = "crate::common::yes",
            skip_serializing_if = "crate::common::is_true"
        )]
        #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
        active: bool,
    },
    /// The presentation video's state (foreground, else background).
    VideoCountdown {
        /// Required state.
        #[serde(default, skip_serializing_if = "is_default")]
        state: MediaCondition,
    },
    /// The audio layer's state.
    AudioCountdown {
        /// Required state.
        #[serde(default, skip_serializing_if = "is_default")]
        state: MediaCondition,
    },
    /// Imported condition this build does not model; the element stays visible.
    Unsupported {
        /// Source kind.
        source_type: String,
        /// Verbatim source JSON.
        #[serde(default, skip_serializing_if = "crate::common::is_null")]
        #[cfg_attr(feature = "ts", ts(type = "unknown", optional = nullable))]
        payload: serde_json::Value,
    },
    /// Unknown tag (the element stays visible).
    #[serde(other)]
    Unknown,
}

/// Timer state for visibility conditions. Exported to TypeScript as `TimerCondition` (the
/// protocol's `TimerState` is the live timer).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "TimerCondition"))]
#[serde(rename_all = "camelCase")]
pub enum TimerState {
    /// Time remains (ProPresenter's default when the criterion is absent).
    #[default]
    HasTimeRemaining,
    /// Reached zero.
    HasExpired,
    /// Running.
    IsRunning,
    /// Not running.
    NotRunning,
    /// Unknown value (condition holds).
    #[serde(other)]
    Unknown,
}

/// Media state for the video and audio countdown conditions. Nothing playing counts as
/// expired and not running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum MediaCondition {
    /// Playing media with time left (running or paused).
    #[default]
    HasTimeRemaining,
    /// Ended, or nothing live.
    HasExpired,
    /// Playing (not paused, not ended).
    IsRunning,
    /// Paused, ended or nothing live.
    NotRunning,
    /// The live media loops.
    Looping,
    /// The live media does not loop (or nothing is live).
    NotLooping,
    /// Unknown value (condition holds).
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_315_points_down_right() {
        let s = Shadow {
            offset: 10.0,
            ..Shadow::default()
        };
        let v = s.offset_vector();
        assert!(v.x > 7.0 && v.y > 7.0, "{v:?}");
    }

    #[test]
    fn disabled_decorations_are_omitted_but_tuned_ones_survive() {
        let mut e = Element::new(Rect::new(0.0, 0.0, 10.0, 10.0));
        let json = serde_json::to_value(&e).unwrap();
        assert!(json.get("stroke").is_none() && json.get("fill").is_none());
        e.stroke.width = 8.0; // tuned but disabled (template setting)
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["stroke"], serde_json::json!({"width": 8.0}));
        let back: Element = serde_json::from_value(json).unwrap();
        assert_eq!(back, e);
        assert!(!back.stroke.enabled);
    }
}
