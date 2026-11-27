//! The one [`Slide`] type, reused by presentation slides, theme template slides, props, masks,
//! stage layouts and the CCLI template (inline), and, through template references, message
//! templates.
//!
//! A slide has **no size of its own**: its canvas size comes from exactly one field of the
//! owning document — `Presentation::canvas_size`, `Theme::canvas_size`, `StageLayout::size`,
//! `Mask::size` or `Prop::size`. Element geometry is in that canvas's pixels; screens scale the
//! canvas to their own size at render time.

use crate::action::SlideAction;
use crate::build::Build;
use crate::common::{Color, HotKey, ImportInfo, Link, is_default, is_true, yes};
use crate::effects::Transition;
use crate::element::{Element, VisibilityCondition};
use crate::ids::{ElementId, SlideId, TemplateSlideId, ThemeId};
use crate::language::SlideTranslation;
use crate::text::RichText;
use serde::{Deserialize, Serialize};

/// A slide: a stack of elements plus actions fired when it goes live.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Slide {
    /// Slide ID (ProPresenter cue UUID for presentation slides).
    pub id: SlideId,
    /// Label under the thumbnail ("3X", a scripture reference).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub label: Option<SlideLabel>,
    /// Disabled slides are skipped during playback but kept.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Shortcut that triggers this slide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub hotkey: Option<HotKey>,
    /// Solid background under all elements. `None` = transparent (lower layers show through);
    /// ProPresenter's stored colour is only used when `drawsBackgroundColor` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub background: Option<Color>,
    /// Elements, back to front.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<Element>,
    /// Extra actions run when the slide is triggered, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<SlideAction>,
    /// Speaker / stage notes (plain notes are unstyled paragraphs).
    #[serde(default, skip_serializing_if = "is_default")]
    pub notes: RichText,
    /// Transition into this slide (overrides the presentation default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub transition: Option<Transition>,
    /// Auto-advance after this many seconds (overrides the presentation interval).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub auto_advance_after: Option<f64>,
    /// Object builds, in the order they play (see [`crate::build`]): elements that appear or
    /// disappear one `next` at a time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub builds: Vec<Build>,
    /// Template slide this slide was last styled from. Provenance only, unless `themeLink`
    /// is set: then the slide **follows** that template slide (a linked theme, see
    /// [`crate::theme_link`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub template_origin: Option<TemplateRef>,
    /// The slide follows `templateOrigin` (a linked theme), overriding the properties listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub theme_link: Option<ThemeLink>,
    /// Import provenance (original cue name, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
    /// Second-language text of its text elements (a translation layer, see
    /// [`crate::language`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub translation: Option<SlideTranslation>,
}

impl Default for Slide {
    fn default() -> Self {
        Slide::new()
    }
}

impl Slide {
    /// An empty enabled slide (`{"id": …}` on the wire).
    pub fn new() -> Self {
        Slide {
            id: SlideId::new(),
            label: None,
            enabled: true,
            hotkey: None,
            background: None,
            elements: Vec::new(),
            actions: Vec::new(),
            notes: RichText::default(),
            transition: None,
            auto_advance_after: None,
            builds: Vec::new(),
            template_origin: None,
            theme_link: None,
            import: None,
            translation: None,
        }
    }

    /// The template slide this slide follows (`templateOrigin` with a `themeLink`).
    pub fn linked_template(&self) -> Option<&TemplateRef> {
        self.theme_link.as_ref().and(self.template_origin.as_ref())
    }

    /// Element by ID.
    pub fn element(&self, id: ElementId) -> Option<&Element> {
        self.elements.iter().find(|e| e.id == id)
    }

    /// Mutable element by ID.
    pub fn element_mut(&mut self, id: ElementId) -> Option<&mut Element> {
        self.elements.iter_mut().find(|e| e.id == id)
    }

    /// Plain text of all text elements in z-order, joined by `\n` (slide grid, search).
    pub fn plain_text(&self) -> String {
        self.elements
            .iter()
            .filter_map(Element::text_box)
            .map(|t| t.plain_text())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Copy with fresh IDs for the slide, its elements and its actions (duplicate / paste).
    /// Element references inside visibility conditions are remapped to the new IDs.
    pub fn duplicate(&self) -> Self {
        let mut s = self.clone();
        s.id = SlideId::new();
        let map: Vec<(ElementId, ElementId)> = s
            .elements
            .iter_mut()
            .map(|e| {
                let new = ElementId::new();
                let old = std::mem::replace(&mut e.id, new);
                (old, new)
            })
            .collect();
        remap_element_refs(&mut s.elements, &map);
        if let Some(t) = &mut s.translation {
            t.remap(&map);
        }
        for b in &mut s.builds {
            if let Some((_, new)) = map.iter().find(|(old, _)| *old == b.element) {
                b.element = *new;
            }
        }
        for a in &mut s.actions {
            *a = a.duplicate();
        }
        s
    }
}

/// Rewrites `ElementHasText` references from old to new element IDs.
pub(crate) fn remap_element_refs(elements: &mut [Element], map: &[(ElementId, ElementId)]) {
    for e in elements {
        for c in &mut e.visibility {
            if let VisibilityCondition::ElementHasText { element, .. } = c
                && let Some((_, new)) = map.iter().find(|(old, _)| old == element)
            {
                *element = *new;
            }
        }
    }
}

/// A slide label.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct SlideLabel {
    /// Label text.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub text: String,
    /// Label colour (`None` = no colour; ProPresenter's alpha-0 white maps to `None`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<Color>,
}

/// A reference to one template slide of a theme.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TemplateRef {
    /// The theme (name hint kept for dangling references).
    pub theme: Link<ThemeId>,
    /// The template slide inside it.
    pub slide: TemplateSlideId,
}

/// How a slide follows its theme's template slide (see [`crate::theme_link`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ThemeLink {
    /// Slide properties it sets itself (`background`, `transition`, `removed:<template element
    /// id>` for a theme element deleted on this slide). Kept by the server.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub overrides: Vec<String>,
}
