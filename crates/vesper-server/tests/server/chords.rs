//! Batch 16, chords: ChordPro into a presentation and back (one language or two),
//! `applyChordPro` and chords that follow edited words, and a stage screen whose chord chart draws a chord **above** its word.

use crate::common::*;
use model::{
    ChordDisplay, ChordMark, ChordView, Color, DataLink, Presentation, Rect, Screen, ScreenKind,
    Section, Size, Slide, SlideWhich, SongInfo, StageLayout,
};
use serde_json::json;

/// The stage screen's index among the running screens (Audience, Key, Stage).
const STAGE: usize = 2;
const GREEN: [u8; 3] = [0, 255, 0];

const SONG: &str = "\
{title: Harbour Lights}
{key: G}
{capo: 2}

{start_of_verse}
[G]Lanterns on the [C]shore
[Em]Calling us [D]home
{end_of_verse}

{soc}
[C]Sing, [G/B]sing
{eoc}

{chorus}
";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chordpro_in_and_out_and_chords_that_follow_the_words() {
    let fx = fixture("chordpro");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;

    // Parse: a new presentation (not saved), styled like the fixture's song.
    let r = http(
        addr,
        "POST",
        "/api/v1/chordpro/parse",
        &[],
        Some(json!({"text": SONG, "like": fx.song.id})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let parsed = r.json();
    assert_eq!(parsed["sections"], 2);
    assert_eq!(parsed["slides"], 2);
    assert_eq!(parsed["chordLines"], 3);
    assert_eq!(parsed["capo"], 2);
    let p: Presentation = serde_json::from_value(parsed["presentation"].clone()).unwrap();
    assert_eq!(p.name, "Harbour Lights");
    assert_eq!(
        p.song_info.as_ref().unwrap().original_key.as_deref(),
        Some("G")
    );
    assert_eq!(p.sections[0].name, "Verse 1");
    assert_eq!(p.sections[1].name, "Chorus");
    assert_eq!(p.arrangements[0].sections.len(), 3, "the chorus repeats");
    let tb = p.sections[0].slides[0].elements[0].text_box().unwrap();
    assert_eq!(tb.plain_text(), "Lanterns on the shore\nCalling us home");
    assert_eq!(
        tb.paragraphs[0].chords,
        vec![ChordMark::new(0, "G"), ChordMark::new(16, "C")]
    );
    assert!(!tb.chords.show, "audience text never shows chords");
    // Nothing in it: 400.
    let r = http(
        addr,
        "POST",
        "/api/v1/chordpro/parse",
        &[],
        Some(json!({"text": "{title: Empty}"})),
    )
    .await;
    assert_eq!(r.status, 400);

    // Create it, then edit a line's words: its chords stay on their words.
    let r = http(
        addr,
        "POST",
        "/api/v1/presentations",
        &[],
        Some(json!({"document": p, "library": fx.library.id})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let slide = p.sections[0].slides[0].id;
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{}/edits", p.id.0),
        &[],
        Some(json!({"ops": [{
            "type": "setSlideText",
            "slide": slide,
            "text": "Lanterns along the shore\nCalling us home",
        }]})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let doc: Presentation =
        serde_json::from_value(get_json(addr, &format!("/api/v1/presentations/{}", p.id.0)).await)
            .unwrap();
    let tb = doc.sections[0].slides[0].elements[0].text_box().unwrap();
    assert_eq!(
        tb.paragraphs[0].chords,
        vec![ChordMark::new(0, "G"), ChordMark::new(19, "C")],
        "the C stays on \"shore\""
    );

    // Export: ChordPro text with the chords inline.
    let r = http(
        addr,
        "GET",
        &format!("/api/v1/presentations/{}/chordpro", p.id.0),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert!(r.header("content-type").unwrap().starts_with("text/plain"));
    let text = String::from_utf8(r.body.clone()).unwrap();
    assert!(text.contains("{key: G}"), "{text}");
    assert!(
        text.contains("[G]Lanterns along the [C]shore\n[Em]Calling us [D]home"),
        "{text}"
    );

    // applyChordPro: the fixture's song gets the chords of its matching lines.
    let chords = "{key: D}\n[D]one\n[A/C#]tw[Bm]o\n";
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{}/edits", fx.song.id.0),
        &[],
        Some(json!({"ops": [{"type": "applyChordPro", "text": chords}]})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let doc: Presentation = serde_json::from_value(
        get_json(addr, &format!("/api/v1/presentations/{}", fx.song.id.0)).await,
    )
    .unwrap();
    let marks = |i: usize| {
        doc.slides()
            .nth(i)
            .unwrap()
            .elements
            .iter()
            .find_map(|e| e.text_box())
            .unwrap()
            .paragraphs[0]
            .chords
            .clone()
    };
    assert_eq!(
        marks(0),
        vec![ChordMark::new(0, "D")],
        "\"ONE\" matches \"one\""
    );
    assert_eq!(
        marks(1),
        vec![ChordMark::new(0, "A/C#"), ChordMark::new(2, "Bm")]
    );
    assert_eq!(
        doc.song_info.unwrap().original_key.as_deref(),
        Some("D"),
        "the key, as the song had none"
    );
    // No matching line: 400, nothing changed.
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{}/edits", fx.song.id.0),
        &[],
        Some(json!({"ops": [{"type": "applyChordPro", "text": "[C]elsewhere"}]})),
    )
    .await;
    assert_eq!(r.status, 400);
}

/// A stage screen with a chord chart (white words, green chords) shows the chord above its
/// word, at its left.
/// A made-up two-language song: the second language on alternating lines.
const TWO: &str = "\
{title: Harbour Lights}
{meta: language en}
{meta: second_language es}

{start_of_verse}
[G]Lanterns on the [C]shore
Faroles en la orilla
[Em]Calling us [D]home
Llamándonos a casa
{end_of_verse}
";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_two_language_chordpro_song_in_and_out() {
    let fx = fixture("chordpro-two");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;

    let r = http(
        addr,
        "POST",
        "/api/v1/chordpro/parse",
        &[],
        Some(json!({"text": TWO, "split": {"type": "alternating"}})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let parsed = r.json();
    assert_eq!(parsed["slides"], 1);
    assert_eq!(parsed["chordLines"], 2);
    assert_eq!(parsed["secondLanguageSlides"], 1);
    let p: Presentation = serde_json::from_value(parsed["presentation"].clone()).unwrap();
    assert_eq!(p.language.as_deref(), Some("en"), "the song's own language");
    let slide = &p.sections[0].slides[0];
    let el = &slide.elements[0];
    assert_eq!(
        el.text_box().unwrap().plain_text(),
        "Lanterns on the shore\nCalling us home",
        "the first language keeps the lines"
    );
    assert_eq!(el.text_box().unwrap().paragraphs[1].chords.len(), 2);
    let tr = slide.translation.as_ref().expect("a second text");
    assert_eq!(tr.language, "es");
    let second: Vec<String> = tr
        .text_of(el.id)
        .unwrap()
        .paragraphs
        .iter()
        .map(|p| p.plain_text())
        .collect();
    assert_eq!(second, ["Faroles en la orilla", "Llamándonos a casa"]);
    assert!(
        tr.text_of(el.id)
            .unwrap()
            .paragraphs
            .iter()
            .all(|p| p.chords.is_empty()),
        "no chords on the second language"
    );
    // Read as one language (no split): four lyric lines, no second text.
    let r = http(
        addr,
        "POST",
        "/api/v1/chordpro/parse",
        &[],
        Some(json!({"text": TWO})),
    )
    .await;
    assert_eq!(r.json()["secondLanguageSlides"], 0);
    assert_eq!(r.json()["unpairedStanzas"], 0);

    // A slide per line: the translations go with their own lines. Three lines and two
    // translations don't pair, so both stay on the stanza's first slide and the answer says
    // so. A stanza with only second-language lines keeps them, on a slide with an empty text.
    let parse = |text: &str, lines: u32| {
        http(
            addr,
            "POST",
            "/api/v1/chordpro/parse",
            &[],
            Some(json!({"text": text, "split": {"type": "line"}, "linesPerSlide": lines})),
        )
    };
    let seconds = |v: &serde_json::Value| -> Vec<Vec<String>> {
        let p: Presentation = serde_json::from_value(v["presentation"].clone()).unwrap();
        p.slides()
            .map(|s| {
                s.translation
                    .as_ref()
                    .map(|t| {
                        t.texts[0]
                            .paragraphs
                            .iter()
                            .map(|p| p.plain_text())
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect()
    };
    let r = parse("[G]One\nTwo\n---\nUno\nDos\n", 1).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(seconds(&r.json()), [vec!["Uno"], vec!["Dos"]]);
    assert_eq!(r.json()["unpairedStanzas"], 0);
    let r = parse("One\nTwo\nThree\n---\nUno\nDos\n\n---\nTres\n", 2).await;
    let v = r.json();
    assert_eq!(v["unpairedStanzas"], 1);
    assert_eq!(
        seconds(&v),
        [vec!["Uno", "Dos"], vec![], vec!["Tres"]],
        "{v}"
    );
    let q: Presentation = serde_json::from_value(v["presentation"].clone()).unwrap();
    let last = q.slides().last().unwrap();
    assert_eq!(last.elements[0].text_box().unwrap().plain_text(), "");

    // Create it and export it: the second language comes back as italic comments under the
    // lines, and the header says so; reading that file again (auto) gives the same slide.
    let r = http(
        addr,
        "POST",
        "/api/v1/presentations",
        &[],
        Some(json!({"document": p, "library": fx.library.id})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let r = http(
        addr,
        "GET",
        &format!("/api/v1/presentations/{}/chordpro", p.id.0),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    let text = String::from_utf8(r.body.clone()).unwrap();
    assert!(
        text.contains("{meta: second_language es}\n{meta: second_language_split comments}\n"),
        "{text}"
    );
    assert!(
        text.contains("[G]Lanterns on the [C]shore\n[Em]Calling us [D]home\n{ci: Faroles en la orilla}\n{ci: Llamándonos a casa}\n"),
        "{text}"
    );
    let r = http(
        addr,
        "POST",
        "/api/v1/chordpro/parse",
        &[],
        Some(json!({"text": text, "split": {"type": "auto"}})),
    )
    .await;
    let again: Presentation = serde_json::from_value(r.json()["presentation"].clone()).unwrap();
    let s2 = &again.sections[0].slides[0];
    assert_eq!(
        s2.elements[0].text_box().unwrap().paragraphs,
        el.text_box().unwrap().paragraphs
    );
    let tr2 = s2.translation.as_ref().unwrap();
    assert_eq!(tr2.language, "es");
    assert_eq!(
        tr2.text_of(s2.elements[0].id).unwrap().paragraphs,
        tr.text_of(el.id).unwrap().paragraphs
    );
    assert!(text.contains("{meta: language en}\n"), "{text}");
    assert_eq!(again.language.as_deref(), Some("en"));

    // The presentation's language through `setPresentation`: stored in its usual case, a value
    // that is not a code refused (nothing changes), `null` clears it.
    let pid = p.id.0.to_string();
    let set = |language: serde_json::Value| json!({"ops": [{"type": "setPresentation", "language": language}]});
    let edit = |body: serde_json::Value| {
        let path = format!("/api/v1/presentations/{pid}/edits");
        async move { http(addr, "POST", &path, &[], Some(body)).await }
    };
    let doc =
        |pid: String| async move { get_json(addr, &format!("/api/v1/presentations/{pid}")).await };
    let r = edit(set(json!("pt_br"))).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(doc(pid.clone()).await["language"], "pt-BR");
    let r = edit(set(json!("Portuguese"))).await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(doc(pid.clone()).await["language"], "pt-BR");
    let r = edit(set(json!(null))).await;
    assert_eq!(r.status, 200);
    assert!(doc(pid.clone()).await.get("language").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chord_renders_above_its_word_on_the_stage() {
    let fx = fixture("stage-chords");
    let screen = Screen {
        name: "Band".into(),
        sort_key: "c".into(),
        size: Size::new(W as f64, H as f64),
        frame_rate: model::FrameRate::FPS30,
        kind: ScreenKind::Stage { layout: None },
        ..Screen::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "screen",
        &screen.id.0.to_string(),
        &screen,
    );
    // The layout: a chord chart across the screen, left-aligned, top; green chords.
    let mut chart = text_element(Rect::new(20.0, 20.0, 600.0, 320.0), "", Color::WHITE);
    chart.data_link = Some(DataLink::ChordChart {
        which: SlideWhich::Current,
    });
    if let Some(tb) = chart.text_box_mut() {
        tb.default_style.size = Some(60.0);
        tb.default_paragraph.alignment = Some(model::TextAlign::Left);
        tb.vertical_align = model::VerticalAlign::Top;
        tb.chords = ChordDisplay {
            show: true,
            color: Some(Color::rgb(0.0, 1.0, 0.0)),
            size: Some(0.6),
        };
    }
    let layout = StageLayout {
        name: "Band".into(),
        size: Size::new(W as f64, H as f64),
        slide: Slide {
            elements: vec![chart],
            ..Slide::new()
        },
        chords: ChordView {
            capo: 2,
            ..ChordView::default()
        },
        ..StageLayout::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "stageLayout",
        &layout.id.0.to_string(),
        &layout,
    );
    // A song whose one line has a chord over its second word ("high", on the right).
    let mut words = text_element(Rect::new(0.0, 0.0, 640.0, 360.0), "", Color::WHITE);
    if let Some(tb) = words.text_box_mut() {
        tb.paragraphs = model::Paragraph::from_lines("Lift      high");
        tb.paragraphs[0].chords = vec![ChordMark::new(10, "A")];
    }
    let mut sec = Section::new("Verse 1");
    sec.slides = vec![Slide {
        elements: vec![words],
        ..Slide::new()
    }];
    let song = Presentation {
        library: Some(fx.library.id),
        canvas_size: Size::new(W as f64, H as f64),
        sections: vec![sec],
        song_info: Some(SongInfo {
            original_key: Some("G".into()),
            transpose: 2,
            ..SongInfo::default()
        }),
        ..Presentation::new("Band Song")
    };
    write_doc(
        &fx.dir,
        "portable",
        "presentation",
        &song.id.0.to_string(),
        &song,
    );
    let server = start_server(&fx, true, false).await;
    assert_eq!(server.shared.screens()[STAGE].id, screen.id);
    let mut a = Client::connect(server.addr, "protocolVersion=1").await;
    a.ok(json!({"type": "triggerSlide", "presentation": song.id, "index": 0}))
        .await;
    let img = wait_frame(&server, STAGE, "stage chord chart", |i| {
        count(i, 0, H, GREEN, 60) > 40 && count(i, 0, H, WHITE, 40) > 200
    })
    .await;
    // Bounding boxes of the chord (green) and the words (white).
    let bbox = |c: [u8; 3], tol: i32| {
        let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0, u32::MAX, 0);
        for (x, y, p) in img.enumerate_pixels() {
            if p.0[3] > 200 && near(p, c, tol) {
                (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
            }
        }
        (x0, x1, y0, y1)
    };
    let (gx0, gx1, gy0, gy1) = bbox(GREEN, 60);
    let (wx0, wx1, wy0, _wy1) = bbox(WHITE, 40);
    assert!(
        gy1 < wy0,
        "the chord ({gy0}..{gy1}) is above the words (from {wy0})"
    );
    // Over "high" (the right part of the line), not over "Lift".
    let mid = (wx0 + wx1) / 2;
    assert!(
        gx0 > mid,
        "the chord ({gx0}..{gx1}) sits over its word (words {wx0}..{wx1})"
    );
    // Capo 2 on a song in G played in A: one letter, shapes of G → the chord drawn is "G"
    // (narrow); nothing else is green.
    assert!(gx1 - gx0 < 80, "one chord letter ({gx0}..{gx1})");
}
