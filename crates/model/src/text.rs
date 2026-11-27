//! Rich text: text boxes, paragraphs, runs, character and paragraph styles, fonts.
//!
//! ## Structure
//! ```text
//! TextBox                       box level: vertical align, fit, margins, text shadow, outline,
//!  │                            line mask, glyph fill, font scale factor
//!  ├─ defaultStyle: CharStyle   the box's character style (ProPresenter `attributes`); alone it
//!  │                            defines the look of an empty box
//!  ├─ defaultParagraph          the box's paragraph style
//!  └─ paragraphs[]              one per hard line (`\par` in RTF)
//!      ├─ style                 paragraph overrides (alignment, spacing, indents, list)
//!      ├─ runs[]                text + sparse character overrides (superscript, ALL CAPS, ...)
//!      └─ endStyle              style of the paragraph mark (height of an empty line)
//! ```
//!
//! ## Style cascade
//! Every [`CharStyle`] field is an `Option`, resolved field by field:
//! run `style` → paragraph `endStyle` (only for empty paragraphs) → box `defaultStyle` →
//! [`CharStyle::fallback`] (the engine default). See [`CharStyle::merged`] and
//! [`TextBox::run_style`]. Paragraph styles cascade the same way: paragraph `style` → box
//! `defaultParagraph` → [`ParagraphStyle::fallback`]. The importer stores in runs only the
//! fields that differ from `defaultStyle`, so changing the box font later restyles the whole
//! box while superscript verse numbers stay superscript.
//!
//! ## Fonts
//! A font is family + weight (CSS 100–900) + italic, plus an optional PostScript name that the
//! resolver tries first ([`FontRef`]). **Bold has one source of truth: `fontWeight`.** The
//! PostScript name is an import precision hint ("Calibri-Bold"); editors clear it whenever the
//! user changes family, weight or italic ([`CharStyle::set_family`] and friends), and the cascade
//! drops an inherited PostScript name when a more specific level changes family, weight or
//! italic (so an italic run never pins the box's upright face, and a family change never keeps a
//! stale face). Substitution of missing fonts is machine-local
//! ([`crate::machine::FontSubstitutionTable`]).
//!
//! ## Units
//! Sizes, spacing, indents, margins and offsets are canvas px; `lineHeightMultiple` and
//! `fontScaleFactor` are multipliers. Text is stored in its original case; `capitalization` is
//! applied at display time.
//!
//! ## ProPresenter import notes (census)
//! - RTF `\par` → [`Paragraph`]; `\b`/`\i` → `fontWeight`/`italic`; `\f` → `postscriptName`
//!   (and the resolved family/weight/italic); `\fs`/2 → `size`; `\cf` → `color` (compare
//!   resolved colours, never colour-table indices); `\super`/`\sub` → `baseline`;
//!   `\expndtw`/20 → `letterSpacing`; `\cb`/`\highlight` → `highlight`; `\qc \ql \qr \qj` →
//!   `alignment`; `\li \fi \ri` → indents; `\sb \sa` → spacing; `\sl\slmult` → line height.
//! - Capitalization is not in the RTF: `attributes.capitalization` → `defaultStyle`, and
//!   `customAttributes[].capitalization` UTF-16 ranges split runs and set run overrides.
//! - `\strokewidth` / `strokeWidth` → box [`TextOutline`]; `customAttributes[].fontScaleFactor`
//!   → box `fontScaleFactor`; `originalFont` → `ImportInfo::unmapped` on the element.
//! - Empty boxes have no runs; their look comes only from `defaultStyle` / `defaultParagraph`,
//!   filled from `attributes`. Never drop `attributes` for empty boxes.

use crate::common::{Color, EdgeInsets, default_fns, is_default};
use crate::element::{Fill, Shadow};
use serde::{Deserialize, Serialize};

default_fns! {
    default_outline_width, is_default_outline_width: f64 = 3.0;
}

/// A font description: family + weight + italic, with an optional exact PostScript face.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FontRef {
    /// Family name as users see it (`"Calibri"`).
    pub family: String,
    /// CSS weight 100–900 (400 regular, 700 bold).
    pub weight: u16,
    /// Italic / oblique face.
    #[serde(default, skip_serializing_if = "crate::common::is_false")]
    pub italic: bool,
    /// Exact face requested by an import (`"Calibri-Bold"`); tried first by the resolver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub postscript_name: Option<String>,
}

/// A text box: paragraphs plus box-level layout and decoration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct TextBox {
    /// Paragraphs in order. Empty = an empty (but styled) box.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub paragraphs: Vec<Paragraph>,
    /// The box's character style. Importers and editors fill it completely enough to render an
    /// empty box (family, weight, size, colour); unset fields fall back to
    /// [`CharStyle::fallback`]. Box-level ALL CAPS is `defaultStyle.capitalization`.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub default_style: CharStyle,
    /// The box's paragraph style.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub default_paragraph: ParagraphStyle,
    /// Vertical placement of the text block inside the box.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub vertical_align: VerticalAlign,
    /// What happens when the text does not fit.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub fit: FitMode,
    /// Extra size multiplier applied to every run (ProPresenter `fontScaleFactor`).
    #[serde(skip_serializing_if = "crate::common::is_one")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub font_scale_factor: f64,
    /// Inner padding between the element bounds and the text, px.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub margins: EdgeInsets,
    /// Drop shadow behind the glyphs (distinct from the element shadow).
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub text_shadow: Shadow,
    /// Glyph outline.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub outline: TextOutline,
    /// Boxes drawn behind each text line.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub line_mask: LineMask,
    /// Non-solid glyph fill (gradient, media, cut-out). When enabled it replaces the run
    /// colours; when disabled (the default) glyphs use the cascaded `color`.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub fill: Fill,
    /// Draw the paragraphs' chords above the words (off by default: audience screens never show
    /// chords). See [`crate::chords`].
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub chords: crate::chords::ChordDisplay,
}

impl Default for TextBox {
    fn default() -> Self {
        TextBox {
            paragraphs: Vec::new(),
            default_style: CharStyle::default(),
            default_paragraph: ParagraphStyle::default(),
            vertical_align: VerticalAlign::default(),
            fit: FitMode::default(),
            font_scale_factor: 1.0,
            margins: EdgeInsets::default(),
            text_shadow: Shadow::default(),
            outline: TextOutline::default(),
            line_mask: LineMask::default(),
            fill: Fill::default(),
            chords: Default::default(),
        }
    }
}

impl TextBox {
    /// Unstyled text, one paragraph per `\n`-separated line.
    pub fn plain(text: &str) -> Self {
        TextBox {
            paragraphs: Paragraph::from_lines(text),
            ..TextBox::default()
        }
    }

    /// Plain text, paragraphs joined by `\n` (search, slide grid, stage display).
    pub fn plain_text(&self) -> String {
        self.paragraphs
            .iter()
            .map(Paragraph::plain_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// True when there is no visible character.
    pub fn is_blank(&self) -> bool {
        self.paragraphs
            .iter()
            .all(|p| p.runs.iter().all(|r| r.text.trim().is_empty()))
    }

    /// Effective character style of a run: run → box default → engine fallback.
    pub fn run_style(&self, run: &TextRun) -> CharStyle {
        run.style
            .merged(&self.default_style)
            .merged(&CharStyle::fallback())
    }

    /// Effective character style of an empty paragraph (its line height and caret style):
    /// `endStyle` → box default → engine fallback.
    pub fn empty_paragraph_style(&self, paragraph: &Paragraph) -> CharStyle {
        paragraph
            .end_style
            .merged(&self.default_style)
            .merged(&CharStyle::fallback())
    }

    /// Effective paragraph style: paragraph → box default → engine fallback.
    pub fn paragraph_style(&self, paragraph: &Paragraph) -> ParagraphStyle {
        paragraph
            .style
            .merged(&self.default_paragraph)
            .merged(&ParagraphStyle::fallback())
    }
}

/// One paragraph (hard line).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Paragraph {
    /// Overrides of the box `defaultParagraph`.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub style: ParagraphStyle,
    /// Text runs. May be empty (blank line).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub runs: Vec<TextRun>,
    /// Character style of the paragraph mark: decides the height of an empty paragraph and the
    /// style used when typing at its end. Empty = inherit the box default.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub end_style: CharStyle,
    /// Chords over this line, in order (see [`crate::chords`]). Not text: drawn only by boxes
    /// whose `chords` display is on.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub chords: Vec<crate::chords::ChordMark>,
}

impl Paragraph {
    /// A paragraph with one unstyled run (no run for empty text).
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Paragraph {
            runs: if text.is_empty() {
                Vec::new()
            } else {
                vec![TextRun::plain(text)]
            },
            ..Paragraph::default()
        }
    }
    /// Splits text on `\n` into unstyled paragraphs.
    pub fn from_lines(text: &str) -> Vec<Self> {
        text.split('\n')
            .map(|l| Paragraph::text(l.trim_end_matches('\r')))
            .collect()
    }
    /// Concatenated run text.
    pub fn plain_text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
    /// True when the line has chords.
    pub fn has_chords(&self) -> bool {
        !self.chords.is_empty()
    }
}

/// A run of characters sharing one style.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct TextRun {
    /// The characters (no paragraph breaks; U+2028 for a soft line break, `\t` for tab).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub text: String,
    /// Sparse overrides of the box `defaultStyle`.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub style: CharStyle,
}

impl TextRun {
    /// An unstyled run.
    pub fn plain(text: impl Into<String>) -> Self {
        TextRun {
            text: text.into(),
            style: CharStyle::default(),
        }
    }
    /// A run with style overrides.
    pub fn styled(text: impl Into<String>, style: CharStyle) -> Self {
        TextRun {
            text: text.into(),
            style,
        }
    }
}

/// Plain or rich text outside a text box (slide and presentation notes).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct RichText {
    /// Paragraphs; plain notes are unstyled runs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub paragraphs: Vec<Paragraph>,
}

impl RichText {
    /// Unstyled text, one paragraph per line.
    pub fn plain(text: &str) -> Self {
        RichText {
            paragraphs: Paragraph::from_lines(text),
        }
    }
    /// Plain text, paragraphs joined by `\n`.
    pub fn plain_text(&self) -> String {
        self.paragraphs
            .iter()
            .map(Paragraph::plain_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
    /// True when there is no paragraph.
    pub fn is_empty(&self) -> bool {
        self.paragraphs.is_empty()
    }
}

/// Character formatting. Every field is optional: `None` inherits (see the module docs).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct CharStyle {
    /// Font family (`"Calibri"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub font_family: Option<String>,
    /// CSS weight 100–900 (400 regular, 700 bold). The only source of "bold".
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub font_weight: Option<u16>,
    /// Italic / oblique.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub italic: Option<bool>,
    /// Exact face from an import (`"Calibri-Bold"`), tried first by the resolver. Editors clear
    /// it when family, weight or italic change.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub postscript_name: Option<String>,
    /// Authored font size, canvas px (before fit scaling and `fontScaleFactor`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub size: Option<f64>,
    /// Solid glyph colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<Color>,
    /// Underline (`none` explicitly turns off an inherited one).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub underline: Option<LineStyle>,
    /// Strikethrough (`none` explicitly turns off an inherited one).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub strikethrough: Option<LineStyle>,
    /// Superscript / subscript (verse numbers).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub baseline: Option<Baseline>,
    /// Display-time case transform; the stored text keeps its case.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub capitalization: Option<Capitalization>,
    /// Extra space between characters, px (ProPresenter "kerning"); may be negative.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub letter_spacing: Option<f64>,
    /// Background colour behind the glyphs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub highlight: Option<Color>,
}

impl CharStyle {
    /// True when no field is set.
    pub fn is_empty(&self) -> bool {
        *self == CharStyle::default()
    }

    /// The engine fallback at the bottom of the cascade: Arial 50 px regular white.
    pub fn fallback() -> CharStyle {
        CharStyle {
            font_family: Some("Arial".into()),
            font_weight: Some(400),
            italic: Some(false),
            postscript_name: None,
            size: Some(50.0),
            color: Some(Color::WHITE),
            underline: Some(LineStyle::None),
            strikethrough: Some(LineStyle::None),
            baseline: Some(Baseline::Normal),
            capitalization: Some(Capitalization::None),
            letter_spacing: Some(0.0),
            highlight: None,
        }
    }

    /// Field-wise cascade: values set in `self` win, missing ones come from `base`.
    ///
    /// Exception: if `self` sets family, weight or italic but no PostScript name, the inherited
    /// PostScript name is dropped (it names a different face).
    pub fn merged(&self, base: &CharStyle) -> CharStyle {
        let face_changed =
            self.font_family.is_some() || self.font_weight.is_some() || self.italic.is_some();
        let postscript_name = match (&self.postscript_name, face_changed) {
            (Some(ps), _) => Some(ps.clone()),
            (None, true) => None,
            (None, false) => base.postscript_name.clone(),
        };
        CharStyle {
            font_family: self
                .font_family
                .clone()
                .or_else(|| base.font_family.clone()),
            font_weight: self.font_weight.or(base.font_weight),
            italic: self.italic.or(base.italic),
            postscript_name,
            size: self.size.or(base.size),
            color: self.color.or(base.color),
            underline: self.underline.or(base.underline),
            strikethrough: self.strikethrough.or(base.strikethrough),
            baseline: self.baseline.or(base.baseline),
            capitalization: self.capitalization.or(base.capitalization),
            letter_spacing: self.letter_spacing.or(base.letter_spacing),
            highlight: self.highlight.or(base.highlight),
        }
    }

    /// The overrides that survive a restyle with "keep inline formatting": baseline, italic,
    /// weight and underline. Everything else (family, size, colour, face, ...) is dropped so the
    /// template's style takes over.
    pub fn inline_formatting(&self) -> CharStyle {
        CharStyle {
            font_weight: self.font_weight,
            italic: self.italic,
            underline: self.underline,
            baseline: self.baseline,
            ..CharStyle::default()
        }
    }

    /// The resolved font of a (fully cascaded) style.
    pub fn font_ref(&self) -> FontRef {
        FontRef {
            family: self.font_family.clone().unwrap_or_else(|| "Arial".into()),
            weight: self.font_weight.unwrap_or(400),
            italic: self.italic.unwrap_or(false),
            postscript_name: self.postscript_name.clone(),
        }
    }

    /// Editor operation: change the family (clears the PostScript hint).
    pub fn set_family(&mut self, family: impl Into<String>) {
        self.font_family = Some(family.into());
        self.postscript_name = None;
    }
    /// Editor operation: change the weight (clears the PostScript hint).
    pub fn set_weight(&mut self, weight: u16) {
        self.font_weight = Some(weight);
        self.postscript_name = None;
    }
    /// Editor operation: change italic (clears the PostScript hint).
    pub fn set_italic(&mut self, italic: bool) {
        self.italic = Some(italic);
        self.postscript_name = None;
    }
}

/// Underline / strikethrough line style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum LineStyle {
    /// No line (overrides an inherited one).
    #[default]
    None,
    /// Single thin line.
    Single,
    /// Single thick line.
    Thick,
    /// Double line.
    Double,
    /// Unknown value (rendered as single).
    #[serde(other)]
    Unknown,
}

/// Vertical position of glyphs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Baseline {
    /// Normal.
    #[default]
    Normal,
    /// Superscript (verse numbers).
    Superscript,
    /// Subscript.
    Subscript,
    /// Unknown value (rendered normal).
    #[serde(other)]
    Unknown,
}

/// Display-time case transform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Capitalization {
    /// As typed.
    #[default]
    None,
    /// ALL CAPS.
    AllCaps,
    /// Small capitals.
    SmallCaps,
    /// Title Case (minor words stay lowercase).
    TitleCase,
    /// Start Case (every word capitalised).
    StartCase,
    /// Unknown value (rendered as typed).
    #[serde(other)]
    Unknown,
}

/// Paragraph formatting. Every field optional: `None` inherits.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ParagraphStyle {
    /// Horizontal alignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub alignment: Option<TextAlign>,
    /// Line height as a multiple of the font's natural line height (1 = single).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub line_height_multiple: Option<f64>,
    /// Extra space between lines, px.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub line_spacing: Option<f64>,
    /// Space before the paragraph, px.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub space_before: Option<f64>,
    /// Space after the paragraph, px.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub space_after: Option<f64>,
    /// Left (leading) indent of all lines, px.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub indent_start: Option<f64>,
    /// Right (trailing) indent, px.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub indent_end: Option<f64>,
    /// First-line indent relative to `indentStart`, px (negative = hanging).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub indent_first_line: Option<f64>,
    /// Bullet / numbering. A marker with `kind: none` ends an inherited list.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub list: Option<ListMarker>,
}

impl ParagraphStyle {
    /// Engine fallback: centred, single spacing, no indents, no list.
    pub fn fallback() -> ParagraphStyle {
        ParagraphStyle {
            alignment: Some(TextAlign::Center),
            line_height_multiple: Some(1.0),
            line_spacing: Some(0.0),
            space_before: Some(0.0),
            space_after: Some(0.0),
            indent_start: Some(0.0),
            indent_end: Some(0.0),
            indent_first_line: Some(0.0),
            list: None,
        }
    }

    /// Field-wise cascade: values set in `self` win.
    pub fn merged(&self, base: &ParagraphStyle) -> ParagraphStyle {
        ParagraphStyle {
            alignment: self.alignment.or(base.alignment),
            line_height_multiple: self.line_height_multiple.or(base.line_height_multiple),
            line_spacing: self.line_spacing.or(base.line_spacing),
            space_before: self.space_before.or(base.space_before),
            space_after: self.space_after.or(base.space_after),
            indent_start: self.indent_start.or(base.indent_start),
            indent_end: self.indent_end.or(base.indent_end),
            indent_first_line: self.indent_first_line.or(base.indent_first_line),
            list: self.list.clone().or_else(|| base.list.clone()),
        }
    }
}

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    /// Left.
    Left,
    /// Centre.
    #[default]
    Center,
    /// Right.
    Right,
    /// Justified.
    Justified,
    /// Natural for the script direction (left for LTR, right for RTL).
    Natural,
    /// Unknown value (rendered centred).
    #[serde(other)]
    Unknown,
}

/// Bullet / numbering of a list paragraph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ListMarker {
    /// Marker kind.
    pub kind: ListKind,
    /// Nesting level, 0-based.
    #[serde(default, skip_serializing_if = "is_default")]
    pub level: u8,
}

/// List marker kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ListKind {
    /// Not a list paragraph (ends an inherited list).
    None,
    /// • filled disc.
    #[default]
    Disc,
    /// ◦ circle.
    Circle,
    /// ▪ square.
    Square,
    /// – hyphen.
    Hyphen,
    /// 1, 2, 3.
    Decimal,
    /// a, b, c.
    LowerAlpha,
    /// A, B, C.
    UpperAlpha,
    /// i, ii, iii.
    LowerRoman,
    /// I, II, III.
    UpperRoman,
    /// Unknown value (rendered as a disc).
    #[serde(other)]
    Unknown,
}

/// Vertical placement of the text block inside its box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum VerticalAlign {
    /// Top.
    Top,
    /// Middle (the common lyric setting).
    #[default]
    Middle,
    /// Bottom.
    Bottom,
    /// Unknown value (rendered middle).
    #[serde(other)]
    Unknown,
}

/// Overflow handling (ProPresenter `scaleBehavior`). Authored sizes are kept; scaling is
/// computed at render time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum FitMode {
    /// Draw at the authored size; may overflow (`SCALE_BEHAVIOR_NONE`).
    #[default]
    None,
    /// Shrink until it fits, never grow (`SCALE_FONT_DOWN`, the usual lyric setting).
    ShrinkToFit,
    /// Shrink or grow to exactly fill the box (`SCALE_FONT_DOWN_UP`).
    ScaleToFit,
    /// Grow to fill the box, never shrink (`SCALE_FONT_UP`).
    GrowToFit,
    /// Keep the font size; resize the box height to the text (`ADJUST_CONTAINER_HEIGHT`).
    ResizeBox,
    /// Unknown value (treated as none).
    #[serde(other)]
    Unknown,
}

/// Glyph outline (stroke), box level.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct TextOutline {
    /// Outline is drawn.
    #[serde(skip_serializing_if = "crate::common::is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// Width in canvas px, centred on the glyph edge, drawn under the fill. ProPresenter's
    /// `strokeWidth` (RTF `\strokewidth`/20) is imported 1:1.
    #[serde(skip_serializing_if = "is_default_outline_width")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub width: f64,
    /// Outline colour.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub color: Color,
}

impl Default for TextOutline {
    /// Disabled, 3 px black (the census house style, so enabling restores it).
    fn default() -> Self {
        TextOutline {
            enabled: false,
            width: default_outline_width(),
            color: Color::BLACK,
        }
    }
}

/// Boxes drawn behind each rendered text line ("box behind each lyric line").
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct LineMask {
    /// Mask is drawn.
    #[serde(skip_serializing_if = "crate::common::is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub enabled: bool,
    /// Width rule of each box.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub style: LineMaskStyle,
    /// Extra width added to each box, px (split evenly left / right).
    #[serde(skip_serializing_if = "crate::common::is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub width_offset: f64,
    /// Extra height added to each box, px (split evenly top / bottom); may be negative.
    #[serde(skip_serializing_if = "crate::common::is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub height_offset: f64,
    /// Horizontal shift of each box, px; may be negative.
    #[serde(skip_serializing_if = "crate::common::is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub horizontal_offset: f64,
    /// Vertical shift of each box, px; may be negative.
    #[serde(skip_serializing_if = "crate::common::is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub vertical_offset: f64,
    /// Box colour.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub fill: Color,
}

/// Width rule for line-mask boxes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum LineMaskStyle {
    /// Each box spans the full text-box width (ProPresenter default).
    #[default]
    FullWidth,
    /// Each box hugs its own line.
    LineWidth,
    /// Every box is as wide as the widest line.
    MaxLineWidth,
    /// Unknown value (full width).
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italic_run_does_not_pin_box_face() {
        let box_style = CharStyle {
            font_family: Some("Calibri".into()),
            font_weight: Some(700),
            postscript_name: Some("Calibri-Bold".into()),
            ..CharStyle::default()
        };
        let run = CharStyle {
            italic: Some(true),
            ..CharStyle::default()
        };
        let eff = run.merged(&box_style);
        assert_eq!(eff.font_family.as_deref(), Some("Calibri"));
        assert_eq!(eff.font_weight, Some(700));
        assert_eq!(eff.italic, Some(true));
        assert_eq!(
            eff.postscript_name, None,
            "inherited face must not pin an italic run"
        );

        let size_only = CharStyle {
            size: Some(80.0),
            ..CharStyle::default()
        };
        assert_eq!(
            size_only.merged(&box_style).postscript_name.as_deref(),
            Some("Calibri-Bold")
        );
    }

    #[test]
    fn empty_paragraph_uses_end_style() {
        let tb = TextBox {
            default_style: CharStyle {
                size: Some(100.0),
                ..CharStyle::default()
            },
            ..TextBox::default()
        };
        let p = Paragraph {
            end_style: CharStyle {
                size: Some(40.0),
                ..CharStyle::default()
            },
            ..Paragraph::default()
        };
        assert_eq!(tb.empty_paragraph_style(&p).size, Some(40.0));
        assert_eq!(
            tb.empty_paragraph_style(&Paragraph::default()).size,
            Some(100.0)
        );
        assert_eq!(
            tb.run_style(&TextRun::plain("x")).font_family.as_deref(),
            Some("Arial")
        );
    }
}
