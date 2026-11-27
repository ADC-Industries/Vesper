//! Song round trip: a synthetic two-section song with rich text and a background video.

use model::*;
use serde_json::json;

fn lyric_box(paragraphs: Vec<Paragraph>) -> Element {
    let mut e = Element::text(
        Rect::new(55.0, 55.0, 1810.0, 970.0),
        TextBox {
            paragraphs,
            default_style: CharStyle {
                font_family: Some("Calibri".into()),
                font_weight: Some(700),
                postscript_name: Some("Calibri-Bold".into()),
                size: Some(100.0),
                color: Some(Color::WHITE),
                ..CharStyle::default()
            },
            default_paragraph: ParagraphStyle {
                alignment: Some(TextAlign::Center),
                ..ParagraphStyle::default()
            },
            fit: FitMode::ShrinkToFit,
            text_shadow: Shadow {
                enabled: true,
                offset: 15.0,
                radius: 8.0,
                ..Shadow::default()
            },
            outline: TextOutline {
                enabled: true,
                ..TextOutline::default()
            },
            ..TextBox::default()
        },
    );
    e.placeholder = Some(Placeholder::text("text1"));
    e.themed = true;
    e
}

fn song() -> Presentation {
    let background = MediaAssetId::new();
    let mut trigger = MediaTrigger::new(background, MediaLayer::Background);
    trigger.playback.repeat = Repeat::Loop;
    trigger.playback.fade_in = Fade {
        enabled: true,
        seconds: None,
    };
    trigger.playback.retrigger = Retrigger::Automatic;
    trigger.display.scale = ScaleMode::Fill;

    let verse_slide = Slide {
        elements: vec![lyric_box(vec![
            Paragraph {
                runs: vec![
                    TextRun::styled(
                        "1",
                        CharStyle {
                            baseline: Some(Baseline::Superscript),
                            ..CharStyle::default()
                        },
                    ),
                    TextRun::plain("Morning light on quiet hills"),
                ],
                ..Paragraph::default()
            },
            Paragraph {
                runs: vec![
                    TextRun::plain("we sing a "),
                    TextRun::styled(
                        "new song",
                        CharStyle {
                            capitalization: Some(Capitalization::AllCaps),
                            ..CharStyle::default()
                        },
                    ),
                ],
                ..Paragraph::default()
            },
            Paragraph {
                style: ParagraphStyle {
                    alignment: Some(TextAlign::Right),
                    ..ParagraphStyle::default()
                },
                runs: vec![TextRun::plain("and walk on")],
                ..Paragraph::default()
            },
            // An empty line whose height comes from its end style.
            Paragraph {
                end_style: CharStyle {
                    size: Some(40.0),
                    ..CharStyle::default()
                },
                ..Paragraph::default()
            },
        ])],
        actions: vec![SlideAction::media(trigger)],
        label: Some(SlideLabel {
            text: "v1".into(),
            color: None,
        }),
        ..Slide::new()
    };
    let chorus_slide = Slide {
        elements: vec![lyric_box(vec![Paragraph::text("Sing it out, sing it out")])],
        hotkey: Some(HotKey::key("KeyC")),
        ..Slide::new()
    };

    let mut verse = Section::new("Verse 1");
    verse.color = Some(Color::from_rgb8(0, 102, 255));
    verse.global_group = Some(Link::named(GlobalGroupId::new(), "Verse 1"));
    verse.slides.push(verse_slide);
    let mut chorus = Section::new("Chorus");
    chorus.slides.push(chorus_slide);

    let mut p = Presentation::new("Morning Light");
    p.library = Some(LibraryId::new());
    p.song_info = Some(SongInfo {
        title: Some("Morning Light".into()),
        ccli_number: Some("0001234".into()),
        authors: Some("A. Writer, B. Writer and C. Writer".into()),
        copyright_year: Some(2001),
        display: true,
        ..SongInfo::default()
    });
    p.arrangements.push(Arrangement {
        id: ArrangementId::new(),
        name: "Sunday".into(),
        sections: vec![verse.id, chorus.id, chorus.id],
    });
    p.sections = vec![verse, chorus];
    p.import = Some(ImportInfo {
        unmapped: json!({"selectedArrangement": "dangling"}),
        ..ImportInfo::propresenter("Libraries/Synthetic/Morning Light.pro")
    });
    p
}

#[test]
fn song_round_trips_exactly() {
    let p = song();
    let text = serde_json::to_string_pretty(&p).unwrap();
    let back: Presentation = serde_json::from_str(&text).unwrap();
    assert_eq!(back, p);
    // Also through the storage path (migrate + decode).
    let back: Presentation = decode(serde_json::to_value(&p).unwrap()).unwrap();
    assert_eq!(back, p);
}

#[test]
fn song_json_shape() {
    let p = song();
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["schemaVersion"], json!(SCHEMA_VERSION));
    assert_eq!(v["songInfo"]["ccliNumber"], json!("0001234"));
    assert!(
        v.get("canvasSize").is_none(),
        "default canvas size is omitted"
    );

    let slide = &v["sections"][0]["slides"][0];
    let el = &slide["elements"][0];
    assert_eq!(el["kind"]["type"], json!("text"));
    assert_eq!(el["placeholder"], json!({"slot": "text1"}));
    assert!(
        el.get("fill").is_none() && el.get("stroke").is_none(),
        "disabled defaults omitted"
    );
    let tb = &el["kind"]["text"];
    assert_eq!(tb["fit"], json!("shrinkToFit"));
    assert_eq!(
        tb["textShadow"],
        json!({"enabled": true, "offset": 15.0, "radius": 8.0})
    );
    assert_eq!(tb["outline"], json!({"enabled": true}));
    let paras = &tb["paragraphs"];
    assert_eq!(
        paras[0]["runs"][0],
        json!({"text": "1", "style": {"baseline": "superscript"}})
    );
    assert_eq!(
        paras[0]["runs"][1],
        json!({"text": "Morning light on quiet hills"})
    );
    assert_eq!(
        paras[1]["runs"][1]["style"],
        json!({"capitalization": "allCaps"})
    );
    assert_eq!(paras[2]["style"], json!({"alignment": "right"}));
    assert_eq!(paras[3], json!({"endStyle": {"size": 40.0}}));

    let action = &slide["actions"][0];
    assert_eq!(action["kind"]["type"], json!("media"));
    assert!(
        action["kind"].get("layer").is_none(),
        "background is the default layer"
    );
    let playback = &action["kind"]["playback"];
    assert_eq!(playback["fadeIn"], json!({"enabled": true}));
    assert!(
        playback.get("retrigger").is_none(),
        "automatic is the default"
    );
    assert_eq!(playback["repeat"], json!({"type": "loop"}));
}

#[test]
fn song_semantics() {
    let p = song();
    let order: Vec<&str> = p
        .play_order(Some(p.arrangements[0].id))
        .into_iter()
        .map(|(sec, _)| p.section(sec).unwrap().name.as_str())
        .collect();
    assert_eq!(
        order,
        ["Verse 1", "Chorus", "Chorus"],
        "repeated sections are preserved"
    );

    let slide = p.slides().next().unwrap();
    let tb = slide.elements[0].text_box().unwrap();
    let sup = tb.run_style(&tb.paragraphs[0].runs[0]);
    assert_eq!(sup.baseline, Some(Baseline::Superscript));
    assert_eq!(sup.font_family.as_deref(), Some("Calibri"));
    assert_eq!(sup.postscript_name.as_deref(), Some("Calibri-Bold"));
    assert_eq!(
        tb.paragraph_style(&tb.paragraphs[2]).alignment,
        Some(TextAlign::Right)
    );
    assert_eq!(
        tb.paragraph_style(&tb.paragraphs[0]).alignment,
        Some(TextAlign::Center)
    );
    assert_eq!(tb.empty_paragraph_style(&tb.paragraphs[3]).size, Some(40.0));

    let ActionKind::Media(t) = &slide.actions[0].kind else {
        panic!("media action")
    };
    assert_eq!(t.layer.layer(), Layer::Media);
    let show = ShowSettings::default();
    assert_eq!(
        t.playback.fade_in.effective(show.default_media_fade),
        Some(0.5)
    );
    assert_eq!(t.playback.fade_out.effective(show.default_media_fade), None);

    // Transition resolution: slide → presentation → layer default.
    let layer_default = show.default_transitions.for_layer(Layer::Slide);
    assert_eq!(
        Transition::resolve(None, None, layer_default),
        &LayerTransitions::BUILT_IN
    );
    let pres = Transition::dissolve(2.0);
    assert_eq!(Transition::resolve(None, Some(&pres), layer_default), &pres);
    assert_eq!(
        Transition::resolve(Some(&Transition::CUT), Some(&pres), layer_default),
        &Transition::CUT
    );
}
