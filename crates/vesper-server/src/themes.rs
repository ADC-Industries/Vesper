//! Themes as documents the operator edits (`/api/v1/themes`), and linked themes: presentations
//! whose slides follow a theme's template slides (see `docs/THEMES-PLAN.md` and
//! [`model::theme_link`]).
//!
//! - A theme is edited like a prop, through a **document**: a presentation wrapper with the
//!   theme's ID, name and canvas size and one section holding its template slides, each
//!   labelled with the template's name ([`theme_document`], back with [`from_document`]).
//! - **Linked slides are kept resolved**: every change of a theme re-resolves the slides that
//!   follow it in the same commit ([`propagate`]), so render, thumbnails and every client keep
//!   reading plain slides. A slide whose template slide is gone keeps its look, unlinked.

use crate::data::CollectionContent;
use crate::edit::{Themes, matching_template};
use model::theme_link;
use model::{
    CharStyle, Color, Element, FitMode, Link, Placeholder, Presentation, PresentationId, Rect,
    Section, SectionId, Size, Slide, SlideLabel, TemplateRef, TemplateSlide, TemplateSlideId,
    TextAlign, TextBox, Theme, ThemeId, VerticalAlign,
};
use protocol::{DocumentUse, PresentationOp};

/// True when one of `p`'s slides follows `theme`.
pub fn follows(p: &Presentation, theme: ThemeId) -> bool {
    p.slides()
        .any(|s| s.linked_template().is_some_and(|r| r.theme.id == theme))
}

/// The theme a presentation shows in its summary: the first theme its slides follow, else its
/// `templateOrigin` provenance (the presentation's, else its first slide's).
pub fn presentation_theme(p: &Presentation) -> Option<&model::Link<ThemeId>> {
    p.slides()
        .find_map(|s| s.linked_template())
        .or(p.template_origin.as_ref())
        .or_else(|| p.slides().find_map(|s| s.template_origin.as_ref()))
        .map(|r| &r.theme)
}

/// Presentations of `c` that follow `theme`, by name.
pub fn users(c: &CollectionContent, theme: ThemeId) -> Vec<DocumentUse> {
    users_of(c, theme, None)
}

/// Presentations of `c` that follow `theme` (with `template`: that template slide of it), by
/// name.
pub fn users_of(
    c: &CollectionContent,
    theme: ThemeId,
    template: Option<TemplateSlideId>,
) -> Vec<DocumentUse> {
    let mut out: Vec<DocumentUse> = c
        .presentations
        .iter()
        .filter(|p| {
            p.slides().any(|s| {
                s.linked_template()
                    .is_some_and(|r| r.theme.id == theme && template.is_none_or(|t| r.slide == t))
            })
        })
        .map(|p| DocumentUse {
            kind: model::DocumentKind::Presentation,
            id: p.id.0.to_string(),
            name: p.name.clone(),
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// How many presentations follow each theme.
pub fn use_counts(c: &CollectionContent) -> std::collections::HashMap<ThemeId, u32> {
    let mut m = std::collections::HashMap::new();
    for p in c.presentations.iter() {
        let mut seen: Vec<ThemeId> = Vec::new();
        for s in p.slides() {
            if let Some(r) = s.linked_template()
                && !seen.contains(&r.theme.id)
            {
                seen.push(r.theme.id);
                *m.entry(r.theme.id).or_insert(0) += 1;
            }
        }
    }
    m
}

/// `p` with its slides that follow `theme` re-resolved against `themes` (a slide whose
/// template slide or theme is gone keeps its look, unlinked; with `reassign`, it follows that
/// theme instead). `None` when nothing changes.
pub fn re_resolved(
    p: &Presentation,
    theme: ThemeId,
    themes: &Themes,
    before: &Themes,
    reassign: Option<&Theme>,
) -> Option<Presentation> {
    if !follows(p, theme) {
        return None;
    }
    let mut n = p.clone();
    for sec in &mut n.sections {
        for s in &mut sec.slides {
            let Some(r) = s.linked_template().cloned() else {
                continue;
            };
            if r.theme.id != theme {
                continue;
            }
            let new = match (
                reassign,
                themes.get(&theme).and_then(|t| t.template(r.slide)),
            ) {
                (Some(to), _) => matching_template(s, to, before)
                    .and_then(|tid| theme_link::link(s, to, tid))
                    .unwrap_or_else(|| theme_link::unlink(s)),
                (None, Some(t)) => {
                    let mut n = theme_link::resolve(s, &t.slide);
                    // The name hint follows a renamed theme.
                    if let (Some(o), Some(th)) = (&mut n.template_origin, themes.get(&theme)) {
                        o.theme.name = Some(th.name.clone());
                    }
                    n
                }
                (None, None) => theme_link::unlink(s),
            };
            *s = new;
        }
    }
    (n != *p).then_some(n)
}

/// The presentations of `c` that change when theme `id` becomes what `themes` holds (see
/// [`re_resolved`]).
pub fn propagate(
    c: &CollectionContent,
    id: ThemeId,
    themes: &Themes,
    reassign: Option<&Theme>,
) -> Vec<Presentation> {
    c.presentations
        .iter()
        .filter_map(|p| re_resolved(p, id, themes, &c.themes, reassign))
        .collect()
}

/// The presentation wrapper a theme is edited through (see the module docs).
pub fn theme_document(t: &Theme) -> Presentation {
    let mut doc = Presentation::new_blank(&t.name);
    doc.id = PresentationId::from(t.id.0);
    doc.canvas_size = t.canvas_size;
    doc.sections = vec![Section {
        id: SectionId::from(t.id.0),
        name: t.name.clone(),
        slides: t
            .slides
            .iter()
            .map(|ts| {
                let mut s = ts.slide.clone();
                s.label = Some(SlideLabel {
                    text: ts.name.clone(),
                    color: None,
                });
                // The template slide's ID travels with the slide (not linked: no theme link),
                // so a slide whose delete is undone comes back as the same template slide
                // ([`from_document`]): songs and messages name it by that ID.
                s.template_origin = Some(TemplateRef {
                    theme: Link::named(t.id, t.name.clone()),
                    slide: ts.id,
                });
                s
            })
            .collect(),
        ..Section::new(&t.name)
    }];
    doc.modified_at = None;
    doc.created_at = None;
    doc
}

/// The theme `cur` after its document was edited: name, canvas size and template slides (a
/// slide's label is its template's name; new slides become new template slides, except that a
/// slide brought back by undoing its delete gets its template slide's ID back).
pub fn from_document(cur: &Theme, doc: &Presentation) -> Theme {
    let mut t = cur.clone();
    let old_of = |s: &Slide| cur.slides.iter().find(|ts| ts.slide.id == s.id);
    // Template IDs the document's existing slides keep; a duplicated slide carries its
    // original's ID in `template_origin` and must not take it.
    let mut taken: std::collections::HashSet<TemplateSlideId> =
        doc.slides().filter_map(old_of).map(|ts| ts.id).collect();
    let name = doc.name.trim();
    if !name.is_empty() {
        t.name = name.to_string();
    }
    t.canvas_size = doc.canvas_size;
    t.slides = doc
        .slides()
        .enumerate()
        .map(|(i, s)| {
            let label = s
                .label
                .as_ref()
                .map(|l| l.text.trim().to_string())
                .filter(|l| !l.is_empty());
            let old = old_of(s);
            let mut slide = s.clone();
            slide.label = old.and_then(|o| o.slide.label.clone());
            slide.template_origin = old.and_then(|o| o.slide.template_origin.clone());
            match old {
                Some(o) => TemplateSlide {
                    name: label.unwrap_or_else(|| o.name.clone()),
                    slide,
                    ..o.clone()
                },
                None => TemplateSlide {
                    id: s
                        .template_origin
                        .as_ref()
                        .filter(|r| r.theme.id == cur.id && taken.insert(r.slide))
                        .map_or_else(TemplateSlideId::new, |r| r.slide),
                    name: label.unwrap_or_else(|| format!("Slide {}", i + 1)),
                    slide,
                    guides: Vec::new(),
                    import: None,
                },
            }
        })
        .collect();
    t
}

/// Edit ops a theme's document takes: element and slide edits, adding, duplicating, moving
/// and deleting template slides; not sections, arrangements, split / merge or theme ops.
pub fn edit_op_allowed(op: &PresentationOp) -> bool {
    !matches!(
        op,
        PresentationOp::AddSection { .. }
            | PresentationOp::DeleteSection { .. }
            | PresentationOp::MoveSection { .. }
            | PresentationOp::SplitSlide { .. }
            | PresentationOp::MergeSlides { .. }
            | PresentationOp::SetArrangements { .. }
            | PresentationOp::ApplyTheme { .. }
            | PresentationOp::ResetToTheme { .. }
            | PresentationOp::UnlinkTheme { .. }
            | PresentationOp::LinkTheme { .. }
    )
}

/// A copy of a theme with fresh IDs (theme, template slides, slides, elements).
pub fn duplicate(t: &Theme) -> Theme {
    let mut n = t.clone();
    n.id = ThemeId::new();
    n.import = None;
    for ts in &mut n.slides {
        ts.id = TemplateSlideId::new();
        ts.slide = ts.slide.duplicate();
        ts.import = None;
    }
    n
}

fn slot(name: &str, slot: &str, bounds: Rect, size: f64, align: TextAlign, bold: bool) -> Element {
    let mut tb = TextBox {
        default_style: CharStyle {
            font_family: Some("Arial".into()),
            font_weight: Some(if bold { 700 } else { 400 }),
            size: Some(size.round()),
            color: Some(Color::WHITE),
            ..CharStyle::default()
        },
        vertical_align: VerticalAlign::Middle,
        fit: FitMode::ShrinkToFit,
        ..TextBox::default()
    };
    tb.default_paragraph.alignment = Some(align);
    let mut e = Element::text(bounds, tb);
    e.name = name.into();
    e.placeholder = Some(Placeholder::text(slot));
    e.themed = true;
    e
}

/// A new theme: Lyrics, Title and Scripture template slides, white Arial on black.
pub fn starter(name: &str, canvas: Size) -> Theme {
    let (w, h) = (canvas.width.max(64.0), canvas.height.max(64.0));
    let template = |name: &str, elements: Vec<Element>| TemplateSlide {
        id: TemplateSlideId::new(),
        name: name.into(),
        slide: Slide {
            background: Some(Color::BLACK),
            elements,
            ..Slide::new()
        },
        guides: Vec::new(),
        import: None,
    };
    Theme {
        name: name.to_string(),
        canvas_size: canvas,
        slides: vec![
            template(
                "Lyrics",
                vec![slot(
                    "Lyrics",
                    "lyrics",
                    Rect::new(w * 0.05, h * 0.05, w * 0.9, h * 0.9),
                    h * 0.074,
                    TextAlign::Center,
                    false,
                )],
            ),
            template(
                "Title",
                vec![slot(
                    "Title",
                    "title",
                    Rect::new(w * 0.05, h * 0.3, w * 0.9, h * 0.4),
                    h * 0.11,
                    TextAlign::Center,
                    true,
                )],
            ),
            template(
                "Scripture",
                vec![
                    slot(
                        "Scripture",
                        "scripture",
                        Rect::new(w * 0.05, h * 0.05, w * 0.9, h * 0.72),
                        h * 0.06,
                        TextAlign::Left,
                        false,
                    ),
                    slot(
                        "Reference",
                        "reference",
                        Rect::new(w * 0.05, h * 0.8, w * 0.9, h * 0.14),
                        h * 0.045,
                        TextAlign::Right,
                        true,
                    ),
                ],
            ),
        ],
        ..Theme::default()
    }
}
