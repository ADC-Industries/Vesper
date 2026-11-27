//! Template algorithms: applying a theme's template slide to a content slide (editing time,
//! copy semantics) and restyling the live slide through a look's template override (render
//! time).
//!
//! Both match content to template slots the same way: by slot name first (case-insensitive;
//! a slot's name is its placeholder `slot`, else the element name), then the remaining ones in
//! order.

use crate::element::{Element, ElementKind};
use crate::ids::ElementId;
use crate::show::{TemplateOverride, TextMapping};
use crate::slide::{Slide, remap_element_refs};
use crate::text::{CharStyle, Paragraph, ParagraphStyle, TextRun};

/// Applies `template` to `slide` and returns the new slide (theme application is a **copy**:
/// the result has no live link to the template; the caller records `templateOrigin`, see
/// [`crate::theme::Theme::apply`]).
///
/// ## Algorithm
/// 1. **Classify** the slide's elements (*content* = non-blank text or a media fill):
///    - *text sources*: text elements whose placeholder takes text, except an `Any`
///      placeholder holding only media (an "image with caption" with text is a text source);
///    - *media sources*: other elements whose placeholder takes media and that have a media
///      fill;
///    - *kept as they are*: data-link elements, any other element with content (e.g. text
///      typed into a media-only placeholder), and user elements (neither `themed` nor a
///      placeholder);
///    - *replaced*: the previous template's decoration (`themed`, no placeholder, no media
///      fill, no data link) and placeholders without content.
/// 2. **Copy the template's elements** in order. Each text slot takes the matching text
///    source (by slot name, then order): the copy **keeps the source element's ID** and its
///    paragraphs, restyled by the slot (paragraph overrides dropped; runs keep only baseline,
///    italic, weight and underline), and the source's media fill if it has one. Unfilled text
///    slots are emptied (no sample text). Media slots take matching media sources the same
///    way (old ID, old fill); a source's text goes into the slot's text box, or, when the slot
///    cannot hold text, the source is not matched and stays as it is. Other template elements
///    get fresh IDs; visibility references are remapped.
/// 3. **Never delete content**: text sources left over are appended (as paragraphs) to the
///    last text slot, or kept as their own elements when the template has no text slot or
///    the source has a media fill (merging would lose it); media sources left over are kept
///    as their own elements.
/// 4. **Z-order**: template elements first (bottom), then kept elements in their original
///    order (on top, as if pasted onto the new layout).
/// 5. **Background** comes from the template. **Actions**: the slide's actions are kept; the
///    template's media actions are added (fresh IDs) for media layers the slide has none for.
/// 6. ID, label, enabled flag, hotkey, notes, transition, auto-advance and provenance of the
///    slide are untouched. Its second-language texts ([`crate::language`]) follow their
///    elements: a leftover merged into the last text slot takes its second text along.
pub fn apply_template(slide: &Slide, template: &Slide) -> Slide {
    enum Role {
        Text,
        Media,
        Keep,
        Drop,
    }
    let roles: Vec<Role> = slide
        .elements
        .iter()
        .map(|e| {
            if e.data_link.is_some() {
                return Role::Keep;
            }
            let media = e.has_media_fill();
            let text = has_text(e);
            match &e.placeholder {
                // A text placeholder; an `Any` one counts as media only when it holds no text.
                Some(ph)
                    if ph.takes_text()
                        && e.text_box().is_some()
                        && (text || !media || !ph.takes_media()) =>
                {
                    Role::Text
                }
                Some(ph) if ph.takes_media() && media => Role::Media,
                // Content that fits no source role is never dropped.
                _ if media || text => Role::Keep,
                Some(_) => Role::Drop,
                None if e.themed => Role::Drop,
                None => Role::Keep,
            }
        })
        .collect();
    let pick = |want: fn(&Role) -> bool| -> Vec<usize> {
        roles
            .iter()
            .enumerate()
            .filter(|(_, r)| want(r))
            .map(|(i, _)| i)
            .collect()
    };
    let text_src = pick(|r| matches!(r, Role::Text));
    let media_src = pick(|r| matches!(r, Role::Media));

    let text_slots: Vec<usize> = (0..template.elements.len())
        .filter(|&i| is_text_slot(&template.elements[i]))
        .collect();
    let media_slots: Vec<usize> = (0..template.elements.len())
        .filter(|&i| {
            let e = &template.elements[i];
            !is_text_slot(e) && e.placeholder.as_ref().is_some_and(|p| p.takes_media())
        })
        .collect();

    let names = |els: &[Element], idx: &[usize]| -> Vec<String> {
        idx.iter()
            .map(|&i| slot_name(&els[i]).to_string())
            .collect()
    };
    let text_match = match_by_name_then_order(
        &names(&template.elements, &text_slots),
        &names(&slide.elements, &text_src),
    );
    let media_match = match_by_name_then_order(
        &names(&template.elements, &media_slots),
        &names(&slide.elements, &media_src),
    );

    // 2. Copy the template.
    let mut keep_too = vec![false; slide.elements.len()];
    let mut out: Vec<Element> = Vec::with_capacity(template.elements.len() + slide.elements.len());
    let mut id_map: Vec<(ElementId, ElementId)> = Vec::new();
    for (ti, te) in template.elements.iter().enumerate() {
        let mut e = te.duplicate();
        if let Some(k) = text_slots.iter().position(|&i| i == ti) {
            let paragraphs = match text_match.iter().position(|m| *m == Some(k)) {
                Some(s) => {
                    let src = &slide.elements[text_src[s]];
                    e.id = src.id;
                    if src.has_media_fill() {
                        e.fill = src.fill.clone();
                    }
                    pour(paragraphs_of(src), true)
                }
                None => Vec::new(),
            };
            if let Some(tb) = e.text_box_mut() {
                tb.paragraphs = paragraphs;
            }
        } else if let Some(k) = media_slots.iter().position(|&i| i == ti)
            && let Some(s) = media_match.iter().position(|m| *m == Some(k))
        {
            let src = &slide.elements[media_src[s]];
            if has_text(src) && e.text_box().is_none() {
                // The slot cannot hold the source's text: keep the source whole instead.
                keep_too[media_src[s]] = true;
            } else {
                e.id = src.id;
                e.fill = src.fill.clone();
                if has_text(src)
                    && let Some(tb) = e.text_box_mut()
                {
                    tb.paragraphs = pour(paragraphs_of(src), true);
                }
            }
        }
        id_map.push((te.id, e.id));
        out.push(e);
    }
    remap_element_refs(&mut out, &id_map);

    // 3. Leftover text goes into the last text slot (or stays as its own element).
    let last_text_slot = text_slots.last().copied();
    let mut merged: Vec<ElementId> = Vec::new();
    for (s, m) in text_match.iter().enumerate() {
        if m.is_some() {
            continue;
        }
        let src = &slide.elements[text_src[s]];
        match last_text_slot.and_then(|ti| out[ti].text_box_mut()) {
            // Merging would lose the media fill: keep such a source whole.
            Some(tb) if !src.has_media_fill() => {
                tb.paragraphs.extend(pour(paragraphs_of(src), true));
                merged.push(src.id);
            }
            _ => keep_too[text_src[s]] = true,
        }
    }
    for (s, m) in media_match.iter().enumerate() {
        if m.is_none() {
            keep_too[media_src[s]] = true;
        }
    }

    // 4. Kept elements on top, original order.
    out.extend(
        slide
            .elements
            .iter()
            .zip(&roles)
            .enumerate()
            .filter(|(i, (_, r))| matches!(r, Role::Keep) || keep_too[*i])
            .map(|(_, (e, _))| e.clone()),
    );

    // 5. Background and actions.
    let mut result = slide.clone();
    // Second-language text follows its element (merged leftovers into the last text slot).
    if let (Some(t), Some(ti)) = (result.translation.as_mut(), last_text_slot) {
        t.merge_into(&merged, out[ti].id);
    }
    result.elements = out;
    result.background = template.background;
    for ta in &template.actions {
        if let Some(layer) = ta.media_layer()
            && !result
                .actions
                .iter()
                .any(|a| a.media_layer() == Some(layer))
        {
            result.actions.push(ta.duplicate());
        }
    }
    result
}

impl TemplateOverride {
    /// Renders the live slide's text through `template` for one screen, following the rule
    /// documented on [`TemplateOverride`]. Returns `None` when nothing must be drawn
    /// (`hideWhenEmpty` and no visible text). Hidden elements of `live` are skipped; resolve
    /// its visibility conditions first (`render::live::resolve_slide`) so they count too. The result is a throwaway render slide: the
    /// template's elements and background with the live text poured in, and no actions.
    pub fn render(&self, live: &Slide, template: &Slide) -> Option<Slide> {
        let is_source = |e: &Element| e.data_link.is_none() && e.text_box().is_some();
        // Only text the audience would see: a hidden element (the eye toggle, or a visibility
        // condition the caller resolved) is neither poured nor counted for hideWhenEmpty.
        let is_shown_source = |e: &Element| !e.hidden && is_source(e);
        let mut sources: Vec<&Element> = live
            .elements
            .iter()
            .filter(|e| is_shown_source(e) && is_text_slot(e))
            .collect();
        if sources.is_empty() {
            sources = live
                .elements
                .iter()
                .filter(|e| is_shown_source(e))
                .collect();
        }
        let has_text = sources
            .iter()
            .any(|e| e.text_box().is_some_and(|t| !t.is_blank()));
        if self.hide_when_empty && !has_text {
            return None;
        }

        let mut out = template.clone();
        out.actions.clear();
        let mut targets: Vec<usize> = (0..out.elements.len())
            .filter(|&i| is_text_slot(&out.elements[i]))
            .collect();
        if targets.is_empty() {
            targets = (0..out.elements.len())
                .filter(|&i| is_source(&out.elements[i]))
                .collect();
        }
        for &t in &targets {
            if let Some(tb) = out.elements[t].text_box_mut() {
                tb.paragraphs.clear();
            }
        }
        let Some(&last) = targets.last() else {
            return Some(out);
        };

        let assignment: Vec<usize> = match self.mapping {
            TextMapping::MergeIntoFirst => vec![targets[0]; sources.len()],
            TextMapping::ByOrder => (0..sources.len())
                .map(|i| targets.get(i).copied().unwrap_or(last))
                .collect(),
            TextMapping::ByNameThenOrder | TextMapping::Unknown => {
                let tnames: Vec<String> = targets
                    .iter()
                    .map(|&i| slot_name(&out.elements[i]).to_string())
                    .collect();
                let snames: Vec<String> =
                    sources.iter().map(|e| slot_name(e).to_string()).collect();
                match_by_name_then_order(&tnames, &snames)
                    .into_iter()
                    .map(|m| m.map_or(last, |k| targets[k]))
                    .collect()
            }
        };
        for (src, target) in sources.iter().zip(assignment) {
            let Some(text) = src.text_box() else { continue };
            if text.is_blank() {
                continue;
            }
            let poured = pour(&text.paragraphs, self.keep_inline_formatting);
            if let Some(tb) = out.elements[target].text_box_mut() {
                tb.paragraphs.extend(poured);
            }
        }
        Some(out)
    }
}

/// True when the element holds non-blank text.
fn has_text(e: &Element) -> bool {
    e.text_box().is_some_and(|t| !t.is_blank())
}

/// The element's paragraphs (none for non-text elements).
fn paragraphs_of(e: &Element) -> &[Paragraph] {
    e.text_box().map_or(&[], |t| t.paragraphs.as_slice())
}

/// True for text elements that are text placeholder slots.
fn is_text_slot(e: &Element) -> bool {
    matches!(e.kind, ElementKind::Text { .. })
        && e.data_link.is_none()
        && e.placeholder.as_ref().is_some_and(|p| p.takes_text())
}

/// Matching name of an element: its placeholder slot, else its element name.
fn slot_name(e: &Element) -> &str {
    match &e.placeholder {
        Some(p) if !p.slot.is_empty() => &p.slot,
        _ => &e.name,
    }
}

/// For each source, the index of the target it goes to: same name first (case-insensitive,
/// non-empty names only), then the remaining sources to the remaining targets in order.
/// `None` = no target left.
fn match_by_name_then_order(targets: &[String], sources: &[String]) -> Vec<Option<usize>> {
    let mut assigned: Vec<Option<usize>> = vec![None; sources.len()];
    let mut used = vec![false; targets.len()];
    for (si, s) in sources.iter().enumerate() {
        if s.is_empty() {
            continue;
        }
        if let Some(ti) =
            (0..targets.len()).find(|&ti| !used[ti] && targets[ti].eq_ignore_ascii_case(s))
        {
            used[ti] = true;
            assigned[si] = Some(ti);
        }
    }
    let mut free = (0..targets.len())
        .filter(|&ti| !used[ti])
        .collect::<Vec<_>>()
        .into_iter();
    for a in assigned.iter_mut().filter(|a| a.is_none()) {
        *a = free.next();
    }
    assigned
}

/// Copies paragraphs for pouring into a template slot: paragraph overrides and end styles are
/// dropped (the slot's box style takes over); runs keep only their inline formatting
/// (baseline, italic, weight, underline) when `keep_inline`, else nothing.
fn pour(paragraphs: &[Paragraph], keep_inline: bool) -> Vec<Paragraph> {
    paragraphs
        .iter()
        .map(|p| Paragraph {
            style: ParagraphStyle::default(),
            end_style: CharStyle::default(),
            chords: p.chords.clone(),
            runs: p
                .runs
                .iter()
                .map(|r| TextRun {
                    text: r.text.clone(),
                    style: if keep_inline {
                        r.style.inline_formatting()
                    } else {
                        CharStyle::default()
                    },
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_then_order_matching() {
        let t = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let m = match_by_name_then_order(&t(&["verse", "reference"]), &t(&["Reference", "", "x"]));
        assert_eq!(m, vec![Some(1), Some(0), None]);
    }
}
