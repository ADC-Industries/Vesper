//! Offline machine translation for second-language slides (`docs/DUAL-LANGUAGE-PLAN.md` §5).
//!
//! - [`Translator`]: the one trait an engine implements (lines in, lines out).
//! - [`marian::Marian`]: OPUS-MT (Marian) models on `candle`, on the CPU.
//! - [`store::Store`]: Vesper's own small model store (per language pair, fetched from the
//!   publisher at a pinned revision, SHA-256 checked); [`catalogue`]: the pairs offered.
//! - [`propose`]: a slide's proposed second texts, keeping its structure (one source line →
//!   one translated line; empty lines stay empty; a paragraph's soft line breaks, U+2028 from
//!   the ProPresenter and PowerPoint imports, each split a line too).
//!
//! A separate crate because candle is a large build: the server links it, but the store, the
//! tokenizer and the engine build and test on their own (`cargo test -p vesper-translate`).
//! Nothing here saves anything: the server answers proposals, and the operator keeps them.

pub mod catalogue;
pub mod marian;
pub mod pth;
pub mod spm;
pub mod store;

use anyhow::{Result, bail};
use model::{ElementTranslation, MachineTranslation, Slide, SlideTranslation};

/// The engine's name, recorded with every kept machine translation.
pub const ENGINE: &str = "opus-mt (candle)";

/// The banner shown with every machine translation (owner, 2026-11-25).
pub const NOTICE: &str =
    "Machine translation: check it, and check your licence allows a translation of this song";

/// A translation engine.
pub trait Translator: Send + Sync {
    /// Translates each line from `from` to `to` (BCP 47 codes): exactly one output line per
    /// input line, in order. Callers pass no empty lines ([`translate_lines`] keeps them).
    fn translate(&self, from: &str, to: &str, lines: &[String]) -> Result<Vec<String>>;

    /// The engine's name (recorded with kept translations).
    fn engine(&self) -> String {
        ENGINE.to_string()
    }
}

/// Translates `lines` keeping their structure: one output line per input line, blank lines
/// stay blank (and are never sent to the engine), and a translated line never spans lines.
pub fn translate_lines(
    t: &dyn Translator,
    from: &str,
    to: &str,
    lines: &[String],
) -> Result<Vec<String>> {
    let todo: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| (i, l.trim().to_string()))
        .collect();
    let mut out = vec![String::new(); lines.len()];
    if todo.is_empty() {
        return Ok(out);
    }
    let input: Vec<String> = todo.iter().map(|(_, l)| l.clone()).collect();
    let done = t.translate(from, to, &input)?;
    if done.len() != input.len() {
        bail!(
            "the engine answered {} lines for {}",
            done.len(),
            input.len()
        );
    }
    for ((i, _), d) in todo.into_iter().zip(done) {
        out[i] = d.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    Ok(out)
}

/// A soft line break inside a paragraph (how the importers keep `\line` and `<a:br/>`).
const SOFT_BREAK: char = '\u{2028}';

/// The lines of one paragraph's text: split at soft line breaks (and stray `\n`).
fn soft_lines(paragraph: &str) -> Vec<String> {
    paragraph
        .split([SOFT_BREAK, '\n'])
        .map(str::to_string)
        .collect()
}

/// The text lines of `slide`'s text elements, by element (one line per paragraph).
pub fn slide_lines(slide: &Slide) -> Vec<(model::ElementId, Vec<String>)> {
    slide
        .elements
        .iter()
        .filter_map(|e| {
            let tb = e.text_box()?;
            let lines: Vec<String> = tb.paragraphs.iter().map(|p| p.plain_text()).collect();
            lines
                .iter()
                .any(|l| !l.trim().is_empty())
                .then_some((e.id, lines))
        })
        .collect()
}

/// The proposed second-language layer of `slide` in `to`: each text element's lines
/// translated, the slide's own layout preference kept, marked as a machine translation by
/// the engine with `model_name`. `None` when the slide has no text.
pub fn propose(
    t: &dyn Translator,
    slide: &Slide,
    from: &str,
    to: &str,
    model_name: &str,
) -> Result<Option<SlideTranslation>> {
    let mut parts = slide_lines(slide);
    parts.truncate(model::language::MAX_TRANSLATED_ELEMENTS);
    if parts.is_empty() {
        return Ok(None);
    }
    // Each paragraph's lines (split at its soft breaks): each is translated as a line of its
    // own, and the paragraph is put back together with the same breaks.
    let split: Vec<(model::ElementId, Vec<Vec<String>>)> = parts
        .iter()
        .map(|(id, paragraphs)| (*id, paragraphs.iter().map(|p| soft_lines(p)).collect()))
        .collect();
    // One engine call per slide: every element's lines together.
    let all: Vec<String> = split
        .iter()
        .flat_map(|(_, ps)| ps.iter().flatten().cloned())
        .collect();
    let done = translate_lines(t, from, to, &all)?;
    let mut it = done.into_iter();
    let texts = split
        .iter()
        .map(|(id, paragraphs)| {
            let text: Vec<String> = paragraphs
                .iter()
                .map(|lines| {
                    let soft = SOFT_BREAK.to_string();
                    it.by_ref()
                        .take(lines.len())
                        .collect::<Vec<_>>()
                        .join(&soft)
                })
                .collect();
            ElementTranslation::plain(*id, &text.join("\n"))
        })
        .collect();
    Ok(Some(SlideTranslation {
        language: to.to_string(),
        texts,
        layout: slide.translation.as_ref().and_then(|t| t.layout),
        machine: Some(MachineTranslation {
            engine: t.engine(),
            model: model_name.to_string(),
        }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Upper-cases each line and records what it was asked.
    struct Upper(Mutex<Vec<Vec<String>>>);

    impl Translator for Upper {
        fn translate(&self, _: &str, _: &str, lines: &[String]) -> Result<Vec<String>> {
            self.0.lock().unwrap().push(lines.to_vec());
            Ok(lines
                .iter()
                .map(|l| format!("{}\n", l.to_uppercase()))
                .collect())
        }
        fn engine(&self) -> String {
            "fake".into()
        }
    }

    #[test]
    fn lines_keep_their_structure() {
        let t = Upper(Mutex::new(Vec::new()));
        let lines: Vec<String> = ["Morning light", "", "  ", "on the hills"]
            .map(String::from)
            .to_vec();
        let out = translate_lines(&t, "en", "es", &lines).unwrap();
        assert_eq!(out, vec!["MORNING LIGHT", "", "", "ON THE HILLS"]);
        // Blank lines never reach the engine.
        assert_eq!(
            t.0.lock().unwrap()[0],
            vec!["Morning light".to_string(), "on the hills".to_string()]
        );
        assert_eq!(
            translate_lines(&t, "en", "es", &[String::new()]).unwrap(),
            vec![String::new()]
        );
    }

    struct Short;
    impl Translator for Short {
        fn translate(&self, _: &str, _: &str, _: &[String]) -> Result<Vec<String>> {
            Ok(vec![])
        }
    }

    #[test]
    fn an_engine_losing_lines_is_an_error() {
        assert!(translate_lines(&Short, "en", "es", &["a".to_string()]).is_err());
    }

    #[test]
    fn a_slide_is_proposed_per_text_element() {
        use model::{Element, Paragraph, Rect, TextBox};
        let mut s = Slide::new();
        let a = TextBox {
            paragraphs: Paragraph::from_lines("Sing out\n\nall the day"),
            ..TextBox::default()
        };
        let b = TextBox {
            paragraphs: Paragraph::from_lines("Verse 1"),
            ..TextBox::default()
        };
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        s.elements.push(Element::text(r, a));
        s.elements.push(Element::new(r));
        s.elements.push(Element::text(r, TextBox::default()));
        s.elements.push(Element::text(r, b));
        s.translation = Some(SlideTranslation {
            layout: Some(model::LanguageDisplay::SideBySide),
            ..SlideTranslation::default()
        });
        let t = Upper(Mutex::new(Vec::new()));
        let p = propose(&t, &s, "en", "es", "opus-mt-en-es")
            .unwrap()
            .unwrap();
        assert_eq!(p.language, "es");
        assert_eq!(p.texts.len(), 2);
        assert_eq!(p.texts[0].element, s.elements[0].id);
        assert_eq!(p.texts[0].plain_text(), "SING OUT\n\nALL THE DAY");
        assert_eq!(p.texts[1].element, s.elements[3].id);
        assert_eq!(p.texts[1].plain_text(), "VERSE 1");
        assert_eq!(p.layout, Some(model::LanguageDisplay::SideBySide));
        assert_eq!(
            p.machine,
            Some(MachineTranslation {
                engine: "fake".into(),
                model: "opus-mt-en-es".into()
            })
        );
        // One engine call for the slide.
        assert_eq!(t.0.lock().unwrap().len(), 1);
        assert!(
            propose(&t, &Slide::new(), "en", "es", "m")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn soft_line_breaks_are_lines_of_their_own() {
        use model::{Element, Paragraph, Rect, TextBox};
        // An imported verse: one paragraph, its lines split by soft breaks; then a blank
        // paragraph and a second one.
        let mut s = Slide::new();
        let tb = TextBox {
            paragraphs: vec![
                Paragraph::text("Morning light\u{2028}on the hills\u{2028}\u{2028}we sing"),
                Paragraph::text(""),
                Paragraph::text("all day"),
            ],
            ..TextBox::default()
        };
        s.elements
            .push(Element::text(Rect::new(0.0, 0.0, 10.0, 10.0), tb));
        let t = Upper(Mutex::new(Vec::new()));
        let p = propose(&t, &s, "en", "es", "m").unwrap().unwrap();
        // Each soft-broken line went to the engine on its own (blank ones never do).
        assert_eq!(
            t.0.lock().unwrap()[0],
            ["Morning light", "on the hills", "we sing", "all day"].map(String::from)
        );
        // The paragraphs and their soft breaks are kept.
        let ps = &p.texts[0].paragraphs;
        assert_eq!(ps.len(), 3);
        assert_eq!(
            ps[0].plain_text(),
            "MORNING LIGHT\u{2028}ON THE HILLS\u{2028}\u{2028}WE SING"
        );
        assert_eq!(ps[1].plain_text(), "");
        assert_eq!(ps[2].plain_text(), "ALL DAY");
    }
}
