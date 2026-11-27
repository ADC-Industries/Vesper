//! Theme application (copy, never deletes content) and the lower-thirds look restyle.

use model::*;
use serde_json::json;

fn text_el(slot: Option<&str>, name: &str, text: &str) -> Element {
    let mut e = Element::text(Rect::new(0.0, 0.0, 100.0, 50.0), TextBox::plain(text));
    e.name = name.into();
    e.placeholder = slot.map(Placeholder::text);
    e.themed = slot.is_some();
    e
}

fn decoration(color: Color) -> Element {
    let mut e = Element::new(Rect::new(0.0, 760.0, 1920.0, 320.0));
    e.fill = Fill::color(color);
    e.themed = true;
    e
}

/// A banner template slide: a themed bar, one text slot styled big and white.
fn lower_third_template() -> Slide {
    let mut slot = text_el(Some("text1"), "Lyrics", "Sample lyric text");
    slot.bounds = Rect::new(20.0, 760.0, 1880.0, 320.0);
    let tb = slot.text_box_mut().unwrap();
    tb.default_style = CharStyle {
        font_family: Some("Calibri".into()),
        font_weight: Some(700),
        size: Some(100.0),
        color: Some(Color::WHITE),
        ..CharStyle::default()
    };
    tb.fit = FitMode::ShrinkToFit;
    slot.stroke = Stroke {
        enabled: true,
        ..Stroke::default()
    };
    slot.shadow = Shadow {
        enabled: true,
        offset: 9.0,
        ..Shadow::default()
    };
    Slide {
        elements: vec![decoration(Color::rgb(0.1, 0.5, 1.0)), slot],
        ..Slide::new()
    }
}

#[test]
fn apply_template_never_deletes_content() {
    let image = Element::media(Rect::new(0.0, 0.0, 1920.0, 1080.0), MediaAssetId::new());
    let old_decoration = decoration(Color::BLACK);
    let mut lyric = text_el(Some("text1"), "Lyrics", "First line");
    lyric.text_box_mut().unwrap().paragraphs[0].runs[0].style = CharStyle {
        color: Some(Color::rgb(1.0, 0.0, 0.0)),
        italic: Some(true),
        ..CharStyle::default()
    };
    let extra = text_el(Some("reference"), "Reference", "Extra text");
    let user_box = text_el(None, "Note", "User text");
    let mut clock = text_el(None, "Clock", "");
    clock.data_link = Some(DataLink::Clock {
        format: ClockFormat::default(),
    });
    let slide = Slide {
        elements: vec![
            image.clone(),
            old_decoration.clone(),
            lyric.clone(),
            extra.clone(),
            user_box.clone(),
            clock.clone(),
        ],
        ..Slide::new()
    };

    let template = lower_third_template();
    let out = apply_template(&slide, &template);
    let ids: Vec<ElementId> = out.elements.iter().map(|e| e.id).collect();

    // The image fill, the user element and the data-link element survive unchanged.
    for kept in [&image, &user_box, &clock] {
        let e = out.element(kept.id).expect("kept element");
        assert_eq!(e, kept);
    }
    // The old theme's decoration is replaced by the template's.
    assert!(!ids.contains(&old_decoration.id));
    assert_eq!(out.elements[0].fill, template.elements[0].fill);
    assert_ne!(
        out.elements[0].id, template.elements[0].id,
        "template copies get fresh IDs"
    );

    // The text slot keeps the old element ID, takes the template style, keeps inline italic,
    // drops the inline colour, and receives the leftover text instead of dropping it.
    let slot = out
        .element(lyric.id)
        .expect("matched slot keeps the old ID");
    let tb = slot.text_box().unwrap();
    assert_eq!(tb.default_style.size, Some(100.0));
    assert!(slot.stroke.enabled && slot.shadow.enabled);
    assert_eq!(tb.plain_text(), "First line\nExtra text");
    assert_eq!(
        tb.paragraphs[0].runs[0].style,
        CharStyle {
            italic: Some(true),
            ..CharStyle::default()
        }
    );
    assert!(
        !ids.contains(&extra.id),
        "leftover text is merged into the last slot"
    );

    // Template first, kept user content on top in original order.
    assert_eq!(ids.len(), 2 + 3);
    assert_eq!(&ids[2..], &[image.id, user_box.id, clock.id]);
    assert_eq!(out.id, slide.id);
}

#[test]
fn apply_template_without_text_slot_keeps_text_as_element() {
    let lyric = text_el(Some("text1"), "Lyrics", "Keep me");
    let slide = Slide {
        elements: vec![lyric.clone()],
        ..Slide::new()
    };
    let bare_template = Slide {
        elements: vec![decoration(Color::WHITE)],
        ..Slide::new()
    };
    let out = apply_template(&slide, &bare_template);
    assert_eq!(out.element(lyric.id), Some(&lyric));
    assert_eq!(out.plain_text(), "Keep me");
}

#[test]
fn theme_apply_records_origin() {
    let template = TemplateSlide {
        name: "Banner".into(),
        slide: lower_third_template(),
        ..TemplateSlide::default()
    };
    let theme = Theme {
        name: "Banner Styles".into(),
        slides: vec![template.clone()],
        ..Theme::default()
    };
    let slide = Slide {
        elements: vec![text_el(Some("text1"), "", "Hello")],
        ..Slide::new()
    };
    let out = theme.apply(template.id, &slide).unwrap();
    let origin = out.template_origin.unwrap();
    assert_eq!(origin.slide, template.id);
    assert_eq!(origin.theme, Link::named(theme.id, "Banner Styles"));
    assert!(theme.apply(TemplateSlideId::new(), &slide).is_none());
}

fn lower_thirds_look(
    audience: ScreenId,
    lower: ScreenId,
    theme: &Theme,
    slide: TemplateSlideId,
) -> Look {
    let mut layers = LayerVisibility::NONE;
    layers.set(Layer::Slide, true);
    Look {
        name: "Key feed".into(),
        screens: vec![
            ScreenLook {
                screen: audience,
                ..ScreenLook::default()
            },
            ScreenLook {
                screen: lower,
                layers,
                template_override: Some(TemplateOverride {
                    theme: Link::named(theme.id, theme.name.clone()),
                    slide,
                    mapping: TextMapping::ByNameThenOrder,
                    keep_inline_formatting: true,
                    hide_when_empty: true,
                }),
                mask: None,
                language: Default::default(),
            },
        ],
        ..Look::default()
    }
}

#[test]
fn lower_thirds_look_with_template_override() {
    let template = TemplateSlide {
        name: "Banner Lyrics".into(),
        slide: lower_third_template(),
        ..TemplateSlide::default()
    };
    let theme = Theme {
        name: "Banner Styles".into(),
        slides: vec![template.clone()],
        ..Theme::default()
    };
    let audience = Screen {
        name: "Audience".into(),
        ..Screen::default()
    };
    let lower = Screen {
        name: "Key Feed".into(),
        alpha: AlphaMode::Straight,
        frame_rate: FrameRate::FPS25,
        ..Screen::default()
    };
    let look = lower_thirds_look(audience.id, lower.id, &theme, template.id);

    // JSON shape and round trip.
    let v = serde_json::to_value(&look).unwrap();
    assert_eq!(v["screens"][0], json!({"screen": audience.id.to_string()}));
    let lower_json = &v["screens"][1];
    assert_eq!(
        lower_json["layers"],
        json!({"videoInput": false, "media": false, "announcements": false, "props": false, "messages": false, "captions": false})
    );
    assert_eq!(
        lower_json["templateOverride"],
        json!({"theme": {"id": theme.id.to_string(), "name": "Banner Styles"}, "slide": template.id.to_string()})
    );
    let back: Look = serde_json::from_value(v).unwrap();
    assert_eq!(back, look);
    let screen_json = serde_json::to_value(&lower).unwrap();
    assert_eq!(screen_json["alpha"], json!("straight"));
    assert_eq!(screen_json["frameRate"], json!({"num": 25, "den": 1}));

    // Rendering: a two-line lyric slide with a superscript verse number and a coloured run.
    let mut lyric = text_el(Some("text1"), "Lyrics", "");
    lyric.text_box_mut().unwrap().paragraphs = vec![
        Paragraph {
            runs: vec![
                TextRun::styled(
                    "2",
                    CharStyle {
                        baseline: Some(Baseline::Superscript),
                        ..CharStyle::default()
                    },
                ),
                TextRun::styled(
                    "Stars above",
                    CharStyle {
                        color: Some(Color::rgb(1.0, 1.0, 0.4)),
                        ..CharStyle::default()
                    },
                ),
            ],
            style: ParagraphStyle {
                alignment: Some(TextAlign::Left),
                ..ParagraphStyle::default()
            },
            ..Paragraph::default()
        },
        Paragraph::text("shine on"),
    ];
    let background = Element::media(Rect::new(0.0, 0.0, 1920.0, 1080.0), MediaAssetId::new());
    let live = Slide {
        elements: vec![background, lyric],
        ..Slide::new()
    };

    let ov = look.for_screen(lower.id).template_override.unwrap();
    let rendered = ov
        .render(&live, &theme.template(ov.slide).unwrap().slide)
        .expect("has text");
    assert_eq!(
        rendered.elements.len(),
        2,
        "template elements only; live image not drawn"
    );
    assert_eq!(rendered.elements[0].fill, template.slide.elements[0].fill);
    let tb = rendered.elements[1].text_box().unwrap();
    assert_eq!(tb.plain_text(), "2Stars above\nshine on");
    assert_eq!(
        tb.paragraphs[0].runs[0].style.baseline,
        Some(Baseline::Superscript)
    );
    assert_eq!(
        tb.paragraphs[0].runs[1].style.color, None,
        "template colour wins"
    );
    assert_eq!(
        tb.paragraphs[0].style,
        ParagraphStyle::default(),
        "template alignment wins"
    );
    assert_eq!(tb.run_style(&tb.paragraphs[1].runs[0]).size, Some(100.0));
    assert!(rendered.actions.is_empty());

    // Image-only or blank slides draw nothing on the lower-thirds screen...
    let image_only = Slide {
        elements: vec![Element::media(
            Rect::new(0.0, 0.0, 1920.0, 1080.0),
            MediaAssetId::new(),
        )],
        ..Slide::new()
    };
    assert!(ov.render(&image_only, &template.slide).is_none());
    // ...unless hideWhenEmpty is off: then the bar is drawn with an empty slot.
    let show_bar = TemplateOverride {
        hide_when_empty: false,
        ..ov.clone()
    };
    let bar = show_bar.render(&image_only, &template.slide).unwrap();
    assert!(
        bar.elements[1].text_box().unwrap().paragraphs.is_empty(),
        "no sample text"
    );

    // Without inline formatting the superscript is dropped too.
    let plain = TemplateOverride {
        keep_inline_formatting: false,
        ..ov
    };
    let r = plain.render(&live, &template.slide).unwrap();
    assert_eq!(
        r.elements[1].text_box().unwrap().paragraphs[0].runs[0].style,
        CharStyle::default()
    );
}

#[test]
fn template_override_maps_by_name_then_order() {
    let mut template = Slide::new();
    template.elements = vec![
        text_el(Some("reference"), "Reference", "sample ref"),
        text_el(Some("verse"), "Verse", "sample verse"),
    ];
    let live = Slide {
        elements: vec![
            text_el(Some("verse"), "Verse", "In the beginning"),
            text_el(Some("reference"), "Reference", "Book 1:1"),
            text_el(Some("extra"), "Extra", "Overflow"),
        ],
        ..Slide::new()
    };
    let ov = TemplateOverride {
        theme: Link::to(ThemeId::new()),
        slide: TemplateSlideId::new(),
        mapping: TextMapping::ByNameThenOrder,
        keep_inline_formatting: true,
        hide_when_empty: true,
    };
    let r = ov.render(&live, &template).unwrap();
    let text = |i: usize| r.elements[i].text_box().unwrap().plain_text();
    assert_eq!(text(0), "Book 1:1");
    assert_eq!(
        text(1),
        "In the beginning\nOverflow",
        "extra text goes to the last slot"
    );

    let merged = TemplateOverride {
        mapping: TextMapping::MergeIntoFirst,
        ..ov.clone()
    };
    let r = merged.render(&live, &template).unwrap();
    assert_eq!(
        r.elements[0].text_box().unwrap().plain_text(),
        "In the beginning\nBook 1:1\nOverflow"
    );
    assert_eq!(r.elements[1].text_box().unwrap().plain_text(), "");

    let by_order = TemplateOverride {
        mapping: TextMapping::ByOrder,
        ..ov
    };
    let r = by_order.render(&live, &template).unwrap();
    assert_eq!(
        r.elements[0].text_box().unwrap().plain_text(),
        "In the beginning"
    );
    assert_eq!(
        r.elements[1].text_box().unwrap().plain_text(),
        "Book 1:1\nOverflow"
    );
}

#[test]
fn template_override_skips_hidden_text() {
    let mut template = Slide::new();
    template.elements = vec![text_el(Some("text1"), "Lyrics", "sample")];
    let ov = TemplateOverride {
        theme: Link::to(ThemeId::new()),
        slide: TemplateSlideId::new(),
        mapping: TextMapping::ByNameThenOrder,
        keep_inline_formatting: true,
        hide_when_empty: true,
    };
    // A slide without slots: every text box is a source, but the hidden one is not drawn.
    let mut chords = text_el(None, "Chords", "G D Em C");
    chords.hidden = true;
    let live = Slide {
        elements: vec![text_el(None, "Lyrics", "Sing a new song"), chords.clone()],
        ..Slide::new()
    };
    let r = ov.render(&live, &template).unwrap();
    assert_eq!(
        r.elements[0].text_box().unwrap().plain_text(),
        "Sing a new song"
    );
    // Only hidden text: nothing visible, so hideWhenEmpty blanks the lower third.
    let only_hidden = Slide {
        elements: vec![text_el(None, "Lyrics", ""), chords.clone()],
        ..Slide::new()
    };
    assert!(ov.render(&only_hidden, &template).is_none());
    // A hidden slot source is skipped too.
    let mut hidden_slot = text_el(Some("text1"), "Lyrics", "Hidden slot");
    hidden_slot.hidden = true;
    let live = Slide {
        elements: vec![hidden_slot, text_el(Some("text2"), "Other", "Shown")],
        ..Slide::new()
    };
    let r = ov.render(&live, &template).unwrap();
    assert_eq!(r.elements[0].text_box().unwrap().plain_text(), "Shown");
}

/// Assets referenced by media fills anywhere on the slide.
fn media_assets(slide: &Slide) -> Vec<MediaAssetId> {
    slide
        .elements
        .iter()
        .filter_map(|e| match &e.fill.kind {
            FillKind::Media { asset, .. } => Some(*asset),
            _ => None,
        })
        .collect()
}

/// A text element with a media fill ("image with caption").
fn captioned_image(placeholder: Placeholder, text: &str) -> (Element, MediaAssetId) {
    let asset = MediaAssetId::new();
    let mut e = text_el(None, "Picture", text);
    e.placeholder = Some(placeholder);
    e.themed = true;
    e.fill = Element::media(Rect::new(0.0, 0.0, 10.0, 10.0), asset).fill;
    (e, asset)
}

fn one_text_slot_template() -> Slide {
    Slide {
        elements: vec![text_el(Some("text1"), "Text", "Sample")],
        ..Slide::new()
    }
}

/// A template with a media slot drawn as a plain shape (it cannot hold text).
fn media_shape_slot_template() -> Slide {
    let mut slot = Element::media(Rect::new(0.0, 0.0, 960.0, 540.0), MediaAssetId::new());
    slot.placeholder = Some(Placeholder::media("picture"));
    slot.themed = true;
    Slide {
        elements: vec![slot],
        ..Slide::new()
    }
}

#[test]
fn apply_template_keeps_media_fill_of_text_source() {
    let (captioned, asset) = captioned_image(Placeholder::text("text1"), "Caption one");
    let slide = Slide {
        elements: vec![captioned.clone()],
        ..Slide::new()
    };
    let out = apply_template(&slide, &one_text_slot_template());
    let e = out
        .element(captioned.id)
        .expect("matched slot keeps the old ID");
    assert_eq!(e.fill, captioned.fill, "the image fill is carried over");
    assert_eq!(e.text_box().unwrap().plain_text(), "Caption one");
    assert_eq!(media_assets(&out), vec![asset]);

    // A second captioned image finds the slot taken: it is kept whole, not merged into the
    // last slot (which would lose its picture).
    let (second, second_asset) = captioned_image(Placeholder::text("text2"), "Caption two");
    let slide = Slide {
        elements: vec![captioned.clone(), second.clone()],
        ..Slide::new()
    };
    let out = apply_template(&slide, &one_text_slot_template());
    assert_eq!(out.element(second.id), Some(&second));
    assert_eq!(media_assets(&out), vec![asset, second_asset]);
}

#[test]
fn apply_template_keeps_text_in_media_placeholder() {
    let mut words = text_el(None, "Picture", "Words typed into a picture slot");
    words.placeholder = Some(Placeholder::media("picture"));
    words.themed = true;
    let slide = Slide {
        elements: vec![words.clone()],
        ..Slide::new()
    };
    let out = apply_template(&slide, &one_text_slot_template());
    assert_eq!(out.element(words.id), Some(&words));
    assert!(out.plain_text().contains("Words typed into a picture slot"));

    // An empty media placeholder is still replaced.
    let mut empty = text_el(None, "Picture", "");
    empty.placeholder = Some(Placeholder::media("picture"));
    let slide = Slide {
        elements: vec![empty.clone()],
        ..Slide::new()
    };
    assert!(
        apply_template(&slide, &one_text_slot_template())
            .element(empty.id)
            .is_none()
    );
}

#[test]
fn apply_template_keeps_text_of_media_source() {
    // `Any` placeholder holding a picture and a caption, against a template with only a
    // media slot that cannot hold text: nothing is lost.
    let any = Placeholder {
        slot: "picture".into(),
        accepts: SlotAccepts::Any,
    };
    let (captioned, asset) = captioned_image(any.clone(), "Caption three");
    let slide = Slide {
        elements: vec![captioned.clone()],
        ..Slide::new()
    };
    let out = apply_template(&slide, &media_shape_slot_template());
    assert!(out.plain_text().contains("Caption three"));
    assert!(media_assets(&out).contains(&asset));
    assert_eq!(out.element(captioned.id), Some(&captioned));

    // The same with a media-only placeholder.
    let (captioned, asset) = captioned_image(Placeholder::media("picture"), "Caption four");
    let slide = Slide {
        elements: vec![captioned.clone()],
        ..Slide::new()
    };
    let out = apply_template(&slide, &media_shape_slot_template());
    assert!(out.plain_text().contains("Caption four"));
    assert!(media_assets(&out).contains(&asset));

    // A media slot that is a text element takes both the picture and the caption.
    let mut text_slot = text_el(None, "Picture", "");
    text_slot.placeholder = Some(Placeholder::media("picture"));
    let template = Slide {
        elements: vec![text_slot],
        ..Slide::new()
    };
    let out = apply_template(&slide, &template);
    assert_eq!(out.elements.len(), 1);
    let e = out
        .element(captioned.id)
        .expect("matched slot keeps the old ID");
    assert_eq!(e.fill, captioned.fill);
    assert_eq!(e.text_box().unwrap().plain_text(), "Caption four");

    // An `Any` placeholder holding only a picture is still a media source.
    let (picture, asset) = captioned_image(any, "");
    let slide = Slide {
        elements: vec![picture.clone()],
        ..Slide::new()
    };
    let out = apply_template(&slide, &media_shape_slot_template());
    assert_eq!(out.elements.len(), 1);
    assert_eq!(out.elements[0].id, picture.id);
    assert_eq!(media_assets(&out), vec![asset]);
}
