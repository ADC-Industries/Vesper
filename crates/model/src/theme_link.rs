//! Linked themes: slides that **follow** a theme's template slide, with per-property overrides.
//!
//! A slide follows its theme when it has `templateOrigin` and a [`ThemeLink`]. Its elements
//! that come from the template carry a [`ThemeElementLink`] naming the template element and
//! the properties ([`PROPERTIES`]) the slide sets itself; elements without one are the slide's
//! own and are never touched.
//!
//! - [`resolve`] computes the slide as the theme draws it: template elements in template order,
//!   each with the slide's element ID, content and overridden properties; template elements
//!   new to the slide are added (empty slots); the slide's own elements on top in their order;
//!   an element whose template element is gone keeps its content as the slide's own (an empty
//!   one goes). Background and transition come from the template unless overridden. Label,
//!   notes, actions, hotkey and auto-advance stay the slide's.
//! - [`link`] makes a slide follow a template slide (theme application, then the link).
//! - [`refresh_overrides`] records, after an edit, which properties differ from the template
//!   (a value set back to the theme's follows the theme again).
//! - [`reset`] clears overrides ("Reset to theme") and resolves.
//! - [`unlink`] keeps the slide's look as a plain copy.
//! - [`link_keeping_look`] links a slide (an imported one) to a template slide **without
//!   changing how it looks**: every difference becomes an override, verified by resolving;
//!   [`link_to_closest`] picks the template slide that fits best.
//!
//! **Content** is never a property: a text slot's paragraphs and the media fill of a slot
//! belong to the slide. All functions are pure.

use crate::element::{Element, ThemeElementLink};
use crate::ids::ElementId;
use crate::ids::TemplateSlideId;
use crate::slide::{Slide, ThemeLink, remap_element_refs};
use crate::theme::Theme;
use serde_json::{Map, Value};

/// Element properties a slide can override, by their JSON names (`text.*`: the text box's).
pub const PROPERTIES: &[&str] = &[
    "name",
    "bounds",
    "rotation",
    "opacity",
    "flip",
    "hidden",
    "locked",
    "aspectLocked",
    "shape",
    "fill",
    "stroke",
    "shadow",
    "feather",
    "scroll",
    "visibility",
    "dataLink",
    "text.paragraphs",
    "text.defaultStyle",
    "text.defaultParagraph",
    "text.verticalAlign",
    "text.fit",
    "text.fontScaleFactor",
    "text.margins",
    "text.textShadow",
    "text.outline",
    "text.lineMask",
    "text.fill",
];

/// Slide properties a slide can override (besides `removed:<template element id>`).
pub const SLIDE_PROPERTIES: &[&str] = &["background", "transition"];

const REMOVED: &str = "removed:";

fn json(e: &Element) -> Value {
    serde_json::to_value(e).unwrap_or(Value::Null)
}

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match key {
        "shape" => v.get("kind")?.get("shape"),
        k if k.starts_with("text.") => v.get("kind")?.get("text")?.get(&k[5..]),
        k => v.get(k),
    }
}

fn set(v: &mut Value, key: &str, value: Option<Value>) {
    fn put(m: &mut Map<String, Value>, k: &str, value: Option<Value>) {
        match value {
            Some(x) => {
                m.insert(k.to_string(), x);
            }
            None => {
                m.remove(k);
            }
        }
    }
    let Some(obj) = v.as_object_mut() else { return };
    match key {
        "shape" => {
            if let Some(kind) = obj.get_mut("kind").and_then(Value::as_object_mut) {
                put(kind, "shape", value);
            } else if let Some(x) = value {
                // A default kind (a rectangle shape) is omitted from the JSON.
                obj.insert(
                    "kind".into(),
                    serde_json::json!({"type": "shape", "shape": x}),
                );
            }
        }
        k if k.starts_with("text.") => {
            let Some(kind) = obj.get_mut("kind").and_then(Value::as_object_mut) else {
                return;
            };
            if kind.get("type").and_then(Value::as_str) != Some("text") {
                return;
            }
            let text = kind
                .entry("text")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(t) = text.as_object_mut() {
                put(t, &k[5..], value);
            }
        }
        k => put(obj, k, value),
    }
}

/// True for text elements that are text placeholder slots (their paragraphs are content).
fn is_text_slot(e: &Element) -> bool {
    e.text_box().is_some()
        && e.data_link.is_none()
        && e.placeholder.as_ref().is_some_and(|p| p.takes_text())
}

/// True when `key` of `e` is the slide's content rather than a property of template element
/// `te`.
fn is_content(key: &str, e: &Element, te: &Element) -> bool {
    match key {
        "text.paragraphs" => is_text_slot(te),
        // A slot's picture (and an "image with caption" text slot's) is content.
        "fill" => te.placeholder.is_some() && e.has_media_fill(),
        _ => false,
    }
}

/// The properties of `e` that differ from template element `te` (content excluded).
pub fn element_overrides(e: &Element, te: &Element) -> Vec<String> {
    let (a, b) = (json(e), json(te));
    PROPERTIES
        .iter()
        .filter(|k| !is_content(k, e, te) && get(&a, k) != get(&b, k))
        .map(|k| k.to_string())
        .collect()
}

/// Template element `te` as slide element `se` follows it.
fn follow(se: &Element, te: &Element) -> Element {
    let mut link = se.from_theme.clone().unwrap_or(ThemeElementLink {
        element: te.id,
        overrides: Vec::new(),
    });
    let src = json(se);
    let mut v = json(te);
    // A slot that stopped being one (its role cleared in the theme): the slide's text stays,
    // now as an override, instead of the template's sample text.
    const PARAGRAPHS: &str = "text.paragraphs";
    if is_text_slot(se)
        && !is_text_slot(te)
        && !link.overrides.iter().any(|o| o == PARAGRAPHS)
        && get(&src, PARAGRAPHS) != get(&v, PARAGRAPHS)
    {
        link.overrides.push(PARAGRAPHS.to_string());
    }
    for k in PROPERTIES {
        if is_content(k, se, te) || link.overrides.iter().any(|o| o == k) {
            set(&mut v, k, get(&src, k).cloned());
        }
    }
    if let Some(obj) = v.as_object_mut() {
        obj.insert("id".into(), serde_json::to_value(se.id).unwrap_or_default());
        // Import provenance is the slide element's own, like its ID.
        match src.get("import") {
            Some(i) => obj.insert("import".into(), i.clone()),
            None => obj.remove("import"),
        };
    }
    match serde_json::from_value::<Element>(v) {
        Ok(mut e) => {
            e.from_theme = Some(link);
            e
        }
        Err(_) => se.clone(),
    }
}

fn removed_elements(link: &ThemeLink) -> Vec<String> {
    link.overrides
        .iter()
        .filter_map(|o| o.strip_prefix(REMOVED).map(str::to_string))
        .collect()
}

fn has_content(e: &Element) -> bool {
    e.has_media_fill() || e.text_box().is_some_and(|t| !t.is_blank())
}

/// The slide as its template slide draws it, with the slide's content and overrides (see the
/// module docs). A slide without a [`ThemeLink`] is returned unchanged.
pub fn resolve(slide: &Slide, template: &Slide) -> Slide {
    let Some(link) = &slide.theme_link else {
        return slide.clone();
    };
    let removed = removed_elements(link);
    let mut used = vec![false; slide.elements.len()];
    let mut out: Vec<Element> = Vec::with_capacity(template.elements.len() + slide.elements.len());
    let mut id_map: Vec<(ElementId, ElementId)> = Vec::new();
    for te in &template.elements {
        let found = slide.elements.iter().enumerate().position(|(i, e)| {
            !used[i] && e.from_theme.as_ref().is_some_and(|l| l.element == te.id)
        });
        let e = match found {
            Some(i) => {
                used[i] = true;
                follow(&slide.elements[i], te)
            }
            None if removed.iter().any(|r| *r == te.id.0.to_string()) => continue,
            None => {
                let mut e = te.clone();
                e.id = ElementId::new();
                if is_text_slot(te)
                    && let Some(tb) = e.text_box_mut()
                {
                    tb.paragraphs.clear();
                }
                e.from_theme = Some(ThemeElementLink {
                    element: te.id,
                    overrides: Vec::new(),
                });
                e
            }
        };
        id_map.push((te.id, e.id));
        out.push(e);
    }
    for (i, e) in slide.elements.iter().enumerate() {
        if used[i] {
            continue;
        }
        match &e.from_theme {
            None => out.push(e.clone()),
            // Its template element is gone: content stays as the slide's own element.
            Some(_) if has_content(e) => {
                let mut e = e.clone();
                e.from_theme = None;
                out.push(e);
            }
            Some(_) => {}
        }
    }
    remap_element_refs(&mut out, &id_map);
    let mut s = slide.clone();
    s.elements = out;
    let overridden = |k: &str| link.overrides.iter().any(|o| o == k);
    if !overridden("background") {
        s.background = template.background;
    }
    if !overridden("transition") {
        s.transition = template.transition.clone();
    }
    s
}

/// Records the slide's overrides against its template slide: each following element's
/// properties that differ from its template element, the slide's background and transition,
/// and theme elements deleted on this slide. A slide without a [`ThemeLink`] is unchanged.
pub fn refresh_overrides(slide: &Slide, template: &Slide) -> Slide {
    let mut s = slide.clone();
    let Some(link) = &mut s.theme_link else {
        return s;
    };
    let mut overrides: Vec<String> = Vec::new();
    if s.background != template.background {
        overrides.push("background".into());
    }
    if s.transition != template.transition {
        overrides.push("transition".into());
    }
    for te in &template.elements {
        if !slide
            .elements
            .iter()
            .any(|e| e.from_theme.as_ref().is_some_and(|l| l.element == te.id))
        {
            overrides.push(format!("{REMOVED}{}", te.id.0));
        }
    }
    link.overrides = overrides;
    let mut seen: Vec<ElementId> = Vec::new();
    for e in &mut s.elements {
        let Some(l) = &e.from_theme else { continue };
        let tid = l.element;
        if seen.contains(&tid) {
            // A second follower (a pasted copy) is the slide's own.
            e.from_theme = None;
            continue;
        }
        seen.push(tid);
        if let Some(te) = template.elements.iter().find(|t| t.id == tid) {
            let o = element_overrides(e, te);
            if let Some(l) = &mut e.from_theme {
                l.overrides = o;
            }
        }
    }
    s
}

/// Makes `slide` follow template slide `template` of `theme`: the template is applied
/// ([`crate::template::apply_template`]; the previous theme's elements go, content is kept;
/// on a slide without a text slot, its largest text box is taken as one),
/// the template's elements are linked, and properties the slide keeps (its transition) are
/// recorded as overrides. `None` when the theme has no such template slide.
pub fn link(slide: &Slide, theme: &Theme, template: TemplateSlideId) -> Option<Slide> {
    let t = theme.template(template)?;
    let mut src = slide.clone();
    // The previous theme's decoration goes; its slots are content sources.
    src.elements
        .retain(|e| e.from_theme.is_none() || e.placeholder.is_some());
    for e in &mut src.elements {
        e.from_theme = None;
    }
    // A slide typed without a theme has no slot: its largest text box is its lyrics.
    if !src
        .elements
        .iter()
        .any(|e| e.placeholder.as_ref().is_some_and(|p| p.takes_text()))
        && let Some(e) = src
            .elements
            .iter_mut()
            .filter(|e| {
                e.data_link.is_none()
                    && !e.has_media_fill()
                    && e.text_box().is_some_and(|t| !t.is_blank())
            })
            .max_by(|a, b| {
                (a.bounds.width * a.bounds.height).total_cmp(&(b.bounds.width * b.bounds.height))
            })
    {
        e.placeholder = Some(crate::element::Placeholder::text(""));
    }
    let mut out = crate::template::apply_template(&src, &t.slide);
    for (e, te) in out.elements.iter_mut().zip(&t.slide.elements) {
        e.from_theme = Some(ThemeElementLink {
            element: te.id,
            overrides: Vec::new(),
        });
    }
    out.template_origin = Some(theme.template_ref(template));
    out.theme_link = Some(ThemeLink::default());
    Some(refresh_overrides(&out, &t.slide))
}

/// "Reset to theme": clears overrides and resolves. `element: None` acts on the slide (its
/// own properties, and with `properties: None` every element's too); `properties: None`
/// clears all of the target's. A slide without a [`ThemeLink`] is unchanged.
pub fn reset(
    slide: &Slide,
    template: &Slide,
    element: Option<ElementId>,
    properties: Option<&[String]>,
) -> Slide {
    let mut s = slide.clone();
    let Some(link) = &mut s.theme_link else {
        return s;
    };
    let clear = |list: &mut Vec<String>| match properties {
        Some(p) => list.retain(|o| !p.contains(o)),
        None => list.clear(),
    };
    match element {
        None => {
            clear(&mut link.overrides);
            if properties.is_none() {
                for e in &mut s.elements {
                    if let Some(l) = &mut e.from_theme {
                        l.overrides.clear();
                    }
                }
            }
        }
        Some(id) => {
            if let Some(l) = s.element_mut(id).and_then(|e| e.from_theme.as_mut()) {
                clear(&mut l.overrides);
            }
        }
    }
    resolve(&s, template)
}

/// The slide as a plain copy of how it looks (no link; `templateOrigin` stays as provenance).
pub fn unlink(slide: &Slide) -> Slide {
    let mut s = slide.clone();
    s.theme_link = None;
    for e in &mut s.elements {
        e.from_theme = None;
    }
    s
}

/// The template slide `slide` follows in `theme`, if it follows that theme.
pub fn template_of<'a>(slide: &Slide, theme: &'a Theme) -> Option<&'a Slide> {
    let r = slide.linked_template()?;
    if r.theme.id != theme.id {
        return None;
    }
    theme.template(r.slide).map(|t| &t.slide)
}

/// A slide linked by [`link_keeping_look`], with how well the template slide fits it.
#[derive(Clone, Debug, PartialEq)]
pub struct KeptLook {
    /// The linked slide (resolving it against the template slide gives the slide back).
    pub slide: Slide,
    /// Elements that follow a template element.
    pub paired: usize,
    /// Overrides recorded (the slide's and its elements').
    pub overrides: usize,
}

/// True when slide element `e` can follow template element `te`: the same slot (or both no
/// slot, then the same non-empty name) and the same kind of element.
fn pairs(e: &Element, te: &Element) -> bool {
    let kind = |x: &Element| std::mem::discriminant(&x.kind);
    if kind(e) != kind(te) || e.placeholder != te.placeholder {
        return false;
    }
    e.placeholder.is_some() || (!e.name.is_empty() && e.name == te.name)
}

/// `s` without what linking adds (the link, the followed template elements), for comparing.
fn plain(s: &Slide) -> Slide {
    let mut s = unlink(s);
    s.template_origin = None;
    s
}

/// Links `slide` to template slide `template` of `theme` **keeping its look**: its elements
/// from the bottom are paired, in template order, with the template elements they match (the
/// same slot, else the same name and kind), up to the first one that doesn't (resolution
/// draws the theme's elements under the slide's own, so the z-order stays); template elements
/// left unpaired are recorded as removed, and every property that differs as an override.
/// The result is verified: resolving it must give the slide back; an element whose
/// resolution differs ends the paired run there. `None` when the slide already follows a
/// theme, the template slide doesn't exist, or no element can be paired.
pub fn link_keeping_look(
    slide: &Slide,
    theme: &Theme,
    template: TemplateSlideId,
) -> Option<KeptLook> {
    if slide.theme_link.is_some() {
        return None;
    }
    let t = &theme.template(template)?.slide;
    // The paired run: slide element i follows template element pairing[i].
    let mut pairing: Vec<ElementId> = Vec::new();
    let mut next = 0;
    for e in &slide.elements {
        match t.elements[next.min(t.elements.len())..]
            .iter()
            .position(|te| pairs(e, te))
        {
            Some(k) => {
                pairing.push(t.elements[next + k].id);
                next += k + 1;
            }
            None => break,
        }
    }
    let want = plain(slide);
    while !pairing.is_empty() {
        let mut s = unlink(slide);
        for (e, te) in s.elements.iter_mut().zip(&pairing) {
            e.from_theme = Some(ThemeElementLink {
                element: *te,
                overrides: Vec::new(),
            });
        }
        s.template_origin = Some(theme.template_ref(template));
        s.theme_link = Some(ThemeLink::default());
        let s = refresh_overrides(&s, t);
        let r = plain(&resolve(&s, t));
        if r == want {
            let overrides = s.theme_link.as_ref().map_or(0, |l| l.overrides.len())
                + s.elements
                    .iter()
                    .filter_map(|e| e.from_theme.as_ref())
                    .map(|l| l.overrides.len())
                    .sum::<usize>();
            return Some(KeptLook {
                slide: s,
                paired: pairing.len(),
                overrides,
            });
        }
        // The first paired element that resolves differently ends the run (or, when the
        // elements agree and something else differs, the last one goes).
        let bad = (0..pairing.len())
            .find(|&i| r.elements.get(i) != want.elements.get(i))
            .unwrap_or(pairing.len() - 1);
        pairing.truncate(bad);
    }
    None
}

/// Links `slide` keeping its look ([`link_keeping_look`]) to the template slide that fits
/// best: its `templateOrigin` (provenance, as imports keep it) when that template slide links,
/// else, over `themes`, the one with the most paired elements, then the fewest overrides.
pub fn link_to_closest<'a>(
    slide: &Slide,
    themes: impl IntoIterator<Item = &'a Theme>,
) -> Option<KeptLook> {
    let themes: Vec<&Theme> = themes.into_iter().collect();
    if let Some(r) = &slide.template_origin
        && let Some(th) = themes.iter().find(|t| t.id == r.theme.id)
        && let Some(k) = link_keeping_look(slide, th, r.slide)
    {
        return Some(k);
    }
    let mut best: Option<KeptLook> = None;
    for th in themes {
        for ts in &th.slides {
            if let Some(k) = link_keeping_look(slide, th, ts.id)
                && best.as_ref().is_none_or(|b| {
                    (k.paired, std::cmp::Reverse(k.overrides))
                        > (b.paired, std::cmp::Reverse(b.overrides))
                })
            {
                best = Some(k);
            }
        }
    }
    best
}
