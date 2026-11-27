//! A passage as slides: the split rule, and the presentation made of them.
//!
//! **Split rule.** Slides are filled by a *fits* check ([`Fits`]): each slide takes as many
//! **whole** verses as fit, at most `versesPerSlide` when it is set. The server's check builds
//! the slide exactly as it will be stored ([`slide`]: the reference and attribution lines it
//! carries, the theme applied) and lays its text out with the renderer's text engine at the
//! authored size, so nothing overflows and nothing has to shrink; without one,
//! [`chars_fit`] counts characters (`maxChars`, default 220). A verse that does not fit a
//! slide on its own breaks between words (the longest start that fits) and continues on the
//! next slide, where following verses may join it: the broken part ends with
//! [`CONTINUES`], the continuation starts with [`CONTINUED`], the verse number is on the first
//! part only and both slides name the verse in their reference.
//!
//! **Slides.** Each slide has a text element (placeholder slot `text1`, verse numbers as
//! superscript runs) and, when it carries one, a reference element (slot `reference`: the
//! verses on that slide and the translation's abbreviation, plus the attribution line on the
//! last slide). With a theme the chosen template slide is applied ([`model::Theme::apply`],
//! copy semantics: the theme is only read); without one a plain layout is used (white text on
//! black).
//!
//! **Parallel Bible** (batch 16). With a second translation ([`Parallel`]) the primary still
//! decides the split; each slide's text element gets the same verses (by chapter and verse) in
//! the second translation as its second-language text (`Slide.translation`, see
//! `model::language`), with the slide's preferred layout (side by side by default). A verse the
//! primary breaks across slides is broken in the second translation in proportion (by words, or
//! by characters for a script written without spaces; an empty part is left out);
//! verses only the second translation has are left out. The reference names both
//! abbreviations ("John 3:16 KJV · RVR"), the attribution both notices. The fits check sees
//! the second text too (the server lays the slide out as the layout draws it).

use crate::format::HeaderBook;
use crate::{LookupError, Reference, Translation, reference};
use model::{
    Baseline, CharStyle, ElementTranslation, FitMode, LanguageDisplay, SlideTranslation,
    VerticalAlign,
};
use model::{
    BibleBook, Color, Element, ElementId, IntRange, Paragraph, ParagraphStyle, Placeholder,
    Presentation, Rect, ScriptureRef, Section, Slide, SlideLabel, TemplateSlideId, TextAlign,
    TextBox, TextRun, Theme,
};
use protocol::{
    ReferencePlacement, ScripturePassage, ScriptureSlideRequest, ScriptureSlideText,
    ScriptureSlides, VerseRange,
};

/// Default `maxChars`.
pub const DEFAULT_MAX_CHARS: u32 = 220;

/// How a passage becomes slides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlideOptions {
    /// Exactly this many verses on a slide (`None` = by `max_chars`).
    pub verses_per_slide: Option<u32>,
    /// About this many characters at most on a slide.
    pub max_chars: u32,
    /// Verse numbers.
    pub verse_numbers: bool,
    /// Where the reference goes.
    pub reference_on: ReferencePlacement,
    /// How a parallel passage's slides prefer to show the second translation.
    pub second_layout: LanguageDisplay,
}

impl Default for SlideOptions {
    fn default() -> Self {
        SlideOptions {
            verses_per_slide: None,
            max_chars: DEFAULT_MAX_CHARS,
            verse_numbers: false,
            reference_on: ReferencePlacement::Each,
            second_layout: LanguageDisplay::SideBySide,
        }
    }
}

impl From<&ScriptureSlideRequest> for SlideOptions {
    fn from(r: &ScriptureSlideRequest) -> Self {
        SlideOptions {
            verses_per_slide: r.verses_per_slide.filter(|n| *n > 0).map(|n| n.min(50)),
            max_chars: r.max_chars.unwrap_or(DEFAULT_MAX_CHARS).clamp(60, 2000),
            verse_numbers: r.verse_numbers,
            reference_on: r.reference_on,
            second_layout: match r.second_layout {
                Some(LanguageDisplay::Auto | LanguageDisplay::Unknown) | None => {
                    LanguageDisplay::SideBySide
                }
                Some(l) => l,
            },
        }
    }
}

/// The second translation of a parallel passage: the same verses in another translation.
pub struct Parallel<'a> {
    /// The translation.
    pub translation: &'a Translation,
    /// The passage's verses in it ([`parallel_passage`]).
    pub passage: ScripturePassage,
}

/// The verses of `r` (a reference parsed by `primary`) in `second`: the same book (by its key)
/// and ranges. `BadReference` when `second` lacks the book.
pub fn parallel_passage(
    primary: &Translation,
    r: &Reference,
    second: &Translation,
) -> Result<ScripturePassage, LookupError> {
    let key = &primary.book(r).key;
    let book = second
        .header()
        .books
        .iter()
        .position(|b| b.key == *key)
        .ok_or_else(|| {
            LookupError::BadReference(format!(
                "{} has no {}",
                second.header().abbreviation,
                primary.book(r).name
            ))
        })?;
    second.passage_of(&Reference {
        book,
        ranges: r.ranges.clone(),
    })
}

/// A piece of a verse on a slide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    /// Chapter.
    pub chapter: u32,
    /// Verse.
    pub verse: u32,
    /// Text.
    pub text: String,
    /// `true` for the verse's first piece (it carries the verse number).
    pub first: bool,
    /// The verse's words on this piece: `start..end` of `words`.
    pub start: usize,
    /// End (exclusive) of the piece's words.
    pub end: usize,
    /// Number of words in the whole verse.
    pub words: usize,
}

impl Piece {
    /// The verse `(chapter, verse)` of `text`, words `start..end` (clamped), with the
    /// continuation marks of a broken verse.
    fn of(chapter: u32, verse: u32, text: &str, start: usize, end: usize) -> Piece {
        let words: Vec<&str> = text.split_whitespace().collect();
        Piece::of_units(chapter, verse, &words, " ", start, end)
    }

    /// Like [`Piece::of`], over `units` joined by `sep` (words, or the characters of a text
    /// written without spaces).
    fn of_units(
        chapter: u32,
        verse: u32,
        units: &[&str],
        sep: &str,
        start: usize,
        end: usize,
    ) -> Piece {
        let n = units.len();
        let end = end.min(n);
        let start = start.min(end);
        let mut text = units[start..end].join(sep).trim().to_string();
        if start > 0 {
            text = format!("{CONTINUED}{text}");
        }
        if end < n {
            text.push_str(CONTINUES);
        }
        Piece {
            chapter,
            verse,
            text,
            first: start == 0,
            start,
            end,
            words: n,
        }
    }

    /// The same part of the verse in `second` (in proportion), if it has the verse and that
    /// part is not empty. A second text written with spaces is broken between words; one
    /// written without (Chinese, Japanese, Thai: few or no spaces) between characters
    /// (grapheme clusters), so a broken verse never leaves an empty part.
    fn in_second(&self, second: &ScripturePassage) -> Option<Piece> {
        let v = second
            .verses
            .iter()
            .find(|v| v.chapter == self.chapter && v.verse == self.verse)?;
        let words: Vec<&str> = v.text.split_whitespace().collect();
        let letters = v.text.chars().filter(|c| !c.is_whitespace()).count();
        // Long "words" mean a script written without spaces between words.
        let spaced = !words.is_empty() && letters <= words.len() * MAX_WORD_CHARS;
        let (units, sep): (Vec<&str>, &str) = if spaced {
            (words, " ")
        } else {
            use unicode_segmentation::UnicodeSegmentation;
            (v.text.trim().graphemes(true).collect(), "")
        };
        let m = units.len();
        let n = self.words.max(1);
        let at = |i: usize| if i >= self.words { m } else { i * m / n };
        let (start, end) = (at(self.start), at(self.end));
        // An empty part is left out: the verse goes on with the next part.
        if start >= end {
            return None;
        }
        Some(Piece::of_units(v.chapter, v.verse, &units, sep, start, end))
    }
}

/// Average characters per whitespace-separated word above which a verse counts as written
/// without spaces (see [`Piece::in_second`]).
const MAX_WORD_CHARS: usize = 12;

/// A planned slide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedSlide {
    /// The verse pieces, in order.
    pub pieces: Vec<Piece>,
    /// The verses on it ("John 3:16–17"), always set.
    pub verses_label: String,
    /// The reference line shown on it ("John 3:16–17 KJV"), if any.
    pub reference: Option<String>,
    /// The attribution line shown on it, if any.
    pub attribution: Option<String>,
    /// A parallel passage's second translation on it: its language and pieces.
    pub second: Option<(String, Vec<Piece>)>,
}

/// Pieces as one string (verse numbers inline when asked for).
fn pieces_text(pieces: &[Piece], numbers: bool) -> String {
    let mut s = String::new();
    for p in pieces {
        if !s.is_empty() {
            s.push(' ');
        }
        if numbers && p.first {
            s.push_str(&format!("{} ", p.verse));
        }
        s.push_str(&p.text);
    }
    s
}

impl PlannedSlide {
    /// The text as one string (verse numbers inline when asked for).
    pub fn text(&self, numbers: bool) -> String {
        pieces_text(&self.pieces, numbers)
    }

    /// The second translation's text as one string, if any.
    pub fn second_text(&self, numbers: bool) -> Option<String> {
        self.second
            .as_ref()
            .map(|(_, pieces)| pieces_text(pieces, numbers))
    }
}

/// Ends a verse part that continues on the next slide.
pub const CONTINUES: &str = " …";
/// Starts the continuation of a verse from the slide before.
pub const CONTINUED: &str = "… ";

/// Whether a planned slide fits (see the module docs).
pub type Fits<'a> = dyn FnMut(&PlannedSlide) -> bool + 'a;

/// The fallback check: the slide's text (verse numbers counted) is at most `max` characters,
/// in each translation.
pub fn chars_fit(max: u32, numbers: bool) -> impl FnMut(&PlannedSlide) -> bool {
    move |p: &PlannedSlide| {
        p.text(numbers).chars().count() <= max as usize
            && p.second_text(numbers)
                .is_none_or(|t| t.chars().count() <= max as usize)
    }
}

/// Ranges covering a list of (chapter, verse), merging neighbours.
fn ranges_of(book: &HeaderBook, verses: &[(u32, u32)]) -> Vec<VerseRange> {
    let mut out: Vec<VerseRange> = Vec::new();
    for &(c, v) in verses {
        if let Some(last) = out.last_mut() {
            let (lc, lv) = (last.end_chapter, last.end_verse);
            if (lc, lv) == (c, v) {
                continue;
            }
            let chapter_end = book.chapters.get(lc as usize - 1).copied().unwrap_or(0);
            if (c == lc && v == lv + 1) || (c == lc + 1 && v == 1 && lv == chapter_end) {
                last.end_chapter = c;
                last.end_verse = v;
                continue;
            }
        }
        out.push(VerseRange {
            chapter: c,
            verse: v,
            end_chapter: c,
            end_verse: v,
        });
    }
    out
}

/// Splits a passage into slides, each one checked with `fits` (see the module docs).
pub fn plan(
    t: &Translation,
    book: &HeaderBook,
    passage: &ScripturePassage,
    opts: &SlideOptions,
    fits: &mut Fits<'_>,
) -> Vec<PlannedSlide> {
    plan_parallel(t, book, passage, None, opts, fits)
}

/// [`plan`], with a second translation's text on each slide when `second` is given.
pub fn plan_parallel(
    t: &Translation,
    book: &HeaderBook,
    passage: &ScripturePassage,
    second: Option<&Parallel<'_>>,
    opts: &SlideOptions,
    fits: &mut Fits<'_>,
) -> Vec<PlannedSlide> {
    let abbreviation = match second {
        Some(s) => format!(
            "{} · {}",
            t.header().abbreviation,
            s.translation.header().abbreviation
        ),
        None => t.header().abbreviation.clone(),
    };
    let attribution = {
        let both: Vec<String> = std::iter::once(t.info().attribution)
            .chain(second.map(|s| s.translation.info().attribution))
            .flatten()
            .collect();
        (!both.is_empty()).then(|| both.join(" · "))
    };
    let finish = |pieces: Vec<Piece>, last: bool| {
        let verses: Vec<(u32, u32)> = pieces.iter().map(|p| (p.chapter, p.verse)).collect();
        let verses_label = reference::label_with(book, &ranges_of(book, &verses), false);
        let reference = match opts.reference_on {
            ReferencePlacement::Each => true,
            ReferencePlacement::Last => last,
            ReferencePlacement::None => false,
        }
        .then(|| format!("{verses_label} {abbreviation}"));
        let second = second.map(|s| {
            (
                s.translation.header().language.clone(),
                pieces
                    .iter()
                    .filter_map(|p| p.in_second(&s.passage))
                    .collect(),
            )
        });
        PlannedSlide {
            pieces,
            verses_label,
            reference,
            attribution: if last { attribution.clone() } else { None },
            second,
        }
    };
    let verses = &passage.verses;
    let max_verses = opts
        .verses_per_slide
        .map_or(usize::MAX, |n| n.max(1) as usize);
    let piece = |i: usize, start: usize, end: usize| {
        let v = &verses[i];
        Piece::of(v.chapter, v.verse, &v.text, start, end)
    };
    let mut out: Vec<PlannedSlide> = Vec::new();
    // The next verse, and how many of its words already went on earlier slides.
    let (mut i, mut done) = (0usize, 0usize);
    while i < verses.len() {
        let mut cur: Vec<Piece> = Vec::new();
        while i < verses.len() && cur.len() < max_verses {
            let words: Vec<&str> = verses[i].text.split_whitespace().collect();
            let left = &words[done.min(words.len())..];
            let whole = piece(i, done, words.len());
            let last = i + 1 == verses.len();
            let mut with = cur.clone();
            with.push(whole);
            if fits(&finish(with.clone(), last)) {
                cur = with;
                i += 1;
                done = 0;
                continue;
            }
            if !cur.is_empty() {
                break;
            }
            // Alone it doesn't fit: the longest start that does (at least one word).
            let (mut lo, mut hi) = (1usize, left.len().saturating_sub(1));
            let mut take = 1;
            while lo <= hi {
                let mid = (lo + hi) / 2;
                if fits(&finish(vec![piece(i, done, done + mid)], false)) {
                    take = mid;
                    lo = mid + 1;
                } else {
                    hi = mid - 1;
                }
            }
            if left.len() <= 1 {
                // Not even one word fits (a word wider than the box, or a box too small for
                // the text and its reference): it goes on its own slide all the same.
                tracing::warn!(
                    verse = verses[i].verse,
                    "scripture text does not fit the theme's text box"
                );
                cur.push(piece(i, done, words.len()));
                i += 1;
                done = 0;
            } else {
                cur.push(piece(i, done, done + take));
                done += take;
            }
            break;
        }
        out.push(finish(cur, false));
    }
    if let Some(last) = out.pop() {
        out.push(finish(last.pieces, true));
    }
    out
}

/// The split as the preview route returns it.
pub fn preview(name: String, planned: &[PlannedSlide], opts: &SlideOptions) -> ScriptureSlides {
    ScriptureSlides {
        name,
        slides: planned
            .iter()
            .map(|p| ScriptureSlideText {
                text: p.text(opts.verse_numbers),
                second: p.second_text(opts.verse_numbers),
                reference: p.reference.clone(),
                attribution: p.attribution.clone(),
            })
            .collect(),
    }
}

/// The presentation's default name: "John 3:16–18 (KJV)".
pub fn name(t: &Translation, r: &Reference) -> String {
    format!("{} ({})", t.label(r), t.header().abbreviation)
}

/// The template slide used from a theme: `wanted`, else the one named like "Scripture", else
/// the first. `None` when the theme has no template slides (or not `wanted`).
pub fn pick_template(theme: &Theme, wanted: Option<TemplateSlideId>) -> Option<TemplateSlideId> {
    match wanted {
        Some(id) => theme.template(id).map(|t| t.id),
        None => theme
            .slides
            .iter()
            .find(|s| s.name.to_lowercase().contains("scripture"))
            .or_else(|| theme.slides.first())
            .map(|t| t.id),
    }
}

fn text_box(paragraphs: Vec<Paragraph>, size: f64, color: Color, align: TextAlign) -> TextBox {
    TextBox {
        paragraphs,
        default_style: CharStyle {
            size: Some(size),
            color: Some(color),
            ..CharStyle::default()
        },
        default_paragraph: ParagraphStyle {
            alignment: Some(align),
            ..ParagraphStyle::default()
        },
        vertical_align: VerticalAlign::Middle,
        fit: FitMode::ShrinkToFit,
        ..TextBox::default()
    }
}

/// One slide as it is stored: the plain layout, with `template`'s slide applied.
pub fn slide(
    p: &PlannedSlide,
    opts: &SlideOptions,
    template: Option<(&Theme, TemplateSlideId)>,
) -> Slide {
    slide_with_text(p, opts, template).0
}

/// [`slide`], and the IDs of its elements that hold the passage's text (applying a theme keeps
/// the IDs of the elements whose text lands in its slots): the ones a fits check measures.
pub fn slide_with_text(
    p: &PlannedSlide,
    opts: &SlideOptions,
    template: Option<(&Theme, TemplateSlideId)>,
) -> (Slide, Vec<ElementId>) {
    let s = plain_slide(p, opts);
    let ids = s.elements.iter().map(|e| e.id).collect();
    let s = match template {
        Some((theme, id)) => theme.apply(id, &s).unwrap_or(s),
        None => s,
    };
    (s, ids)
}

/// Text runs of pieces (verse numbers as superscript runs when asked for).
fn pieces_runs(pieces: &[Piece], numbers: bool) -> Vec<TextRun> {
    let mut runs = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        let sep = if i == 0 { "" } else { " " };
        if numbers && piece.first {
            if !sep.is_empty() {
                runs.push(TextRun::plain(sep));
            }
            runs.push(TextRun::styled(
                piece.verse.to_string(),
                CharStyle {
                    baseline: Some(Baseline::Superscript),
                    ..CharStyle::default()
                },
            ));
            runs.push(TextRun::plain(format!(" {}", piece.text)));
        } else {
            runs.push(TextRun::plain(format!("{sep}{}", piece.text)));
        }
    }
    runs
}

/// One slide of the built-in plain layout (1920 × 1080).
fn plain_slide(p: &PlannedSlide, opts: &SlideOptions) -> Slide {
    let numbers = opts.verse_numbers;
    let body = Paragraph {
        runs: pieces_runs(&p.pieces, numbers),
        ..Paragraph::default()
    };
    let mut text = Element::text(
        Rect::new(140.0, 110.0, 1640.0, 740.0),
        text_box(vec![body], 72.0, Color::WHITE, TextAlign::Center),
    );
    text.name = "Scripture".into();
    text.placeholder = Some(Placeholder::text("text1"));
    let translation = p
        .second
        .as_ref()
        .map(|(language, pieces)| SlideTranslation {
            language: language.clone(),
            texts: vec![ElementTranslation {
                element: text.id,
                paragraphs: vec![Paragraph {
                    runs: pieces_runs(pieces, numbers),
                    ..Paragraph::default()
                }],
            }],
            layout: Some(opts.second_layout),
            machine: None,
        });
    let mut elements = vec![text];
    let mut lines = Vec::new();
    if let Some(r) = &p.reference {
        lines.push(Paragraph::text(r.clone()));
    }
    if let Some(a) = &p.attribution {
        let mut para = Paragraph::text(a.clone());
        for run in &mut para.runs {
            run.style.size = Some(26.0);
        }
        lines.push(para);
    }
    if !lines.is_empty() {
        let mut r = Element::text(
            Rect::new(140.0, 870.0, 1640.0, 150.0),
            text_box(lines, 44.0, Color::rgb(0.82, 0.82, 0.86), TextAlign::Right),
        );
        r.name = "Reference".into();
        r.placeholder = Some(Placeholder::text("reference"));
        elements.push(r);
    }
    Slide {
        background: Some(Color::BLACK),
        elements,
        label: Some(SlideLabel {
            text: p.verses_label.clone(),
            color: None,
        }),
        translation,
        ..Slide::new()
    }
}

/// The presentation of a passage. With `template`, its slide is applied to every slide.
pub fn presentation(
    t: &Translation,
    r: &Reference,
    planned: &[PlannedSlide],
    opts: &SlideOptions,
    template: Option<(&Theme, TemplateSlideId)>,
    name: String,
) -> Presentation {
    let book = t.book(r);
    let first = r.ranges.first().copied();
    let last = r.ranges.last().copied();
    let (fc, lc) = (
        first.map_or(1, |f| f.chapter),
        last.map_or(1, |l| l.end_chapter),
    );
    let whole = r.ranges.iter().all(|x| {
        x.verse == 1 && book.chapters.get(x.end_chapter as usize - 1) == Some(&x.end_verse)
    });
    let mut section = Section::new(t.label(r));
    section.slides = planned.iter().map(|p| slide(p, opts, template)).collect();
    let mut p = Presentation::new(name);
    p.category = Some("Scripture".into());
    // The translation's language is the presentation's (the Translate dialog's default source).
    p.language = model::language_code(&t.header().language);
    p.scripture = Some(ScriptureRef {
        book: BibleBook {
            name: book.name.clone(),
            key: Some(book.key.clone()),
            index: crate::books::index_of(&book.key),
        },
        chapters: IntRange { start: fc, end: lc },
        verses: (!whole).then(|| IntRange {
            start: first.map_or(1, |f| f.verse),
            end: last.map_or(1, |l| l.end_verse),
        }),
        label: Some(format!("{} ({})", t.label(r), t.header().abbreviation)),
        translation: Some(t.header().abbreviation.clone()),
        generated: None,
    });
    if let Some((theme, id)) = template {
        p.template_origin = Some(theme.template_ref(id));
        p.canvas_size = theme.canvas_size;
    }
    p.sections.push(section);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata::translation;

    fn opts() -> SlideOptions {
        SlideOptions::default()
    }

    /// The split with the character check (no measurer in the crate's tests).
    fn plan_chars(
        t: &Translation,
        book: &HeaderBook,
        p: &ScripturePassage,
        o: &SlideOptions,
    ) -> Vec<PlannedSlide> {
        plan(t, book, p, o, &mut chars_fit(o.max_chars, o.verse_numbers))
    }

    #[test]
    fn verses_fill_slides_by_characters() {
        let t = translation();
        let r = t.parse("Ps 2:1-6").unwrap();
        let p = t.passage_of(&r).unwrap();
        // Each made-up verse is about 50 characters: 4 fit in 220.
        let planned = plan_chars(&t, t.book(&r), &p, &opts());
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].pieces.len(), 4);
        assert_eq!(planned[0].reference.as_deref(), Some("Psalm 2:1–4 TST"));
        assert_eq!(planned[1].verses_label, "Psalm 2:5–6");
    }

    #[test]
    fn verses_per_slide_and_reference_placement() {
        let t = translation();
        let r = t.parse("Ps 2:1-4, 6").unwrap();
        let p = t.passage_of(&r).unwrap();
        let o = SlideOptions {
            verses_per_slide: Some(2),
            reference_on: ReferencePlacement::Last,
            verse_numbers: true,
            ..opts()
        };
        let planned = plan_chars(&t, t.book(&r), &p, &o);
        assert_eq!(planned.len(), 3);
        assert_eq!(planned[0].reference, None);
        assert_eq!(planned[2].reference.as_deref(), Some("Psalm 2:6 TST"));
        assert!(planned[0].text(true).starts_with("1 The lantern"));
        assert!(planned[0].text(true).contains(" 2 The lantern"));
        // A gap in the selection shows in the slide's label.
        let o = SlideOptions {
            verses_per_slide: Some(2),
            ..opts()
        };
        let planned = plan_chars(&t, t.book(&r), &p, &o);
        assert_eq!(planned[1].verses_label, "Psalm 2:3–4");
        let r2 = t.parse("Ps 2:4, 6").unwrap();
        let planned = plan_chars(&t, t.book(&r2), &t.passage_of(&r2).unwrap(), &o);
        assert_eq!(planned[0].verses_label, "Psalm 2:4, 6");
    }

    #[test]
    fn a_long_verse_breaks_between_words_and_continues() {
        let t = translation();
        let r = t.parse("Gen 1:1-3").unwrap();
        let p = t.passage_of(&r).unwrap();
        let o = SlideOptions {
            max_chars: 80,
            verse_numbers: true,
            ..opts()
        };
        let planned = plan_chars(&t, t.book(&r), &p, &o);
        // Verse 1 (about 170 characters) in parts, verse 2 missing, verse 3 joins the last
        // part when it fits.
        assert!(planned.len() >= 3, "{planned:?}");
        assert!(planned.iter().all(|s| s.text(true).chars().count() <= 80));
        let first = &planned[0].pieces[0];
        assert!(first.first && first.text.ends_with(CONTINUES), "{first:?}");
        assert!(planned[0].text(true).starts_with("1 Morning came"));
        let second = &planned[1].pieces[0];
        assert!(
            !second.first && second.text.starts_with(CONTINUED),
            "{second:?}"
        );
        // Both name the verse; nothing is lost or repeated.
        assert_eq!(planned[0].verses_label, "Genesis 1:1");
        assert_eq!(planned[1].verses_label, "Genesis 1:1");
        let words: Vec<String> = planned
            .iter()
            .flat_map(|s| s.pieces.iter())
            .filter(|p| p.verse == 1)
            .flat_map(|p| {
                p.text
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|w| w != "…")
            .collect();
        assert_eq!(words.join(" "), p.verses[0].text);
        assert_eq!(planned.last().unwrap().pieces.last().unwrap().verse, 3);
    }

    #[test]
    fn the_fits_check_decides_and_verses_per_slide_is_an_upper_bound() {
        let t = translation();
        let r = t.parse("Ps 2:1-6").unwrap();
        let p = t.passage_of(&r).unwrap();
        // A narrow box: two verses at most, whatever their characters (a measured split
        // differs from the character rule, which puts four on a slide).
        let mut two = |s: &PlannedSlide| s.pieces.len() <= 2;
        let planned = plan(&t, t.book(&r), &p, &opts(), &mut two);
        assert_eq!(
            planned.iter().map(|s| s.pieces.len()).collect::<Vec<_>>(),
            [2, 2, 2]
        );
        assert_eq!(planned[1].verses_label, "Psalm 2:3–4");
        // `versesPerSlide` never puts more on a slide than fits...
        let o = SlideOptions {
            verses_per_slide: Some(3),
            ..opts()
        };
        let planned = plan(&t, t.book(&r), &p, &o, &mut two);
        assert!(planned.iter().all(|s| s.pieces.len() <= 2));
        // ...and fewer when it says so, although more would fit.
        let o = SlideOptions {
            verses_per_slide: Some(1),
            ..opts()
        };
        let planned = plan(&t, t.book(&r), &p, &o, &mut |_: &PlannedSlide| true);
        assert_eq!(planned.len(), 6);
        // The check sees the reference and attribution as they will be: only the last slide
        // carries the reference with `referenceOn: last`.
        let o = SlideOptions {
            reference_on: ReferencePlacement::Last,
            ..opts()
        };
        let mut seen = Vec::new();
        let planned = plan(&t, t.book(&r), &p, &o, &mut |s: &PlannedSlide| {
            seen.push(s.reference.is_some());
            s.pieces.len() <= 2 || s.reference.is_some()
        });
        assert_eq!(planned.len(), 3);
        assert!(planned[2].reference.is_some() && planned[1].reference.is_none());
        assert!(seen.contains(&true) && seen.contains(&false));
    }

    #[test]
    fn the_presentation_is_scripture_with_the_theme_applied() {
        let t = translation();
        let r = t.parse("John 2:1-3").unwrap();
        let p = t.passage_of(&r).unwrap();
        let o = SlideOptions {
            verse_numbers: true,
            ..opts()
        };
        let planned = plan_chars(&t, t.book(&r), &p, &o);
        let plain = presentation(&t, &r, &planned, &o, None, name(&t, &r));
        assert_eq!(plain.name, "John 2:1–3 (TST)");
        assert_eq!(plain.kind(), model::PresentationKind::Scripture);
        // The translation's language is the presentation's.
        assert_eq!(plain.language.as_deref(), Some("en"));
        let sr = plain.scripture.as_ref().unwrap();
        assert_eq!(sr.book.index, Some(43));
        assert_eq!(sr.verses, Some(IntRange { start: 1, end: 3 }));
        let s = plain.slides().next().unwrap();
        assert!(s.plain_text().contains("The lantern keeper of JHN"));
        assert!(s.plain_text().contains("John 2:1–3 TST"));
        let sup = s.elements[0].text_box().unwrap().paragraphs[0]
            .runs
            .iter()
            .filter(|r| r.style.baseline == Some(Baseline::Superscript))
            .count();
        assert_eq!(sup, 3);

        // A theme with one text slot and a coloured background.
        let mut theme = Theme::default();
        let mut tmpl = Slide::new();
        tmpl.background = Some(Color::rgb(0.1, 0.2, 0.3));
        let mut slot = Element::text(Rect::new(0.0, 0.0, 1920.0, 1080.0), TextBox::default());
        slot.placeholder = Some(Placeholder::text("text1"));
        tmpl.elements.push(slot);
        theme.slides.push(model::TemplateSlide {
            id: TemplateSlideId::new(),
            name: "Scripture".into(),
            slide: tmpl,
            guides: vec![],
            import: None,
        });
        let id = pick_template(&theme, None).unwrap();
        let themed = presentation(&t, &r, &planned, &o, Some((&theme, id)), name(&t, &r));
        let s = themed.slides().next().unwrap();
        assert_eq!(s.background, Some(Color::rgb(0.1, 0.2, 0.3)));
        assert_eq!(s.template_origin.as_ref().map(|o| o.slide), Some(id));
        // Nothing is lost: the reference joins the theme's only text slot.
        assert!(s.plain_text().contains("John 2:1–3 TST"));
        assert_eq!(themed.template_origin.as_ref().map(|o| o.slide), Some(id));
    }

    /// A second made-up translation ("TS2", Spanish-tagged): different words, same
    /// versification, Genesis 1:1 missing.
    fn second_translation() -> Translation {
        let mut h = crate::testdata::header("TS2");
        h.language = "es".into();
        h.attribution = Some("Second synthetic notice.".into());
        let mut v: Vec<String> = crate::testdata::verses(&h)
            .iter()
            .map(|t| t.replace("The lantern keeper of", "El farolero de"))
            .collect();
        v[0] = String::new();
        Translation::from_parts(h, v).unwrap()
    }

    #[test]
    fn a_parallel_passage_carries_the_same_verses_as_second_language() {
        let (t, t2) = (translation(), second_translation());
        let r = t.parse("Ps 2:1-6").unwrap();
        let p = t.passage_of(&r).unwrap();
        let second = Parallel {
            translation: &t2,
            passage: parallel_passage(&t, &r, &t2).unwrap(),
        };
        let o = SlideOptions {
            verse_numbers: true,
            ..opts()
        };
        let mut fit = chars_fit(o.max_chars, o.verse_numbers);
        let planned = plan_parallel(&t, t.book(&r), &p, Some(&second), &o, &mut fit);
        assert!(!planned.is_empty());
        for ps in &planned {
            let (lang, pieces) = ps.second.as_ref().unwrap();
            assert_eq!(lang, "es");
            let a: Vec<_> = ps.pieces.iter().map(|x| (x.chapter, x.verse)).collect();
            let b: Vec<_> = pieces.iter().map(|x| (x.chapter, x.verse)).collect();
            assert_eq!(a, b, "verse-aligned");
            assert!(pieces.iter().all(|x| x.text.starts_with("El farolero")));
        }
        assert_eq!(
            planned[0].reference.as_deref(),
            Some(format!("{} TST · TS2", planned[0].verses_label).as_str())
        );
        assert_eq!(
            planned.last().unwrap().attribution.as_deref(),
            Some("Second synthetic notice.")
        );
        // The stored slide: the second text on the text element, side by side by default.
        let s = slide(&planned[0], &o, None);
        let tr = s.translation.as_ref().unwrap();
        assert_eq!(tr.language, "es");
        assert_eq!(tr.layout, Some(LanguageDisplay::SideBySide));
        assert_eq!(tr.texts[0].element, s.elements[0].id);
        assert!(tr.plain_text(&s).contains("El farolero de PSA"));
        let sup = tr.texts[0].paragraphs[0]
            .runs
            .iter()
            .filter(|r| r.style.baseline == Some(Baseline::Superscript))
            .count();
        assert_eq!(sup, planned[0].pieces.len(), "verse numbers in both");
        // The preview names both texts.
        let pv = preview("x".into(), &planned, &o);
        assert!(
            pv.slides[0]
                .second
                .as_deref()
                .unwrap()
                .contains("El farolero")
        );
    }

    #[test]
    fn a_broken_verse_breaks_the_second_translation_in_proportion() {
        let (t, t2) = (translation(), second_translation());
        // Genesis 1:1 is long in the primary and missing in the second; 1:2 is missing in
        // the primary. Break the long verse with a tight check.
        let r = t.parse("Gen 1:1-3").unwrap();
        let p = t.passage_of(&r).unwrap();
        let second = Parallel {
            translation: &t2,
            passage: parallel_passage(&t, &r, &t2).unwrap(),
        };
        let mut fit = chars_fit(80, false);
        let planned = plan_parallel(&t, t.book(&r), &p, Some(&second), &opts(), &mut fit);
        assert!(planned.len() >= 2);
        // Verse 1 has no second text: those slides carry none of it.
        assert!(planned[0].second.as_ref().unwrap().1.is_empty());
        // Verse 3 (whole in both) is on the last slide.
        let last = planned.last().unwrap();
        assert_eq!(last.second.as_ref().unwrap().1.last().unwrap().verse, 3);

        // A verse broken in the primary is broken in proportion in the second.
        let v = &second.passage.verses[0];
        let n = 10;
        let half = Piece {
            chapter: v.chapter,
            verse: v.verse,
            text: String::new(),
            first: true,
            start: 0,
            end: 5,
            words: n,
        };
        let m = v.text.split_whitespace().count();
        let a = half.in_second(&second.passage).unwrap();
        assert_eq!(a.end, m / 2);
        assert!(a.text.ends_with(CONTINUES));
        let rest = Piece {
            start: 5,
            end: 10,
            first: false,
            ..half
        };
        let b = rest.in_second(&second.passage).unwrap();
        assert_eq!((b.start, b.end), (m / 2, m));
        assert!(b.text.starts_with(CONTINUED) && !b.first);
    }

    #[test]
    fn a_second_text_without_spaces_breaks_between_characters() {
        // Synthetic text in a script written without spaces (H07-4).
        let second = ScripturePassage {
            translation: "ZZ".into(),
            book: "JHN".into(),
            label: String::new(),
            ranges: Vec::new(),
            attribution: None,
            verses: vec![protocol::ScriptureVerse {
                book: "JHN".into(),
                chapter: 1,
                verse: 1,
                text: "甲乙丙丁戊己庚辛壬癸，一二三四五六七八九十。".into(),
            }],
        };
        let part = |start, end| Piece {
            chapter: 1,
            verse: 1,
            text: String::new(),
            first: start == 0,
            start,
            end,
            words: 10,
        };
        let a = part(0, 5).in_second(&second).unwrap();
        let b = part(5, 10).in_second(&second).unwrap();
        assert!(a.first && !b.first);
        assert_eq!(a.text, "甲乙丙丁戊己庚辛壬癸， …");
        assert_eq!(b.text, "… 一二三四五六七八九十。");
        // A part too small to carry a character is left out, never an empty "…" piece.
        let long = |start, end| Piece {
            words: 1000,
            ..part(start, end)
        };
        assert_eq!(long(0, 2).in_second(&second), None);
        let rest = long(2, 1000).in_second(&second).unwrap();
        assert!(rest.first && rest.text.starts_with('甲'));
    }

    #[test]
    fn the_attribution_goes_on_the_last_slide() {
        let mut h = crate::testdata::header("ATT");
        h.attribution = Some("Synthetic text, used by permission of nobody.".into());
        let v = crate::testdata::verses(&h);
        let t = Translation::from_parts(h, v).unwrap();
        let r = t.parse("John 2").unwrap();
        let p = t.passage_of(&r).unwrap();
        let o = SlideOptions {
            verses_per_slide: Some(1),
            reference_on: ReferencePlacement::None,
            ..opts()
        };
        let planned = plan_chars(&t, t.book(&r), &p, &o);
        assert_eq!(planned.len(), 4);
        assert!(planned[..3].iter().all(|s| s.attribution.is_none()));
        assert!(planned[3].attribution.is_some());
        let pres = presentation(&t, &r, &planned, &o, None, name(&t, &r));
        let last = pres.slides().last().unwrap();
        assert!(last.plain_text().contains("used by permission of nobody"));
        assert_eq!(pres.scripture.unwrap().verses, None);
    }
}
