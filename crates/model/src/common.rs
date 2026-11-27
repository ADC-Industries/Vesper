//! Shared primitives: colours, geometry, frame rates, hotkeys, links, ordering keys, timestamps
//! and import provenance.
//!
//! ## Units and conventions (apply everywhere unless a field says otherwise)
//! - **Lengths** are `f64` **pixels in slide-canvas space**: origin at the top-left of the canvas,
//!   +x right, +y down. A 1920×1080 slide spans `0..1920 × 0..1080`. ProPresenter points map 1:1
//!   onto these pixels. Geometry stays `f64` so imported values such as `100.50000000000001`
//!   round-trip bit-exact.
//! - **Font sizes** are canvas pixels too (ProPresenter "130 pt" on a 1080p slide = 130 px).
//! - **Angles** are degrees; element rotation is clockwise (y-down). Shadow angles keep the
//!   ProPresenter convention, see [`crate::element::Shadow`].
//! - **Times / durations** are `f64` seconds. **Frame rates** are rational [`FrameRate`]s.
//! - **Colours** are [`Color`]: straight (non-premultiplied) sRGB, `f32` components `0..=1`.
//! - **Opacity** is `0..=1`. **Audio gain** is in dB (0 = unity).
//! - **Timestamps** are RFC 3339 UTC strings ([`Timestamp`]).

use serde::{Deserialize, Serialize};

// ---- serde helpers -----------------------------------------------------------------------

pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}
pub(crate) fn is_true(b: &bool) -> bool {
    *b
}
pub(crate) fn yes() -> bool {
    true
}
pub(crate) fn one() -> f64 {
    1.0
}
pub(crate) fn is_one(v: &f64) -> bool {
    *v == 1.0
}
pub(crate) fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
pub(crate) fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}
pub(crate) fn is_null(v: &serde_json::Value) -> bool {
    v.is_null()
}

/// Gain in JSON for "silent": JSON has no infinities, so a `-inf` dB gain (or NaN) is written as
/// this floor, and a `null` gain (how older writers stored `-inf`) reads back as it. Finite
/// gains are written as they are. Prefer an explicit mute flag over a very low gain.
pub const SILENCE_DB: f64 = -200.0;

/// Serde adapter for dB gains (`#[serde(with = "crate::common::gain_db")]`): never writes a
/// non-finite number (which serde_json would write as `null` and then fail to read).
pub(crate) mod gain_db {
    use super::SILENCE_DB;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
        let v = match *v {
            v if v.is_nan() => SILENCE_DB,
            f64::INFINITY => f64::MAX,
            f64::NEG_INFINITY => SILENCE_DB,
            v => v,
        };
        s.serialize_f64(v)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(d)?.unwrap_or(SILENCE_DB))
    }
}

/// Declares a serde default function and the matching `skip_serializing_if` predicate for a
/// field whose default is not its type's `Default`, so both always agree.
macro_rules! default_fns {
    ($($default_fn:ident, $skip_fn:ident: $t:ty = $v:expr;)*) => {$(
        pub(crate) fn $default_fn() -> $t {
            $v
        }
        pub(crate) fn $skip_fn(v: &$t) -> bool {
            *v == $v
        }
    )*};
}
pub(crate) use default_fns;

// ---- colour --------------------------------------------------------------------------------

/// An RGBA colour: straight (non-premultiplied) alpha, sRGB-encoded components `0..=1` (the
/// numbers a colour picker shows, divided by 255).
///
/// `a` defaults to 1 when omitted, so opaque colours serialize as `{r,g,b}`. ProPresenter writes
/// `(1,1,1,0)` to mean "no colour"; the importer maps that to `None` where the field is optional.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Color {
    /// Red, sRGB `0..=1`.
    pub r: f32,
    /// Green, sRGB `0..=1`.
    pub g: f32,
    /// Blue, sRGB `0..=1`.
    pub b: f32,
    /// Alpha (coverage): 0 = transparent, 1 = opaque. Not premultiplied into r/g/b.
    #[serde(default = "one_f32", skip_serializing_if = "is_one_f32")]
    pub a: f32,
}

fn one_f32() -> f32 {
    1.0
}
fn is_one_f32(v: &f32) -> bool {
    *v == 1.0
}

impl Color {
    /// Opaque black.
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    /// Opaque white.
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
    /// Fully transparent black.
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);

    /// Opaque colour from sRGB components.
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Color { r, g, b, a: 1.0 }
    }
    /// Colour with explicit alpha.
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }
    /// From 8-bit sRGB components (opaque).
    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Color::rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    }
}

impl Default for Color {
    /// Opaque black (consistent with `a` defaulting to 1).
    fn default() -> Self {
        Color::BLACK
    }
}

// ---- geometry ------------------------------------------------------------------------------

/// A point in canvas pixels (or unit coordinates where a field says so).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Point {
    /// Horizontal position, rightwards.
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub x: f64,
    /// Vertical position, downwards.
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub y: f64,
}

impl Point {
    /// Point from coordinates.
    pub const fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}

/// A size in pixels. Defaults to 1920×1080, the size of every slide in the reference library.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Size {
    /// Width in px.
    pub width: f64,
    /// Height in px.
    pub height: f64,
}

impl Size {
    /// 1920×1080.
    pub const HD: Size = Size {
        width: 1920.0,
        height: 1080.0,
    };

    /// Size from width and height.
    pub const fn new(width: f64, height: f64) -> Self {
        Size { width, height }
    }
}

impl Default for Size {
    /// [`Size::HD`].
    fn default() -> Self {
        Size::HD
    }
}

/// An axis-aligned rectangle in canvas pixels. `x`,`y` is the top-left corner *before*
/// rotation (elements rotate about the centre).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Rect {
    /// Left edge, px.
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub x: f64,
    /// Top edge, px.
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub y: f64,
    /// Width, px (≥ 0).
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub width: f64,
    /// Height, px (≥ 0).
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub height: f64,
}

impl Rect {
    /// Rectangle from position and size.
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
    /// Rectangle covering a whole canvas.
    pub const fn full(canvas: Size) -> Self {
        Rect::new(0.0, 0.0, canvas.width, canvas.height)
    }
}

/// Insets from each edge of a rectangle, in px (text margins, crop insets).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct EdgeInsets {
    /// Inset from the left edge.
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub left: f64,
    /// Inset from the right edge.
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub right: f64,
    /// Inset from the top edge.
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub top: f64,
    /// Inset from the bottom edge.
    #[serde(skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub bottom: f64,
}

impl EdgeInsets {
    /// Same inset on all sides.
    pub const fn uniform(v: f64) -> Self {
        EdgeInsets {
            left: v,
            right: v,
            top: v,
            bottom: v,
        }
    }
}

/// Mirroring. ProPresenter `FLIP_MODE_BOTH` = both flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Flip {
    /// Mirror left↔right.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub horizontal: bool,
    /// Mirror top↔bottom.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub vertical: bool,
}

/// An inclusive range of integers (chapters, verses).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct IntRange {
    /// First value (inclusive).
    pub start: u32,
    /// Last value (inclusive). Equal to `start` for a single value.
    pub end: u32,
}

impl IntRange {
    /// A single value.
    pub const fn single(v: u32) -> Self {
        IntRange { start: v, end: v }
    }
}

/// A frame rate as an exact rational: `num / den` frames per second (30000/1001 = 29.97).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameRate {
    /// Numerator (frames).
    pub num: u32,
    /// Denominator (seconds); 1 for integer rates, 1001 for NTSC rates.
    pub den: u32,
}

impl FrameRate {
    /// 25 fps.
    pub const FPS25: FrameRate = FrameRate { num: 25, den: 1 };
    /// 29.97 fps.
    pub const FPS29_97: FrameRate = FrameRate {
        num: 30000,
        den: 1001,
    };
    /// 30 fps.
    pub const FPS30: FrameRate = FrameRate { num: 30, den: 1 };
    /// 50 fps.
    pub const FPS50: FrameRate = FrameRate { num: 50, den: 1 };
    /// 59.94 fps.
    pub const FPS59_94: FrameRate = FrameRate {
        num: 60000,
        den: 1001,
    };
    /// 60 fps.
    pub const FPS60: FrameRate = FrameRate { num: 60, den: 1 };

    /// Integer frame rate.
    pub const fn fps(n: u32) -> Self {
        FrameRate { num: n, den: 1 }
    }
    /// Frames per second as a float.
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den.max(1) as f64
    }
}

impl Default for FrameRate {
    /// 30 fps.
    fn default() -> Self {
        FrameRate::FPS30
    }
}

// ---- hotkeys and links -----------------------------------------------------------------------

/// A keyboard shortcut.
///
/// `code` is a W3C `KeyboardEvent.code` string (`"KeyA"`, `"Digit1"`, `"F5"`), kept as an open
/// string so new keys never need a schema change. ProPresenter `KEY_CODE_ANSI_A` → `"KeyA"`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct HotKey {
    /// Physical key, W3C `KeyboardEvent.code`.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub code: String,
    /// Shift held.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub shift: bool,
    /// Control held.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub ctrl: bool,
    /// Alt / Option held.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub alt: bool,
    /// Meta / Command / Windows key held.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub meta: bool,
}

impl HotKey {
    /// A key without modifiers.
    pub fn key(code: impl Into<String>) -> Self {
        HotKey {
            code: code.into(),
            ..HotKey::default()
        }
    }
}

/// A reference to another entity by ID, plus the name it had when the link was made.
///
/// The name is a *hint*: it lets the UI show "Walk-in Timer (missing)" for a dangling link
/// and lets a later import re-resolve by name. ProPresenter references timers, themes and global
/// groups the same way (UUID + name), and many such links dangle in real libraries, so both are
/// kept instead of dropping unresolved links.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Link<T> {
    /// Target ID. May not resolve (target deleted, or never imported).
    pub id: T,
    /// Target's name when the link was made (display / re-resolution hint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<String>,
}

impl<T> Link<T> {
    /// A link without a name hint.
    pub fn to(id: T) -> Self {
        Link { id, name: None }
    }
    /// A link with a name hint.
    pub fn named(id: T, name: impl Into<String>) -> Self {
        Link {
            id,
            name: Some(name.into()),
        }
    }
}

/// An RFC 3339 UTC timestamp string, e.g. `"2026-10-18T09:30:00Z"`.
pub type Timestamp = String;

// ---- ordering ----------------------------------------------------------------------------------

/// Fractional-index ordering key for sibling documents (libraries, playlist nodes, bins, looks,
/// timers…). Compare as plain byte strings; ties are broken by ID. Insert between two neighbours
/// with [`sort_key_between`]: no other document changes, so concurrent inserts from two clients
/// never conflict. An empty key means "unordered" and sorts first.
pub type SortKey = String;

const KEY_DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn key_digit(c: u8) -> usize {
    KEY_DIGITS.iter().position(|&d| d == c).unwrap_or(0)
}

fn key_midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        // Skip the common prefix (missing digits of `a` count as '0').
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(b'0') == b[n] {
            n += 1;
        }
        if n > 0 {
            let mut out = b[..n].to_vec();
            out.extend(key_midpoint(a.get(n..).unwrap_or(&[]), Some(&b[n..])));
            return out;
        }
    }
    let da = a.first().map(|&c| key_digit(c)).unwrap_or(0);
    let db = b
        .and_then(|b| b.first())
        .map(|&c| key_digit(c))
        .unwrap_or(KEY_DIGITS.len());
    if db > da + 1 {
        return vec![KEY_DIGITS[(da + db) / 2]];
    }
    if let Some(b) = b
        && b.len() > 1
    {
        return vec![b[0]];
    }
    let mut out = vec![KEY_DIGITS[da]];
    out.extend(key_midpoint(a.get(1..).unwrap_or(&[]), None));
    out
}

/// A sort key strictly between `before` and `after` (`None` = open end). Keys use the digits
/// `0-9A-Za-z` and never end in `'0'`, which keeps the space dense. `before` must sort strictly
/// before `after` (checked in debug builds).
///
/// Stored keys are client-supplied strings, so a neighbour may use other characters or end in
/// `'0'`. Such a key never panics; the result is then still strictly between the two whenever
/// any string is (it may contain the neighbour's own characters).
pub fn sort_key_between(before: Option<&str>, after: Option<&str>) -> SortKey {
    debug_assert!(match (before, after) {
        (Some(a), Some(b)) => a < b,
        _ => true,
    });
    let a = before.unwrap_or("");
    let valid = |k: &str| !k.ends_with('0') && k.bytes().all(|c| KEY_DIGITS.contains(&c));
    if valid(a) && after.is_none_or(valid) {
        let k = key_midpoint(a.as_bytes(), after.map(str::as_bytes));
        return String::from_utf8(k).expect("sort keys are ASCII");
    }
    // Any key: extend `before`. That sorts after it, and before `after` unless `before` is a
    // prefix of `after`.
    let Some(rest) = after.and_then(|b| b.strip_prefix(a)) else {
        return format!("{a}V");
    };
    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        // `before` + the first character of the rest: a proper prefix of `after`.
        (Some(c), Some(_)) => format!("{a}{c}"),
        (Some(c), None) if c > '0' => format!("{a}0V"),
        (Some(c), None) if c > '\0' => {
            let below = char::from_u32(u32::from(c) - 1).unwrap_or('\0');
            format!("{a}{below}V")
        }
        // Nothing sorts strictly between (`after` is `before` + "" or + NUL).
        _ => format!("{a}V"),
    }
}

// ---- import provenance ---------------------------------------------------------------------------

/// Where an imported entity came from.
///
/// The importer owns warnings and reports; this record only keeps what is needed to
/// (a) re-import idempotently, (b) explain an entity's origin in the UI, and (c) never lose
/// source data the model does not represent yet (`unmapped`). Carried as `import` on every
/// addressable entity: presentation, section, slide, element, action, playlist item, media
/// asset, theme, template slide, look and screen.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ImportInfo {
    /// Product / format the entity was imported from.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub source: ImportSource,
    /// Path of the source document relative to the imported backup root, `/`-separated.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub document_path: Option<String>,
    /// Original identifier when it differs from our ID (e.g. a de-duplicated theme copy, or a
    /// dangling playlist arrangement UUID that became `None`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub original_id: Option<String>,
    /// Original display name when it was not carried into the model (e.g. ProPresenter cue
    /// names like `" 3"`, which are auto-numbering noise).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub original_name: Option<String>,
    /// Version of the application that last wrote the source document (e.g. `"21.2.0"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub writer_version: Option<String>,
    /// When the import ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub imported_at: Option<Timestamp>,
    /// Verbatim JSON of source fields the model has no slot for, keyed by source field name
    /// (e.g. `{"info": 7, "originalFont": {...}, "buildIn": {...}}`). Opaque to everything except
    /// a future importer / migration; never interpreted by the renderer. `null` = nothing.
    #[serde(skip_serializing_if = "is_null")]
    #[cfg_attr(feature = "ts", ts(type = "unknown", optional = nullable))]
    pub unmapped: serde_json::Value,
}

impl ImportInfo {
    /// Provenance for a ProPresenter 7 document path.
    pub fn propresenter(document_path: impl Into<String>) -> Self {
        ImportInfo {
            source: ImportSource::ProPresenter7,
            document_path: Some(document_path.into()),
            ..ImportInfo::default()
        }
    }
}

/// The product / format an entity was imported from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ImportSource {
    /// ProPresenter 7.x and later (`.pro`, playlists, themes, `Configuration/`).
    #[default]
    ProPresenter7,
    /// ProPresenter 6 XML (`.pro6`), reserved.
    ProPresenter6,
    /// Plain text / lyrics paste.
    PlainText,
    /// A PowerPoint `.pptx` (also Keynote and Google Slides, exported as `.pptx`).
    PowerPoint,
    /// A source this build does not know.
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_keys_stay_ordered() {
        let first = sort_key_between(None, None);
        let mut keys = vec![first];
        // Append at the end, prepend at the start, insert in the middle, repeatedly.
        for i in 0..200 {
            let k = match i % 3 {
                0 => sort_key_between(keys.last().map(String::as_str), None),
                1 => sort_key_between(None, keys.first().map(String::as_str)),
                _ => {
                    let mid = keys.len() / 2;
                    sort_key_between(Some(&keys[mid - 1]), Some(&keys[mid]))
                }
            };
            keys.push(k);
            keys.sort();
            for w in keys.windows(2) {
                assert!(w[0] < w[1], "{:?} !< {:?}", w[0], w[1]);
            }
        }
        assert!(keys.iter().all(|k| !k.ends_with('0') && !k.is_empty()));
    }

    #[test]
    fn foreign_sort_keys_neither_panic_nor_misorder() {
        // Stored keys are client-supplied: other characters, or a trailing '0'. These used to
        // panic ("z" < "~": subtract overflow; "1" < "é": non-UTF-8 result) or to return a key
        // outside the gap ("a0V" for "a".."a0", "V" after "~").
        let pairs = [
            (Some("z"), Some("~")),
            (Some("1"), Some("é")),
            (Some("a"), Some("a0")),
            (Some("A"), Some("a-")),
            (Some("a"), Some("a-")),
            (Some("x"), Some("x-b")),
            (Some("é"), Some("éé")),
            (None, Some("0")),
            (None, Some("-")),
            (Some("~"), None),
            (Some("é"), None),
            (Some("a0"), None),
            (Some("a-"), Some("b")),
        ];
        for (a, b) in pairs {
            let k = sort_key_between(a, b);
            assert!(a.is_none_or(|a| a < k.as_str()), "{a:?} !< {k:?}");
            assert!(b.is_none_or(|b| k.as_str() < b), "{k:?} !< {b:?}");
        }
    }

    #[test]
    fn color_alpha_defaults_to_opaque() {
        let c: Color = serde_json::from_str(r#"{"r":1,"g":0,"b":0}"#).unwrap();
        assert_eq!(c, Color::rgb(1.0, 0.0, 0.0));
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"r":1.0,"g":0.0,"b":0.0}"#
        );
        assert_eq!(Color::default().a, 1.0);
    }
}
