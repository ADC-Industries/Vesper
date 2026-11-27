//! Edit op tests on synthetic documents: every op and its inverse round-trip byte for byte,
//! text edits keep styling, and failures leave nothing applied.

use super::*;
use model::{
    ArrangementId, Color, Link, Paragraph, PlaylistItem, PlaylistItemKind, Rect, Section, Size,
    SlideLabel, TextRun,
};
use protocol::{ElementPatch, PlaylistOp, PresentationOp as Op, TextRange};

fn styled_box(lines: &[&str]) -> TextBox {
    let mut tb = TextBox {
        default_style: CharStyle {
            font_family: Some("Test Sans".into()),
            size: Some(72.0),
            color: Some(Color::WHITE),
            ..CharStyle::default()
        },
        ..TextBox::default()
    };
    tb.default_paragraph.alignment = Some(TextAlign::Center);
    for (i, l) in lines.iter().enumerate() {
        let mut p = Paragraph::text(*l);
        if i == 0 && !l.is_empty() {
            // A superscript verse number in front of the first line.
            p.runs.insert(
                0,
                TextRun {
                    text: "1".into(),
                    style: CharStyle {
                        size: Some(36.0),
                        ..CharStyle::default()
                    },
                },
            );
        }
        p.style.space_after = Some(i as f64);
        tb.paragraphs.push(p);
    }
    tb
}

fn text_slide(lines: &[&str]) -> Slide {
    let mut back = Element::new(Rect::new(0.0, 0.0, 1920.0, 1080.0));
    back.name = "Backdrop".into();
    let mut e = Element::text(Rect::new(100.0, 100.0, 1720.0, 880.0), styled_box(lines));
    e.name = "Lyrics".into();
    let mut small = Element::text(Rect::new(0.0, 1000.0, 300.0, 80.0), TextBox::plain("tag"));
    small.name = "Tag".into();
    Slide {
        elements: vec![back, e, small],
        ..Slide::new()
    }
}

fn sample() -> Presentation {
    let mut v = Section::new("Verse 1");
    v.color = Some(Color::rgb(0.2, 0.4, 1.0));
    v.slides = vec![
        text_slide(&["Line one", "Line two"]),
        text_slide(&["Line three", "", "Line four"]),
    ];
    let mut c = Section::new("Chorus");
    c.slides = vec![text_slide(&["Chorus A", "Chorus B", "Chorus C"])];
    let arr = model::Arrangement {
        id: ArrangementId::new(),
        name: "Sunday".into(),
        sections: vec![v.id, c.id, c.id],
    };
    Presentation {
        sections: vec![v, c],
        selected_arrangement: Some(arr.id),
        arrangements: vec![arr],
        ..Presentation::new("Synthetic Song")
    }
}

fn json(p: &Presentation) -> String {
    serde_json::to_string(p).unwrap()
}

fn ids(p: &Presentation) -> (Vec<SectionId>, Vec<SlideId>) {
    (
        p.sections.iter().map(|s| s.id).collect(),
        p.slides().map(|s| s.id).collect(),
    )
}

fn lyrics_id(p: &Presentation, slide: usize) -> ElementId {
    p.slides().nth(slide).unwrap().elements[1].id
}

/// Applies `ops`, checks the document changed, applies the inverse, checks it is back.
fn round_trip(p: &Presentation, ops: Vec<Op>) -> Presentation {
    let mut w = p.clone();
    let a = apply_presentation(&mut w, &ops).unwrap_or_else(|e| panic!("{ops:?}: {e:?}"));
    assert_ne!(json(&w), json(p), "{ops:?} changed nothing");
    let edited = w.clone();
    let b = apply_presentation(&mut w, &a.inverse).expect("inverse applies");
    assert_eq!(
        json(&w),
        json(p),
        "inverse of {ops:?} did not restore the document"
    );
    // Redo: the inverse of the inverse gives the edit again.
    apply_presentation(&mut w, &b.inverse).expect("redo applies");
    assert_eq!(json(&w), json(&edited), "redo of {ops:?}");
    edited
}

#[test]
fn every_op_round_trips() {
    let p = sample();
    let (secs, slides) = ids(&p);
    let el = lyrics_id(&p, 0);
    let mut new_section = Section::new("Bridge");
    new_section.slides.push(text_slide(&["Bridge"]));
    let cases: Vec<Vec<Op>> = vec![
        vec![Op::SetPresentation {
            name: Some("Renamed".into()),
            song_info: Some(Some(model::SongInfo {
                ccli_number: Some("123".into()),
                ..Default::default()
            })),
            scripture: None,
            transition: None,
            canvas_size: Some(Size::new(1280.0, 720.0)),
            notes: Some(model::RichText::plain("notes")),
            category: Some(Some("Hymns".into())),
            language: Some(Some("pt-BR".into())),
        }],
        vec![Op::AddSection {
            after: Some(secs[0]),
            section: new_section.clone(),
        }],
        vec![Op::AddSection {
            after: None,
            section: Section::new("Intro"),
        }],
        vec![Op::DeleteSection { section: secs[0] }],
        vec![Op::DeleteSection { section: secs[1] }],
        vec![Op::MoveSection {
            section: secs[0],
            after: Some(secs[1]),
        }],
        vec![Op::MoveSection {
            section: secs[1],
            after: None,
        }],
        vec![Op::SetSection {
            section: secs[0],
            name: Some("Verse 2".into()),
            color: Some(None),
            hotkey: Some(Some(model::HotKey::default())),
            global_group: Some(Some(Link::named(model::GlobalGroupId::new(), "Verse"))),
        }],
        vec![Op::AddSlide {
            section: secs[1],
            after: Some(slides[2]),
            slide: None,
        }],
        vec![Op::AddSlide {
            section: secs[0],
            after: None,
            slide: Some(text_slide(&["first"])),
        }],
        vec![Op::DeleteSlide { slide: slides[0] }],
        vec![Op::DeleteSlide { slide: slides[1] }],
        vec![Op::MoveSlide {
            slide: slides[0],
            to_section: secs[1],
            after: Some(slides[2]),
        }],
        vec![Op::MoveSlide {
            slide: slides[1],
            to_section: secs[0],
            after: None,
        }],
        vec![Op::DuplicateSlide { slide: slides[1] }],
        vec![Op::SetSlide {
            slide: slides[0],
            label: Some(Some(SlideLabel {
                text: "3X".into(),
                color: None,
            })),
            enabled: Some(false),
            notes: Some(model::RichText::plain("watch the key change")),
            transition: None,
            background: Some(Some(Color::BLACK)),
            hotkey: None,
            builds: None,
        }],
        vec![Op::SetSlideText {
            slide: slides[0],
            element: None,
            text: "Line one\nA new second line\nand a third".into(),
            expect_text: Some("1Line one\nLine two".into()),
        }],
        vec![Op::SetSlideText {
            slide: slides[1],
            element: Some(p.slides().nth(1).unwrap().elements[2].id),
            text: "TAG".into(),
            expect_text: None,
        }],
        vec![Op::SplitSlide {
            slide: slides[2],
            at_paragraph: 1,
        }],
        vec![Op::SplitSlide {
            slide: slides[0],
            at_paragraph: 0,
        }],
        vec![Op::MergeSlides {
            first: slides[0],
            second: slides[1],
        }],
        vec![Op::MergeSlides {
            first: slides[1],
            second: slides[2],
        }],
        vec![Op::AddElement {
            slide: slides[0],
            element: Element::new(Rect::new(1.0, 2.0, 3.0, 4.0)),
            index: Some(1),
        }],
        vec![Op::DeleteElement {
            slide: slides[0],
            element: el,
        }],
        vec![Op::ReorderElement {
            slide: slides[0],
            element: el,
            index: 0,
        }],
        vec![Op::SetElement {
            slide: slides[0],
            element: el,
            patch: ElementPatch {
                name: Some("Main".into()),
                bounds: Some(Rect::new(10.0, 20.0, 300.0, 400.0)),
                rotation: Some(12.0),
                opacity: Some(0.5),
                hidden: Some(true),
                fill: Some(model::Fill::color(Color::BLACK)),
                ..ElementPatch::default()
            },
        }],
        vec![Op::SetTextStyle {
            slide: slides[0],
            element: el,
            range: None,
            character: Some(CharStyle {
                font_family: Some("Other".into()),
                ..CharStyle::default()
            }),
            paragraph: Some(ParagraphStyle {
                alignment: Some(TextAlign::Left),
                ..ParagraphStyle::default()
            }),
            clear: vec!["color".into()],
        }],
        vec![Op::SetTextStyle {
            slide: slides[0],
            element: el,
            range: Some(TextRange { start: 3, end: 12 }),
            character: Some(CharStyle {
                italic: Some(true),
                ..CharStyle::default()
            }),
            paragraph: None,
            clear: vec![],
        }],
        vec![Op::SetTextBox {
            slide: slides[0],
            element: el,
            vertical_align: Some(VerticalAlign::Bottom),
            fit: Some(model::FitMode::ShrinkToFit),
            margins: None,
            outline: Some(model::TextOutline {
                enabled: true,
                ..Default::default()
            }),
            text_shadow: None,
            line_mask: None,
        }],
        vec![Op::ReplaceTextBox {
            slide: slides[0],
            element: el,
            text_box: TextBox::plain("replaced"),
        }],
        vec![Op::SetArrangements {
            arrangements: vec![],
            selected: None,
        }],
        vec![Op::SetArrangements {
            arrangements: vec![model::Arrangement {
                id: ArrangementId::new(),
                name: "Short".into(),
                sections: vec![secs[1]],
            }],
            selected: Some(None),
        }],
        // A batch: one undo step for all of it.
        vec![
            Op::SetSlideText {
                slide: slides[0],
                element: None,
                text: "a\nb\nc".into(),
                expect_text: None,
            },
            Op::SplitSlide {
                slide: slides[0],
                at_paragraph: 2,
            },
            Op::DeleteSection { section: secs[1] },
        ],
    ];
    for ops in cases {
        round_trip(&p, ops);
    }
}

#[test]
fn set_slide_text_keeps_box_style_and_unchanged_runs() {
    let p = sample();
    let s0 = p.slides().next().unwrap().id;
    let mut w = p.clone();
    apply_presentation(
        &mut w,
        &[Op::SetSlideText {
            slide: s0,
            element: None,
            text: "1Line one\nLine 2 changed\nnew third".into(),
            expect_text: None,
        }],
    )
    .unwrap();
    let old = p.slides().next().unwrap().elements[1].text_box().unwrap();
    let tb = w.slides().next().unwrap().elements[1].text_box().unwrap();
    assert_eq!(tb.default_style, old.default_style);
    assert_eq!(tb.default_paragraph, old.default_paragraph);
    // Unchanged first paragraph keeps both runs (superscript verse number).
    assert_eq!(tb.paragraphs[0], old.paragraphs[0]);
    assert_eq!(tb.paragraphs[0].runs.len(), 2);
    // Changed paragraph: style by index, one run with the old first run's style.
    assert_eq!(tb.paragraphs[1].style, old.paragraphs[1].style);
    assert_eq!(tb.paragraphs[1].runs.len(), 1);
    assert_eq!(
        tb.paragraphs[1].runs[0].style,
        old.paragraphs[1].runs[0].style
    );
    // New paragraph copies the last old paragraph's style.
    assert_eq!(tb.paragraphs[2].style, old.paragraphs[1].style);
    assert_eq!(tb.plain_text(), "1Line one\nLine 2 changed\nnew third");
    // The other elements are untouched.
    assert_eq!(
        w.slides().next().unwrap().elements[2],
        p.slides().next().unwrap().elements[2]
    );
}

#[test]
fn set_slide_text_keeps_chords_on_their_lines() {
    // H06-1: lines are matched by content, not by index.
    use model::ChordMark;
    let para = |t: &str, chords: Vec<ChordMark>| Paragraph {
        chords,
        ..Paragraph::text(t)
    };
    let tb = || TextBox {
        paragraphs: vec![
            para("Morning light", vec![ChordMark::new(0, "G")]),
            para(
                "Sing a song of joy",
                vec![
                    ChordMark::new(0, "C"),
                    ChordMark::new(7, "D"),
                    ChordMark::new(15, "A"),
                ],
            ),
            para("Evening rest", vec![ChordMark::new(0, "Em")]),
        ],
        ..TextBox::default()
    };
    let chords = |tb: &TextBox| -> Vec<Vec<(u32, String)>> {
        tb.paragraphs
            .iter()
            .map(|p| p.chords.iter().map(|c| (c.at, c.chord.clone())).collect())
            .collect()
    };
    let c = |v: &[(u32, &str)]| -> Vec<(u32, String)> {
        v.iter().map(|(a, s)| (*a, s.to_string())).collect()
    };

    // Enter (or Ctrl+Enter's split point) in the middle of a line: each chord stays with
    // its word, the lines after keep theirs.
    let mut t = tb();
    set_text(&mut t, "Morning light\nSing a\nsong of joy\nEvening rest");
    assert_eq!(
        chords(&t),
        vec![
            c(&[(0, "G")]),
            c(&[(0, "C")]),
            c(&[(0, "D"), (8, "A")]),
            c(&[(0, "Em")]),
        ]
    );
    assert_eq!(
        t.paragraphs[3],
        tb().paragraphs[2],
        "unchanged line kept whole"
    );

    // A blank line inserted at the top: no chord moves to another line; the new line has none.
    let mut t = tb();
    set_text(&mut t, "\nMorning light\nSing a song of joy\nEvening rest");
    assert_eq!(chords(&t)[0], c(&[]));
    assert_eq!(&chords(&t)[1..], &chords(&tb())[..]);

    // A line deleted: the others keep their own chords.
    let mut t = tb();
    set_text(&mut t, "Morning light\nEvening rest");
    assert_eq!(chords(&t), vec![c(&[(0, "G")]), c(&[(0, "Em")])]);

    // Two lines joined: the chords of both, on their words.
    let mut t = tb();
    set_text(&mut t, "Morning light Sing a song of joy\nEvening rest");
    assert_eq!(
        chords(&t)[0],
        c(&[(0, "G"), (14, "C"), (21, "D"), (29, "A")])
    );
}

#[test]
fn set_slide_text_creates_a_text_element_when_there_is_none() {
    let mut p = sample();
    let bare = Slide::new();
    let id = bare.id;
    p.sections[1].slides.push(bare);
    let a = apply_presentation(
        &mut p,
        &[Op::SetSlideText {
            slide: id,
            element: None,
            text: "Hello".into(),
            expect_text: Some(String::new()),
        }],
    )
    .unwrap();
    assert!(matches!(a.applied[0], Op::AddElement { .. }));
    let s = p.slides().find(|s| s.id == id).unwrap();
    let tb = s.elements[0].text_box().unwrap();
    assert_eq!(tb.plain_text(), "Hello");
    // The prevailing style: the nearest earlier slide's lyrics box.
    assert_eq!(tb.default_style.font_family.as_deref(), Some("Test Sans"));
    assert_eq!(s.elements[0].bounds, Rect::new(100.0, 100.0, 1720.0, 880.0));
}

#[test]
fn split_and_merge_keep_styles_and_geometry() {
    let p = sample();
    let s2 = p.slides().nth(2).unwrap().clone();
    let mut w = p.clone();
    let a = apply_presentation(
        &mut w,
        &[Op::SplitSlide {
            slide: s2.id,
            at_paragraph: 1,
        }],
    )
    .unwrap();
    assert_eq!(a.applied.len(), 2);
    assert_eq!(w.slides().count(), 4);
    let first = w.slides().nth(2).unwrap();
    let second = w.slides().nth(3).unwrap();
    assert_eq!(first.id, s2.id);
    assert_eq!(
        first.elements[1].text_box().unwrap().plain_text(),
        "1Chorus A"
    );
    let tb2 = second.elements[1].text_box().unwrap();
    assert_eq!(tb2.plain_text(), "Chorus B\nChorus C");
    assert_eq!(
        tb2.default_style,
        s2.elements[1].text_box().unwrap().default_style
    );
    assert_eq!(
        tb2.paragraphs[0].style,
        s2.elements[1].text_box().unwrap().paragraphs[1].style
    );
    assert_eq!(second.elements[1].bounds, s2.elements[1].bounds);
    assert_eq!(second.elements.len(), s2.elements.len());
    assert_ne!(second.elements[1].id, s2.elements[1].id);
    // Merging them back gives the original slide.
    let second_id = second.id;
    apply_presentation(
        &mut w,
        &[Op::MergeSlides {
            first: s2.id,
            second: second_id,
        }],
    )
    .unwrap();
    assert_eq!(json(&w), json(&p));
}

#[test]
fn split_and_merge_carry_the_second_language_text() {
    // H07-3 / R3-1: split gives each slide its part of the second text, merge appends it.
    let mut p = sample();
    let lyr = lyrics_id(&p, 2);
    let mut tr = model::SlideTranslation::new("es");
    tr.set_plain(lyr, "Coro A\nCoro B\nCoro C");
    p.sections[1].slides[0].translation = Some(tr);
    let s2 = p.slides().nth(2).unwrap().id;
    let second_text = |w: &Presentation, i: usize| {
        let s = w.slides().nth(i).unwrap();
        s.translation
            .as_ref()
            .map(|t| t.plain_text(s))
            .unwrap_or_default()
    };
    let split = round_trip(
        &p,
        vec![Op::SplitSlide {
            slide: s2,
            at_paragraph: 1,
        }],
    );
    assert_eq!(second_text(&split, 2), "Coro A");
    assert_eq!(second_text(&split, 3), "Coro B\nCoro C");
    let new_lyr = split.slides().nth(3).unwrap().elements[1].id;
    assert!(
        split
            .slides()
            .nth(3)
            .unwrap()
            .translation
            .as_ref()
            .unwrap()
            .text_of(new_lyr)
            .is_some(),
        "the new slide's second text names its own element"
    );

    // Merging them back gives the original slide, second text included; undo restores both.
    let second = split.slides().nth(3).unwrap().id;
    let merged = round_trip(&split, vec![Op::MergeSlides { first: s2, second }]);
    assert_eq!(json(&merged), json(&p));

    // Merging a translated slide into one without a second text keeps it.
    let mut q = p.clone();
    q.sections[1].slides[0].translation = None;
    let mut tr = model::SlideTranslation::new("es");
    tr.set_plain(q.slides().nth(1).unwrap().elements[1].id, "Línea tres");
    let first = q.slides().nth(1).unwrap().id;
    q.sections[0].slides[1].translation = Some(tr);
    let merged = round_trip(
        &q,
        vec![Op::MergeSlides {
            first: q.slides().next().unwrap().id,
            second: first,
        }],
    );
    assert_eq!(second_text(&merged, 0), "Línea tres");
    assert_eq!(
        merged
            .slides()
            .next()
            .unwrap()
            .translation
            .as_ref()
            .unwrap()
            .language,
        "es"
    );
}

#[test]
fn failures_apply_nothing() {
    let p = sample();
    let (_, slides) = ids(&p);
    // A deleted target.
    let mut w = p.clone();
    let e = apply_presentation(
        &mut w,
        &[
            Op::SetSlide {
                slide: slides[0],
                label: None,
                enabled: Some(false),
                notes: None,
                transition: None,
                background: None,
                hotkey: None,
                builds: None,
            },
            Op::DeleteSlide {
                slide: SlideId::new(),
            },
        ],
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert_eq!(e.op_index, 1);
    // An expectText mismatch reports the current text.
    let mut w = p.clone();
    let e = apply_presentation(
        &mut w,
        &[Op::SetSlideText {
            slide: slides[0],
            element: None,
            text: "mine".into(),
            expect_text: Some("stale".into()),
        }],
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert_eq!(e.text.as_deref(), Some("1Line one\nLine two"));
    // Malformed: a duplicate ID, an unknown style field.
    let mut w = p.clone();
    let dup = p.slides().next().unwrap().clone();
    let e = apply_presentation(
        &mut w,
        &[Op::AddSlide {
            section: p.sections[0].id,
            after: None,
            slide: Some(dup),
        }],
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::BadRequest);
    let e = apply_presentation(
        &mut w,
        &[Op::SetTextStyle {
            slide: slides[0],
            element: lyrics_id(&p, 0),
            range: None,
            character: None,
            paragraph: None,
            clear: vec!["bogus".into()],
        }],
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::BadRequest);
    // D03-1: scrolling speeds and gaps the renderer can't draw are refused.
    for (speed, gap) in [
        (1e300, 120.0),
        (f64::NAN, 120.0),
        (-5.0, 120.0),
        (120.0, 1e30),
    ] {
        let e = apply_presentation(
            &mut w,
            &[Op::SetElement {
                slide: slides[0],
                element: lyrics_id(&p, 0),
                patch: ElementPatch {
                    scroll: Some(model::TextScroll {
                        enabled: true,
                        speed,
                        gap,
                        ..Default::default()
                    }),
                    ..ElementPatch::default()
                },
            }],
        )
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::BadRequest, "{speed} {gap}");
    }
}

#[test]
fn range_styles_split_runs() {
    let p = sample();
    let (_, slides) = ids(&p);
    let el = lyrics_id(&p, 0);
    let mut w = p.clone();
    // "1Line one\nLine two": italicise "Line" of the second paragraph (chars 10..14).
    apply_presentation(
        &mut w,
        &[Op::SetTextStyle {
            slide: slides[0],
            element: el,
            range: Some(TextRange { start: 10, end: 14 }),
            character: Some(CharStyle {
                italic: Some(true),
                ..CharStyle::default()
            }),
            paragraph: Some(ParagraphStyle {
                alignment: Some(TextAlign::Right),
                ..ParagraphStyle::default()
            }),
            clear: vec![],
        }],
    )
    .unwrap();
    let tb = w.slides().next().unwrap().elements[1].text_box().unwrap();
    assert_eq!(tb.plain_text(), "1Line one\nLine two");
    assert_eq!(tb.paragraphs[0].style.alignment, None);
    assert_eq!(tb.paragraphs[1].style.alignment, Some(TextAlign::Right));
    let runs = &tb.paragraphs[1].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].text, "Line");
    assert_eq!(runs[0].style.italic, Some(true));
    assert_eq!(runs[1].text, " two");
    assert_eq!(runs[1].style.italic, None);
}

#[test]
fn playlist_ops_round_trip() {
    let header = PlaylistItem::new(PlaylistItemKind::Header {
        name: "Worship".into(),
        color: Some(Color::WHITE),
    });
    let song = PlaylistItem::new(PlaylistItemKind::Presentation {
        presentation: Link::named(model::PresentationId::new(), "Song"),
        arrangement: None,
        unresolved: None,
    });
    let items = vec![header.clone(), song.clone()];
    let new = PlaylistItem::new(PlaylistItemKind::Placeholder);
    let cases = vec![
        vec![PlaylistOp::AddItem {
            after: Some(header.id),
            item: new.clone(),
        }],
        vec![PlaylistOp::AddItem {
            after: None,
            item: new.clone(),
        }],
        vec![PlaylistOp::RemoveItem { item: header.id }],
        vec![PlaylistOp::MoveItem {
            item: header.id,
            after: Some(song.id),
        }],
        vec![PlaylistOp::SetItem {
            item: header.id,
            arrangement: None,
            enabled: Some(false),
            name: Some(Some("Sermon".into())),
            color: Some(None),
        }],
        vec![PlaylistOp::SetItem {
            item: song.id,
            arrangement: Some(Some(ArrangementId::new())),
            enabled: None,
            name: Some(Some("Opening song".into())),
            color: None,
        }],
    ];
    let js = |v: &Vec<PlaylistItem>| serde_json::to_string(v).unwrap();
    for ops in cases {
        let mut w = items.clone();
        let a = apply_playlist(&mut w, &ops).unwrap();
        assert_ne!(js(&w), js(&items), "{ops:?}");
        apply_playlist(&mut w, &a.inverse).unwrap();
        assert_eq!(js(&w), js(&items), "{ops:?}");
    }
    let mut w = items.clone();
    let e = apply_playlist(
        &mut w,
        &[PlaylistOp::RemoveItem {
            item: model::PlaylistItemId::new(),
        }],
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    let e = apply_playlist(
        &mut w,
        &[PlaylistOp::SetItem {
            item: header.id,
            arrangement: Some(None),
            enabled: None,
            name: None,
            color: None,
        }],
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::BadRequest);
}

#[test]
fn blank_and_duplicate_presentations() {
    let b = blank_presentation("New");
    assert_eq!(b.canvas_size, Size::HD);
    assert_eq!(b.sections.len(), 1);
    let s = b.slides().next().unwrap();
    assert!(primary_text(s).is_some());
    let p = sample();
    let d = duplicate_presentation(&p);
    assert_ne!(d.id, p.id);
    assert_eq!(d.slides().count(), p.slides().count());
    assert!(d.slides().zip(p.slides()).all(|(a, b)| a.id != b.id));
    // Arrangements follow the new section IDs.
    assert_eq!(d.arrangements[0].sections[0], d.sections[0].id);
    assert_eq!(d.selected_arrangement, Some(d.arrangements[0].id));
}

#[test]
fn slide_action_ops_round_trip() {
    use model::{ActionKind, SlideAction, TimerOperation};
    let mut p = sample();
    let slide = p.slides().next().unwrap().id;
    let timer = SlideAction::new(ActionKind::Timer {
        timer: Link::to(model::TimerId::new()),
        operation: TimerOperation::Start,
    });
    let prop = SlideAction::new(ActionKind::Prop {
        prop: Link::to(model::PropId::new()),
        show: true,
    });
    // Add to an empty list, then after it.
    let p1 = round_trip(
        &p,
        vec![Op::AddSlideAction {
            slide,
            after: None,
            action: timer.clone(),
        }],
    );
    let p2 = round_trip(
        &p1,
        vec![Op::AddSlideAction {
            slide,
            after: Some(timer.id),
            action: prop.clone(),
        }],
    );
    let order = |p: &Presentation| -> Vec<model::ActionId> {
        p.slides()
            .next()
            .unwrap()
            .actions
            .iter()
            .map(|a| a.id)
            .collect()
    };
    assert_eq!(order(&p2), vec![timer.id, prop.id]);
    // Move first, set, remove: each with its inverse.
    let p3 = round_trip(
        &p2,
        vec![Op::MoveSlideAction {
            slide,
            action: prop.id,
            after: None,
        }],
    );
    assert_eq!(order(&p3), vec![prop.id, timer.id]);
    let mut changed = timer.clone();
    changed.kind = ActionKind::Timer {
        timer: Link::to(model::TimerId::new()),
        operation: TimerOperation::Set { seconds: 90.0 },
    };
    changed.enabled = false;
    let p4 = round_trip(
        &p3,
        vec![Op::SetSlideAction {
            slide,
            action: changed.clone(),
        }],
    );
    assert_eq!(p4.slides().next().unwrap().actions[1], changed);
    let p5 = round_trip(
        &p4,
        vec![Op::RemoveSlideAction {
            slide,
            action: prop.id,
        }],
    );
    assert_eq!(order(&p5), vec![timer.id]);
    // Failures: duplicate ID, unknown target, moving after itself; nothing applied.
    p = p2.clone();
    for (op, code) in [
        (
            Op::AddSlideAction {
                slide,
                after: None,
                action: timer.clone(),
            },
            ErrorCode::BadRequest,
        ),
        (
            Op::RemoveSlideAction {
                slide,
                action: model::ActionId::new(),
            },
            ErrorCode::Conflict,
        ),
        (
            Op::MoveSlideAction {
                slide,
                action: timer.id,
                after: Some(model::ActionId::new()),
            },
            ErrorCode::Conflict,
        ),
        (
            Op::MoveSlideAction {
                slide,
                action: timer.id,
                after: Some(timer.id),
            },
            ErrorCode::BadRequest,
        ),
    ] {
        let mut w = p.clone();
        let e = apply_presentation(&mut w, &[op]).unwrap_err();
        assert_eq!(e.code, code);
        assert_eq!(order(&w), order(&p2));
    }
}

/// D07-1: after an object with a build is deleted, the slide's other build edits still apply
/// (the dangling build is sent back with the order), while a new build of a missing element
/// is still refused.
#[test]
fn a_build_of_a_deleted_object_does_not_block_build_edits() {
    use model::{Build, BuildMode, Transition};
    let p = sample();
    let (_, slides) = ids(&p);
    let s = p.slides().next().unwrap();
    let (a, b) = (s.elements[1].id, s.elements[2].id);
    let set = |builds: Vec<Build>| Op::SetSlide {
        slide: slides[0],
        label: None,
        enabled: None,
        notes: None,
        transition: None,
        background: None,
        hotkey: None,
        builds: Some(builds),
    };
    let ba = Build::new(a, BuildMode::In, Transition::CUT);
    let bb = Build::new(b, BuildMode::In, Transition::CUT);
    let mut w = p.clone();
    apply_presentation(&mut w, &[set(vec![ba.clone(), bb.clone()])]).unwrap();
    apply_presentation(
        &mut w,
        &[Op::DeleteElement {
            slide: slides[0],
            element: b,
        }],
    )
    .unwrap();
    // Another effect for A, with B's row kept (as the editor sends it).
    let ba2 = Build::new(a, BuildMode::In, Transition::dissolve(0.5));
    let done = apply_presentation(&mut w, &[set(vec![ba2.clone(), bb.clone()])])
        .unwrap_or_else(|e| panic!("{e:?}"));
    // Its inverse applies too.
    let mut u = w.clone();
    apply_presentation(&mut u, &done.inverse).unwrap_or_else(|e| panic!("{e:?}"));
    // Moving the rows.
    apply_presentation(&mut w, &[set(vec![bb.clone(), ba2.clone()])]).unwrap();
    // A build of an element the slide doesn't have is kept (and skipped); too many are not.
    let ghost = Build::new(ElementId::new(), BuildMode::In, Transition::CUT);
    apply_presentation(&mut w, &[set(vec![ba2.clone(), bb.clone(), ghost])])
        .unwrap_or_else(|e| panic!("{e:?}"));
    let e = apply_presentation(&mut w, &[set(vec![ba2; MAX_BUILDS + 1])]).unwrap_err();
    assert_eq!(e.code, ErrorCode::BadRequest, "{e:?}");
}

/// Batch 10b: the "Deleted object" row removed in the Build tab, then Undo: the undo sends the
/// order with that build again (no longer on the slide), and it applies. Undoing the object's
/// delete then brings its build back into play.
#[test]
fn removing_a_deleted_objects_build_can_be_undone() {
    use model::{Build, BuildMode, Transition, build::step_count};
    let p = sample();
    let (_, slides) = ids(&p);
    let s = p.slides().next().unwrap();
    let (a, b) = (s.elements[1].id, s.elements[2].id);
    let set = |builds: Vec<Build>| Op::SetSlide {
        slide: slides[0],
        label: None,
        enabled: None,
        notes: None,
        transition: None,
        background: None,
        hotkey: None,
        builds: Some(builds),
    };
    let first = |w: &Presentation| w.slides().next().unwrap().clone();
    let ba = Build::new(a, BuildMode::In, Transition::CUT);
    let bb = Build::new(b, BuildMode::In, Transition::CUT);
    let mut w = p.clone();
    apply_presentation(&mut w, &[set(vec![ba.clone(), bb.clone()])]).unwrap();
    let deleted = apply_presentation(
        &mut w,
        &[Op::DeleteElement {
            slide: slides[0],
            element: b,
        }],
    )
    .unwrap();
    // B's build dangles: it takes no step.
    assert_eq!(step_count(&first(&w)), 1);
    // Remove B's row (the "Deleted object" row), then undo that.
    let removed = apply_presentation(&mut w, &[set(vec![ba.clone()])]).unwrap();
    apply_presentation(&mut w, &removed.inverse).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(first(&w).builds, vec![ba.clone(), bb.clone()]);
    assert_eq!(step_count(&first(&w)), 1);
    // Undo the delete: B is back with its build, which plays again.
    apply_presentation(&mut w, &deleted.inverse).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(step_count(&first(&w)), 2);
}

#[test]
fn apply_chordpro_refuses_a_text_past_the_parse_limit() {
    // H06-3: the same limit as `POST /chordpro/parse`.
    let mut p = sample();
    let big = "[C]Line one\n".repeat(MAX_CHORDPRO_TEXT / 10);
    assert!(big.len() > MAX_CHORDPRO_TEXT);
    let e = apply_presentation(&mut p, &[Op::ApplyChordPro { text: big }]).unwrap_err();
    assert_eq!(e.code, ErrorCode::BadRequest);
    assert!(e.message.contains("too long"), "{}", e.message);
    // Many distinct lines within the limit are matched quickly (a map, not a scan).
    let text: String = (0..20_000)
        .map(|i| format!("[G]Synthetic line {i}\n"))
        .collect();
    let started = std::time::Instant::now();
    let a = apply_presentation(
        &mut p,
        &[Op::ApplyChordPro {
            text: format!("{text}[D]Line four\n"),
        }],
    )
    .unwrap();
    assert!(!a.applied.is_empty());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

/// `setPresentation {language}`: stored in its usual case, cleared with `null`, undone; a value
/// that is not a language code is refused and nothing changes.
#[test]
fn a_presentations_language_is_checked_and_stored_in_its_usual_case() {
    let set = |language: Option<&str>| Op::SetPresentation {
        name: None,
        song_info: None,
        scripture: None,
        transition: None,
        canvas_size: None,
        notes: None,
        category: None,
        language: Some(language.map(str::to_string)),
    };
    let p = sample();
    let edited = round_trip(&p, vec![set(Some(" pt_br "))]);
    assert_eq!(edited.language.as_deref(), Some("pt-BR"));
    let cleared = round_trip(&edited, vec![set(None)]);
    assert_eq!(cleared.language, None);
    for bad in ["Spanish language", "e", "en-", "español"] {
        let mut w = p.clone();
        let err = apply_presentation(&mut w, &[set(Some(bad))]).unwrap_err();
        assert!(
            format!("{err:?}").contains("not a language code"),
            "{bad}: {err:?}"
        );
        assert_eq!(json(&w), json(&p));
    }
}
