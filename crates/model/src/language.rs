//! Dual-language slides: a slide's second-language text, how a screen shows it, and a theme's
//! second-language style.
//!
//! ## Where the second text lives: a translation layer per slide
//! A slide carries at most one [`SlideTranslation`]: a language code and, for each text element
//! that has one, that element's second text ([`ElementTranslation`], keyed by element ID). It is
//! a layer *beside* the elements, not a field inside each text box, because:
//! - every slide-level algorithm keeps working unchanged (template application, linked themes,
//!   template overrides, builds, Reflow and the stage display all see the primary text only);
//! - one slide has one second language, so the code is stored once;
//! - a second text never has its own geometry or box style: how it looks comes from the
//!   screen ([`LanguageDisplay`]) and the theme ([`SecondLanguageStyle`]) at render time.
//!
//! IDs follow the elements: [`crate::Slide::duplicate`] remaps them, applying a template keeps
//! a source's ID (and moves a merged leftover's text with it), and a translation whose element
//! is gone is ignored when drawing.
//!
//! ## Showing it ([`show_language`])
//! A screen's look picks a [`LanguageDisplay`]: `primary` (as authored), `second` (the second
//! text in the primary's place and style), `stacked` (the second text under the primary, in
//! the same box, in the second-language style) or `sideBySide` (the box split in two: primary
//! left, second right; the right half keeps the element's visibility conditions and builds
//! with it). `auto`, the default, uses the slide's own preference
//! ([`SlideTranslation::layout`], e.g. a parallel Bible passage made side by side), else
//! `stacked`. Slides without a translation are always drawn as authored.

use crate::build::{Build, BuildStart};
use crate::common::{Color, default_fns};
use crate::element::Element;
use crate::ids::ElementId;
use crate::slide::Slide;
use crate::text::{CharStyle, Paragraph, ParagraphStyle, TextBox};
use serde::{Deserialize, Serialize};

default_fns! {
    default_size_scale, is_default_size_scale: f64 = 0.8;
    default_gap, is_default_gap: f64 = 24.0;
    default_second_color, is_default_second_color: Color = Color::rgb(1.0, 0.87, 0.56);
}

/// The most text elements a slide translation can name (a slide never has more text boxes
/// than this in practice; bounds what an edit may send).
pub const MAX_TRANSLATED_ELEMENTS: usize = 64;

/// A slide's second-language text (see the module docs).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct SlideTranslation {
    /// BCP 47 language code of the second text (`"es"`, `"pt-BR"`); empty = not given.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub language: String,
    /// The second text of each text element that has one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub texts: Vec<ElementTranslation>,
    /// How the slide prefers to be shown on screens whose look says `auto` (`None` = stacked).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub layout: Option<LanguageDisplay>,
    /// Set when the texts are a machine translation the operator kept (`POST
    /// /translate/preview`, then Keep): shown labelled as such until someone edits them
    /// (an edit sends the layer without it).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub machine: Option<MachineTranslation>,
}

/// Which engine and model made a kept machine translation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct MachineTranslation {
    /// The engine (`opus-mt (candle)`).
    pub engine: String,
    /// The model (`opus-mt-en-es`).
    pub model: String,
}

/// The second text of one text element.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ElementTranslation {
    /// The text element (of the same slide).
    pub element: ElementId,
    /// Its second text: paragraphs like a text box's (runs may carry sparse styles, e.g.
    /// superscript verse numbers); the box's own default styles apply.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub paragraphs: Vec<Paragraph>,
}

impl ElementTranslation {
    /// Unstyled text, one paragraph per `\n`-separated line.
    pub fn plain(element: ElementId, text: &str) -> Self {
        ElementTranslation {
            element,
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

    /// True when no paragraph has a visible character.
    pub fn is_blank(&self) -> bool {
        self.paragraphs
            .iter()
            .all(|p| p.runs.iter().all(|r| r.text.trim().is_empty()))
    }
}

impl SlideTranslation {
    /// A translation in `language` with no texts yet.
    pub fn new(language: impl Into<String>) -> Self {
        SlideTranslation {
            language: language.into(),
            ..SlideTranslation::default()
        }
    }

    /// The second text of `element`, if any.
    pub fn text_of(&self, element: ElementId) -> Option<&ElementTranslation> {
        self.texts.iter().find(|t| t.element == element)
    }

    /// Sets (or, with blank text, removes) the plain second text of `element`.
    pub fn set_plain(&mut self, element: ElementId, text: &str) {
        self.texts.retain(|t| t.element != element);
        let t = ElementTranslation::plain(element, text);
        if !t.is_blank() {
            self.texts.push(t);
        }
    }

    /// True when no element has visible second text.
    pub fn is_blank(&self) -> bool {
        self.texts.iter().all(ElementTranslation::is_blank)
    }

    /// Plain second text of all elements, in the order of `slide`'s elements (search, Reflow).
    pub fn plain_text(&self, slide: &Slide) -> String {
        slide
            .elements
            .iter()
            .filter_map(|e| self.text_of(e.id))
            .map(ElementTranslation::plain_text)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Element IDs changed (`old → new`): texts follow their elements.
    pub fn remap(&mut self, map: &[(ElementId, ElementId)]) {
        for t in &mut self.texts {
            if let Some((_, new)) = map.iter().find(|(old, _)| *old == t.element) {
                t.element = *new;
            }
        }
    }

    /// The texts of `merged` elements are appended to `into`'s (template application merged
    /// those elements' text into `into`), in the order given.
    pub fn merge_into(&mut self, merged: &[ElementId], into: ElementId) {
        let mut moved: Vec<Paragraph> = Vec::new();
        for id in merged {
            if let Some(i) = self.texts.iter().position(|t| t.element == *id) {
                moved.extend(self.texts.remove(i).paragraphs);
            }
        }
        if moved.is_empty() {
            return;
        }
        match self.texts.iter_mut().find(|t| t.element == into) {
            Some(t) => t.paragraphs.extend(moved),
            None => self.texts.push(ElementTranslation {
                element: into,
                paragraphs: moved,
            }),
        }
    }
}

/// How a screen shows slides that have a second language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum LanguageDisplay {
    /// The slide's own preference, else stacked.
    #[default]
    Auto,
    /// The primary text only (as authored).
    Primary,
    /// The second text only, in the primary's place and style.
    Second,
    /// Both, in the same box: the second under the primary, in the second-language style.
    Stacked,
    /// Both, side by side: the box split in two, primary left, second right.
    SideBySide,
    /// Unknown value (auto).
    #[serde(other)]
    Unknown,
}

impl LanguageDisplay {
    /// The mode drawn for `slide`: `auto` (and unknown) resolved by the slide's preference.
    pub fn resolve(self, slide: &Slide) -> LanguageDisplay {
        match self {
            LanguageDisplay::Auto | LanguageDisplay::Unknown => {
                match slide.translation.as_ref().and_then(|t| t.layout) {
                    Some(LanguageDisplay::Auto | LanguageDisplay::Unknown) | None => {
                        LanguageDisplay::Stacked
                    }
                    Some(m) => m,
                }
            }
            m => m,
        }
    }
}

/// How the second language looks when shown with the primary (a theme's "second language"
/// text style). Sizes are relative so one style suits every box of the theme.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct SecondLanguageStyle {
    /// Size of the second text relative to the primary's (0.8 = 80 %).
    #[serde(skip_serializing_if = "is_default_size_scale")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub size_scale: f64,
    /// Italic.
    #[serde(skip_serializing_if = "crate::common::is_true")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub italic: bool,
    /// Glyph colour (a pale gold by default).
    #[serde(skip_serializing_if = "is_default_second_color")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub color: Color,
    /// Font family (`None` = the primary's).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub font_family: Option<String>,
    /// Space between the primary and second text, canvas px: above the second text when
    /// stacked, between the two halves side by side.
    #[serde(skip_serializing_if = "is_default_gap")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub gap: f64,
}

impl Default for SecondLanguageStyle {
    fn default() -> Self {
        SecondLanguageStyle {
            size_scale: default_size_scale(),
            italic: true,
            color: default_second_color(),
            font_family: None,
            gap: default_gap(),
        }
    }
}

impl SecondLanguageStyle {
    /// The style with its numbers clamped to sane ranges.
    pub fn sane(&self) -> SecondLanguageStyle {
        SecondLanguageStyle {
            size_scale: if self.size_scale.is_finite() {
                self.size_scale.clamp(0.2, 2.0)
            } else {
                default_size_scale()
            },
            gap: if self.gap.is_finite() {
                self.gap.clamp(0.0, 400.0)
            } else {
                default_gap()
            },
            ..self.clone()
        }
    }

    /// `paragraphs` restyled as second text of `tb`: every run (and empty paragraph mark) gets
    /// an explicit size (`sizeScale` × its size in `tb`), italic, colour and family.
    fn restyle(&self, tb: &TextBox, paragraphs: &[Paragraph]) -> Vec<Paragraph> {
        let over = |eff: CharStyle, own: &CharStyle| CharStyle {
            size: eff.size.map(|s| s * self.size_scale),
            italic: Some(self.italic),
            color: Some(self.color),
            font_family: self.font_family.clone().or_else(|| own.font_family.clone()),
            postscript_name: None,
            ..own.clone()
        };
        paragraphs
            .iter()
            .map(|p| {
                let mut p = p.clone();
                for r in &mut p.runs {
                    r.style = over(tb.run_style(r), &r.style);
                }
                p.end_style = over(tb.empty_paragraph_style(&p), &p.end_style);
                p
            })
            .collect()
    }
}

/// The longest language code accepted ([`language_code`]).
pub const MAX_LANGUAGE_CODE: usize = 35;

/// A BCP 47 language code in its usual case (`"en"`, `"pt-BR"`, `"zh-Hant-TW"`, `"es-419"`),
/// or `None` when `code` is not one. Accepted: a primary subtag of 2–8 letters (2–3 in
/// practice), then subtags of 1–8 letters or digits separated by `-` (an `_` is read as `-`, as
/// in `pt_BR`); at most [`MAX_LANGUAGE_CODE`] characters. Case: the language lower case, a
/// 4-letter script title case (`Hant`), a 2-letter region upper case (`BR`), the rest lower
/// case. Surrounding blanks are ignored. A syntax check only: the code need not be registered.
pub fn language_code(code: &str) -> Option<String> {
    let code = code.trim();
    if code.is_empty() || code.len() > MAX_LANGUAGE_CODE {
        return None;
    }
    let mut out = Vec::new();
    for (i, tag) in code.split(['-', '_']).enumerate() {
        let letters = tag.chars().all(|c| c.is_ascii_alphabetic());
        let ok = if i == 0 {
            (2..=8).contains(&tag.len()) && letters
        } else {
            (1..=8).contains(&tag.len()) && tag.chars().all(|c| c.is_ascii_alphanumeric())
        };
        if !ok {
            return None;
        }
        let lower = tag.to_ascii_lowercase();
        out.push(if i > 0 && letters && tag.len() == 2 {
            tag.to_ascii_uppercase()
        } else if i > 0 && letters && tag.len() == 4 {
            lower[..1].to_ascii_uppercase() + &lower[1..]
        } else {
            lower
        });
    }
    Some(out.join("-"))
}

/// `slide` as a screen showing `display` draws it, with `style` for the second text. `None`
/// when it is drawn as authored (no translation, `primary`, or nothing to show). See the
/// module docs.
pub fn show_language(
    slide: &Slide,
    display: LanguageDisplay,
    style: &SecondLanguageStyle,
) -> Option<Slide> {
    let tr = slide.translation.as_ref()?;
    if tr.is_blank() {
        return None;
    }
    let display = display.resolve(slide);
    if display == LanguageDisplay::Primary {
        return None;
    }
    let style = style.sane();
    let mut out = slide.clone();
    out.translation = None;
    let mut added: Vec<(usize, Element)> = Vec::new();
    for (i, e) in out.elements.iter_mut().enumerate() {
        let Some(t) = tr.text_of(e.id).filter(|t| !t.is_blank()) else {
            continue;
        };
        let bounds = e.bounds;
        let Some(tb) = e.text_box_mut() else { continue };
        match display {
            LanguageDisplay::Second => tb.paragraphs = t.paragraphs.clone(),
            LanguageDisplay::SideBySide => {
                let half = ((bounds.width - style.gap) / 2.0).max(1.0);
                // The right half keeps the element's visibility conditions (it is the same
                // text, in another language).
                let mut right = Element {
                    id: second_half_id(e.id),
                    name: String::new(),
                    ..e.clone()
                };
                right.bounds.x = bounds.x + bounds.width - half;
                right.bounds.width = half;
                // A picture or video fill stays with the first half (one player, one picture).
                if right.has_media_fill() {
                    right.fill = Default::default();
                }
                if let Some(rtb) = right.text_box_mut() {
                    rtb.paragraphs = style.restyle(rtb, &t.paragraphs);
                }
                e.bounds.width = half;
                added.push((i + 1, right));
            }
            // Stacked (auto already resolved).
            _ => {
                let mut second = style.restyle(tb, &t.paragraphs);
                if let Some(first) = second.first_mut() {
                    let before = tb.paragraph_style(first).space_before.unwrap_or(0.0);
                    first.style = ParagraphStyle {
                        space_before: Some(before + style.gap),
                        ..first.style.clone()
                    };
                }
                tb.paragraphs.extend(second);
            }
        }
    }
    // A right half builds with its left half: each build of a split element gets a twin for
    // the right half, at the same time (same step, start and transition).
    if !added.is_empty() {
        let split: Vec<ElementId> = added.iter().map(|(_, e)| e.id).collect();
        out.builds = out
            .builds
            .iter()
            .flat_map(|b| {
                let half = second_half_id(b.element);
                let twin = split.contains(&half).then(|| Build {
                    element: half,
                    start: match b.start {
                        BuildStart::WithSlide => BuildStart::WithSlide,
                        _ => BuildStart::WithPrevious,
                    },
                    delay: match b.start {
                        BuildStart::WithSlide => b.delay,
                        _ => 0.0,
                    },
                    ..b.clone()
                });
                std::iter::once(b.clone()).chain(twin)
            })
            .collect();
    }
    // Right halves go just above their left halves (same place in the z-order).
    for (at, e) in added.into_iter().rev() {
        out.elements.insert(at, e);
    }
    Some(out)
}

/// The ID of the right half drawn for `left` side by side: stable across renders (a v5 UUID
/// of the left's), so caches and transitions see the same element every frame.
fn second_half_id(left: ElementId) -> ElementId {
    ElementId(uuid::Uuid::new_v5(&left.0, b"vesper.second-language"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Rect;
    use crate::text::{TextRun, VerticalAlign};

    fn slide() -> (Slide, ElementId) {
        let tb = TextBox {
            default_style: CharStyle {
                size: Some(100.0),
                ..CharStyle::default()
            },
            vertical_align: VerticalAlign::Middle,
            ..TextBox::plain("Grace so free\nGrace for me")
        };
        let e = Element::text(Rect::new(100.0, 100.0, 1000.0, 600.0), tb);
        let id = e.id;
        let mut s = Slide::new();
        s.elements.push(e);
        let mut t = SlideTranslation::new("es");
        t.set_plain(id, "Gracia tan libre\nGracia para mí");
        s.translation = Some(t);
        (s, id)
    }

    #[test]
    fn language_codes_are_checked_and_put_in_their_usual_case() {
        for (code, want) in [
            ("en", "en"),
            (" ES ", "es"),
            ("pt-br", "pt-BR"),
            ("pt_BR", "pt-BR"),
            ("zh-hant-tw", "zh-Hant-TW"),
            ("es-419", "es-419"),
            ("fil", "fil"),
            ("sr-Latn", "sr-Latn"),
            ("de-CH-1996", "de-CH-1996"),
        ] {
            assert_eq!(language_code(code).as_deref(), Some(want), "{code}");
        }
        for bad in [
            "",
            "  ",
            "e",
            "english language",
            "en-",
            "-en",
            "en--US",
            "1en",
            "en-toolongsubtag",
            "en/US",
            "ελ",
            &"a".repeat(MAX_LANGUAGE_CODE + 1),
            &format!("en{}", "-ab".repeat(12)),
        ] {
            assert_eq!(language_code(bad), None, "{bad:?}");
        }
    }

    fn text(s: &Slide, i: usize) -> String {
        s.elements[i].text_box().unwrap().plain_text()
    }

    #[test]
    fn primary_and_untranslated_slides_are_drawn_as_authored() {
        let (s, _) = slide();
        let st = SecondLanguageStyle::default();
        assert!(show_language(&s, LanguageDisplay::Primary, &st).is_none());
        let mut plain = s.clone();
        plain.translation = None;
        assert!(show_language(&plain, LanguageDisplay::Stacked, &st).is_none());
        let mut blank = s.clone();
        blank.translation = Some(SlideTranslation::new("es"));
        assert!(show_language(&blank, LanguageDisplay::Auto, &st).is_none());
    }

    #[test]
    fn second_replaces_the_text_in_the_primary_style() {
        let (s, _) = slide();
        let out =
            show_language(&s, LanguageDisplay::Second, &SecondLanguageStyle::default()).unwrap();
        assert_eq!(text(&out, 0), "Gracia tan libre\nGracia para mí");
        let tb = out.elements[0].text_box().unwrap();
        assert_eq!(tb.paragraphs[0].runs[0].style, CharStyle::default());
        assert!(out.translation.is_none());
    }

    #[test]
    fn stacked_appends_the_second_text_styled() {
        let (s, _) = slide();
        let st = SecondLanguageStyle::default();
        let out = show_language(&s, LanguageDisplay::Auto, &st).unwrap();
        assert_eq!(out.elements.len(), 1);
        let tb = out.elements[0].text_box().unwrap();
        assert_eq!(tb.paragraphs.len(), 4);
        let second = &tb.paragraphs[2];
        assert_eq!(second.style.space_before, Some(24.0));
        let r = &second.runs[0].style;
        assert_eq!(r.size, Some(80.0));
        assert_eq!(r.italic, Some(true));
        assert_eq!(r.color, Some(st.color));
        // The primary runs are untouched.
        assert_eq!(tb.paragraphs[0].runs[0].style, CharStyle::default());
    }

    #[test]
    fn side_by_side_splits_the_box() {
        let (mut s, id) = slide();
        s.translation.as_mut().unwrap().layout = Some(LanguageDisplay::SideBySide);
        let st = SecondLanguageStyle {
            gap: 40.0,
            font_family: Some("Serif Face".into()),
            ..SecondLanguageStyle::default()
        };
        let out = show_language(&s, LanguageDisplay::Auto, &st).unwrap();
        assert_eq!(out.elements.len(), 2);
        let (l, r) = (&out.elements[0], &out.elements[1]);
        assert_eq!(l.id, id);
        assert_ne!(r.id, id);
        assert_eq!(r.id, second_half_id(id), "stable across renders");
        assert_eq!(l.bounds, Rect::new(100.0, 100.0, 480.0, 600.0));
        assert_eq!(r.bounds, Rect::new(620.0, 100.0, 480.0, 600.0));
        assert_eq!(text(&out, 0), "Grace so free\nGrace for me");
        assert_eq!(text(&out, 1), "Gracia tan libre\nGracia para mí");
        let rs = &r.text_box().unwrap().paragraphs[0].runs[0].style;
        assert_eq!(rs.font_family.as_deref(), Some("Serif Face"));
        // A screen that forces a mode wins over the slide's preference.
        let out = show_language(&s, LanguageDisplay::Stacked, &st).unwrap();
        assert_eq!(out.elements.len(), 1);
    }

    #[test]
    fn a_right_half_builds_and_hides_with_its_left_half() {
        use crate::build::{BuildMode, frame, step_count};
        use crate::common::Size;
        use crate::element::VisibilityCondition;
        let (mut s, id) = slide();
        s.translation.as_mut().unwrap().layout = Some(LanguageDisplay::SideBySide);
        let other = crate::ids::ElementId::new();
        let cond = VisibilityCondition::ElementHasText {
            element: other,
            has_text: true,
        };
        s.elements[0].visibility = vec![cond.clone()];
        s.builds = vec![Build::new(id, BuildMode::In, Default::default())];
        let out =
            show_language(&s, LanguageDisplay::Auto, &SecondLanguageStyle::default()).unwrap();
        let right = second_half_id(id);
        assert_eq!(out.element(right).unwrap().visibility, vec![cond]);
        assert_eq!(out.builds.len(), 2);
        assert_eq!(step_count(&out), step_count(&s), "no extra step");
        let canvas = Size::HD;
        let hidden = |f: &Slide, e| f.element(e).unwrap().hidden;
        let before = frame(&out, canvas, 0, None, None);
        assert!(hidden(&before, id) && hidden(&before, right));
        let after = frame(&out, canvas, 1, None, None);
        assert!(!hidden(&after, id) && !hidden(&after, right));
    }

    #[test]
    fn verse_numbers_keep_their_baseline_and_scale_from_their_own_size() {
        let (mut s, id) = slide();
        let t = s.translation.as_mut().unwrap();
        t.texts[0].paragraphs = vec![Paragraph {
            runs: vec![
                TextRun::styled(
                    "16",
                    CharStyle {
                        baseline: Some(crate::text::Baseline::Superscript),
                        size: Some(50.0),
                        ..CharStyle::default()
                    },
                ),
                TextRun::plain(" Porque de tal manera"),
            ],
            ..Paragraph::default()
        }];
        let out = show_language(
            &s,
            LanguageDisplay::Stacked,
            &SecondLanguageStyle::default(),
        )
        .unwrap();
        let tb = out.element(id).unwrap().text_box().unwrap();
        let runs = &tb.paragraphs[2].runs;
        assert_eq!(
            runs[0].style.baseline,
            Some(crate::text::Baseline::Superscript)
        );
        assert_eq!(runs[0].style.size, Some(40.0));
        assert_eq!(runs[1].style.size, Some(80.0));
    }

    #[test]
    fn duplicate_and_merge_keep_texts_with_their_elements() {
        let (s, id) = slide();
        let d = s.duplicate();
        let nid = d.elements[0].id;
        assert_ne!(nid, id);
        assert_eq!(
            d.translation.as_ref().unwrap().texts[0].element,
            nid,
            "the copy's translation names the copy's element"
        );
        let mut t = s.translation.clone().unwrap();
        let other = ElementId::new();
        t.set_plain(other, "Otra línea");
        t.merge_into(&[other], id);
        assert_eq!(t.texts.len(), 1);
        assert_eq!(
            t.texts[0].plain_text(),
            "Gracia tan libre\nGracia para mí\nOtra línea"
        );
    }

    #[test]
    fn applying_a_template_keeps_second_texts_with_their_text() {
        use crate::element::Placeholder;
        let slot = |name: &str, text: &str| {
            let mut e = Element::text(Rect::new(0.0, 0.0, 800.0, 400.0), TextBox::plain(text));
            e.placeholder = Some(Placeholder::text(name));
            e
        };
        let mut template = Slide::new();
        template.elements.push(slot("text1", ""));
        let (a, b) = (slot("text1", "Verse"), slot("other", "Leftover"));
        let (aid, bid) = (a.id, b.id);
        let mut s = Slide::new();
        s.elements = vec![a, b];
        let mut t = SlideTranslation::new("es");
        t.set_plain(aid, "Verso");
        t.set_plain(bid, "Sobrante");
        s.translation = Some(t);
        let out = crate::template::apply_template(&s, &template);
        assert_eq!(out.elements.len(), 1);
        assert_eq!(out.elements[0].id, aid);
        let t = out.translation.unwrap();
        assert_eq!(t.texts.len(), 1);
        assert_eq!(t.text_of(aid).unwrap().plain_text(), "Verso\nSobrante");
    }

    #[test]
    fn styles_are_compact_and_clamped() {
        let st = SecondLanguageStyle::default();
        assert_eq!(serde_json::to_string(&st).unwrap(), "{}");
        let wild = SecondLanguageStyle {
            size_scale: 50.0,
            gap: f64::NAN,
            ..st.clone()
        }
        .sane();
        assert_eq!(wild.size_scale, 2.0);
        assert_eq!(wild.gap, 24.0);
        assert_eq!(
            serde_json::to_string(&LanguageDisplay::SideBySide).unwrap(),
            "\"sideBySide\""
        );
    }
}
