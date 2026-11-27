//! Edit operations on presentations, playlists and media bins (pure: no I/O, no shared state).
//!
//! [`apply_presentation`] applies a batch of [`PresentationOp`]s to a copy of a presentation:
//! every high-level op is first **expanded** against the current document into primitive ops
//! (`duplicateSlide` → `addSlide`, `splitSlide` → `replaceTextBox` + `addSlide`, `addSlide`
//! without a slide → `addSlide` with the server-made slide, `setSlideText` without an element →
//! the primary element, or `addElement` + `setSlideText` when the slide has no text), then each
//! primitive is applied and yields its inverse. The answer's `applied` lists the primitives and
//! `inverse` undoes the whole batch (each primitive's inverse, in reverse order). Applying
//! `inverse` restores the document exactly (only `modifiedAt` differs).
//!
//! A batch fails as a whole: a target that does not exist (or no longer exists) or a failed
//! precondition (`expectText`) is a conflict (`409`), a malformed op (duplicate ID, unknown
//! style field) is `400`.
//!
//! **Primary text element** of a slide ([`primary_text`]): the first text element with a text
//! placeholder slot, else the largest text box without a data link (the first one on a tie).
//! Reflow edits this element; the control app uses the same rule.

use model::{
    Arrangement, CharStyle, Color, Element, ElementId, MediaBinItem, Paragraph, ParagraphStyle,
    PlaylistItem, PlaylistItemKind, Presentation, Rect, Section, SectionId, Slide, SlideId,
    TemplateSlideId, TextAlign, TextBox, TextRun, Theme, ThemeId, VerticalAlign,
};
use protocol::{ElementPatch, ErrorCode, MediaBinOp, PlaylistOp, PresentationOp as Op, TextRange};
use serde_json::{Map, Value};

/// Why a batch failed.
#[derive(Clone, Debug)]
pub struct EditFailure {
    /// `conflict` (missing target, failed precondition) or `badRequest`.
    pub code: ErrorCode,
    /// What went wrong.
    pub message: String,
    /// Index of the failing op in the batch.
    pub op_index: u32,
    /// For `expectText`: the slide, element and current text.
    pub slide: Option<SlideId>,
    /// See `slide`.
    pub element: Option<ElementId>,
    /// See `slide`.
    pub text: Option<String>,
}

type R<T> = Result<T, Fail>;

/// A failure before the op index is known.
#[derive(Debug)]
struct Fail {
    code: ErrorCode,
    message: String,
    slide: Option<SlideId>,
    element: Option<ElementId>,
    text: Option<String>,
}

fn gone(what: impl Into<String>) -> Fail {
    Fail {
        code: ErrorCode::Conflict,
        message: what.into(),
        slide: None,
        element: None,
        text: None,
    }
}

fn bad(what: impl Into<String>) -> Fail {
    Fail {
        code: ErrorCode::BadRequest,
        ..gone(what)
    }
}

/// A batch as applied, and how to undo it.
#[derive(Clone, Debug)]
pub struct Applied<O> {
    /// Primitive ops in the order applied.
    pub applied: Vec<O>,
    /// Ops that undo the batch, in order.
    pub inverse: Vec<O>,
}

// ---- lookups --------------------------------------------------------------------------------

fn section_index(p: &Presentation, id: SectionId) -> R<usize> {
    p.sections
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| gone(format!("section {} no longer exists", id.0)))
}

fn slide_pos(p: &Presentation, id: SlideId) -> R<(usize, usize)> {
    for (si, s) in p.sections.iter().enumerate() {
        if let Some(k) = s.slides.iter().position(|x| x.id == id) {
            return Ok((si, k));
        }
    }
    Err(gone(format!("slide {} no longer exists", id.0)))
}

fn slide_mut(p: &mut Presentation, id: SlideId) -> R<&mut Slide> {
    let (si, k) = slide_pos(p, id)?;
    Ok(&mut p.sections[si].slides[k])
}

fn element_index(s: &Slide, id: ElementId) -> R<usize> {
    s.elements
        .iter()
        .position(|e| e.id == id)
        .ok_or_else(|| gone(format!("element {} no longer exists", id.0)))
}

fn text_box_mut(p: &mut Presentation, slide: SlideId, element: ElementId) -> R<&mut TextBox> {
    let s = slide_mut(p, slide)?;
    let i = element_index(s, element)?;
    s.elements[i]
        .text_box_mut()
        .ok_or_else(|| bad(format!("element {} is not a text element", element.0)))
}

fn action_index(s: &Slide, id: model::ActionId) -> R<usize> {
    s.actions
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| gone(format!("action {} no longer exists", id.0)))
}

fn slide_exists(p: &Presentation, id: SlideId) -> bool {
    p.slides().any(|s| s.id == id)
}

/// The slide's primary text element (see the module docs).
pub fn primary_text(slide: &Slide) -> Option<usize> {
    if let Some(i) = slide.elements.iter().position(|e| {
        e.text_box().is_some() && e.placeholder.as_ref().is_some_and(|p| p.takes_text())
    }) {
        return Some(i);
    }
    let mut best: Option<(usize, f64)> = None;
    for (i, e) in slide.elements.iter().enumerate() {
        if e.text_box().is_none() || e.data_link.is_some() {
            continue;
        }
        let area = e.bounds.width.abs() * e.bounds.height.abs();
        if best.is_none_or(|(_, a)| area > a) {
            best = Some((i, area));
        }
    }
    best.map(|(i, _)| i)
}

/// The slide before position `(si, k)` in document order (`k` = insertion index).
fn slide_before(p: &Presentation, si: usize, k: usize) -> Option<&Slide> {
    if k > 0 {
        return p.sections[si].slides.get(k - 1);
    }
    p.sections[..si].iter().rev().find_map(|s| s.slides.last())
}

/// An empty text box styled like `tb`: one empty paragraph with the first paragraph's style,
/// whose end style is the first run's style (so typing continues in that style).
fn emptied(tb: &TextBox) -> TextBox {
    let mut out = tb.clone();
    out.paragraphs = tb
        .paragraphs
        .first()
        .map(|p0| {
            vec![Paragraph {
                style: p0.style.clone(),
                runs: Vec::new(),
                end_style: first_run_style(p0),
                chords: Vec::new(),
            }]
        })
        .unwrap_or_default();
    out
}

fn first_run_style(p: &Paragraph) -> CharStyle {
    p.runs
        .first()
        .map(|r| r.style.clone())
        .unwrap_or_else(|| p.end_style.clone())
}

/// A copy of `s` with fresh IDs, empty text, and no label, hotkey, notes or actions.
fn styled_copy(s: &Slide) -> Slide {
    let mut n = s.duplicate();
    n.label = None;
    n.hotkey = None;
    n.notes = Default::default();
    n.actions.clear();
    n.import = None;
    for e in &mut n.elements {
        if let Some(tb) = e.text_box_mut() {
            *tb = emptied(tb);
        }
        e.import = None;
    }
    n
}

/// The default text element for a canvas: centred white text, 5 % inset.
pub fn default_text_element(canvas: model::Size) -> Element {
    let (w, h) = (canvas.width.max(64.0), canvas.height.max(64.0));
    let mut tb = TextBox {
        default_style: CharStyle {
            font_family: Some("Arial".into()),
            font_weight: Some(400),
            size: Some((h * 0.074).round()),
            color: Some(Color::WHITE),
            ..CharStyle::default()
        },
        vertical_align: VerticalAlign::Middle,
        ..TextBox::default()
    };
    tb.default_paragraph.alignment = Some(TextAlign::Center);
    let mut e = Element::text(Rect::new(w * 0.05, h * 0.05, w * 0.9, h * 0.9), tb);
    e.name = "Text".into();
    e
}

/// A text slide in the presentation's style: a styled copy of the slide before `(si, k)`, else
/// of the first slide with text, else a default text slide.
fn default_slide(p: &Presentation, si: usize, k: usize) -> Slide {
    let template = slide_before(p, si, k)
        .filter(|s| primary_text(s).is_some())
        .or_else(|| p.slides().find(|s| primary_text(s).is_some()));
    match template {
        Some(t) => styled_copy(t),
        None => Slide {
            elements: vec![default_text_element(p.canvas_size)],
            ..Slide::new()
        },
    }
}

/// A new text element in the presentation's prevailing style for slide `id` (which has none):
/// the primary text element of the nearest slide before it that has one, else after it, else
/// the default.
fn prevailing_text_element(p: &Presentation, id: SlideId) -> Element {
    let slides: Vec<&Slide> = p.slides().collect();
    let at = slides.iter().position(|s| s.id == id).unwrap_or(0);
    let near = slides[..at]
        .iter()
        .rev()
        .chain(slides[at..].iter())
        .find_map(|s| primary_text(s).map(|i| &s.elements[i]));
    match near {
        Some(e) => {
            let mut n = e.duplicate();
            if let Some(tb) = n.text_box_mut() {
                *tb = emptied(tb);
            }
            n.import = None;
            n
        }
        None => default_text_element(p.canvas_size),
    }
}

// ---- text -----------------------------------------------------------------------------------

/// Replaces the plain text of a box (see `PresentationOp::SetSlideText` for the rules).
///
/// Old and new lines are matched by content: the unchanged lines at the start and at the end
/// keep their own paragraphs (and chords); the changed lines between them replace the old
/// lines between, in order. Chords there follow their characters across the whole changed
/// block, so a line split with Enter keeps each chord on the half that holds its word, and an
/// inserted line has none (H06-1).
pub fn set_text(tb: &mut TextBox, text: &str) {
    let old = std::mem::take(&mut tb.paragraphs);
    let last = old.last().cloned().unwrap_or_default();
    let new: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let old_text: Vec<String> = old.iter().map(Paragraph::plain_text).collect();
    let same = |i: usize, j: usize| old_text[i] == new[j];
    let head = (0..old.len().min(new.len()))
        .take_while(|&i| same(i, i))
        .count();
    let tail = (0..(old.len().min(new.len()) - head))
        .take_while(|&k| same(old.len() - 1 - k, new.len() - 1 - k))
        .count();
    let (old_mid, new_mid) = (head..old.len() - tail, head..new.len() - tail);
    let chords = mid_chords(&old[old_mid.clone()], &new[new_mid.clone()]);
    let mut out: Vec<Paragraph> = old[..head].to_vec();
    for (j, (line, chords)) in new[new_mid].iter().zip(chords).enumerate() {
        // Style: the old line it replaces, else the last replaced one, else its neighbours.
        let base = old
            .get(old_mid.start + j)
            .filter(|_| old_mid.start + j < old_mid.end)
            .or_else(|| old_mid.clone().last().map(|i| &old[i]))
            .or_else(|| old.get(old_mid.end))
            .or_else(|| head.checked_sub(1).map(|i| &old[i]))
            .unwrap_or(&last);
        out.push(Paragraph {
            style: base.style.clone(),
            runs: if line.is_empty() {
                Vec::new()
            } else {
                vec![TextRun {
                    text: line.to_string(),
                    style: first_run_style(base),
                }]
            },
            end_style: base.end_style.clone(),
            chords,
        });
    }
    out.extend_from_slice(&old[old.len() - tail..]);
    tb.paragraphs = out;
}

/// The chords of the changed lines `new` that replace the paragraphs `old`: one line for one
/// line remaps that line's chords; otherwise the block is remapped as one text (lines joined
/// by `\n`) and each chord goes to the new line holding its character.
fn mid_chords(old: &[Paragraph], new: &[&str]) -> Vec<Vec<model::ChordMark>> {
    if old.len() == new.len() {
        return old
            .iter()
            .zip(new)
            .map(|(o, line)| model::chords::remap_chords(&o.plain_text(), line, &o.chords))
            .collect();
    }
    let mut joined = String::new();
    let mut marks: Vec<model::ChordMark> = Vec::new();
    let mut at = 0usize;
    for (i, o) in old.iter().enumerate() {
        if i > 0 {
            joined.push('\n');
            at += 1;
        }
        let t = o.plain_text();
        let len = t.chars().count();
        marks.extend(o.chords.iter().map(|c| model::ChordMark {
            at: (at + (c.at as usize).min(len)) as u32,
            chord: c.chord.clone(),
        }));
        joined.push_str(&t);
        at += len;
    }
    let next = new.join("\n");
    let moved = model::chords::remap_chords(&joined, &next, &marks);
    let mut out: Vec<Vec<model::ChordMark>> = vec![Vec::new(); new.len()];
    let mut start = 0usize;
    for (j, line) in new.iter().enumerate() {
        let len = line.chars().count();
        for c in &moved {
            let x = c.at as usize;
            // A line holds its characters and its end (the `\n` after it).
            if x >= start && x <= start + len {
                out[j].push(model::ChordMark {
                    at: (x - start) as u32,
                    chord: c.chord.clone(),
                });
            }
        }
        start += len + 1;
    }
    out
}

const CHAR_FIELDS: &[&str] = &[
    "fontFamily",
    "fontWeight",
    "italic",
    "postscriptName",
    "size",
    "color",
    "underline",
    "strikethrough",
    "baseline",
    "capitalization",
    "letterSpacing",
    "highlight",
];
const PARAGRAPH_FIELDS: &[&str] = &[
    "alignment",
    "lineHeightMultiple",
    "lineSpacing",
    "spaceBefore",
    "spaceAfter",
    "indentStart",
    "indentEnd",
    "indentFirstLine",
    "list",
];

fn to_map<T: serde::Serialize>(v: &T) -> Map<String, Value> {
    match serde_json::to_value(v) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn patch_char(style: &mut CharStyle, set: Option<&CharStyle>, clear: &[String]) -> R<()> {
    let mut m = to_map(style);
    if let Some(set) = set {
        let s = to_map(set);
        let face = ["fontFamily", "fontWeight", "italic"]
            .iter()
            .any(|k| s.contains_key(*k))
            && !s.contains_key("postscriptName");
        for (k, v) in s {
            m.insert(k, v);
        }
        if face {
            m.remove("postscriptName");
        }
    }
    for k in clear.iter().filter(|k| CHAR_FIELDS.contains(&k.as_str())) {
        m.remove(k);
    }
    *style = serde_json::from_value(Value::Object(m)).map_err(|e| bad(e.to_string()))?;
    Ok(())
}

fn patch_paragraph(
    style: &mut ParagraphStyle,
    set: Option<&ParagraphStyle>,
    clear: &[String],
) -> R<()> {
    let mut m = to_map(style);
    if let Some(set) = set {
        m.extend(to_map(set));
    }
    for k in clear
        .iter()
        .filter(|k| PARAGRAPH_FIELDS.contains(&k.as_str()))
    {
        m.remove(k);
    }
    *style = serde_json::from_value(Value::Object(m)).map_err(|e| bad(e.to_string()))?;
    Ok(())
}

/// Splits `p`'s runs so that local character offsets `a` and `b` fall on run boundaries.
fn split_runs(p: &mut Paragraph, cuts: &[usize]) {
    let mut out = Vec::new();
    let mut pos = 0usize;
    for r in std::mem::take(&mut p.runs) {
        let chars: Vec<char> = r.text.chars().collect();
        let (rs, re) = (pos, pos + chars.len());
        let mut start = 0;
        for &c in cuts {
            if c > rs && c < re {
                out.push(TextRun {
                    text: chars[start..c - rs].iter().collect(),
                    style: r.style.clone(),
                });
                start = c - rs;
            }
        }
        out.push(TextRun {
            text: chars[start..].iter().collect(),
            style: r.style.clone(),
        });
        pos = re;
    }
    p.runs = out;
}

/// Applies a character / paragraph style patch to the characters in `range`.
fn style_range(
    tb: &mut TextBox,
    range: TextRange,
    character: Option<&CharStyle>,
    paragraph: Option<&ParagraphStyle>,
    clear: &[String],
) -> R<()> {
    let (start, end) = (range.start as usize, range.end.max(range.start) as usize);
    let mut ps = 0usize;
    for p in &mut tb.paragraphs {
        let len = p.plain_text().chars().count();
        let pe = ps + len;
        let touched = start <= pe && end >= ps && (end > ps || start == ps);
        if touched {
            patch_paragraph(&mut p.style, paragraph, clear)?;
            let (a, b) = (start.max(ps) - ps, end.min(pe) - ps);
            if a < b {
                split_runs(p, &[a, b]);
                let mut pos = 0;
                for r in &mut p.runs {
                    let n = r.text.chars().count();
                    if pos >= a && pos + n <= b && n > 0 {
                        patch_char(&mut r.style, character, clear)?;
                    }
                    pos += n;
                }
                // Merge neighbours that ended up with the same style.
                let mut merged: Vec<TextRun> = Vec::new();
                for r in std::mem::take(&mut p.runs) {
                    match merged.last_mut() {
                        Some(l) if l.style == r.style => l.text.push_str(&r.text),
                        _ => merged.push(r),
                    }
                }
                p.runs = merged;
            } else if len == 0 {
                patch_char(&mut p.end_style, character, clear)?;
            }
        }
        ps = pe + 1;
    }
    Ok(())
}

fn check_clear(clear: &[String]) -> R<()> {
    for k in clear {
        if !CHAR_FIELDS.contains(&k.as_str()) && !PARAGRAPH_FIELDS.contains(&k.as_str()) {
            return Err(bad(format!("unknown style field {k:?} in clear")));
        }
    }
    Ok(())
}

// ---- presentation ops -----------------------------------------------------------------------

/// Applies a batch to `p` (a working copy: on error it may be half-changed, discard it).
pub fn apply_presentation(p: &mut Presentation, ops: &[Op]) -> Result<Applied<Op>, EditFailure> {
    apply_presentation_with(p, ops, None)
}

/// The collection's themes, for the theme ops and the overrides of linked slides.
pub type Themes = std::collections::HashMap<ThemeId, Theme>;

/// [`apply_presentation`] with the collection's themes: the theme ops (`applyTheme`,
/// `resetToTheme`, `unlinkTheme`, `linkTheme`) are expanded into `replaceSlide`, and after the batch every
/// changed slide that follows its theme gets its overrides recorded
/// ([`model::theme_link::refresh_overrides`]), as one more `replaceSlide` each. Without themes
/// the theme ops are refused (`400`).
pub fn apply_presentation_with(
    p: &mut Presentation,
    ops: &[Op],
    themes: Option<&Themes>,
) -> Result<Applied<Op>, EditFailure> {
    let mut out = Applied {
        applied: Vec::new(),
        inverse: Vec::new(),
    };
    let before = themes.map(|_| p.clone());
    let mut inverses: Vec<Vec<Op>> = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let fail = |f: Fail| EditFailure {
            code: f.code,
            message: format!("edit refused (op {i}, {}): {}", op.name(), f.message),
            op_index: i as u32,
            slide: f.slide,
            element: f.element,
            text: f.text,
        };
        let prims = match op {
            Op::ApplyTheme { .. }
            | Op::ResetToTheme { .. }
            | Op::UnlinkTheme { .. }
            | Op::LinkTheme { .. } => expand_theme(p, op, themes).map_err(fail)?,
            _ => expand(p, op).map_err(fail)?,
        };
        for prim in prims {
            let inv = apply_primitive(p, &prim).map_err(fail)?;
            out.applied.push(prim);
            inverses.push(inv);
        }
    }
    if let (Some(themes), Some(before)) = (themes, before) {
        // Overrides of the linked slides the batch changed.
        let mut refreshed = Vec::new();
        for s in p.slides() {
            let Some(t) = linked_template(s, themes) else {
                continue;
            };
            if before.slides().any(|b| b == s) {
                continue;
            }
            let r = model::theme_link::refresh_overrides(s, t);
            if r != *s {
                refreshed.push((r, s.clone()));
            }
        }
        for (new, old) in refreshed {
            let op = Op::ReplaceSlide { slide: new };
            apply_primitive(p, &op).map_err(|f| EditFailure {
                code: f.code,
                message: f.message,
                op_index: ops.len().saturating_sub(1) as u32,
                slide: None,
                element: None,
                text: None,
            })?;
            out.applied.push(op);
            inverses.push(vec![Op::ReplaceSlide { slide: old }]);
        }
    }
    out.inverse = inverses.into_iter().rev().flatten().collect();
    Ok(out)
}

/// The template slide `s` follows, when its theme is one of `themes`.
pub fn linked_template<'a>(s: &Slide, themes: &'a Themes) -> Option<&'a Slide> {
    let r = s.linked_template()?;
    themes.get(&r.theme.id)?.template(r.slide).map(|t| &t.slide)
}

/// The template slide of `theme` a slide gets when the theme is applied without one: the
/// template of the same name as the slide's current one, else the theme's first.
pub fn matching_template(s: &Slide, theme: &Theme, themes: &Themes) -> Option<TemplateSlideId> {
    let current = s.template_origin.as_ref().and_then(|r| {
        if r.theme.id == theme.id {
            return theme.template(r.slide).map(|t| t.name.clone());
        }
        themes
            .get(&r.theme.id)
            .and_then(|th| th.template(r.slide))
            .map(|t| t.name.clone())
    });
    current
        .and_then(|n| {
            theme
                .slides
                .iter()
                .find(|t| !n.is_empty() && t.name.eq_ignore_ascii_case(&n))
        })
        .or(theme.slides.first())
        .map(|t| t.id)
}

/// Expands a theme op into `replaceSlide` primitives.
fn expand_theme(p: &Presentation, op: &Op, themes: Option<&Themes>) -> R<Vec<Op>> {
    let themes = themes.ok_or_else(|| bad("themes can't be applied to this document"))?;
    let targets = |ids: &[SlideId]| -> R<Vec<&Slide>> {
        if ids.is_empty() {
            return Ok(p.slides().collect());
        }
        ids.iter()
            .map(|id| {
                let (si, k) = slide_pos(p, *id)?;
                Ok(&p.sections[si].slides[k])
            })
            .collect()
    };
    let mut out = Vec::new();
    match op {
        Op::ApplyTheme {
            theme,
            template,
            slides,
        } => {
            let th = themes
                .get(theme)
                .ok_or_else(|| gone(format!("theme {} no longer exists", theme.0)))?;
            if let Some(t) = template
                && th.template(*t).is_none()
            {
                return Err(gone(format!("template slide {} is not in that theme", t.0)));
            }
            for s in targets(slides)? {
                let Some(tid) = template.or_else(|| matching_template(s, th, themes)) else {
                    return Err(bad("the theme has no template slides"));
                };
                if let Some(new) = model::theme_link::link(s, th, tid)
                    && new != *s
                {
                    out.push(Op::ReplaceSlide { slide: new });
                }
            }
        }
        Op::ResetToTheme {
            slide,
            element,
            properties,
        } => {
            let (si, k) = slide_pos(p, *slide)?;
            let s = &p.sections[si].slides[k];
            let t = linked_template(s, themes)
                .ok_or_else(|| bad(format!("slide {} doesn't follow a theme", slide.0)))?;
            if let Some(e) = element {
                element_index(s, *e)?;
            }
            let new = model::theme_link::reset(s, t, *element, properties.as_deref());
            if new != *s {
                out.push(Op::ReplaceSlide { slide: new });
            }
        }
        Op::LinkTheme { theme, slides } => {
            if let Some(t) = theme
                && !themes.contains_key(t)
            {
                return Err(gone(format!("theme {} no longer exists", t.0)));
            }
            // In a stable order, so the closest-theme choice doesn't depend on the map.
            let mut all: Vec<&Theme> = themes
                .values()
                .filter(|t| theme.is_none_or(|x| t.id == x))
                .collect();
            all.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.id.0.cmp(&b.id.0)));
            for s in targets(slides)? {
                if s.theme_link.is_none()
                    && let Some(k) = model::theme_link::link_to_closest(s, all.iter().copied())
                {
                    out.push(Op::ReplaceSlide { slide: k.slide });
                }
            }
        }
        Op::UnlinkTheme { slides } => {
            for s in targets(slides)? {
                if s.theme_link.is_some() {
                    out.push(Op::ReplaceSlide {
                        slide: model::theme_link::unlink(s),
                    });
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

/// Expands a high-level op into primitives against the current document.
fn expand(p: &Presentation, op: &Op) -> R<Vec<Op>> {
    Ok(match op {
        Op::DuplicateSlide { slide } => {
            let (si, k) = slide_pos(p, *slide)?;
            vec![Op::AddSlide {
                section: p.sections[si].id,
                after: Some(*slide),
                slide: Some(p.sections[si].slides[k].duplicate()),
            }]
        }
        Op::AddSlide {
            section,
            after,
            slide: None,
        } => {
            let si = section_index(p, *section)?;
            let k = match after {
                Some(a) => {
                    p.sections[si]
                        .slides
                        .iter()
                        .position(|s| s.id == *a)
                        .ok_or_else(|| gone(format!("slide {} is not in that section", a.0)))?
                        + 1
                }
                None => 0,
            };
            vec![Op::AddSlide {
                section: *section,
                after: *after,
                slide: Some(default_slide(p, si, k)),
            }]
        }
        Op::SetSlideText {
            slide,
            element: None,
            text,
            expect_text,
        } => {
            let (si, k) = slide_pos(p, *slide)?;
            let s = &p.sections[si].slides[k];
            match primary_text(s) {
                Some(i) => vec![Op::SetSlideText {
                    slide: *slide,
                    element: Some(s.elements[i].id),
                    text: text.clone(),
                    expect_text: expect_text.clone(),
                }],
                None => {
                    if let Some(e) = expect_text.as_deref().filter(|e| !e.is_empty()) {
                        return Err(Fail {
                            text: Some(String::new()),
                            slide: Some(*slide),
                            ..gone(format!("the slide's text changed (expected {e:?})"))
                        });
                    }
                    let el = prevailing_text_element(p, *slide);
                    let id = el.id;
                    vec![
                        Op::AddElement {
                            slide: *slide,
                            element: el,
                            index: None,
                        },
                        Op::SetSlideText {
                            slide: *slide,
                            element: Some(id),
                            text: text.clone(),
                            expect_text: None,
                        },
                    ]
                }
            }
        }
        Op::SplitSlide {
            slide,
            at_paragraph,
        } => {
            let (si, k) = slide_pos(p, *slide)?;
            let s = &p.sections[si].slides[k];
            let section = p.sections[si].id;
            match primary_text(s) {
                None => vec![Op::AddSlide {
                    section,
                    after: Some(*slide),
                    slide: Some(default_slide(p, si, k + 1)),
                }],
                Some(ei) => {
                    let tb = s.elements[ei].text_box().cloned().unwrap_or_default();
                    let at = (*at_paragraph as usize).min(tb.paragraphs.len());
                    let mut first = tb.clone();
                    first.paragraphs.truncate(at);
                    let mut new = s.duplicate();
                    new.label = None;
                    new.hotkey = None;
                    new.notes = Default::default();
                    new.actions.clear();
                    new.import = None;
                    if let Some(t) = new.elements[ei].text_box_mut() {
                        t.paragraphs = tb.paragraphs[at..].to_vec();
                    }
                    let mut ops = vec![Op::ReplaceTextBox {
                        slide: *slide,
                        element: s.elements[ei].id,
                        text_box: first,
                    }];
                    // The second-language text splits with it: each slide keeps its part
                    // (H07-3).
                    let (eid, nid) = (s.elements[ei].id, new.elements[ei].id);
                    if let Some(tr) = s.translation.as_ref()
                        && let Some(t) = tr.text_of(eid)
                    {
                        let m = t.paragraphs.len();
                        let n = tb.paragraphs.len().max(1);
                        let cut = ((at * m + n / 2) / n).min(m);
                        let mut kept = tr.clone();
                        set_translation_paragraphs(&mut kept, eid, t.paragraphs[..cut].to_vec());
                        ops.push(Op::SetSlideTranslation {
                            slide: *slide,
                            translation: Some(kept),
                        });
                        if let Some(nt) = new.translation.as_mut() {
                            set_translation_paragraphs(nt, nid, t.paragraphs[cut..].to_vec());
                        }
                    }
                    ops.push(Op::AddSlide {
                        section,
                        after: Some(*slide),
                        slide: Some(new),
                    });
                    ops
                }
            }
        }
        Op::MergeSlides { first, second } => {
            if first == second {
                return Err(bad("cannot merge a slide with itself"));
            }
            let (fs, fk) = slide_pos(p, *first)?;
            let (ss, sk) = slide_pos(p, *second)?;
            let a = &p.sections[fs].slides[fk];
            let b = &p.sections[ss].slides[sk];
            let mut ops = Vec::new();
            if let Some(bi) = primary_text(b) {
                let btb = b.elements[bi].text_box().cloned().unwrap_or_default();
                let into = match primary_text(a) {
                    Some(ai) => {
                        let mut tb = a.elements[ai].text_box().cloned().unwrap_or_default();
                        tb.paragraphs.extend(btb.paragraphs);
                        ops.push(Op::ReplaceTextBox {
                            slide: *first,
                            element: a.elements[ai].id,
                            text_box: tb,
                        });
                        a.elements[ai].id
                    }
                    None => {
                        let element = b.elements[bi].duplicate();
                        let id = element.id;
                        ops.push(Op::AddElement {
                            slide: *first,
                            element,
                            index: None,
                        });
                        id
                    }
                };
                // The second slide's second-language text is appended too (H07-3, R3-1).
                if let Some(bt) = b
                    .translation
                    .as_ref()
                    .and_then(|t| t.text_of(b.elements[bi].id))
                    .filter(|t| !t.paragraphs.is_empty())
                {
                    let mut tr = a
                        .translation
                        .clone()
                        .unwrap_or_else(|| model::SlideTranslation {
                            texts: Vec::new(),
                            ..b.translation.clone().unwrap_or_default()
                        });
                    let mut paragraphs = tr
                        .text_of(into)
                        .map(|t| t.paragraphs.clone())
                        .unwrap_or_default();
                    paragraphs.extend(bt.paragraphs.iter().cloned());
                    set_translation_paragraphs(&mut tr, into, paragraphs);
                    ops.push(Op::SetSlideTranslation {
                        slide: *first,
                        translation: Some(tr),
                    });
                }
            }
            ops.push(Op::DeleteSlide { slide: *second });
            ops
        }
        Op::SetTextStyle {
            slide,
            element,
            range: Some(range),
            character,
            paragraph,
            clear,
        } => {
            check_clear(clear)?;
            let (si, k) = slide_pos(p, *slide)?;
            let s = &p.sections[si].slides[k];
            let ei = element_index(s, *element)?;
            let mut tb = s.elements[ei]
                .text_box()
                .cloned()
                .ok_or_else(|| bad(format!("element {} is not a text element", element.0)))?;
            style_range(
                &mut tb,
                *range,
                character.as_ref(),
                paragraph.as_ref(),
                clear,
            )?;
            vec![Op::ReplaceTextBox {
                slide: *slide,
                element: *element,
                text_box: tb,
            }]
        }
        Op::ApplyChordPro { text } => apply_chordpro(p, text)?,
        other => vec![other.clone()],
    })
}

/// Sets `element`'s second-text paragraphs in `tr` (in place; none removes its entry).
fn set_translation_paragraphs(
    tr: &mut model::SlideTranslation,
    element: ElementId,
    paragraphs: Vec<Paragraph>,
) {
    match tr.texts.iter().position(|t| t.element == element) {
        Some(i) if paragraphs.is_empty() => {
            tr.texts.remove(i);
        }
        Some(i) => tr.texts[i].paragraphs = paragraphs,
        None if paragraphs.is_empty() => {}
        None => tr.texts.push(model::ElementTranslation {
            element,
            paragraphs,
        }),
    }
}

/// A lyric line's words for matching (lower case, letters and digits, single spaces).
fn line_key(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Largest ChordPro text accepted, by `applyChordPro` and `POST /chordpro/parse` (a long song
/// is a few KB).
pub(crate) const MAX_CHORDPRO_TEXT: usize = 1 << 20;

/// `applyChordPro` (see `PresentationOp::ApplyChordPro`).
fn apply_chordpro(p: &Presentation, text: &str) -> R<Vec<Op>> {
    if text.len() > MAX_CHORDPRO_TEXT {
        return Err(bad("the ChordPro text is too long"));
    }
    let song = model::chordpro::parse(text);
    // Each distinct line's chords, the first one wins (a map: no quadratic scan, H06-3).
    let mut lines: std::collections::HashMap<String, &Paragraph> = std::collections::HashMap::new();
    for sec in &song.sections {
        for para in sec.stanzas.iter().flatten() {
            let k = line_key(&para.plain_text());
            if !k.is_empty() && para.has_chords() {
                lines.entry(k).or_insert(para);
            }
        }
    }
    if lines.is_empty() {
        return Err(bad("the ChordPro text has no lines with chords"));
    }
    let mut ops = Vec::new();
    let mut matched = 0;
    for s in p.slides() {
        let Some(i) = primary_text(s) else { continue };
        let e = &s.elements[i];
        let Some(tb) = e.text_box() else { continue };
        let mut new = tb.clone();
        for para in new.paragraphs.iter_mut() {
            let words = para.plain_text();
            let k = line_key(&words);
            if let Some(src) = lines.get(&k) {
                matched += 1;
                para.chords = model::chords::remap_chords(&src.plain_text(), &words, &src.chords);
            }
        }
        if new != *tb {
            ops.push(Op::ReplaceTextBox {
                slide: s.id,
                element: e.id,
                text_box: new,
            });
        }
    }
    if matched == 0 {
        return Err(bad("no line of the ChordPro text matches the song's words"));
    }
    if let Some(key) = song.key.filter(|_| {
        p.song_info
            .as_ref()
            .is_none_or(|i| i.original_key.is_none())
    }) {
        let mut info = p.song_info.clone().unwrap_or_default();
        info.original_key = Some(key);
        ops.push(Op::SetPresentation {
            name: None,
            song_info: Some(Some(info)),
            scripture: None,
            transition: None,
            canvas_size: None,
            notes: None,
            category: None,
            language: None,
        });
    }
    Ok(ops)
}

/// A presentation made from a ChordPro song (`POST /chordpro/parse`): its metadata, one
/// section per ChordPro section (linked to the global group of that name, coloured like it),
/// a slide per stanza (or per `lines_per_slide` lines) whose text element is styled like
/// `like`'s text (else the default text element), with the stanza's second-language lines as
/// its second text (a two-language file, `model::chordpro::parse_two`), an arrangement when
/// the song repeats sections.
pub fn presentation_from_chordpro(
    song: &model::chordpro::ChordProSong,
    name: Option<&str>,
    lines_per_slide: usize,
    like: Option<&Presentation>,
    groups: &[model::GlobalGroup],
) -> Presentation {
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or_else(|| song.title.clone())
        .unwrap_or_else(|| "Untitled".into());
    let mut p = Presentation::new(name);
    if let Some(l) = like {
        p.canvas_size = l.canvas_size;
    }
    p.song_info = song.song_info();
    p.language = song.language.clone();
    let template = match like {
        Some(l) if l.slides().any(|s| primary_text(s).is_some()) => {
            let id = l.slides().find(|s| primary_text(s).is_some()).map(|s| s.id);
            id.map(|id| prevailing_text_element(l, id))
                .unwrap_or_else(|| default_text_element(p.canvas_size))
        }
        _ => default_text_element(p.canvas_size),
    };
    let mut section_ids = Vec::new();
    for (sec, slides) in song.sections.iter().zip(song.slides_two(lines_per_slide)) {
        let mut section = match groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(sec.name.trim()))
        {
            Some(g) => Section::from_global(g),
            None => Section::new(sec.name.clone()),
        };
        for (paras, second) in slides {
            let mut el = template.duplicate();
            if let Some(tb) = el.text_box_mut() {
                let base = emptied(tb)
                    .paragraphs
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                // A stanza of second-language lines only: an empty first text.
                let paras = if paras.is_empty() {
                    vec![Paragraph::default()]
                } else {
                    paras
                };
                tb.paragraphs = paras
                    .into_iter()
                    .map(|para| Paragraph {
                        runs: para
                            .runs
                            .into_iter()
                            .map(|r| TextRun {
                                text: r.text,
                                style: base.end_style.clone(),
                            })
                            .collect(),
                        chords: para.chords,
                        ..base.clone()
                    })
                    .collect();
            }
            let mut slide = Slide::new();
            if !second.is_empty() {
                slide.translation = Some(model::SlideTranslation {
                    language: song.second_language.clone().unwrap_or_default(),
                    texts: vec![model::ElementTranslation {
                        element: el.id,
                        paragraphs: second
                            .iter()
                            .map(|l| Paragraph {
                                runs: vec![TextRun::plain(l.as_str())],
                                ..Paragraph::default()
                            })
                            .collect(),
                    }],
                    ..model::SlideTranslation::default()
                });
            }
            slide.elements.push(el);
            section.slides.push(slide);
        }
        section_ids.push(section.id);
        p.sections.push(section);
    }
    if song.has_repeats() {
        let arr = Arrangement {
            id: model::ArrangementId::new(),
            name: "ChordPro".into(),
            sections: song.order.iter().map(|i| section_ids[*i]).collect(),
        };
        p.selected_arrangement = Some(arr.id);
        p.arrangements.push(arr);
    }
    p
}

/// Most builds on one slide (an object has one or two; a deleted object's builds stay).
const MAX_BUILDS: usize = 1000;

/// Fastest scrolling text, canvas px per second (Oriel offers up to 2000).
const MAX_SCROLL_SPEED: f64 = 10_000.0;
/// Widest gap between scrolling copies, canvas px (Oriel offers up to 4000).
const MAX_SCROLL_GAP: f64 = 50_000.0;

/// A scrolling text setting the renderer can draw (finite, in a sane range).
fn check_scroll(sc: &model::TextScroll) -> R<()> {
    if !(0.0..=MAX_SCROLL_SPEED).contains(&sc.speed) {
        return Err(bad(format!(
            "the scrolling speed must be between 0 and {MAX_SCROLL_SPEED} px/s"
        )));
    }
    if !(0.0..=MAX_SCROLL_GAP).contains(&sc.gap) {
        return Err(bad(format!(
            "the scrolling gap must be between 0 and {MAX_SCROLL_GAP} px"
        )));
    }
    Ok(())
}

fn swap<T: Clone>(slot: &mut T, new: &Option<T>) -> Option<T> {
    new.as_ref().map(|v| std::mem::replace(slot, v.clone()))
}

fn check_new_ids(p: &Presentation, slides: &[&Slide], sections: &[SectionId]) -> R<()> {
    for id in sections {
        if p.sections.iter().any(|s| s.id == *id) {
            return Err(bad(format!("section {} already exists", id.0)));
        }
    }
    for s in slides {
        if slide_exists(p, s.id) {
            return Err(bad(format!("slide {} already exists", s.id.0)));
        }
    }
    Ok(())
}

/// Applies one primitive op; returns the ops that undo it.
fn apply_primitive(p: &mut Presentation, op: &Op) -> R<Vec<Op>> {
    Ok(match op {
        Op::SetPresentation {
            name,
            song_info,
            scripture,
            transition,
            canvas_size,
            notes,
            category,
            language,
        } => {
            if let Some(n) = name {
                if n.trim().is_empty() {
                    return Err(bad("a presentation needs a name"));
                }
                // The rule creating and renaming apply (so a copy's name fits too).
                if !crate::content::name_ok(n.trim()) {
                    return Err(bad(format!(
                        "names are at most {} characters, without control characters",
                        crate::content::MAX_NAME
                    )));
                }
            }
            if let Some(c) = canvas_size
                && (c.width < 1.0
                    || c.height < 1.0
                    || !c.width.is_finite()
                    || !c.height.is_finite())
            {
                return Err(bad("the canvas size must be positive"));
            }
            let language = match language {
                Some(Some(code)) => Some(Some(model::language_code(code).ok_or_else(|| {
                    bad(format!(
                        "{code:?} is not a language code (such as en, es or pt-BR)"
                    ))
                })?)),
                other => other.clone(),
            };
            vec![Op::SetPresentation {
                name: swap(&mut p.name, &name.as_ref().map(|n| n.trim().to_string())),
                song_info: swap(&mut p.song_info, song_info),
                scripture: swap(&mut p.scripture, scripture),
                transition: swap(&mut p.transition, transition),
                canvas_size: swap(&mut p.canvas_size, canvas_size),
                notes: swap(&mut p.notes, notes),
                category: swap(&mut p.category, category),
                language: swap(&mut p.language, &language),
            }]
        }
        Op::AddSection { after, section } => {
            let slides: Vec<&Slide> = section.slides.iter().collect();
            check_new_ids(p, &slides, &[section.id])?;
            let at = match after {
                Some(a) => section_index(p, *a)? + 1,
                None => 0,
            };
            p.sections.insert(at, section.clone());
            vec![Op::DeleteSection {
                section: section.id,
            }]
        }
        Op::DeleteSection { section } => {
            let i = section_index(p, *section)?;
            let after = i.checked_sub(1).map(|j| p.sections[j].id);
            let removed = p.sections.remove(i);
            vec![Op::AddSection {
                after,
                section: removed,
            }]
        }
        Op::MoveSection { section, after } => {
            if after == &Some(*section) {
                return Err(bad("a section cannot move after itself"));
            }
            let i = section_index(p, *section)?;
            let old_after = i.checked_sub(1).map(|j| p.sections[j].id);
            if let Some(a) = after {
                section_index(p, *a)?;
            }
            let s = p.sections.remove(i);
            let at = match after {
                Some(a) => section_index(p, *a)? + 1,
                None => 0,
            };
            p.sections.insert(at, s);
            vec![Op::MoveSection {
                section: *section,
                after: old_after,
            }]
        }
        Op::SetSection {
            section,
            name,
            color,
            hotkey,
            global_group,
        } => {
            let i = section_index(p, *section)?;
            let s = &mut p.sections[i];
            vec![Op::SetSection {
                section: *section,
                name: swap(&mut s.name, name),
                color: swap(&mut s.color, color),
                hotkey: swap(&mut s.hotkey, hotkey),
                global_group: swap(&mut s.global_group, global_group),
            }]
        }
        Op::AddSlide {
            section,
            after,
            slide,
        } => {
            let slide = slide
                .as_ref()
                .ok_or_else(|| bad("addSlide needs a slide here"))?;
            check_new_ids(p, &[slide], &[])?;
            let si = section_index(p, *section)?;
            let k = match after {
                Some(a) => {
                    p.sections[si]
                        .slides
                        .iter()
                        .position(|s| s.id == *a)
                        .ok_or_else(|| gone(format!("slide {} is not in that section", a.0)))?
                        + 1
                }
                None => 0,
            };
            p.sections[si].slides.insert(k, slide.clone());
            vec![Op::DeleteSlide { slide: slide.id }]
        }
        Op::DeleteSlide { slide } => {
            let (si, k) = slide_pos(p, *slide)?;
            let after = k.checked_sub(1).map(|j| p.sections[si].slides[j].id);
            let removed = p.sections[si].slides.remove(k);
            vec![Op::AddSlide {
                section: p.sections[si].id,
                after,
                slide: Some(removed),
            }]
        }
        Op::MoveSlide {
            slide,
            to_section,
            after,
        } => {
            if after == &Some(*slide) {
                return Err(bad("a slide cannot move after itself"));
            }
            let (si, k) = slide_pos(p, *slide)?;
            let old = (
                p.sections[si].id,
                k.checked_sub(1).map(|j| p.sections[si].slides[j].id),
            );
            let ti = section_index(p, *to_section)?;
            if let Some(a) = after
                && !p.sections[ti].slides.iter().any(|s| s.id == *a)
            {
                return Err(gone(format!("slide {} is not in that section", a.0)));
            }
            let s = p.sections[si].slides.remove(k);
            let at = match after {
                Some(a) => {
                    p.sections[ti]
                        .slides
                        .iter()
                        .position(|s| s.id == *a)
                        .unwrap_or(0)
                        + 1
                }
                None => 0,
            };
            p.sections[ti].slides.insert(at, s);
            vec![Op::MoveSlide {
                slide: *slide,
                to_section: old.0,
                after: old.1,
            }]
        }
        Op::SetSlide {
            slide,
            label,
            enabled,
            notes,
            transition,
            background,
            hotkey,
            builds,
        } => {
            let s = slide_mut(p, *slide)?;
            // A build of an element the slide doesn't have is kept and skipped (stepping and
            // drawing only count builds of its elements): its element was deleted, and the
            // editor sends it back with the order, an undo restores it, and it plays again
            // when deleting the element is undone. Only the list's length is bounded.
            if builds.as_ref().is_some_and(|bs| bs.len() > MAX_BUILDS) {
                return Err(bad(format!("a slide has at most {MAX_BUILDS} builds")));
            }
            vec![Op::SetSlide {
                slide: *slide,
                label: swap(&mut s.label, label),
                enabled: swap(&mut s.enabled, enabled),
                notes: swap(&mut s.notes, notes),
                transition: swap(&mut s.transition, transition),
                background: swap(&mut s.background, background),
                hotkey: swap(&mut s.hotkey, hotkey),
                builds: swap(&mut s.builds, builds),
            }]
        }
        Op::SetSlideText {
            slide,
            element: Some(element),
            text,
            expect_text,
        } => {
            let tb = text_box_mut(p, *slide, *element)?;
            let now = tb.plain_text();
            if let Some(e) = expect_text
                && *e != now
            {
                return Err(Fail {
                    slide: Some(*slide),
                    element: Some(*element),
                    text: Some(now),
                    ..gone("the slide's text was changed by someone else")
                });
            }
            let old = tb.clone();
            set_text(tb, text);
            vec![Op::ReplaceTextBox {
                slide: *slide,
                element: *element,
                text_box: old,
            }]
        }
        Op::AddElement {
            slide,
            element,
            index,
        } => {
            let s = slide_mut(p, *slide)?;
            if s.elements.iter().any(|e| e.id == element.id) {
                return Err(bad(format!("element {} already exists", element.id.0)));
            }
            let at = index
                .map(|i| i as usize)
                .unwrap_or(usize::MAX)
                .min(s.elements.len());
            s.elements.insert(at, element.clone());
            vec![Op::DeleteElement {
                slide: *slide,
                element: element.id,
            }]
        }
        Op::DeleteElement { slide, element } => {
            let s = slide_mut(p, *slide)?;
            let i = element_index(s, *element)?;
            let removed = s.elements.remove(i);
            vec![Op::AddElement {
                slide: *slide,
                element: removed,
                index: Some(i as u32),
            }]
        }
        Op::ReorderElement {
            slide,
            element,
            index,
        } => {
            let s = slide_mut(p, *slide)?;
            let i = element_index(s, *element)?;
            let e = s.elements.remove(i);
            let at = (*index as usize).min(s.elements.len());
            s.elements.insert(at, e);
            vec![Op::ReorderElement {
                slide: *slide,
                element: *element,
                index: i as u32,
            }]
        }
        Op::SetElement {
            slide,
            element,
            patch,
        } => {
            if let Some(o) = patch.opacity
                && !(0.0..=1.0).contains(&o)
            {
                return Err(bad("opacity must be between 0 and 1"));
            }
            if let Some(sc) = &patch.scroll {
                check_scroll(sc)?;
            }
            let s = slide_mut(p, *slide)?;
            let i = element_index(s, *element)?;
            let e = &mut s.elements[i];
            vec![Op::SetElement {
                slide: *slide,
                element: *element,
                patch: ElementPatch {
                    name: swap(&mut e.name, &patch.name),
                    bounds: swap(&mut e.bounds, &patch.bounds),
                    rotation: swap(&mut e.rotation, &patch.rotation),
                    opacity: swap(&mut e.opacity, &patch.opacity),
                    flip: swap(&mut e.flip, &patch.flip),
                    hidden: swap(&mut e.hidden, &patch.hidden),
                    locked: swap(&mut e.locked, &patch.locked),
                    aspect_locked: swap(&mut e.aspect_locked, &patch.aspect_locked),
                    fill: swap(&mut e.fill, &patch.fill),
                    stroke: swap(&mut e.stroke, &patch.stroke),
                    shadow: swap(&mut e.shadow, &patch.shadow),
                    feather: swap(&mut e.feather, &patch.feather),
                    scroll: swap(&mut e.scroll, &patch.scroll),
                    placeholder: swap(&mut e.placeholder, &patch.placeholder),
                    visibility: swap(&mut e.visibility, &patch.visibility),
                    data_link: swap(&mut e.data_link, &patch.data_link),
                },
            }]
        }
        Op::SetTextStyle {
            slide,
            element,
            range: None,
            character,
            paragraph,
            clear,
        } => {
            check_clear(clear)?;
            let tb = text_box_mut(p, *slide, *element)?;
            let old = tb.clone();
            patch_char(&mut tb.default_style, character.as_ref(), clear)?;
            patch_paragraph(&mut tb.default_paragraph, paragraph.as_ref(), clear)?;
            vec![Op::ReplaceTextBox {
                slide: *slide,
                element: *element,
                text_box: old,
            }]
        }
        Op::SetTextBox {
            slide,
            element,
            vertical_align,
            fit,
            margins,
            outline,
            text_shadow,
            line_mask,
        } => {
            let tb = text_box_mut(p, *slide, *element)?;
            vec![Op::SetTextBox {
                slide: *slide,
                element: *element,
                vertical_align: swap(&mut tb.vertical_align, vertical_align),
                fit: swap(&mut tb.fit, fit),
                margins: swap(&mut tb.margins, margins),
                outline: swap(&mut tb.outline, outline),
                text_shadow: swap(&mut tb.text_shadow, text_shadow),
                line_mask: swap(&mut tb.line_mask, line_mask),
            }]
        }
        Op::ReplaceTextBox {
            slide,
            element,
            text_box,
        } => {
            let tb = text_box_mut(p, *slide, *element)?;
            let old = std::mem::replace(tb, text_box.clone());
            vec![Op::ReplaceTextBox {
                slide: *slide,
                element: *element,
                text_box: old,
            }]
        }
        Op::SetArrangements {
            arrangements,
            selected,
        } => {
            check_arrangements(arrangements)?;
            if let Some(Some(id)) = selected
                && !arrangements.iter().any(|a| a.id == *id)
            {
                return Err(bad("the selected arrangement is not in the list"));
            }
            let old_list = std::mem::replace(&mut p.arrangements, arrangements.clone());
            let old_sel = p.selected_arrangement;
            match selected {
                Some(s) => p.selected_arrangement = *s,
                None => {
                    if let Some(id) = old_sel
                        && !arrangements.iter().any(|a| a.id == id)
                    {
                        p.selected_arrangement = None;
                    }
                }
            }
            vec![Op::SetArrangements {
                arrangements: old_list,
                selected: Some(old_sel),
            }]
        }
        Op::AddSlideAction {
            slide,
            after,
            action,
        } => {
            let s = slide_mut(p, *slide)?;
            if s.actions.iter().any(|a| a.id == action.id) {
                return Err(bad(format!("action {} already exists", action.id.0)));
            }
            let at = match after {
                Some(a) => action_index(s, *a)? + 1,
                None => 0,
            };
            s.actions.insert(at, action.clone());
            vec![Op::RemoveSlideAction {
                slide: *slide,
                action: action.id,
            }]
        }
        Op::RemoveSlideAction { slide, action } => {
            let s = slide_mut(p, *slide)?;
            let i = action_index(s, *action)?;
            let after = i.checked_sub(1).map(|j| s.actions[j].id);
            let removed = s.actions.remove(i);
            vec![Op::AddSlideAction {
                slide: *slide,
                after,
                action: removed,
            }]
        }
        Op::MoveSlideAction {
            slide,
            action,
            after,
        } => {
            let s = slide_mut(p, *slide)?;
            let i = action_index(s, *action)?;
            if *after == Some(*action) {
                return Err(bad("an action cannot move after itself"));
            }
            if let Some(a) = after {
                action_index(s, *a)?;
            }
            let old_after = i.checked_sub(1).map(|j| s.actions[j].id);
            let moved = s.actions.remove(i);
            let at = match after {
                Some(a) => action_index(s, *a)? + 1,
                None => 0,
            };
            s.actions.insert(at, moved);
            vec![Op::MoveSlideAction {
                slide: *slide,
                action: *action,
                after: old_after,
            }]
        }
        Op::SetSlideAction { slide, action } => {
            let s = slide_mut(p, *slide)?;
            let i = action_index(s, action.id)?;
            let old = std::mem::replace(&mut s.actions[i], action.clone());
            vec![Op::SetSlideAction {
                slide: *slide,
                action: old,
            }]
        }
        Op::SetSlideTranslation { slide, translation } => {
            if translation
                .as_ref()
                .is_some_and(|t| t.texts.len() > model::language::MAX_TRANSLATED_ELEMENTS)
            {
                return Err(bad(format!(
                    "a slide translation has at most {} element texts",
                    model::language::MAX_TRANSLATED_ELEMENTS
                )));
            }
            let s = slide_mut(p, *slide)?;
            let old = std::mem::replace(&mut s.translation, translation.clone());
            vec![Op::SetSlideTranslation {
                slide: *slide,
                translation: old,
            }]
        }
        Op::ReplaceSlide { slide } => {
            let (si, k) = slide_pos(p, slide.id)?;
            let old = std::mem::replace(&mut p.sections[si].slides[k], slide.clone());
            vec![Op::ReplaceSlide { slide: old }]
        }
        Op::DuplicateSlide { .. }
        | Op::ApplyTheme { .. }
        | Op::ResetToTheme { .. }
        | Op::UnlinkTheme { .. }
        | Op::LinkTheme { .. }
        | Op::SplitSlide { .. }
        | Op::MergeSlides { .. }
        | Op::SetSlideText { element: None, .. }
        | Op::ApplyChordPro { .. }
        | Op::SetTextStyle { range: Some(_), .. } => {
            return Err(bad("internal: op was not expanded"));
        }
    })
}

fn check_arrangements(list: &[Arrangement]) -> R<()> {
    for (i, a) in list.iter().enumerate() {
        if list[..i].iter().any(|b| b.id == a.id) {
            return Err(bad(format!("arrangement {} is listed twice", a.id.0)));
        }
    }
    Ok(())
}

// ---- new presentations ----------------------------------------------------------------------

/// What "New Presentation" creates: one section ("Group") with one empty text slide,
/// 1920 × 1080.
pub fn blank_presentation(name: &str) -> Presentation {
    let mut p = Presentation::new_blank(name);
    p.canvas_size = model::Size::HD;
    let canvas = p.canvas_size;
    if let Some(s) = p.sections.first_mut().and_then(|s| s.slides.first_mut()) {
        s.elements.push(default_text_element(canvas));
    }
    p
}

/// A copy of a presentation with fresh IDs for it, its sections, slides, elements and
/// arrangements (arrangement section lists are remapped).
pub fn duplicate_presentation(p: &Presentation) -> Presentation {
    let mut n = p.clone();
    n.id = model::PresentationId::new();
    n.import = None;
    n.last_used_at = None;
    let mut map: Vec<(SectionId, SectionId)> = Vec::new();
    for s in &mut n.sections {
        let new = SectionId::new();
        map.push((s.id, new));
        s.id = new;
        s.import = None;
        for sl in &mut s.slides {
            *sl = sl.duplicate();
        }
    }
    for a in &mut n.arrangements {
        let old = a.id;
        a.id = model::ArrangementId::new();
        if n.selected_arrangement == Some(old) {
            n.selected_arrangement = Some(a.id);
        }
        for s in &mut a.sections {
            if let Some((_, new)) = map.iter().find(|(o, _)| o == s) {
                *s = *new;
            }
        }
    }
    n
}

/// The `Section` a group picker creates.
pub fn new_section(name: &str, color: Option<Color>) -> Section {
    Section {
        color,
        ..Section::new(name)
    }
}

// ---- playlist ops ---------------------------------------------------------------------------

fn item_index(items: &[PlaylistItem], id: model::PlaylistItemId) -> R<usize> {
    items
        .iter()
        .position(|i| i.id == id)
        .ok_or_else(|| gone(format!("playlist item {} no longer exists", id.0)))
}

/// Applies a batch of playlist ops to `items` (a working copy).
pub fn apply_playlist(
    items: &mut Vec<PlaylistItem>,
    ops: &[PlaylistOp],
) -> Result<Applied<PlaylistOp>, EditFailure> {
    let mut out = Applied {
        applied: Vec::new(),
        inverse: Vec::new(),
    };
    let mut inverses = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let inv = apply_playlist_op(items, op).map_err(|f| EditFailure {
            code: f.code,
            message: format!("edit refused (op {i}): {}", f.message),
            op_index: i as u32,
            slide: None,
            element: None,
            text: None,
        })?;
        out.applied.push(op.clone());
        inverses.push(inv);
    }
    out.inverse = inverses.into_iter().rev().collect();
    Ok(out)
}

fn apply_playlist_op(items: &mut Vec<PlaylistItem>, op: &PlaylistOp) -> R<PlaylistOp> {
    Ok(match op {
        PlaylistOp::AddItem { after, item } => {
            if items.iter().any(|i| i.id == item.id) {
                return Err(bad(format!("playlist item {} already exists", item.id.0)));
            }
            let at = match after {
                Some(a) => item_index(items, *a)? + 1,
                None => 0,
            };
            items.insert(at, item.clone());
            PlaylistOp::RemoveItem { item: item.id }
        }
        PlaylistOp::RemoveItem { item } => {
            let i = item_index(items, *item)?;
            let after = i.checked_sub(1).map(|j| items[j].id);
            PlaylistOp::AddItem {
                after,
                item: items.remove(i),
            }
        }
        PlaylistOp::MoveItem { item, after } => {
            if after == &Some(*item) {
                return Err(bad("an item cannot move after itself"));
            }
            let i = item_index(items, *item)?;
            let old_after = i.checked_sub(1).map(|j| items[j].id);
            if let Some(a) = after {
                item_index(items, *a)?;
            }
            let it = items.remove(i);
            let at = match after {
                Some(a) => item_index(items, *a)? + 1,
                None => 0,
            };
            items.insert(at, it);
            PlaylistOp::MoveItem {
                item: *item,
                after: old_after,
            }
        }
        PlaylistOp::SetItem {
            item,
            arrangement,
            enabled,
            name,
            color,
        } => {
            let i = item_index(items, *item)?;
            let it = &mut items[i];
            let enabled_old = swap(&mut it.enabled, enabled);
            match &mut it.kind {
                PlaylistItemKind::Header {
                    name: hn,
                    color: hc,
                } => {
                    if arrangement.is_some() {
                        return Err(bad("headers have no arrangement"));
                    }
                    let old_name = name.as_ref().map(|n| {
                        let old = std::mem::take(hn);
                        *hn = n.clone().unwrap_or_default();
                        Some(old)
                    });
                    PlaylistOp::SetItem {
                        item: *item,
                        arrangement: None,
                        enabled: enabled_old,
                        name: old_name,
                        color: swap(hc, color),
                    }
                }
                kind => {
                    if color.is_some() {
                        return Err(bad("only headers have a colour"));
                    }
                    let arr_old = match (kind, arrangement) {
                        (PlaylistItemKind::Presentation { arrangement: a, .. }, Some(n)) => {
                            Some(std::mem::replace(a, *n))
                        }
                        (_, Some(_)) => {
                            return Err(bad("only presentation items have an arrangement"));
                        }
                        (_, None) => None,
                    };
                    PlaylistOp::SetItem {
                        item: *item,
                        arrangement: arr_old,
                        enabled: enabled_old,
                        name: swap(&mut it.name, name),
                        color: None,
                    }
                }
            }
        }
    })
}

fn bin_item_index(items: &[MediaBinItem], id: model::MediaBinItemId) -> R<usize> {
    items
        .iter()
        .position(|i| i.id == id)
        .ok_or_else(|| gone(format!("media bin item {} no longer exists", id.0)))
}

/// Applies a batch of media bin item ops to `items` (a working copy of a manual bin's items).
pub fn apply_media_bin(
    items: &mut Vec<MediaBinItem>,
    ops: &[MediaBinOp],
) -> Result<Applied<MediaBinOp>, EditFailure> {
    let mut out = Applied {
        applied: Vec::new(),
        inverse: Vec::new(),
    };
    for (i, op) in ops.iter().enumerate() {
        let inv = apply_media_bin_op(items, op).map_err(|f| EditFailure {
            code: f.code,
            message: format!("edit refused (op {i}): {}", f.message),
            op_index: i as u32,
            slide: None,
            element: None,
            text: None,
        })?;
        out.applied.push(op.clone());
        out.inverse.push(inv);
    }
    out.inverse.reverse();
    Ok(out)
}

fn apply_media_bin_op(items: &mut Vec<MediaBinItem>, op: &MediaBinOp) -> R<MediaBinOp> {
    Ok(match op {
        MediaBinOp::AddItem { after, item } => {
            if items.iter().any(|i| i.id == item.id) {
                return Err(bad(format!("media bin item {} already exists", item.id.0)));
            }
            let at = match after {
                Some(a) => bin_item_index(items, *a)? + 1,
                None => 0,
            };
            items.insert(at, item.clone());
            MediaBinOp::RemoveItem { item: item.id }
        }
        MediaBinOp::RemoveItem { item } => {
            let i = bin_item_index(items, *item)?;
            let after = i.checked_sub(1).map(|j| items[j].id);
            MediaBinOp::AddItem {
                after,
                item: items.remove(i),
            }
        }
        MediaBinOp::MoveItem { item, after } => {
            if after == &Some(*item) {
                return Err(bad("an item cannot move after itself"));
            }
            let i = bin_item_index(items, *item)?;
            let old_after = i.checked_sub(1).map(|j| items[j].id);
            if let Some(a) = after {
                bin_item_index(items, *a)?;
            }
            let it = items.remove(i);
            let at = match after {
                Some(a) => bin_item_index(items, *a)? + 1,
                None => 0,
            };
            items.insert(at, it);
            MediaBinOp::MoveItem {
                item: *item,
                after: old_after,
            }
        }
        MediaBinOp::SetItem {
            item,
            enabled,
            name,
        } => {
            if let Some(Some(n)) = name
                && n.chars().any(char::is_control)
            {
                return Err(bad("names have no control characters"));
            }
            let i = bin_item_index(items, *item)?;
            let it = &mut items[i];
            MediaBinOp::SetItem {
                item: *item,
                enabled: swap(&mut it.enabled, enabled),
                name: swap(&mut it.name, name),
            }
        }
    })
}

#[cfg(test)]
mod tests;
