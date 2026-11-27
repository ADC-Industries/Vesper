//! Text layout: [`model::TextBox`] → positioned glyphs, decorations and line boxes.
//!
//! Shaping, bidi, wrapping and per-glyph font fallback come from cosmic-text. One cosmic-text
//! `Buffer` is laid out per model paragraph, so paragraph spacing, indents and per-paragraph
//! alignment are applied here. Everything is laid out directly in output pixels (canvas px ×
//! screen scale), so glyphs are rasterized at their final size.
//!
//! ## Style mapping
//! - Font: [`crate::fonts`] resolution; synthetic bold / italic when the face lacks them.
//! - Size: `size × fontScaleFactor × fit scale × screen scale`.
//! - Line height: the face's natural line height (ascent + descent + line gap) ×
//!   `lineHeightMultiple`, plus `lineSpacing` px; glyphs are centred in the line box.
//! - Superscript / subscript: [`SCRIPT_SIZE`] of the size, baseline raised by
//!   [`SUPERSCRIPT_RISE`] / lowered by [`SUBSCRIPT_DROP`] of the run's normal size.
//! - Capitalization is applied to the displayed string (stored text keeps its case); small caps
//!   are uppercase letters at [`SMALL_CAPS_SIZE`] of the size.
//! - Letter spacing px → cosmic-text EM tracking.
//!
//! ## Fit
//! `shrinkToFit` binary-searches the largest scale ≤ 1 at which the wrapped block fits the box
//! (height, and width of unbreakable words); `scaleToFit` searches 0.05–20; `growToFit` ≥ 1;
//! `resizeBox` keeps the size and reports the box height the text needs.

use crate::fonts::{FontResolver, Resolved};
use crate::geom::RectF;
use cosmic_text::{Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Wrap};
use model::{
    Baseline, Capitalization, CharStyle, Color, FitMode, LineStyle, ListKind, Paragraph,
    ParagraphStyle, TextAlign, TextBox, VerticalAlign,
};

/// Superscript / subscript glyph size relative to the run size.
pub const SCRIPT_SIZE: f32 = 0.65;
/// Superscript baseline rise, fraction of the run's normal size.
pub const SUPERSCRIPT_RISE: f32 = 0.35;
/// Subscript baseline drop, fraction of the run's normal size.
pub const SUBSCRIPT_DROP: f32 = 0.15;
/// Small-caps size of lowercase letters relative to the run size.
pub const SMALL_CAPS_SIZE: f32 = 0.75;

/// Per-span rendering info (indexed by cosmic-text `metadata`).
#[derive(Clone, Debug)]
pub(crate) struct SpanInfo {
    pub color: Color,
    pub underline: LineStyle,
    pub strikethrough: LineStyle,
    pub highlight: Option<Color>,
    /// Baseline shift, px, positive = up.
    pub shift: f32,
    /// Ascent / descent of the run's face at its *normal* (unscripted) size, px: they decide the
    /// line's baseline, so a line of only superscript glyphs keeps the normal baseline.
    pub ascent: f32,
    pub descent: f32,
    pub faux_bold: bool,
    pub faux_italic: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PlacedGlyph {
    pub font: fontdb::ID,
    pub weight: fontdb::Weight,
    pub glyph: u16,
    pub size: f32,
    /// Pen position (baseline origin), frame px.
    pub x: f32,
    pub y: f32,
    /// Advance width (for decorations / highlights).
    pub w: f32,
    pub span: usize,
}

/// A coloured rectangle (underline, strikethrough, highlight).
#[derive(Clone, Copy, Debug)]
pub(crate) struct DecoRect {
    pub rect: RectF,
    pub color: Color,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TextLayout {
    pub glyphs: Vec<PlacedGlyph>,
    pub spans: Vec<SpanInfo>,
    pub decorations: Vec<DecoRect>,
    pub highlights: Vec<DecoRect>,
    /// One box per laid-out line: x from the line's first to last glyph, y the line box.
    pub lines: Vec<RectF>,
    /// The content box (element rect minus margins), frame px.
    pub content: RectF,
    /// Applied fit scale (1 = authored size).
    pub fit_scale: f32,
    /// Height of the laid-out block, frame px.
    pub block_height: f32,
    /// For `resizeBox`: the element height the text needs (frame px).
    pub resized_height: Option<f32>,
}

struct LineOut {
    top: f32,
    height: f32,
    glyphs: Vec<PlacedGlyph>,
}

struct ParaOut {
    lines: Vec<LineOut>,
    height: f32,
    max_w: f32,
    /// Horizontal origin of the paragraph inside the content box (indent).
    x: f32,
    space_before: f32,
    space_after: f32,
}

pub(crate) struct TextEngine {
    pub fs: FontSystem,
    pub fonts: FontResolver,
}

fn to_cosmic_style(s: fontdb::Style) -> cosmic_text::Style {
    match s {
        fontdb::Style::Normal => cosmic_text::Style::Normal,
        fontdb::Style::Italic => cosmic_text::Style::Italic,
        fontdb::Style::Oblique => cosmic_text::Style::Oblique,
    }
}

fn is_minor_word(w: &str) -> bool {
    const MINOR: &[&str] = &[
        "a", "an", "the", "and", "but", "or", "for", "nor", "on", "at", "to", "from", "by", "of",
        "in", "as", "so", "yet",
    ];
    MINOR.iter().any(|m| m.eq_ignore_ascii_case(w))
}

/// Applies a display-time case transform. `word_start` carries word state across runs.
pub(crate) fn transform_case(
    text: &str,
    cap: Capitalization,
    at_start: &mut bool,
    first_word: &mut bool,
) -> String {
    match cap {
        Capitalization::AllCaps => {
            *at_start = text.chars().last().is_none_or(|c| !c.is_alphanumeric());
            text.to_uppercase()
        }
        Capitalization::StartCase | Capitalization::TitleCase => {
            let mut out = String::with_capacity(text.len());
            let chars: Vec<char> = text.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                let c = chars[i];
                if c.is_alphanumeric() {
                    if *at_start {
                        let end = chars[i..]
                            .iter()
                            .position(|c| !c.is_alphanumeric() && *c != '\'')
                            .map(|p| i + p)
                            .unwrap_or(chars.len());
                        let word: String = chars[i..end].iter().collect();
                        let keep_lower = cap == Capitalization::TitleCase
                            && !*first_word
                            && is_minor_word(&word);
                        if keep_lower {
                            out.push_str(&word);
                        } else {
                            let mut wc = word.chars();
                            if let Some(f) = wc.next() {
                                out.extend(f.to_uppercase());
                                out.push_str(wc.as_str());
                            }
                        }
                        *first_word = false;
                        *at_start = false;
                        i = end;
                        continue;
                    }
                    out.push(c);
                } else {
                    *at_start = c != '\'';
                    out.push(c);
                }
                i += 1;
            }
            out
        }
        _ => {
            *at_start = text.chars().last().is_none_or(|c| !c.is_alphanumeric());
            text.to_string()
        }
    }
}

fn list_marker(kind: ListKind, n: usize) -> String {
    fn roman(mut n: usize) -> String {
        const T: &[(usize, &str)] = &[
            (1000, "m"),
            (900, "cm"),
            (500, "d"),
            (400, "cd"),
            (100, "c"),
            (90, "xc"),
            (50, "l"),
            (40, "xl"),
            (10, "x"),
            (9, "ix"),
            (5, "v"),
            (4, "iv"),
            (1, "i"),
        ];
        let mut s = String::new();
        for (v, r) in T {
            while n >= *v {
                s.push_str(r);
                n -= v;
            }
        }
        s
    }
    let alpha = |n: usize| ((b'a' + ((n - 1) % 26) as u8) as char).to_string();
    match kind {
        ListKind::None => String::new(),
        ListKind::Disc | ListKind::Unknown => "•\t".into(),
        ListKind::Circle => "◦\t".into(),
        ListKind::Square => "▪\t".into(),
        ListKind::Hyphen => "–\t".into(),
        ListKind::Decimal => format!("{n}.\t"),
        ListKind::LowerAlpha => format!("{}.\t", alpha(n)),
        ListKind::UpperAlpha => format!("{}.\t", alpha(n).to_uppercase()),
        ListKind::LowerRoman => format!("{}.\t", roman(n)),
        ListKind::UpperRoman => format!("{}.\t", roman(n).to_uppercase()),
    }
}

impl TextEngine {
    pub fn new(fs: FontSystem, fonts: FontResolver) -> Self {
        TextEngine { fs, fonts }
    }

    fn resolve(&mut self, style: &CharStyle) -> Option<Resolved> {
        let fr = style.font_ref();
        self.fonts.resolve(self.fs.db(), &fr)
    }

    /// Ascent and descent of a face as fractions of the em.
    fn ascent_descent(&mut self, r: &Resolved) -> (f32, f32) {
        if let Some(font) = self.fs.get_font(r.face, r.weight) {
            let m = font.as_swash().metrics(&[]);
            let upem = m.units_per_em.max(1) as f32;
            return (m.ascent / upem, m.descent.abs() / upem);
        }
        (0.8, 0.2)
    }

    /// Natural line height (ascent + descent + gap) of a face at `size` px.
    fn natural_line_height(&mut self, r: &Resolved, size: f32) -> f32 {
        if let Some(font) = self.fs.get_font(r.face, r.weight) {
            let m = font.as_swash().metrics(&[]);
            let upem = m.units_per_em.max(1) as f32;
            let lh = (m.ascent + m.descent + m.leading) / upem;
            if lh.is_finite() && lh > 0.1 {
                return lh * size;
            }
        }
        size * 1.2
    }

    /// Lays out a text box inside `rect` (frame px). `scale` converts canvas px to frame px.
    pub fn layout(&mut self, tb: &TextBox, rect: RectF, scale: f32) -> TextLayout {
        let m = &tb.margins;
        let content = RectF::new(
            rect.x + m.left as f32 * scale,
            rect.y + m.top as f32 * scale,
            (rect.w - (m.left + m.right) as f32 * scale).max(0.0),
            (rect.h - (m.top + m.bottom) as f32 * scale).max(0.0),
        );
        let mut out = TextLayout {
            content,
            fit_scale: 1.0,
            ..TextLayout::default()
        };
        if tb.paragraphs.is_empty() || tb.is_blank() {
            return out;
        }
        let base = (tb.font_scale_factor as f32).max(0.01) * scale;
        let fits = |eng: &mut TextEngine, f: f32| -> (bool, Vec<ParaOut>, Vec<SpanInfo>) {
            let (paras, spans) = eng.layout_paragraphs(tb, content.w, base * f, scale);
            let h = block_height(&paras);
            let w = paras.iter().map(|p| p.max_w + p.x).fold(0.0, f32::max);
            (h <= content.h + 0.5 && w <= content.w + 0.5, paras, spans)
        };
        let search = |eng: &mut TextEngine, lo: f32, hi: f32| -> f32 {
            // largest f in [lo, hi] that fits (lo is returned if nothing fits)
            let (mut lo, mut hi) = (lo, hi);
            if fits(eng, hi).0 {
                return hi;
            }
            for _ in 0..14 {
                let mid = (lo + hi) * 0.5;
                if fits(eng, mid).0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
                if hi - lo < 0.002 {
                    break;
                }
            }
            lo
        };
        let f = match tb.fit {
            FitMode::ShrinkToFit => search(self, 0.05, 1.0),
            FitMode::ScaleToFit => search(self, 0.05, 20.0),
            FitMode::GrowToFit => search(self, 1.0, 20.0),
            _ => 1.0,
        };
        let (_, paras, spans) = fits(self, f);
        out.fit_scale = f;
        out.spans = spans;
        let block_h = block_height(&paras);
        out.block_height = block_h;
        let y0 = if tb.fit == FitMode::ResizeBox {
            out.resized_height = Some(block_h + (m.top + m.bottom) as f32 * scale);
            content.y
        } else {
            match tb.vertical_align {
                VerticalAlign::Top => content.y,
                VerticalAlign::Bottom => content.y + content.h - block_h,
                VerticalAlign::Middle | VerticalAlign::Unknown => {
                    content.y + (content.h - block_h) * 0.5
                }
            }
        };
        let mut y = y0;
        let n = paras.len();
        for (i, p) in paras.into_iter().enumerate() {
            if i > 0 {
                y += p.space_before;
            }
            for line in p.lines {
                let dx = content.x + p.x;
                let mut x_min = f32::INFINITY;
                let mut x_max = f32::NEG_INFINITY;
                for mut g in line.glyphs {
                    g.x += dx;
                    g.y += y;
                    x_min = x_min.min(g.x);
                    x_max = x_max.max(g.x + g.w);
                    out.glyphs.push(g);
                }
                if x_min.is_finite() {
                    out.lines
                        .push(RectF::new(x_min, y + line.top, x_max - x_min, line.height));
                }
            }
            y += p.height;
            if i + 1 < n {
                y += p.space_after;
            }
        }
        self.decorate(&mut out);
        out
    }

    /// Underline / strikethrough / highlight rectangles from the placed glyphs.
    fn decorate(&mut self, out: &mut TextLayout) {
        let mut i = 0;
        while i < out.glyphs.len() {
            let g0 = out.glyphs[i].clone();
            let mut j = i + 1;
            while j < out.glyphs.len() && out.glyphs[j].span == g0.span && out.glyphs[j].y == g0.y {
                j += 1;
            }
            let span = out.spans[g0.span].clone();
            let x0 = g0.x;
            let last = &out.glyphs[j - 1];
            let x1 = last.x + last.w;
            let size = g0.size;
            let (ul_off, ul_th, st_off) = self
                .fs
                .get_font(g0.font, g0.weight)
                .map(|f| {
                    let m = f.as_swash().metrics(&[]);
                    let u = m.units_per_em.max(1) as f32;
                    (
                        m.underline_offset / u,
                        (m.stroke_size / u).max(0.04),
                        if m.strikeout_offset != 0.0 {
                            m.strikeout_offset / u
                        } else {
                            m.x_height / u * 0.5
                        },
                    )
                })
                .unwrap_or((-0.1, 0.05, 0.3));
            let baseline = g0.y;
            let mut push = |style: LineStyle, center: f32| {
                let th = (ul_th * size).max(1.0);
                let th = if style == LineStyle::Thick {
                    th * 2.0
                } else {
                    th
                };
                match style {
                    LineStyle::None => {}
                    LineStyle::Double => {
                        out.decorations.push(DecoRect {
                            rect: RectF::new(x0, center - th * 1.5, x1 - x0, th),
                            color: span.color,
                        });
                        out.decorations.push(DecoRect {
                            rect: RectF::new(x0, center + th * 0.5, x1 - x0, th),
                            color: span.color,
                        });
                    }
                    _ => out.decorations.push(DecoRect {
                        rect: RectF::new(x0, center - th * 0.5, x1 - x0, th),
                        color: span.color,
                    }),
                }
            };
            push(span.underline, baseline - ul_off * size);
            push(span.strikethrough, baseline - st_off * size);
            if let Some(h) = span.highlight
                && let Some(line) = out
                    .lines
                    .iter()
                    .find(|l| baseline >= l.y && baseline <= l.bottom())
            {
                out.highlights.push(DecoRect {
                    rect: RectF::new(x0, line.y, x1 - x0, line.h),
                    color: h,
                });
            }
            i = j;
        }
    }

    fn layout_paragraphs(
        &mut self,
        tb: &TextBox,
        width: f32,
        size_scale: f32,
        scale: f32,
    ) -> (Vec<ParaOut>, Vec<SpanInfo>) {
        let mut spans = Vec::new();
        let mut paras = Vec::with_capacity(tb.paragraphs.len());
        let mut list_counter: Vec<usize> = Vec::new();
        // Chord rows: every line gets one when the box shows chords and any line has some.
        let chord_rows = tb.chords.show && tb.paragraphs.iter().any(|p| p.has_chords());
        for p in &tb.paragraphs {
            let ps = tb.paragraph_style(p);
            let marker = match &ps.list {
                Some(l) if l.kind != ListKind::None => {
                    let lvl = l.level as usize;
                    list_counter.resize(lvl + 1, 0);
                    list_counter[lvl] += 1;
                    Some(list_marker(l.kind, list_counter[lvl]))
                }
                _ => {
                    list_counter.clear();
                    None
                }
            };
            paras.push(self.layout_paragraph(
                tb, p, &ps, marker, width, size_scale, scale, chord_rows, &mut spans,
            ));
        }
        (paras, spans)
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_paragraph(
        &mut self,
        tb: &TextBox,
        p: &Paragraph,
        ps: &ParagraphStyle,
        marker: Option<String>,
        width: f32,
        size_scale: f32,
        scale: f32,
        chord_rows: bool,
        spans: &mut Vec<SpanInfo>,
    ) -> ParaOut {
        let lhm = ps.line_height_multiple.unwrap_or(1.0).max(0.05) as f32;
        let extra = ps.line_spacing.unwrap_or(0.0) as f32 * scale;
        let indent_start = ps.indent_start.unwrap_or(0.0) as f32 * scale;
        let indent_end = ps.indent_end.unwrap_or(0.0) as f32 * scale;
        let wrap_w = (width - indent_start - indent_end).max(1.0);
        let align = match ps.alignment.unwrap_or(TextAlign::Center) {
            TextAlign::Left => Some(Align::Left),
            TextAlign::Right => Some(Align::Right),
            TextAlign::Justified => Some(Align::Justified),
            TextAlign::Natural => None,
            TextAlign::Center | TextAlign::Unknown => Some(Align::Center),
        };

        // Pieces: (display text, cascaded style, size multiplier).
        let mut pieces: Vec<(String, CharStyle, f32)> = Vec::new();
        let mut at_start = true;
        let mut first_word = true;
        let mut marker_chars = 0;
        if let Some(m) = marker
            && let Some(r) = p.runs.first()
        {
            marker_chars = m.chars().count();
            pieces.push((m, tb.run_style(r), 1.0));
        }
        for run in &p.runs {
            if run.text.is_empty() {
                continue;
            }
            let st = tb.run_style(run);
            let cap = st.capitalization.unwrap_or(Capitalization::None);
            let raw = run.text.replace('\u{2028}', "\n");
            if cap == Capitalization::SmallCaps {
                // lowercase letters → uppercase at reduced size
                let mut cur = String::new();
                let mut cur_small = None;
                for c in raw.chars() {
                    let small = c.is_lowercase();
                    if cur_small.is_some_and(|s| s != small) {
                        let mul = if cur_small == Some(true) {
                            SMALL_CAPS_SIZE
                        } else {
                            1.0
                        };
                        pieces.push((std::mem::take(&mut cur), st.clone(), mul));
                    }
                    cur_small = Some(small);
                    cur.extend(c.to_uppercase());
                }
                if !cur.is_empty() {
                    let mul = if cur_small == Some(true) {
                        SMALL_CAPS_SIZE
                    } else {
                        1.0
                    };
                    pieces.push((cur, st.clone(), mul));
                }
                at_start = raw.chars().last().is_none_or(|c| !c.is_alphanumeric());
            } else {
                let text = transform_case(&raw, cap, &mut at_start, &mut first_word);
                pieces.push((text, st, 1.0));
            }
        }

        // Default (paragraph mark) style: decides empty-line height and buffer metrics.
        let mark_style = if p.runs.iter().all(|r| r.text.is_empty()) {
            tb.empty_paragraph_style(p)
        } else {
            tb.run_style(&p.runs[0])
        };
        let mark_size = (mark_style.size.unwrap_or(50.0) as f32 * size_scale).max(0.5);
        let mark_res = self.resolve(&mark_style);
        let mark_lh = match &mark_res {
            Some(r) => self.natural_line_height(r, mark_size),
            None => mark_size * 1.2,
        } * lhm
            + extra;

        struct PieceAttrs {
            text: String,
            family: String,
            weight: fontdb::Weight,
            style: fontdb::Style,
            stretch: fontdb::Stretch,
            size: f32,
            line_height: f32,
            letter_spacing: f32,
            span: usize,
        }
        let mut piece_attrs = Vec::with_capacity(pieces.len());
        for (text, st, mul) in &pieces {
            let Some(res) = self.resolve(st) else {
                continue;
            };
            let base_size = (st.size.unwrap_or(50.0) as f32 * size_scale).max(0.5);
            let (size, shift) = match st.baseline.unwrap_or(Baseline::Normal) {
                Baseline::Superscript => (base_size * SCRIPT_SIZE, base_size * SUPERSCRIPT_RISE),
                Baseline::Subscript => (base_size * SCRIPT_SIZE, -base_size * SUBSCRIPT_DROP),
                _ => (base_size, 0.0),
            };
            let size = size * mul;
            let line_height = self.natural_line_height(&res, base_size) * lhm + extra;
            let (asc, desc) = self.ascent_descent(&res);
            let span = spans.len();
            spans.push(SpanInfo {
                color: st.color.unwrap_or(Color::WHITE),
                underline: st.underline.unwrap_or(LineStyle::None),
                strikethrough: st.strikethrough.unwrap_or(LineStyle::None),
                highlight: st.highlight,
                shift,
                ascent: asc * base_size,
                descent: desc * base_size,
                faux_bold: res.faux_bold,
                faux_italic: res.faux_italic,
            });
            piece_attrs.push(PieceAttrs {
                text: text.clone(),
                family: res.family.clone(),
                weight: res.weight,
                style: res.style,
                stretch: res.stretch,
                size,
                line_height,
                letter_spacing: st.letter_spacing.unwrap_or(0.0) as f32 * scale,
                span,
            });
        }

        let mut buffer = Buffer::new(&mut self.fs, Metrics::new(mark_size, mark_lh));
        buffer.set_wrap(Wrap::Word);
        buffer.set_size(Some(wrap_w), None);
        let default_family = mark_res
            .as_ref()
            .map(|r| r.family.clone())
            .unwrap_or_default();
        let default_attrs = Attrs::new()
            .family(Family::Name(&default_family))
            .metrics(Metrics::new(mark_size, mark_lh));
        buffer.set_rich_text(
            piece_attrs.iter().map(|p| {
                (
                    p.text.as_str(),
                    Attrs::new()
                        .family(Family::Name(&p.family))
                        .weight(p.weight)
                        .style(to_cosmic_style(p.style))
                        .stretch(p.stretch)
                        .metrics(Metrics::new(p.size, p.line_height))
                        .letter_spacing(p.letter_spacing / p.size.max(0.01))
                        .metadata(p.span),
                )
            }),
            &default_attrs,
            Shaping::Advanced,
            align,
        );
        buffer.shape_until_scroll(&mut self.fs, false);

        let mut lines = Vec::new();
        let mut height: f32 = 0.0;
        let mut max_w: f32 = 0.0;
        // Per laid-out line: its buffer line and each glyph's (start, end, x, w), for chords.
        let mut clusters: Vec<LineClusters> = Vec::new();
        for run in buffer.layout_runs() {
            if chord_rows {
                clusters.push((
                    run.line_i,
                    run.glyphs
                        .iter()
                        .map(|g| (g.start, g.end, indent_start + g.x, g.w))
                        .collect(),
                ));
            }
            let mut glyphs = Vec::with_capacity(run.glyphs.len());
            // Baseline from the runs' normal sizes (cosmic-text centres the glyph box in the line
            // box; do the same with unscripted ascent / descent).
            let (mut a, mut d) = (0.0f32, 0.0f32);
            for g in run.glyphs {
                if let Some(sp) = spans.get(g.metadata) {
                    a = a.max(sp.ascent);
                    d = d.max(sp.descent);
                }
            }
            let line_y = if a + d > 0.0 {
                run.line_top + (run.line_height - (a + d)) * 0.5 + a
            } else {
                run.line_y
            };
            for g in run.glyphs {
                let shift = spans.get(g.metadata).map(|s| s.shift).unwrap_or(0.0);
                glyphs.push(PlacedGlyph {
                    font: g.font_id,
                    weight: g.font_weight,
                    glyph: g.glyph_id,
                    size: g.font_size,
                    x: indent_start + g.x + g.font_size * g.x_offset,
                    y: line_y + g.y - g.font_size * g.y_offset - shift,
                    w: g.w,
                    span: g.metadata,
                });
            }
            max_w = max_w.max(run.line_w + indent_start);
            height = height.max(run.line_top + run.line_height);
            lines.push(LineOut {
                top: run.line_top,
                height: run.line_height,
                glyphs,
            });
        }
        if lines.is_empty() {
            height = mark_lh;
        }
        if chord_rows {
            let display: String = piece_attrs.iter().map(|p| p.text.as_str()).collect();
            height = self.chord_rows(
                tb,
                p,
                ChordLine {
                    display: &display,
                    marker_chars,
                    clusters: &clusters,
                    mark_style: &mark_style,
                    mark_size,
                    wrap_w,
                    indent_start,
                    align: ps.alignment.unwrap_or(TextAlign::Center),
                },
                &mut lines,
                height,
                spans,
            );
        }
        ParaOut {
            lines,
            height,
            max_w: (max_w - indent_start).max(0.0),
            x: 0.0,
            space_before: ps.space_before.unwrap_or(0.0) as f32 * scale,
            space_after: ps.space_after.unwrap_or(0.0) as f32 * scale,
        }
    }
}

/// A laid-out line's buffer line and each glyph's (start byte, end byte, x, width).
type LineClusters = (usize, Vec<(usize, usize, f32, f32)>);

/// What [`TextEngine::chord_rows`] needs of a laid-out paragraph.
struct ChordLine<'a> {
    /// The text in the buffer (list marker, case transforms applied; soft breaks as `\n`).
    display: &'a str,
    /// Characters of the list marker before the paragraph's text.
    marker_chars: usize,
    /// Per laid-out line: its buffer line and each glyph's (start, end, x, w).
    clusters: &'a [LineClusters],
    mark_style: &'a CharStyle,
    mark_size: f32,
    wrap_w: f32,
    indent_start: f32,
    align: TextAlign,
}

/// The buffer line and byte offset in it of display character `d`.
fn locate(display: &str, d: usize) -> (usize, usize) {
    let (mut line, mut start) = (0, 0);
    for (i, (b, c)) in display.char_indices().enumerate() {
        if i == d {
            return (line, b - start);
        }
        if c == '\n' {
            line += 1;
            start = b + c.len_utf8();
        }
    }
    (line, display.len() - start)
}

impl TextEngine {
    /// Adds a chord row above every laid-out line of a paragraph (see [`model::chords`]):
    /// moves the lines down, places each chord's glyphs left-aligned over its character (pushed
    /// right past the previous chord when they would overlap). A line of only chords (empty
    /// text) gets a chord row alone. Returns the paragraph's new height.
    #[allow(clippy::too_many_arguments)]
    fn chord_rows(
        &mut self,
        tb: &TextBox,
        p: &Paragraph,
        cl: ChordLine<'_>,
        lines: &mut Vec<LineOut>,
        height: f32,
        spans: &mut Vec<SpanInfo>,
    ) -> f32 {
        let ratio = tb
            .chords
            .size
            .unwrap_or(model::ChordDisplay::DEFAULT_SIZE)
            .clamp(0.1, 3.0) as f32;
        let size = (cl.mark_size * ratio).max(0.5);
        let style = CharStyle {
            font_weight: Some(700),
            postscript_name: None,
            ..cl.mark_style.clone()
        };
        let Some(res) = self.resolve(&style) else {
            return height;
        };
        let row_h = self.natural_line_height(&res, size);
        let (asc, desc) = self.ascent_descent(&res);
        let span = spans.len();
        spans.push(SpanInfo {
            color: tb
                .chords
                .color
                .unwrap_or(model::ChordDisplay::DEFAULT_COLOR),
            underline: LineStyle::None,
            strikethrough: LineStyle::None,
            highlight: None,
            shift: 0.0,
            ascent: asc * size,
            descent: desc * size,
            faux_bold: res.faux_bold,
            faux_italic: res.faux_italic,
        });
        let gap = size * 0.35;
        // The chords' x (left edge) per laid-out line, in order.
        let mut per_line: Vec<Vec<(f32, &str)>> = vec![Vec::new(); cl.clusters.len().max(1)];
        let mut sorted: Vec<&model::ChordMark> = p.chords.iter().collect();
        sorted.sort_by_key(|c| c.at);
        let empty = cl.clusters.iter().all(|(_, g)| g.is_empty());
        for c in sorted {
            if empty {
                per_line[0].push((0.0, c.chord.as_str()));
                continue;
            }
            let (li, byte) = locate(cl.display, c.at as usize + cl.marker_chars);
            let mut found = None;
            for (k, (line_i, glyphs)) in cl.clusters.iter().enumerate() {
                if *line_i != li {
                    continue;
                }
                if let Some(g) = glyphs
                    .iter()
                    .find(|g| g.0 <= byte && byte < g.1.max(g.0 + 1))
                {
                    found = Some((k, g.2));
                    break;
                }
                // Past this laid-out line's glyphs: its end, unless a later line of the same
                // buffer line has it.
                if let Some(last) = glyphs.iter().max_by_key(|g| g.1)
                    && byte >= last.1
                {
                    found = Some((k, last.2 + last.3));
                }
            }
            let (k, x) = found.unwrap_or((0, cl.indent_start));
            per_line[k].push((x, c.chord.as_str()));
        }
        // Shape each chord.
        let attrs = Attrs::new()
            .family(Family::Name(&res.family))
            .weight(res.weight)
            .style(to_cosmic_style(res.style))
            .stretch(res.stretch)
            .metrics(Metrics::new(size, row_h))
            .metadata(span);
        let mut shaped: Vec<Vec<(Vec<PlacedGlyph>, f32, f32)>> = Vec::new();
        for row in &per_line {
            let mut out = Vec::new();
            let mut end = f32::NEG_INFINITY;
            for (x, chord) in row {
                let mut buf = Buffer::new(&mut self.fs, Metrics::new(size, row_h));
                buf.set_wrap(Wrap::None);
                buf.set_size(None, None);
                buf.set_text(chord, &attrs, Shaping::Advanced, None);
                buf.shape_until_scroll(&mut self.fs, false);
                let mut glyphs = Vec::new();
                let mut w = 0.0;
                for run in buf.layout_runs() {
                    w = run.line_w;
                    for g in run.glyphs {
                        glyphs.push(PlacedGlyph {
                            font: g.font_id,
                            weight: g.font_weight,
                            glyph: g.glyph_id,
                            size: g.font_size,
                            x: g.x + g.font_size * g.x_offset,
                            y: 0.0,
                            w: g.w,
                            span,
                        });
                    }
                }
                let x0 = if *x < end + gap { end + gap } else { *x };
                end = x0 + w;
                out.push((glyphs, x0, w));
            }
            // A row of chords alone: placed by the paragraph's alignment.
            if empty && !out.is_empty() {
                let total = out.last().map(|(_, x, w)| x + w).unwrap_or(0.0);
                let dx = match cl.align {
                    TextAlign::Right => cl.wrap_w - total,
                    TextAlign::Center | TextAlign::Unknown => (cl.wrap_w - total) * 0.5,
                    _ => 0.0,
                }
                .max(0.0)
                    + cl.indent_start;
                for o in out.iter_mut() {
                    o.1 += dx;
                }
            }
            shaped.push(out);
        }
        let baseline_in_row = (row_h - (asc + desc) * size) * 0.5 + asc * size;
        // An empty line still lays out as one empty line (cosmic-text): chords alone take the
        // chord row instead of a row over a blank line (H06-5).
        if empty || lines.is_empty() {
            lines.clear();
            // Chords alone: one chord row.
            let mut glyphs = Vec::new();
            for (gs, x0, _) in shaped.into_iter().flatten() {
                for mut g in gs {
                    g.x += x0;
                    g.y = baseline_in_row;
                    glyphs.push(g);
                }
            }
            lines.push(LineOut {
                top: 0.0,
                height: row_h,
                glyphs,
            });
            return row_h;
        }
        let mut new_height: f32 = 0.0;
        for (k, line) in lines.iter_mut().enumerate() {
            let push = (k as f32 + 1.0) * row_h;
            let row_top = line.top + k as f32 * row_h;
            for g in line.glyphs.iter_mut() {
                g.y += push;
            }
            if let Some(row) = shaped.get_mut(k) {
                for (gs, x0, _) in row.drain(..) {
                    for mut g in gs {
                        g.x += x0;
                        g.y = row_top + baseline_in_row;
                        line.glyphs.push(g);
                    }
                }
            }
            line.top = row_top;
            line.height += row_h;
            new_height = new_height.max(line.top + line.height);
        }
        new_height.max(height + lines.len() as f32 * row_h)
    }
}

fn block_height(paras: &[ParaOut]) -> f32 {
    let n = paras.len();
    paras
        .iter()
        .enumerate()
        .map(|(i, p)| {
            p.height
                + if i > 0 { p.space_before } else { 0.0 }
                + if i + 1 < n { p.space_after } else { 0.0 }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_of_chords_alone_is_one_chord_row() {
        let mut eng = crate::text_engine(&crate::RendererOptions::default());
        let mut tb = TextBox::plain("Morning light\n");
        tb.default_style.size = Some(60.0);
        tb.chords.show = true;
        tb.paragraphs[0].chords = vec![model::ChordMark::new(0, "G")];
        tb.paragraphs[1].chords = vec![
            model::ChordMark::new(0, "G"),
            model::ChordMark::new(0, "D"),
            model::ChordMark::new(0, "Em"),
        ];
        let (paras, _) = eng.layout_paragraphs(&tb, 1000.0, 1.0, 1.0);
        let (words, alone) = (&paras[0], &paras[1]);
        assert_eq!(alone.lines.len(), 1);
        assert_eq!(alone.lines[0].glyphs.len(), "GDEm".len());
        // A words line is its chord row plus the text line; chords alone only the row.
        assert!(
            alone.height < words.height * 0.6,
            "chords alone {} vs words {}",
            alone.height,
            words.height
        );
    }

    #[test]
    fn case_transforms() {
        let (mut a, mut f) = (true, true);
        assert_eq!(
            transform_case("morning light", Capitalization::AllCaps, &mut a, &mut f),
            "MORNING LIGHT"
        );
        let (mut a, mut f) = (true, true);
        assert_eq!(
            transform_case(
                "the lord of the rings",
                Capitalization::TitleCase,
                &mut a,
                &mut f
            ),
            "The Lord of the Rings"
        );
        let (mut a, mut f) = (true, true);
        assert_eq!(
            transform_case(
                "the lord of the rings",
                Capitalization::StartCase,
                &mut a,
                &mut f
            ),
            "The Lord Of The Rings"
        );
        let (mut a, mut f) = (true, true);
        assert_eq!(
            transform_case("it's", Capitalization::StartCase, &mut a, &mut f),
            "It's"
        );
    }
}
