//! **Area: slides.** Text boxes and notes: RTF (`crates/rtf`) + `Graphics.Text.attributes` →
//! [`TextBox`] / [`RichText`].
//!
//! ## Owns
//! - [`text_box`]: `rtfData` parsed with `rtf::parse_with_diagnostics` → paragraphs / runs.
//!   - Box `defaultStyle` from `attributes` ([`char_style_from_attributes`]: font → family +
//!     weight + italic + postscriptName via `fonts::font_ref`; size; `textSolidFill`;
//!     capitalization; kerning; underline / strikethrough; superscript), so empty boxes still
//!     render. `defaultParagraph` from `attributes.paragraphStyle`.
//!   - Runs store only fields that differ from the box default ([`sparse_overrides`]); colours are
//!     compared as resolved values (within one 8-bit step, since RTF colour tables are 8-bit),
//!     never colour-table indices. Adjacent runs whose overrides end up equal are merged.
//!   - `customAttributes[]` UTF-16 ranges (over the whole text, one unit per paragraph break,
//!     exclusive end): capitalization splits runs and sets run overrides (a character outside every
//!     range keeps the box default); `fontScaleFactor` → box `fontScaleFactor`; `originalFont` →
//!     element `import.unmapped.originalFont` (returned through [`TextImport::unmapped`]);
//!     `originalFontSize` is dropped when it equals the RTF size (it always does in the census),
//!     else kept as `originalFontSize`; other custom attributes → `customAttributes`.
//!   - Outline: RTF `\strokewidth` / `\strokec` and `attributes.strokeWidth` / `strokeColor` →
//!     box [`TextOutline`] ([`text_outline`]); a per-run deviation is unmapped (`runOutline`).
//!     A zero width is "off" with the model's default settings (ProPresenter keeps a meaningless
//!     white `strokeColor` there).
//!   - `verticalAlignment`, `scaleBehavior` → `fit`, `margins`, text `shadow` (polar), fill
//!     (gradient / media / cut-out / background effect → box `fill`).
//!   - Every font use is recorded with `fonts::record_font_use` (once per box default, once per
//!     RTF run).
//!   - `customAttributes[].chord` → the paragraph's `chords` (the range start is the character
//!     the chord sits over); `chordPro.enabled` → box `chords` display (shown, its colour).
//!   - Unmodelled `Text` fields are kept when not default: `transform`, `chordPro` (enabled, a
//!     notation other than chord letters),
//!     `alternateTexts`, `ligatureStyle`, underline patterns, tab stops / min / max line height.
//! - [`line_mask`]: `Graphics.Element.mask.textLineMask` → [`LineMask`] (set on the text box by
//!   `element.rs`).
//! - [`notes`] / [`rich_text_from_rtf`]: slide and presentation notes → [`RichText`]. Notes keep
//!   inline formatting only (weight, italic, underline, baseline) relative to the notes'
//!   `attributes` (or the dominant run style), and no trailing empty paragraphs.
//!
//! ## Text fidelity
//! The concatenated run text of every paragraph equals the RTF paragraph text, with one
//! exception: an RTF `\line` (`'\n'` inside a run) becomes U+2028, as the model reserves `'\n'`
//! for paragraph breaks. `\listtext` bullets are not text (the list marker is a paragraph style).
//!
//! ## May call
//! `common::*`, `fonts::*`, `element::shadow`, `element::gradient`, `element::background_effect`,
//! `media::media_fill`, `ImportContext` helpers.
//!
//! ## Units
//! RTF sizes are points (`\fs` half-points already halved by the rtf crate) = canvas px; kerning
//! `\expndtw` twentieths → px.

use model::{
    Baseline, Capitalization, CharStyle, Color, FillKind, FitMode, FontRef, LineMask,
    LineMaskStyle, LineStyle, ListKind, ListMarker, Paragraph, ParagraphStyle, RichText, TextAlign,
    TextBox, TextOutline, TextRun, VerticalAlign,
};
use serde_json::{Value, json};

use crate::context::{FontKey, ImportContext};
use crate::convert::{common, element, fonts, media};
use crate::pb::rv::data as pp;
use crate::report::Category;

use pp::graphics::text::attributes::{CustomAttribute, custom_attribute::Attribute};

/// A converted text box plus element-level data the element converter must keep.
#[derive(Clone, Debug, Default)]
pub struct TextImport {
    /// The text box.
    pub text: TextBox,
    /// Source data for the element's `import.unmapped`, as (feature, JSON) pairs
    /// (`("text.originalFont", …)`, `("text.chordPro", …)`). The element converter passes each to
    /// `ctx.keep_unmapped`.
    pub unmapped: Vec<(String, Value)>,
}

/// Converts a `Graphics.Text` (see the module docs).
pub fn text_box(ctx: &mut ImportContext, t: &pp::graphics::Text) -> TextImport {
    let mut unmapped: Vec<(String, Value)> = Vec::new();
    let a = t.attributes.as_ref();
    let default_style = a
        .map(|a| char_style_from_attributes(ctx, a))
        .unwrap_or_default();
    let default_paragraph = a
        .and_then(|a| a.paragraph_style.as_ref())
        .map(paragraph_style_from_attributes)
        .unwrap_or_default();
    let doc = parse_rtf(ctx, &t.rtf_data);
    let custom: &[CustomAttribute] = a.map(|a| a.custom_attributes.as_slice()).unwrap_or(&[]);
    let hint = a
        .and_then(|a| a.font.as_ref())
        .filter(|f| !f.name.is_empty())
        .map(|f| (f.name.clone(), fonts::font_ref(f)));
    let mut built = build_paragraphs(
        ctx,
        &doc,
        &Base {
            style: &default_style,
            paragraph: &default_paragraph,
            box_list: default_paragraph.list.clone(),
        },
        custom,
        hint.as_ref(),
        true,
    );

    let mut tb = TextBox {
        paragraphs: std::mem::take(&mut built.paragraphs),
        default_style,
        default_paragraph,
        vertical_align: vertical_align(t.vertical_alignment),
        fit: fit_mode(t.scale_behavior),
        font_scale_factor: 1.0,
        margins: common::edge_insets(t.margins.as_ref()),
        text_shadow: element::shadow(t.shadow.as_ref()),
        outline: text_outline(ctx, a, &doc),
        line_mask: LineMask::default(),
        fill: Default::default(),
        chords: Default::default(),
    };

    // Run outlines that deviate from the box outline.
    let deviations: Vec<Value> = doc
        .paragraphs
        .iter()
        .enumerate()
        .flat_map(|(pi, p)| p.runs.iter().map(move |r| (pi, r)))
        .filter(|(_, r)| {
            let w = f64::from(r.style.stroke_width.abs());
            if tb.outline.enabled {
                (w - tb.outline.width).abs() > 1e-3
                    || r.style
                        .stroke_color
                        .is_some_and(|c| !colors_close(rtf_color(&c), tb.outline.color))
            } else {
                w > 1e-3
            }
        })
        .map(|(pi, r)| {
            json!({
                "paragraph": pi,
                "text": r.text,
                "strokeWidth": r.style.stroke_width,
                "strokeColor": r.style.stroke_color.map(|c| json!([c.r, c.g, c.b, c.a])),
            })
        })
        .collect();
    if !deviations.is_empty() {
        unmapped.push(("text.runOutline".into(), Value::Array(deviations)));
    }

    // Custom attributes other than capitalization.
    let mut original_fonts = Vec::new();
    let mut other_custom = Vec::new();
    let mut original_sizes = Vec::new();
    let mut scale: Option<f64> = None;
    for c in custom {
        match &c.attribute {
            Some(Attribute::Capitalization(_)) | Some(Attribute::Chord(_)) | None => {}
            Some(Attribute::OriginalFont(_)) => original_fonts.push(ctx.to_json(c)),
            Some(Attribute::OriginalFontSize(s)) => {
                let start = c.range.as_ref().map_or(0, |r| r.start.max(0) as usize);
                let rtf_size = built.size_at(start).or(tb.default_style.size);
                if rtf_size.is_none_or(|r| (r - s).abs() > 1e-3) {
                    original_sizes.push(ctx.to_json(c));
                }
            }
            Some(Attribute::FontScaleFactor(f)) => match scale {
                None => scale = Some(*f),
                Some(prev) if (prev - f).abs() < 1e-9 => {}
                Some(_) => other_custom.push(ctx.to_json(c)),
            },
            Some(_) => other_custom.push(ctx.to_json(c)),
        }
    }
    if let Some(f) = scale.filter(|f| *f > 0.0) {
        tb.font_scale_factor = f;
    }
    if !original_fonts.is_empty() {
        unmapped.push(("text.originalFont".into(), Value::Array(original_fonts)));
    }
    if !original_sizes.is_empty() {
        unmapped.push(("text.originalFontSize".into(), Value::Array(original_sizes)));
    }
    if !other_custom.is_empty() {
        unmapped.push(("text.customAttributes".into(), Value::Array(other_custom)));
    }

    // Non-solid glyph fills.
    if let Some(fill) = a.and_then(|a| a.fill.as_ref()) {
        use pp::graphics::text::attributes::Fill as F;
        let kind = match fill {
            F::TextSolidFill(_) => None,
            F::TextGradientFill(g) => {
                if g.stretch_to_document_bounds {
                    unmapped.push(("text.stretchToDocumentBounds".into(), Value::Bool(true)));
                }
                Some(FillKind::Gradient {
                    gradient: g
                        .gradient
                        .as_ref()
                        .map(element::gradient)
                        .unwrap_or_default(),
                })
            }
            F::CutOutFill(_) => Some(FillKind::CutOut),
            F::MediaFill(m) => Some(match m.media.as_ref() {
                Some(m) => {
                    let scope = ctx.current.asset_scope.clone();
                    media::media_fill(ctx, m, &scope)
                }
                None => FillKind::Unsupported {
                    source_type: "propresenter.text.mediaFill".into(),
                    payload: ctx.to_json(m),
                },
            }),
            F::BackgroundEffect(e) => Some(match element::background_effect(e) {
                Some(effect) => FillKind::BackgroundEffect { effect },
                None => FillKind::Unsupported {
                    source_type: "propresenter.text.backgroundEffect".into(),
                    payload: ctx.to_json(e),
                },
            }),
        };
        if let Some(kind) = kind {
            tb.fill = model::Fill::enabled(kind);
        }
    }

    // Unmodelled Text / Attributes fields.
    if t.transform != 0 {
        unmapped.push((
            "text.transform".into(),
            json!({
                "transform": pp::graphics::text::Transform::try_from(t.transform)
                    .map(|v| v.as_str_name().to_string())
                    .unwrap_or_else(|_| t.transform.to_string()),
                "delimiter": t.transform_delimiter,
            }),
        ));
    }
    if let Some(c) = t.chord_pro.as_ref().filter(|c| c.enabled) {
        // Chords shown on the box: its chord display, in its colour. Other notations than
        // chord letters are kept as unmapped.
        tb.chords = model::ChordDisplay {
            show: true,
            color: c.color.as_ref().map(common::color),
            size: None,
        };
        if c.notation != 0 {
            unmapped.push(("text.chordPro".into(), ctx.to_json(c)));
        }
    }
    if !t.alternate_texts.is_empty() {
        let v = t.alternate_texts.iter().map(|x| ctx.to_json(x)).collect();
        unmapped.push(("text.alternateTexts".into(), Value::Array(v)));
    }
    if let Some(a) = a {
        if a.ligature_style != 0 {
            unmapped.push(("text.ligatureStyle".into(), json!(a.ligature_style)));
        }
        for (name, u) in [
            ("text.underlineStyle", a.underline_style.as_ref()),
            ("text.strikethroughStyle", a.strikethrough_style.as_ref()),
        ] {
            if let Some(u) = u.filter(|u| u.pattern != 0 || u.by_word) {
                unmapped.push((name.into(), ctx.to_json(u)));
            }
        }
        if let Some(p) = a.paragraph_style.as_ref().filter(|p| {
            !p.tab_stops.is_empty()
                || p.maximum_line_height != 0.0
                || p.minimum_line_height != 0.0
                || p.text_lists.len() > 1
        }) {
            unmapped.push(("text.paragraphStyle".into(), ctx.to_json(p)));
        }
    }

    TextImport { text: tb, unmapped }
}

/// Parses RTF bytes (empty → empty document) and reports structural problems.
fn parse_rtf(ctx: &mut ImportContext, bytes: &[u8]) -> rtf::Document {
    if bytes.is_empty() {
        return rtf::Document::default();
    }
    let (doc, diag) = rtf::parse_with_diagnostics(bytes);
    if !diag.is_clean() {
        ctx.warn(
            Category::Other,
            format!("RTF text parsed with warnings: {:?}", diag.warnings),
        );
    }
    doc
}

/// Box default character style from `attributes` (complete enough to render an empty box).
pub fn char_style_from_attributes(
    ctx: &mut ImportContext,
    a: &pp::graphics::text::Attributes,
) -> CharStyle {
    attr_char_style(ctx, a, true)
}

/// [`char_style_from_attributes`] with optional font-use recording.
fn attr_char_style(
    ctx: &mut ImportContext,
    a: &pp::graphics::text::Attributes,
    record: bool,
) -> CharStyle {
    let mut s = CharStyle::default();
    if let Some(f) = a.font.as_ref() {
        if !f.name.is_empty() || !f.family.is_empty() {
            let r = fonts::font_ref(f);
            if !r.family.is_empty() {
                s.font_family = Some(r.family);
            }
            s.font_weight = Some(r.weight);
            s.italic = Some(r.italic);
            s.postscript_name = r.postscript_name;
            if record {
                fonts::record_font_use(
                    ctx,
                    FontKey {
                        postscript_name: f.name.clone(),
                        family: f.family.clone(),
                        face: f.face.clone(),
                    },
                );
            }
        }
        if f.size > 0.0 {
            s.size = Some(f.size);
        }
    }
    if let Some(pp::graphics::text::attributes::Fill::TextSolidFill(c)) = a.fill.as_ref() {
        s.color = Some(common::color(c));
    }
    s.capitalization = capitalization(a.capitalization);
    s.underline = a
        .underline_style
        .as_ref()
        .map(|u| line_style(u.style))
        .filter(|l| *l != LineStyle::None);
    s.strikethrough = a
        .strikethrough_style
        .as_ref()
        .map(|u| line_style(u.style))
        .filter(|l| *l != LineStyle::None);
    s.baseline = match a.superscript {
        0 => None,
        n if n > 0 => Some(Baseline::Superscript),
        _ => Some(Baseline::Subscript),
    };
    if a.kerning != 0.0 {
        s.letter_spacing = Some(a.kerning);
    }
    s.highlight = a
        .background_color
        .as_ref()
        .filter(|c| c.alpha > 0.0)
        .map(common::color);
    s
}

/// `Underline.Style` → [`LineStyle`].
fn line_style(v: i32) -> LineStyle {
    match v {
        0 => LineStyle::None,
        1 => LineStyle::Single,
        2 => LineStyle::Thick,
        3 => LineStyle::Double,
        _ => LineStyle::Unknown,
    }
}

/// `Attributes.Alignment` → [`TextAlign`].
fn text_align(v: i32) -> TextAlign {
    match v {
        0 => TextAlign::Left,
        1 => TextAlign::Right,
        2 => TextAlign::Center,
        3 => TextAlign::Justified,
        4 => TextAlign::Natural,
        _ => TextAlign::Unknown,
    }
}

/// `TextList.NumberType` → [`ListKind`].
fn list_kind(v: i32) -> ListKind {
    match v {
        0 | 6 => ListKind::Square,  // box, square
        1 | 3 => ListKind::Unknown, // check, diamond
        2 => ListKind::Circle,
        4 => ListKind::Disc,
        5 => ListKind::Hyphen,
        7 => ListKind::Decimal,
        8 => ListKind::LowerAlpha,
        9 => ListKind::UpperAlpha,
        10 => ListKind::LowerRoman,
        11 => ListKind::UpperRoman,
        _ => ListKind::Unknown,
    }
}

/// List kind guessed from a rendered `\listtext` marker (`"\t•\t"`, `"1."`).
fn list_kind_from_marker(marker: &str) -> ListKind {
    let m = marker.trim();
    match m.chars().next() {
        Some('•' | '●') => ListKind::Disc,
        Some('◦' | '○') => ListKind::Circle,
        Some('▪' | '■' | '◼') => ListKind::Square,
        Some('-' | '–' | '—' | '⁃') => ListKind::Hyphen,
        Some(c) if c.is_ascii_digit() => ListKind::Decimal,
        Some(c) if c.is_ascii_lowercase() => ListKind::LowerAlpha,
        Some(c) if c.is_ascii_uppercase() => ListKind::UpperAlpha,
        _ => ListKind::Disc,
    }
}

/// Box default paragraph style from `attributes.paragraphStyle`.
pub fn paragraph_style_from_attributes(
    p: &pp::graphics::text::attributes::Paragraph,
) -> ParagraphStyle {
    let nz = |v: f64| (v != 0.0).then_some(v);
    ParagraphStyle {
        alignment: Some(text_align(p.alignment)),
        line_height_multiple: (p.line_height_multiple > 0.0).then_some(p.line_height_multiple),
        line_spacing: nz(p.line_spacing),
        space_before: nz(p.paragraph_spacing_before),
        space_after: nz(p.paragraph_spacing),
        indent_start: nz(p.head_indent),
        indent_end: nz(p.tail_indent),
        indent_first_line: nz(p.first_line_head_indent),
        list: p
            .text_list
            .as_ref()
            .filter(|l| l.is_enabled)
            .map(|l| ListMarker {
                kind: list_kind(l.number_type),
                level: 0,
            }),
    }
}

/// Paragraphs from RTF bytes, with run styles made sparse against `base` and capitalization
/// ranges from `custom` applied.
pub fn paragraphs_from_rtf(
    ctx: &mut ImportContext,
    rtf: &[u8],
    base: &CharStyle,
    custom: &[pp::graphics::text::attributes::CustomAttribute],
) -> Vec<Paragraph> {
    let doc = parse_rtf(ctx, rtf);
    let para = ParagraphStyle::default();
    build_paragraphs(
        ctx,
        &doc,
        &Base {
            style: base,
            paragraph: &para,
            box_list: None,
        },
        custom,
        None,
        true,
    )
    .paragraphs
}

/// The box defaults runs and paragraphs are made sparse against.
struct Base<'a> {
    style: &'a CharStyle,
    paragraph: &'a ParagraphStyle,
    box_list: Option<ListMarker>,
}

/// Built paragraphs plus the resolved size of every run (for `originalFontSize` checks).
struct Built {
    paragraphs: Vec<Paragraph>,
    /// (UTF-16 start offset, end offset, resolved size) per source run.
    sizes: Vec<(usize, usize, Option<f64>)>,
}

impl Built {
    /// The size of the run covering `offset`, else of the run ending there (a range at the end
    /// of a paragraph or of the text).
    fn size_at(&self, offset: usize) -> Option<f64> {
        self.sizes
            .iter()
            .find(|(s, e, _)| *s <= offset && offset < *e)
            .or_else(|| self.sizes.iter().rev().find(|(_, e, _)| *e == offset))
            .and_then(|(_, _, size)| *size)
    }
}

/// A UTF-16 capitalization range.
struct CapRange {
    start: usize,
    end: usize,
    cap: Capitalization,
}

fn cap_ranges(custom: &[CustomAttribute]) -> Vec<CapRange> {
    custom
        .iter()
        .filter_map(|c| match (&c.attribute, &c.range) {
            (Some(Attribute::Capitalization(v)), Some(r)) => Some(CapRange {
                start: r.start.max(0) as usize,
                end: r.end.max(0) as usize,
                cap: capitalization(*v).unwrap_or(Capitalization::None),
            }),
            _ => None,
        })
        .collect()
}

/// Chord custom attributes: (UTF-16 offset of the character the chord sits over, chord).
fn chord_marks(custom: &[CustomAttribute]) -> Vec<(usize, String)> {
    let mut v: Vec<(usize, String)> = custom
        .iter()
        .filter_map(|c| match (&c.attribute, &c.range) {
            (Some(Attribute::Chord(ch)), Some(r)) if !ch.trim().is_empty() => {
                Some((r.start.max(0) as usize, ch.trim().to_string()))
            }
            _ => None,
        })
        .collect();
    v.sort_by_key(|(at, _)| *at);
    v
}

/// The chords of the paragraph spanning UTF-16 offsets `start..=end` (end = its paragraph
/// break), as character positions in its text.
fn paragraph_chords(
    chords: &[(usize, String)],
    start: usize,
    end: usize,
    runs: &[TextRun],
) -> Vec<model::ChordMark> {
    let mine: Vec<&(usize, String)> = chords
        .iter()
        .filter(|(at, _)| *at >= start && *at <= end)
        .collect();
    if mine.is_empty() {
        return Vec::new();
    }
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    // UTF-16 offset (from the paragraph start) → character index.
    let mut map = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.chars().enumerate() {
        for _ in 0..c.len_utf16() {
            map.push(i as u32);
        }
    }
    let len = text.chars().count() as u32;
    mine.into_iter()
        .map(|(at, ch)| model::ChordMark::new(map.get(at - start).copied().unwrap_or(len), ch))
        .collect()
}

/// Capitalization of the UTF-16 unit at `offset` (the last range covering it wins).
fn cap_at(ranges: &[CapRange], offset: usize) -> Option<Capitalization> {
    ranges
        .iter()
        .rev()
        .find(|r| r.start <= offset && offset < r.end)
        .map(|r| r.cap)
}

fn build_paragraphs(
    ctx: &mut ImportContext,
    doc: &rtf::Document,
    base: &Base<'_>,
    custom: &[CustomAttribute],
    hint: Option<&(String, FontRef)>,
    record_fonts: bool,
) -> Built {
    let ranges = cap_ranges(custom);
    let chords = chord_marks(custom);
    let mut offset = 0usize;
    let mut sizes = Vec::new();
    let mut paragraphs = Vec::with_capacity(doc.paragraphs.len());
    for (pi, p) in doc.paragraphs.iter().enumerate() {
        if pi > 0 {
            offset += 1; // the paragraph break
        }
        let para_start = offset;
        let mut runs: Vec<TextRun> = Vec::new();
        for r in &p.runs {
            let full = rtf_char_style(ctx, &r.style, hint, record_fonts);
            let start = offset;
            // Split on capitalization changes.
            let mut seg = String::new();
            let mut seg_cap: Option<Option<Capitalization>> = None;
            for c in r.text.chars() {
                let cap = cap_at(&ranges, offset).or(full.capitalization);
                if seg_cap.is_some_and(|sc| sc != cap) && !seg.is_empty() {
                    push_run(&mut runs, &seg, &full, seg_cap.flatten(), base.style);
                    seg.clear();
                }
                seg_cap = Some(cap);
                seg.push(if c == '\n' { '\u{2028}' } else { c });
                offset += c.len_utf16();
            }
            if !seg.is_empty() {
                push_run(&mut runs, &seg, &full, seg_cap.flatten(), base.style);
            }
            sizes.push((start, offset, full.size));
        }
        let mut end_full = rtf_char_style(ctx, &p.end_style, hint, false);
        end_full.capitalization = cap_at(&ranges, offset).or(end_full.capitalization);
        let end_sparse = sparse_overrides(&end_full, base.style);
        let end_style = match runs.last() {
            Some(last) if last.style == end_sparse => CharStyle::default(),
            _ => end_sparse,
        };
        let mut style = sparse_paragraph(&paragraph_style_from_rtf(&p.style), base.paragraph);
        style.list = match (&p.style.list, &base.box_list) {
            (Some(l), box_list) => {
                let kind = box_list.as_ref().map(|b| b.kind).unwrap_or_else(|| {
                    p.list_marker
                        .as_deref()
                        .map(list_kind_from_marker)
                        .unwrap_or(ListKind::Disc)
                });
                let m = ListMarker {
                    kind,
                    level: u8::try_from(l.level.clamp(0, 255)).unwrap_or(0),
                };
                (box_list.as_ref() != Some(&m)).then_some(m)
            }
            (None, Some(_)) => Some(ListMarker {
                kind: ListKind::None,
                level: 0,
            }),
            (None, None) => None,
        };
        let marks = paragraph_chords(&chords, para_start, offset, &runs);
        paragraphs.push(Paragraph {
            style,
            runs,
            end_style,
            chords: marks,
        });
    }
    Built { paragraphs, sizes }
}

/// Appends a run (sparse against `base`), merging with the previous run when the overrides are
/// equal.
fn push_run(
    runs: &mut Vec<TextRun>,
    text: &str,
    full: &CharStyle,
    cap: Option<Capitalization>,
    base: &CharStyle,
) {
    let mut full = full.clone();
    full.capitalization = cap;
    let style = sparse_overrides(&full, base);
    match runs.last_mut() {
        Some(last) if last.style == style => last.text.push_str(text),
        _ => runs.push(TextRun {
            text: text.to_string(),
            style,
        }),
    }
}

/// A fully resolved RTF character style → model [`CharStyle`] (all fields set that RTF knows).
pub fn char_style_from_rtf(ctx: &mut ImportContext, s: &rtf::CharStyle) -> CharStyle {
    rtf_char_style(ctx, s, None, true)
}

/// [`char_style_from_rtf`] with a font hint (the box font: its PostScript name and the family /
/// face split from `attributes`) and optional font-use recording.
fn rtf_char_style(
    ctx: &mut ImportContext,
    s: &rtf::CharStyle,
    hint: Option<&(String, FontRef)>,
    record: bool,
) -> CharStyle {
    let mut out = CharStyle::default();
    match s.font.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => {
            let fr = match hint {
                Some((ps, r)) if ps == name => r.clone(),
                _ => fonts::font_ref_from_postscript(name),
            };
            if !fr.family.is_empty() {
                out.font_family = Some(fr.family);
            }
            out.font_weight = Some(if s.bold {
                fr.weight.max(700)
            } else {
                fr.weight
            });
            out.italic = Some(fr.italic || s.italic);
            out.postscript_name = Some(name.to_string());
            if record {
                fonts::record_font_use(
                    ctx,
                    FontKey {
                        postscript_name: name.to_string(),
                        ..FontKey::default()
                    },
                );
            }
        }
        None => {
            out.font_weight = Some(if s.bold { 700 } else { 400 });
            out.italic = Some(s.italic);
        }
    }
    out.size = Some(round3(f64::from(s.size_pt)));
    out.color = s.color.as_ref().map(rtf_color);
    out.underline = Some(if s.underline {
        LineStyle::Single
    } else {
        LineStyle::None
    });
    out.strikethrough = Some(if s.strikethrough {
        LineStyle::Single
    } else {
        LineStyle::None
    });
    out.baseline = Some(match s.script {
        rtf::Script::Normal => Baseline::Normal,
        rtf::Script::Superscript => Baseline::Superscript,
        rtf::Script::Subscript => Baseline::Subscript,
    });
    out.capitalization = s.caps.then_some(Capitalization::AllCaps);
    out.letter_spacing = Some(round3(f64::from(s.kerning_pt)));
    out.highlight = s.highlight.or(s.background).as_ref().map(rtf_color);
    out
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

/// An RTF paragraph style → model [`ParagraphStyle`].
pub fn paragraph_style_from_rtf(p: &rtf::ParagraphStyle) -> ParagraphStyle {
    let pt = |v: f32| Some(round3(f64::from(v)));
    ParagraphStyle {
        alignment: Some(match p.alignment {
            rtf::Alignment::Natural => TextAlign::Natural,
            rtf::Alignment::Left => TextAlign::Left,
            rtf::Alignment::Center => TextAlign::Center,
            rtf::Alignment::Right => TextAlign::Right,
            rtf::Alignment::Justified => TextAlign::Justified,
        }),
        line_height_multiple: match p.line_spacing {
            rtf::LineSpacing::Auto => Some(1.0),
            rtf::LineSpacing::Multiple(m) => Some(round4(f64::from(m))),
            // Cocoa writes `\sl20\slmult0` (at least 1 pt) for single spacing.
            rtf::LineSpacing::AtLeast(v) if v <= 1.0 => Some(1.0),
            rtf::LineSpacing::AtLeast(_) | rtf::LineSpacing::Exactly(_) => None,
        },
        line_spacing: pt(p.line_leading_pt),
        space_before: pt(p.space_before_pt),
        space_after: pt(p.space_after_pt),
        indent_start: pt(p.left_indent_pt),
        indent_end: pt(p.right_indent_pt),
        indent_first_line: pt(p.first_line_indent_pt),
        list: None,
    }
}

/// Paragraph fields that differ from `base` (resolved against the engine fallback). `list` is
/// left `None` (set by the caller).
fn sparse_paragraph(p: &ParagraphStyle, base: &ParagraphStyle) -> ParagraphStyle {
    let b = base.merged(&ParagraphStyle::fallback());
    let f = |v: Option<f64>, bv: Option<f64>| match (v, bv) {
        (Some(v), Some(bv)) if (v - bv).abs() <= 1e-3 => None,
        (Some(v), _) => Some(v),
        _ => None,
    };
    let alignment = match (p.alignment, b.alignment) {
        (Some(a), Some(ba)) if a == ba => None,
        // Natural is left for the left-to-right text of the census.
        (Some(TextAlign::Natural), Some(TextAlign::Left)) => None,
        (a, _) => a,
    };
    ParagraphStyle {
        alignment,
        line_height_multiple: f(p.line_height_multiple, b.line_height_multiple),
        line_spacing: f(p.line_spacing, b.line_spacing),
        space_before: f(p.space_before, b.space_before),
        space_after: f(p.space_after, b.space_after),
        indent_start: f(p.indent_start, b.indent_start),
        indent_end: f(p.indent_end, b.indent_end),
        indent_first_line: f(p.indent_first_line, b.indent_first_line),
        list: None,
    }
}

/// True when two colours are equal within one 8-bit step per channel.
fn colors_close(a: Color, b: Color) -> bool {
    const EPS: f32 = 1.01 / 255.0;
    (a.r - b.r).abs() <= EPS
        && (a.g - b.g).abs() <= EPS
        && (a.b - b.b).abs() <= EPS
        && (a.a - b.a).abs() <= EPS
}

/// The fields of `run` that differ from `base` (everything else `None`).
///
/// `base` is resolved against [`CharStyle::fallback`] first, so a run equal to the engine
/// default stores nothing even when the box default leaves the field unset. Sizes and spacing
/// compare within 0.001 px, colours within one 8-bit step, and a missing or fully transparent
/// highlight equals "no highlight".
pub fn sparse_overrides(run: &CharStyle, base: &CharStyle) -> CharStyle {
    let b = base.merged(&CharStyle::fallback());
    fn diff<T: PartialEq + Clone>(v: &Option<T>, bv: &Option<T>) -> Option<T> {
        match v {
            Some(v) if bv.as_ref() != Some(v) => Some(v.clone()),
            _ => None,
        }
    }
    let num = |v: Option<f64>, bv: Option<f64>| match (v, bv) {
        (Some(v), Some(bv)) if (v - bv).abs() <= 1e-3 => None,
        (v, _) => v,
    };
    let color = |v: Option<Color>, bv: Option<Color>| match (v, bv) {
        (Some(v), Some(bv)) if colors_close(v, bv) => None,
        (v, _) => v,
    };
    let visible = |c: Option<Color>| c.filter(|c| c.a > 0.0);
    let highlight = match (visible(run.highlight), visible(b.highlight)) {
        (None, None) => None,
        (Some(v), Some(bv)) if colors_close(v, bv) => None,
        (None, Some(_)) => run.highlight, // explicit transparent highlight, if the run has one
        (Some(v), _) => Some(v),
    };
    CharStyle {
        font_family: diff(&run.font_family, &b.font_family),
        font_weight: diff(&run.font_weight, &b.font_weight),
        italic: diff(&run.italic, &b.italic),
        postscript_name: diff(&run.postscript_name, &b.postscript_name),
        size: num(run.size, b.size),
        color: color(run.color, b.color),
        underline: diff(&run.underline, &b.underline),
        strikethrough: diff(&run.strikethrough, &b.strikethrough),
        baseline: diff(&run.baseline, &b.baseline),
        capitalization: diff(&run.capitalization, &b.capitalization),
        letter_spacing: num(run.letter_spacing, b.letter_spacing),
        highlight,
    }
}

/// `Attributes.Capitalization` → model capitalization (`NONE` → `None`).
pub fn capitalization(value: i32) -> Option<Capitalization> {
    match value {
        0 => None,
        1 => Some(Capitalization::AllCaps),
        2 => Some(Capitalization::SmallCaps),
        3 => Some(Capitalization::TitleCase),
        4 => Some(Capitalization::StartCase),
        _ => Some(Capitalization::Unknown),
    }
}

/// `Text.VerticalAlignment` → [`VerticalAlign`].
pub fn vertical_align(value: i32) -> VerticalAlign {
    match value {
        0 => VerticalAlign::Top,
        1 => VerticalAlign::Middle,
        2 => VerticalAlign::Bottom,
        _ => VerticalAlign::Unknown,
    }
}

/// `Text.ScaleBehavior` → [`FitMode`].
pub fn fit_mode(value: i32) -> FitMode {
    match value {
        0 => FitMode::None,
        1 => FitMode::ResizeBox,
        2 => FitMode::ShrinkToFit,
        3 => FitMode::GrowToFit,
        4 => FitMode::ScaleToFit,
        _ => FitMode::Unknown,
    }
}

/// Box outline from `attributes.strokeWidth/strokeColor` and the RTF stroke of the runs.
///
/// `attributes` wins; the first RTF run is used when there are no attributes or they have no
/// stroke while the RTF does. ProPresenter's width is used 1:1 (absolute value: Cocoa's negative
/// "fill and stroke" sign carries no meaning here). Width 0 → the model's default (disabled).
pub fn text_outline(
    ctx: &mut ImportContext,
    a: Option<&pp::graphics::text::Attributes>,
    doc: &rtf::Document,
) -> TextOutline {
    let _ = ctx;
    let from_attrs = a.map(|a| {
        (
            a.stroke_width.abs(),
            a.stroke_color.as_ref().map(common::color),
        )
    });
    let from_rtf = doc
        .paragraphs
        .iter()
        .flat_map(|p| p.runs.first())
        .next()
        .map(|r| {
            (
                f64::from(r.style.stroke_width.abs()),
                r.style.stroke_color.as_ref().map(rtf_color),
            )
        });
    let (width, color) = match (from_attrs, from_rtf) {
        (Some((w, c)), _) if w > 0.0 => (w, c),
        (_, Some((w, c))) if w > 0.0 => (round3(w), c),
        _ => return TextOutline::default(),
    };
    TextOutline {
        enabled: true,
        width,
        color: color.unwrap_or(Color::BLACK),
    }
}

/// `Text.LineFillMask` → [`LineMask`].
///
/// ProPresenter draws the line boxes with the *element* fill; the element converter moves that
/// colour into [`LineMask::fill`]. Here `fill` stays at its default.
pub fn line_mask(m: &pp::graphics::text::LineFillMask) -> LineMask {
    LineMask {
        enabled: m.enabled,
        style: match m.mask_style {
            0 => LineMaskStyle::FullWidth,
            1 => LineMaskStyle::LineWidth,
            2 => LineMaskStyle::MaxLineWidth,
            _ => LineMaskStyle::Unknown,
        },
        width_offset: m.width_offset,
        height_offset: m.height_offset,
        horizontal_offset: m.horizontal_offset,
        vertical_offset: m.vertical_offset,
        fill: Color::default(),
    }
}

/// Slide notes (`PresentationSlide.notes`: RTF + attributes) → [`RichText`] (plain notes are
/// unstyled paragraphs; an empty RTF shell → empty).
pub fn notes(ctx: &mut ImportContext, n: &pp::presentation_slide::Notes) -> RichText {
    let doc = parse_rtf(ctx, &n.rtf_data);
    let base = match n.attributes.as_ref() {
        // Notes are not rendered: their fonts are not counted.
        Some(a) => attr_char_style(ctx, a, false),
        None => dominant_style(ctx, &doc),
    };
    rich_from_doc(ctx, &doc, &base)
}

/// RTF bytes → [`RichText`] (presentation notes are plain strings: use `RichText::plain`).
pub fn rich_text_from_rtf(ctx: &mut ImportContext, rtf: &[u8]) -> RichText {
    let doc = parse_rtf(ctx, rtf);
    let base = dominant_style(ctx, &doc);
    rich_from_doc(ctx, &doc, &base)
}

/// The style covering the most characters (ties: first seen).
fn dominant_style(ctx: &mut ImportContext, doc: &rtf::Document) -> CharStyle {
    let mut best: Vec<(CharStyle, usize)> = Vec::new();
    for r in doc.paragraphs.iter().flat_map(|p| &p.runs) {
        let s = rtf_char_style(ctx, &r.style, None, false);
        let n = r.text.chars().count();
        match best.iter_mut().find(|(b, _)| *b == s) {
            Some((_, c)) => *c += n,
            None => best.push((s, n)),
        }
    }
    let mut top: Option<(CharStyle, usize)> = None;
    for (s, n) in best {
        if top.as_ref().is_none_or(|(_, tn)| n > *tn) {
            top = Some((s, n));
        }
    }
    top.map(|(s, _)| s).unwrap_or_default()
}

/// Notes paragraphs: inline formatting only, no trailing empty paragraphs.
fn rich_from_doc(ctx: &mut ImportContext, doc: &rtf::Document, base: &CharStyle) -> RichText {
    if doc.is_empty() {
        return RichText::default();
    }
    let mut paragraphs: Vec<Paragraph> = Vec::with_capacity(doc.paragraphs.len());
    for p in &doc.paragraphs {
        let mut runs: Vec<TextRun> = Vec::new();
        for r in &p.runs {
            let full = rtf_char_style(ctx, &r.style, None, false);
            let style = sparse_overrides(&full, base).inline_formatting();
            let text: String = r
                .text
                .chars()
                .map(|c| if c == '\n' { '\u{2028}' } else { c })
                .collect();
            match runs.last_mut() {
                Some(last) if last.style == style => last.text.push_str(&text),
                _ => runs.push(TextRun { text, style }),
            }
        }
        paragraphs.push(Paragraph {
            runs,
            ..Paragraph::default()
        });
    }
    while paragraphs.last().is_some_and(|p| p.runs.is_empty()) {
        paragraphs.pop();
    }
    RichText { paragraphs }
}

/// An RTF colour → model colour.
pub fn rtf_color(c: &rtf::Color) -> model::Color {
    model::Color::rgba(c.r, c.g, c.b, c.a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pp::graphics::text::attributes as attrs;

    fn ctx() -> ImportContext {
        ImportContext::new(
            std::path::Path::new("/nonexistent"),
            crate::ImportOptions::default(),
        )
    }

    fn color(r: f32, g: f32, b: f32, a: f32) -> pp::Color {
        pp::Color {
            red: r,
            green: g,
            blue: b,
            alpha: a,
        }
    }

    /// The house lyric style: Calibri-Bold 130 white, 3 px black outline, centred.
    fn lyric_attributes() -> pp::graphics::text::Attributes {
        pp::graphics::text::Attributes {
            font: Some(pp::Font {
                name: "Calibri-Bold".into(),
                size: 130.0,
                bold: true,
                family: "Calibri".into(),
                face: "Bold".into(),
                ..Default::default()
            }),
            fill: Some(attrs::Fill::TextSolidFill(color(1.0, 1.0, 1.0, 1.0))),
            stroke_width: 3.0,
            stroke_color: Some(color(0.0, 0.0, 0.0, 1.0)),
            paragraph_style: Some(attrs::Paragraph {
                alignment: 2,
                line_height_multiple: 1.0,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    const PROLOGUE: &str = r"{\rtf0\ansi\ansicpg1252{\fonttbl\f0\fnil\fcharset0 Calibri-Bold;\f1\fnil\fcharset0 ArialMT;}{\colortbl;\red255\green255\blue255;\red0\green0\blue0;\red255\green255\blue255;}{\*\expandedcolortbl;\csgenericrgb\c100000\c100000\c100000\c100000;\csgenericrgb\c0\c0\c0\c100000;\csgenericrgb\c100000\c100000\c100000\c100000;}";

    fn lyric_text(body: &str) -> pp::graphics::Text {
        pp::graphics::Text {
            attributes: Some(lyric_attributes()),
            rtf_data: format!("{PROLOGUE}{body}}}").into_bytes(),
            vertical_alignment: 1,
            scale_behavior: 2,
            shadow: Some(pp::graphics::Shadow {
                angle: 315.0,
                offset: 15.0,
                radius: 8.0,
                color: Some(color(0.0, 0.0, 0.0, 1.0)),
                opacity: 0.75,
                enable: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// An RTF unicode escape for code point `n` (built at run time to keep the source ASCII).
    fn u(n: i32) -> String {
        format!("{}u{n} ?", '\x5c')
    }

    const PAR: &str =
        r"\pard\pardirnatural\qc\partightenfactor0 \f0\b\fs260 \cf1 \strokewidth60 \strokec2 ";

    #[test]
    fn lyric_box_has_sparse_runs() {
        let mut ctx = ctx();
        let t = lyric_text(&format!(r"{PAR}Line one\par {PAR}Line two {}", u(8217)));
        let ti = text_box(&mut ctx, &t);
        let tb = &ti.text;
        assert_eq!(tb.plain_text(), "Line one\nLine two \u{2019}");
        assert_eq!(tb.paragraphs.len(), 2);
        for p in &tb.paragraphs {
            assert_eq!(p.runs.len(), 1);
            assert!(p.runs[0].style.is_empty(), "{:?}", p.runs[0].style);
            assert_eq!(p.style, ParagraphStyle::default());
            assert!(p.end_style.is_empty());
        }
        let d = &tb.default_style;
        assert_eq!(d.font_family.as_deref(), Some("Calibri"));
        assert_eq!(d.font_weight, Some(700));
        assert_eq!(d.postscript_name.as_deref(), Some("Calibri-Bold"));
        assert_eq!(d.size, Some(130.0));
        assert_eq!(d.color, Some(Color::WHITE));
        assert_eq!(tb.default_paragraph.alignment, Some(TextAlign::Center));
        assert_eq!(tb.vertical_align, VerticalAlign::Middle);
        assert_eq!(tb.fit, FitMode::ShrinkToFit);
        assert!(tb.outline.enabled);
        assert_eq!(tb.outline.width, 3.0);
        assert_eq!(tb.outline.color, Color::BLACK);
        assert!(tb.text_shadow.enabled);
        assert_eq!(tb.text_shadow.offset, 15.0);
        assert!(ti.unmapped.is_empty(), "{:?}", ti.unmapped);
        // Fonts: one box default + two runs.
        assert_eq!(ctx.lookups.fonts.values().sum::<usize>(), 3);
    }

    #[test]
    fn superscript_and_font_change_are_run_overrides() {
        let mut ctx = ctx();
        let t = lyric_text(&format!(r"{PAR}\super 9\nosupersub Some words \f1\b0 here"));
        let tb = text_box(&mut ctx, &t).text;
        let runs = &tb.paragraphs[0].runs;
        assert_eq!(runs.len(), 3, "{runs:?}");
        assert_eq!(runs[0].text, "9");
        assert_eq!(runs[0].style.baseline, Some(Baseline::Superscript));
        assert_eq!(runs[0].style.font_family, None);
        assert_eq!(runs[1].text, "Some words ");
        assert!(runs[1].style.is_empty());
        assert_eq!(runs[2].style.font_family.as_deref(), Some("Arial"));
        assert_eq!(runs[2].style.font_weight, Some(400));
        assert_eq!(runs[2].style.postscript_name.as_deref(), Some("ArialMT"));
        assert_eq!(runs[2].style.italic, None);
        let eff = tb.run_style(&runs[2]);
        assert_eq!(eff.postscript_name.as_deref(), Some("ArialMT"));
        assert_eq!(eff.size, Some(130.0));
    }

    #[test]
    fn colour_table_index_changes_do_not_split_runs() {
        let mut ctx = ctx();
        // cf1 and cf3 are the same white: one run.
        let t = lyric_text(&format!(r"{PAR}Same \cf3 colour"));
        let tb = text_box(&mut ctx, &t).text;
        assert_eq!(tb.paragraphs[0].runs.len(), 1);
        assert_eq!(tb.paragraphs[0].runs[0].text, "Same colour");
    }

    #[test]
    fn capitalization_ranges_split_runs_in_utf16_units() {
        let mut ctx = ctx();
        let mut t = lyric_text(&format!(r"{PAR}a{}{}bc\par {PAR}def", u(55357), u(56832)));
        // "a😀bc" is 5 UTF-16 units; the break is 1; "def" starts at 6.
        t.attributes.as_mut().unwrap().custom_attributes = vec![
            attrs::CustomAttribute {
                range: Some(pp::IntRange { start: 3, end: 5 }),
                attribute: Some(Attribute::Capitalization(1)),
            },
            attrs::CustomAttribute {
                range: Some(pp::IntRange { start: 7, end: 9 }),
                attribute: Some(Attribute::Capitalization(3)),
            },
        ];
        let tb = text_box(&mut ctx, &t).text;
        assert_eq!(tb.plain_text(), "a\u{1F600}bc\ndef");
        let p0 = &tb.paragraphs[0].runs;
        assert_eq!(p0.len(), 2, "{p0:?}");
        assert_eq!(p0[0].text, "a\u{1F600}");
        assert_eq!(p0[1].text, "bc");
        assert_eq!(p0[1].style.capitalization, Some(Capitalization::AllCaps));
        let p1 = &tb.paragraphs[1].runs;
        assert_eq!(
            p1.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            ["d", "ef"]
        );
        assert_eq!(p1[1].style.capitalization, Some(Capitalization::TitleCase));
    }

    #[test]
    fn box_level_all_caps_needs_no_run_overrides() {
        let mut ctx = ctx();
        let mut t = lyric_text(&format!(r"{PAR}Alpha one\par {PAR}two"));
        let a = t.attributes.as_mut().unwrap();
        a.capitalization = 1;
        a.custom_attributes = vec![
            attrs::CustomAttribute {
                range: Some(pp::IntRange { start: 0, end: 9 }),
                attribute: Some(Attribute::Capitalization(1)),
            },
            attrs::CustomAttribute {
                range: Some(pp::IntRange { start: 10, end: 13 }),
                attribute: Some(Attribute::Capitalization(1)),
            },
        ];
        let tb = text_box(&mut ctx, &t).text;
        assert_eq!(
            tb.default_style.capitalization,
            Some(Capitalization::AllCaps)
        );
        assert!(
            tb.paragraphs
                .iter()
                .all(|p| p.runs.iter().all(|r| r.style.is_empty()))
        );
    }

    #[test]
    fn empty_box_keeps_attributes_and_custom_attributes() {
        let mut ctx = ctx();
        let mut t = lyric_text(r"\pard\qc\f0\b\fs260\cf1 ");
        let a = t.attributes.as_mut().unwrap();
        a.custom_attributes = vec![
            attrs::CustomAttribute {
                range: Some(pp::IntRange { start: 0, end: 0 }),
                attribute: Some(Attribute::OriginalFont(pp::Font {
                    name: "ArialMT".into(),
                    ..Default::default()
                })),
            },
            attrs::CustomAttribute {
                range: Some(pp::IntRange { start: 0, end: 0 }),
                attribute: Some(Attribute::FontScaleFactor(0.98)),
            },
        ];
        let ti = text_box(&mut ctx, &t);
        assert!(ti.text.is_blank());
        assert_eq!(ti.text.paragraphs.len(), 1);
        assert_eq!(ti.text.default_style.size, Some(130.0));
        assert_eq!(ti.text.font_scale_factor, 0.98);
        assert_eq!(ti.unmapped.len(), 1);
        assert_eq!(ti.unmapped[0].0, "text.originalFont");
        assert_eq!(
            ti.unmapped[0].1[0]["originalFont"]["name"],
            serde_json::json!("ArialMT")
        );
    }

    #[test]
    fn paragraph_overrides_and_lists() {
        let mut ctx = ctx();
        let body = format!(
            r"\pard\ql\li720\fi-720\sb400\ls1\ilvl0 {{\listtext\t{}\t}}\f0\b\fs260\cf1 Item\par \pard\qr\f0\b\fs260\cf1 Ref",
            u(8226)
        );
        let t = lyric_text(&body);
        let tb = text_box(&mut ctx, &t).text;
        assert_eq!(tb.plain_text(), "Item\nRef");
        let p0 = &tb.paragraphs[0].style;
        assert_eq!(p0.alignment, Some(TextAlign::Left));
        assert_eq!(p0.indent_start, Some(36.0));
        assert_eq!(p0.indent_first_line, Some(-36.0));
        assert_eq!(p0.space_before, Some(20.0));
        assert_eq!(
            p0.list,
            Some(ListMarker {
                kind: ListKind::Disc,
                level: 0
            })
        );
        let p1 = &tb.paragraphs[1].style;
        assert_eq!(p1.alignment, Some(TextAlign::Right));
        assert_eq!(p1.list, None);
    }

    #[test]
    fn outline_and_disabled_outline() {
        let mut ctx = ctx();
        let mut t = lyric_text(&format!(r"{PAR}x"));
        let a = t.attributes.as_mut().unwrap();
        a.stroke_width = 0.0;
        a.stroke_color = Some(color(1.0, 1.0, 1.0, 1.0));
        t.rtf_data = format!(r"{PROLOGUE}\pard\qc\f0\b\fs260\cf1\strokewidth0 x}}").into_bytes();
        let tb = text_box(&mut ctx, &t).text;
        assert_eq!(tb.outline, TextOutline::default());
        // A run with a stroke the box does not have is unmapped.
        let t2 = {
            let mut t2 = t.clone();
            t2.rtf_data =
                format!(r"{PROLOGUE}\pard\qc\f0\b\fs260\cf1 x \strokewidth80\strokec2 y}}")
                    .into_bytes();
            t2
        };
        let ti = text_box(&mut ctx, &t2);
        assert!(
            !ti.text.outline.enabled,
            "the first run decides when attributes have none"
        );
        assert!(ti.unmapped.iter().any(|(k, _)| k == "text.runOutline"));
    }

    #[test]
    fn empty_rtf_gives_no_paragraphs() {
        let mut ctx = ctx();
        let t = pp::graphics::Text::default();
        let tb = text_box(&mut ctx, &t).text;
        assert!(tb.paragraphs.is_empty());
        assert_eq!(tb.vertical_align, VerticalAlign::Top);
        assert_eq!(tb.fit, FitMode::None);
    }

    #[test]
    fn soft_line_breaks_become_u2028() {
        let mut ctx = ctx();
        let t = lyric_text(&format!(r"{PAR}one\line two"));
        let tb = text_box(&mut ctx, &t).text;
        assert_eq!(tb.paragraphs[0].runs[0].text, "one\u{2028}two");
    }

    #[test]
    fn notes_keep_inline_formatting_only() {
        let mut ctx = ctx();
        let n = pp::presentation_slide::Notes {
            rtf_data: format!(
                r"{PROLOGUE}\pard\f1\fs100\cf1 Watch the \b cue\b0 \par\pard\f1\fs100\cf1 \par}}"
            )
            .into_bytes(),
            attributes: Some(pp::graphics::text::Attributes {
                font: Some(pp::Font {
                    name: "ArialMT".into(),
                    size: 50.0,
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };
        let r = notes(&mut ctx, &n);
        assert_eq!(r.plain_text(), "Watch the cue");
        assert_eq!(r.paragraphs[0].runs.len(), 2);
        assert_eq!(r.paragraphs[0].runs[1].style.font_weight, Some(700));
        assert_eq!(r.paragraphs[0].runs[1].style.size, None);
        assert!(ctx.lookups.fonts.is_empty(), "notes fonts are not counted");
        let empty = pp::presentation_slide::Notes {
            rtf_data: format!(r"{PROLOGUE}\pard\f1\fs100\cf1 }}").into_bytes(),
            attributes: None,
        };
        assert!(notes(&mut ctx, &empty).is_empty());
        let rich = rich_text_from_rtf(&mut ctx, format!(r"{PROLOGUE}\f1 a \i b}}").as_bytes());
        assert_eq!(rich.paragraphs[0].runs[1].style.italic, Some(true));
        assert!(rich.paragraphs[0].runs[0].style.is_empty());
    }

    #[test]
    fn text_fills_and_unmapped_fields() {
        let mut ctx = ctx();
        let mut t = lyric_text(&format!(r"{PAR}x"));
        t.attributes.as_mut().unwrap().fill =
            Some(attrs::Fill::CutOutFill(pp::graphics::text::CutOutFill {}));
        t.chord_pro = Some(pp::graphics::text::ChordPro {
            enabled: true,
            notation: 1,
            ..Default::default()
        });
        t.transform = 1;
        let ti = text_box(&mut ctx, &t);
        assert!(ti.text.fill.enabled);
        assert_eq!(ti.text.fill.kind, FillKind::CutOut);
        let keys: Vec<_> = ti.unmapped.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"text.chordPro") && keys.contains(&"text.transform"));
        assert!(ti.text.chords.show);
    }

    #[test]
    fn chords_sit_over_their_characters() {
        let mut ctx = ctx();
        // "Sing a song" / "of joy": UTF-16 offsets over the whole text, one per paragraph break.
        let mut t = lyric_text(&format!(r"{PAR}Sing a song\par {PAR}of joy"));
        let chord = |start: i32, c: &str| attrs::CustomAttribute {
            range: Some(pp::IntRange {
                start,
                end: start + 1,
            }),
            attribute: Some(Attribute::Chord(c.into())),
        };
        t.attributes.as_mut().unwrap().custom_attributes = vec![
            chord(7, "C"),
            chord(0, "G"),
            chord(12, "D/F#"),
            chord(18, "G"),
        ];
        t.chord_pro = Some(pp::graphics::text::ChordPro {
            enabled: true,
            color: Some(color(1.0, 0.0, 0.0, 1.0)),
            ..Default::default()
        });
        let ti = text_box(&mut ctx, &t);
        let tb = &ti.text;
        let marks = |i: usize| -> Vec<(u32, &str)> {
            tb.paragraphs[i]
                .chords
                .iter()
                .map(|c| (c.at, c.chord.as_str()))
                .collect()
        };
        assert_eq!(marks(0), [(0, "G"), (7, "C")]);
        assert_eq!(marks(1), [(0, "D/F#"), (6, "G")]);
        assert!(tb.chords.show);
        assert_eq!(tb.chords.color, Some(Color::rgb(1.0, 0.0, 0.0)));
        assert!(ti.unmapped.iter().all(|(k, _)| k != "text.chordPro"));
        assert!(
            ti.unmapped
                .iter()
                .all(|(k, _)| k != "text.customAttributes")
        );
    }

    #[test]
    fn enum_mappings() {
        assert_eq!(capitalization(0), None);
        assert_eq!(capitalization(4), Some(Capitalization::StartCase));
        assert_eq!(fit_mode(1), FitMode::ResizeBox);
        assert_eq!(fit_mode(4), FitMode::ScaleToFit);
        assert_eq!(vertical_align(2), VerticalAlign::Bottom);
        let m = line_mask(&pp::graphics::text::LineFillMask {
            enabled: true,
            mask_style: 2,
            width_offset: 29.0,
            height_offset: -3.0,
            ..Default::default()
        });
        assert_eq!(m.style, LineMaskStyle::MaxLineWidth);
        assert_eq!(m.height_offset, -3.0);
    }

    #[test]
    fn sparse_overrides_against_fallback() {
        let run = CharStyle {
            font_family: Some("Arial".into()),
            font_weight: Some(400),
            italic: Some(false),
            size: Some(50.0),
            color: Some(Color::rgba(1.0, 1.0, 1.0, 1.0)),
            underline: Some(LineStyle::None),
            highlight: Some(Color::TRANSPARENT),
            ..CharStyle::default()
        };
        assert!(sparse_overrides(&run, &CharStyle::default()).is_empty());
        let base = CharStyle {
            color: Some(Color::rgba(0.9395542, 0.0, 0.0, 1.0)),
            ..CharStyle::default()
        };
        let run = CharStyle {
            color: Some(Color::from_rgb8(240, 0, 0)),
            ..CharStyle::default()
        };
        assert!(sparse_overrides(&run, &base).is_empty());
    }

    /// Text fidelity on a real ProPresenter data folder: for every `Graphics.Text` in every
    /// document, the model's paragraph / run text equals the RTF text. Runs only when `PP_BACKUP`
    /// points at a backup (private data, never committed):
    ///
    /// ```text
    /// PP_BACKUP=/path/to/backup cargo test -p pro-import --lib backup_text_fidelity -- --nocapture
    /// ```
    #[test]
    fn backup_text_fidelity() {
        use prost::Message as _;
        use prost_reflect::{DynamicMessage, ReflectMessage as _, Value as V};
        let Some(root) = std::env::var_os("PP_BACKUP").map(std::path::PathBuf::from) else {
            eprintln!("PP_BACKUP not set; skipping");
            return;
        };
        fn type_for(rel: &std::path::Path) -> Option<&'static str> {
            if rel.extension().is_some_and(|e| e == "pro") {
                return Some("rv.data.Presentation");
            }
            if rel.extension().is_some() || rel.starts_with("Media") {
                return None;
            }
            let parent = rel.parent()?.file_name()?.to_str()?;
            Some(match (parent, rel.file_name()?.to_str()?) {
                ("Playlists", _) => "rv.data.PlaylistDocument",
                (_, "Workspace") => "rv.data.ProPresenterWorkspace",
                (_, "Stage") => "rv.data.Stage.Document",
                (_, "Props") => "rv.data.PropDocument",
                (_, "Macros") => "rv.data.MacrosDocument",
                (_, "Messages") => "rv.data.MessageDocument",
                (_, "CCLI") => "rv.data.CCLIDocument",
                (_, "Theme") => "rv.data.Template.Document",
                _ => return None,
            })
        }
        fn collect(m: &DynamicMessage, name: &str, out: &mut Vec<Vec<u8>>) {
            if m.descriptor().full_name() == name {
                out.push(m.encode_to_vec());
                return;
            }
            for (_, v) in m.fields() {
                match v {
                    V::Message(m) => collect(m, name, out),
                    V::List(l) => l.iter().for_each(|v| {
                        if let V::Message(m) = v {
                            collect(m, name, out)
                        }
                    }),
                    _ => {}
                }
            }
        }
        let mut ctx = ctx();
        let pool = ctx.pool().clone();
        let (mut files, mut texts, mut with_text, mut notes_seen) = (0, 0, 0, 0);
        let (mut uncovered, mut capped_boxes) = (0usize, 0usize);
        let mut bad: Vec<String> = Vec::new();
        for entry in walkdir::WalkDir::new(&root)
            .sort_by_file_name()
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let rel = entry.path().strip_prefix(&root).unwrap().to_path_buf();
            let Some(ty) = type_for(&rel) else { continue };
            let bytes = std::fs::read(entry.path()).unwrap();
            let msg =
                DynamicMessage::decode(pool.get_message_by_name(ty).unwrap(), &bytes[..]).unwrap();
            files += 1;
            ctx.current.path = Some(rel.to_string_lossy().replace('\\', "/"));
            let mut found = Vec::new();
            collect(&msg, "rv.data.Graphics.Text", &mut found);
            for (i, b) in found.iter().enumerate() {
                let t = pp::graphics::Text::decode(&b[..]).unwrap();
                let tb = text_box(&mut ctx, &t).text;
                texts += 1;
                let got: Vec<String> = tb.paragraphs.iter().map(Paragraph::plain_text).collect();
                let src: Vec<String> = rtf::parse(&t.rtf_data)
                    .paragraphs
                    .iter()
                    .map(|p| p.text().replace('\n', "\u{2028}"))
                    .collect();
                if !t.rtf_data.is_empty() && got != src {
                    bad.push(format!("{} text #{i}", rel.display()));
                }
                if got.iter().any(|p| !p.is_empty()) {
                    with_text += 1;
                }
                // Capitalization coverage: characters outside every range.
                let ranges = cap_ranges(
                    t.attributes
                        .as_ref()
                        .map(|a| a.custom_attributes.as_slice())
                        .unwrap_or(&[]),
                );
                if !ranges.is_empty() {
                    capped_boxes += 1;
                    let mut off = 0usize;
                    for (pi, p) in src.iter().enumerate() {
                        if pi > 0 {
                            off += 1;
                        }
                        for c in p.chars() {
                            if cap_at(&ranges, off).is_none() {
                                uncovered += 1;
                            }
                            off += c.len_utf16();
                        }
                    }
                }
            }
            let mut found = Vec::new();
            collect(&msg, "rv.data.PresentationSlide.Notes", &mut found);
            for b in &found {
                let n = pp::presentation_slide::Notes::decode(&b[..]).unwrap();
                let r = notes(&mut ctx, &n);
                let mut src: Vec<String> = rtf::parse(&n.rtf_data)
                    .paragraphs
                    .iter()
                    .map(|p| p.text().replace('\n', "\u{2028}"))
                    .collect();
                while src.last().is_some_and(String::is_empty) {
                    src.pop();
                }
                let got: Vec<String> = r.paragraphs.iter().map(Paragraph::plain_text).collect();
                if got != src {
                    bad.push(format!("{} notes", rel.display()));
                }
                if !r.is_empty() {
                    notes_seen += 1;
                }
            }
        }
        eprintln!(
            "{files} files, {texts} text boxes ({with_text} with text), {notes_seen} non-empty notes, \
             {capped_boxes} boxes with capitalization ranges ({uncovered} chars outside them), \
             {} mismatches",
            bad.len()
        );
        for b in bad.iter().take(20) {
            eprintln!("mismatch: {b}");
        }
        assert!(bad.is_empty());
    }
}
