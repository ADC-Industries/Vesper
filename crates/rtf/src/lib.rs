//! A small, panic-free parser for the Cocoa-style RTF that ProPresenter
//! stores in the `rtfData` field of its text elements.
//!
//! The output is a deliberately small rich-text tree:
//! [`Document`] → [`Paragraph`]s → [`Run`]s, where each run is a string with
//! one fully resolved [`CharStyle`]. Font indices are resolved to PostScript
//! names through `\fonttbl`, and colour indices are resolved to RGBA through
//! `\colortbl` plus the alpha channel of `\*\expandedcolortbl`.
//!
//! ```
//! let doc = rtf::parse(br"{\rtf1\ansi{\fonttbl\f0\fnil Helvetica;}\f0\fs48 Hello \b world\par}");
//! assert_eq!(doc.paragraphs.len(), 1);
//! assert_eq!(doc.paragraphs[0].runs[1].text, "world");
//! assert!(doc.paragraphs[0].runs[1].style.bold);
//! assert_eq!(doc.paragraphs[0].runs[1].style.size_pt, 24.0);
//! ```
//!
//! # What is supported
//!
//! * Groups with a full formatting state stack; `\plain` and `\pard` resets.
//! * `\ansicpg1252` text, raw high bytes and `\'hh` escapes (decoded as
//!   Windows-1252, the only code page ProPresenter writes).
//! * `\uN` with the `\ucN` fallback skip count (default 1, as in the
//!   specification), including negative `N` (`N + 65536`) and UTF-16
//!   surrogate pairs written as two `\u` words. A fallback unit is a byte,
//!   a `\'hh` escape, or a control word or symbol, except that `\par` (and
//!   other paragraph-structure words) and a following `\u` end the fallback
//!   early, for writers that omit it.
//! * Escaped `\\`, `\{`, `\}`, plus `\~`, `\-`, `\_` and the named
//!   punctuation words (`\emdash`, `\rquote`, …).
//! * Destinations: `\fonttbl`, `\colortbl` and `\*\expandedcolortbl` are
//!   read; `\listtext` is captured as [`Paragraph::list_marker`] instead of
//!   text; every other `\*` destination and the known non-text destinations
//!   (`\listtable`, `\listoverridetable`, `\info`, `\stylesheet`, `\pict`, …)
//!   are skipped.
//! * CR and LF bytes are ignored, and exactly one space after a control word
//!   is consumed as its delimiter.
//!
//! # Paragraph rules
//!
//! `\par` (or the equivalent `\` + newline) is the only paragraph break.
//! `\line` becomes `'\n'` and `\tab` becomes `'\t'` inside a run's text.
//!
//! A paragraph's [`Paragraph::style`] and [`Paragraph::end_style`] are the
//! paragraph and character state in force at the `\par` that ends it (or at
//! the end of the document for the last paragraph), which is how RTF and
//! Cocoa assign paragraph attributes.
//!
//! **Trailing `\par`:** whatever follows the last `\par` forms a final
//! paragraph, even when it is only formatting words with no text. Only when
//! nothing at all (apart from closing braces) follows the last `\par` is no
//! extra empty paragraph created. So `a\par b` and `a\par b\par` both give
//! two paragraphs, `a\par\par` gives `["a", ""]`, and `a\par\pard\fs24`
//! gives `["a", ""]` with the empty paragraph styled at 12pt.
//!
//! ProPresenter does write a trailing `\par` when a text box ends with an
//! empty line: the `\par` is followed by a full paragraph prologue
//! (`\pard…\fs…\cf…`) and no text, and ProPresenter counts that empty last
//! line in its own text ranges. Other Cocoa writers terminate every
//! paragraph with a bare `\par}`, which users mean as no extra line.
//!
//! A [`Document`] always has at least one paragraph: a body with no text
//! (ProPresenter's empty text boxes) gives one paragraph with no runs, whose
//! [`Paragraph::end_style`] still carries the box's font, size and colours.
//!
//! # Errors
//!
//! Parsing never fails and never panics. Structural problems (truncation,
//! unbalanced braces, a missing `{\rtf` header) and control words the parser
//! does not know are reported through [`Diagnostics`] by
//! [`parse_with_diagnostics`].

#![forbid(unsafe_code)]

mod codepage;
mod lexer;
mod parser;

use std::collections::BTreeMap;

pub use parser::{parse, parse_with_diagnostics};

/// A parsed RTF document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Document {
    /// The paragraphs in order. Never empty for parser output.
    pub paragraphs: Vec<Paragraph>,
}

impl Document {
    /// The document's text, with paragraphs joined by `'\n'`.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for (i, p) in self.paragraphs.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            for r in &p.runs {
                out.push_str(&r.text);
            }
        }
        out
    }

    /// True when no paragraph has any text.
    pub fn is_empty(&self) -> bool {
        self.paragraphs.iter().all(|p| p.runs.is_empty())
    }
}

/// One paragraph: the text between two `\par`s.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Paragraph {
    /// Paragraph formatting in force at the end of the paragraph.
    pub style: ParagraphStyle,
    /// The text runs. Adjacent runs always differ in style, and no run is
    /// empty. An empty paragraph has no runs.
    pub runs: Vec<Run>,
    /// The character style in force at the paragraph mark. This is the
    /// style of an empty paragraph, and the style to type with at its end.
    pub end_style: CharStyle,
    /// The rendered list marker from a `\listtext` group, such as `"\t•\t"`,
    /// kept apart from the text.
    pub list_marker: Option<String>,
}

impl Paragraph {
    /// The paragraph's text, without its list marker.
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

/// A string with one character style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Run {
    /// The text. It contains `'\n'` for `\line` and `'\t'` for `\tab`, but
    /// never a paragraph break.
    pub text: String,
    /// The fully resolved character style.
    pub style: CharStyle,
}

/// Horizontal paragraph alignment (`\ql`, `\qc`, `\qr`, `\qj`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Alignment {
    /// No alignment word given: Cocoa's "natural" (left for left-to-right text).
    #[default]
    Natural,
    Left,
    Center,
    Right,
    Justified,
}

/// Line spacing from `\slN` and `\slmultM`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LineSpacing {
    /// `\sl0` or no `\sl`: single spacing.
    #[default]
    Auto,
    /// `\slN\slmult1` with `N > 0`: a multiple of single spacing, `N / 240`.
    /// `\sl240\slmult1` is 1.0 and `\sl216\slmult1` is 0.9.
    Multiple(f32),
    /// `\slN\slmult0` with `N > 0`: at least `N / 20` points. Cocoa's
    /// `\sl20\slmult0` (at least 1pt) renders as single spacing.
    AtLeast(f32),
    /// `\slN` with `N < 0`: exactly `-N / 20` points.
    Exactly(f32),
}

/// A reference to a list from `\lsN` and `\ilvlN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListRef {
    /// The `\ls` list-override index (1-based in the `\listoverridetable`).
    pub override_index: i32,
    /// The `\ilvl` nesting level, 0-based.
    pub level: i32,
}

/// Paragraph formatting. Lengths are in points (RTF twips / 20).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParagraphStyle {
    pub alignment: Alignment,
    pub line_spacing: LineSpacing,
    /// Cocoa's `\slleadingN` extra leading between lines, in points.
    pub line_leading_pt: f32,
    /// `\sb`
    pub space_before_pt: f32,
    /// `\sa`
    pub space_after_pt: f32,
    /// `\li`: left (head) indent of every line.
    pub left_indent_pt: f32,
    /// `\fi`: first-line indent, relative to `left_indent_pt` (often negative
    /// for hanging list bullets).
    pub first_line_indent_pt: f32,
    /// `\ri`
    pub right_indent_pt: f32,
    /// `\tx` tab stop positions, at most 64 (later ones are dropped).
    pub tab_stops_pt: Vec<f32>,
    /// `\ls` / `\ilvl` list membership.
    pub list: Option<ListRef>,
}

/// Vertical text position (`\super`, `\sub`, `\nosupersub`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Script {
    #[default]
    Normal,
    Superscript,
    Subscript,
}

/// An RGBA colour with components in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// An opaque colour from 8-bit components.
    pub fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: 1.0,
        }
    }
}

/// Character formatting, fully resolved.
///
/// Colours are `None` when the RTF selects the "auto" entry (index 0 of a
/// Cocoa colour table, written as an empty `;`) or an index that is not in
/// the table. For the text colour, "auto" means the renderer's default.
#[derive(Debug, Clone, PartialEq)]
pub struct CharStyle {
    /// The font's name from `\fonttbl`. ProPresenter writes PostScript names
    /// such as `Calibri-Bold` or `ArialMT`.
    pub font: Option<String>,
    /// `\fsN` / 2. The RTF default is 12pt.
    pub size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    /// Any `\ul…` underline style.
    pub underline: bool,
    /// `\ulc`
    pub underline_color: Option<Color>,
    /// `\strike` or `\striked`.
    pub strikethrough: bool,
    /// `\caps`. ProPresenter does not write it (its ALL_CAPS setting lives
    /// outside the RTF), but other writers do.
    pub caps: bool,
    /// `\cf` foreground colour.
    pub color: Option<Color>,
    /// `\strokewidthN` / 20. In Cocoa this is `NSStrokeWidthAttributeName`
    /// (percent of font size, negative means fill and stroke). It is the
    /// same number as ProPresenter's `strokeWidth` attribute (60 → 3.0).
    pub stroke_width: f32,
    /// `\strokec`
    pub stroke_color: Option<Color>,
    /// Extra spacing between characters in points, from `\expndtwN` (twips)
    /// or `\expndN` (quarter points), whichever came last. This is Cocoa's
    /// `NSKernAttributeName` and ProPresenter's `kerning` attribute.
    pub kerning_pt: f32,
    /// `\super` / `\sub` / `\nosupersub`.
    pub script: Script,
    /// `\upN` / `\dnN` baseline shift in points (positive is up).
    pub baseline_offset_pt: f32,
    /// `\highlight`
    pub highlight: Option<Color>,
    /// `\cb` character background.
    pub background: Option<Color>,
}

impl Default for CharStyle {
    fn default() -> Self {
        Self {
            font: None,
            size_pt: 12.0,
            bold: false,
            italic: false,
            underline: false,
            underline_color: None,
            strikethrough: false,
            caps: false,
            color: None,
            stroke_width: 0.0,
            stroke_color: None,
            kerning_pt: 0.0,
            script: Script::Normal,
            baseline_offset_pt: 0.0,
            highlight: None,
            background: None,
        }
    }
}

/// A structural problem found while parsing. The document is still returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Warning {
    /// The input does not start with `{\rtf`.
    MissingRtfHeader,
    /// The input ended with groups still open (truncated input).
    UnexpectedEof { open_groups: usize },
    /// A `}` with no open group.
    UnmatchedGroupEnd,
    /// Non-whitespace after the document's closing `}`.
    TrailingContent,
    /// `\ansicpgN` other than 1252, or `\mac`/`\pc`/`\pca`. Text is still
    /// decoded as Windows-1252.
    UnsupportedCodePage(i32),
    /// `\'` not followed by two hex digits.
    MalformedHexEscape,
    /// A `\u` UTF-16 surrogate without its partner (decoded as U+FFFD).
    LoneSurrogate,
}

/// Everything noteworthy the parser saw, for diagnostics and corpus checks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Diagnostics {
    /// Each kind of warning at most once, in the order first seen.
    pub warnings: Vec<Warning>,
    /// Control words outside skipped destinations that the parser does not
    /// recognise, with counts.
    pub unknown_words: BTreeMap<String, usize>,
    /// Control words that the parser recognises but deliberately ignores
    /// (page size, Cocoa housekeeping, list-table internals …), with counts.
    pub ignored_words: BTreeMap<String, usize>,
    /// Unrecognised control symbols (`\` + non-letter), with counts.
    pub unknown_symbols: BTreeMap<char, usize>,
}

impl Diagnostics {
    /// True when there were no structural warnings.
    pub fn is_clean(&self) -> bool {
        self.warnings.is_empty()
    }

    pub(crate) fn warn(&mut self, w: Warning) {
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }
}

#[cfg(test)]
mod tests;
