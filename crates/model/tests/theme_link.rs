//! Linked themes: link, resolve after a theme change, overrides, reset, unlink.

use model::theme_link::{link, refresh_overrides, reset, resolve, unlink};
use model::*;

fn theme() -> (Theme, TemplateSlideId) {
    let mut slot = Element::text(
        Rect::new(100.0, 700.0, 1720.0, 300.0),
        TextBox::plain("Sample"),
    );
    slot.name = "Lyrics".into();
    slot.placeholder = Some(Placeholder::text("lyrics"));
    slot.text_box_mut().unwrap().default_style = CharStyle {
        font_family: Some("Sans".into()),
        size: Some(72.0),
        color: Some(Color::WHITE),
        ..CharStyle::default()
    };
    let mut logo = Element::new(Rect::new(1700.0, 40.0, 160.0, 80.0));
    logo.name = "Logo".into();
    logo.fill = Fill::color(Color::rgb(0.9, 0.2, 0.2));
    let t = TemplateSlide {
        id: TemplateSlideId::new(),
        name: "Lyrics".into(),
        slide: Slide {
            background: Some(Color::BLACK),
            elements: vec![logo, slot],
            ..Slide::new()
        },
        ..TemplateSlide::default()
    };
    let id = t.id;
    (
        Theme {
            name: "Plain".into(),
            slides: vec![t],
            ..Theme::default()
        },
        id,
    )
}

fn song_slide(text: &str) -> Slide {
    let mut e = Element::text(Rect::new(0.0, 0.0, 800.0, 200.0), TextBox::plain(text));
    e.placeholder = Some(Placeholder::text("lyrics"));
    Slide {
        transition: None,
        elements: vec![e],
        ..Slide::new()
    }
}

fn template(t: &Theme, id: TemplateSlideId) -> &Slide {
    &t.template(id).unwrap().slide
}

fn slot(s: &Slide) -> &Element {
    s.elements
        .iter()
        .find(|e| e.placeholder.is_some())
        .expect("slot")
}

#[test]
fn linked_slide_follows_the_theme_and_keeps_its_text() {
    let (mut th, tid) = theme();
    let src = song_slide("Line one\nLine two");
    let linked = link(&src, &th, tid).unwrap();
    assert!(linked.theme_link.is_some());
    assert_eq!(linked.template_origin.as_ref().unwrap().slide, tid);
    assert_eq!(linked.background, Some(Color::BLACK));
    assert_eq!(
        slot(&linked).id,
        src.elements[0].id,
        "the slot keeps the text's ID"
    );
    assert_eq!(slot(&linked).bounds, Rect::new(100.0, 700.0, 1720.0, 300.0));
    // Resolving an up-to-date slide changes nothing.
    assert_eq!(resolve(&linked, template(&th, tid)), linked);

    // The theme changes: colour, placement and background follow; the text stays.
    let t = &mut th.slides[0].slide;
    t.background = Some(Color::rgb(0.0, 0.0, 0.3));
    t.elements[1].bounds = Rect::new(50.0, 50.0, 900.0, 400.0);
    t.elements[1].text_box_mut().unwrap().default_style.color = Some(Color::rgb(1.0, 1.0, 0.0));
    let r = resolve(&linked, template(&th, tid));
    assert_eq!(r.background, Some(Color::rgb(0.0, 0.0, 0.3)));
    assert_eq!(slot(&r).bounds, Rect::new(50.0, 50.0, 900.0, 400.0));
    assert_eq!(
        slot(&r).text_box().unwrap().default_style.color,
        Some(Color::rgb(1.0, 1.0, 0.0))
    );
    assert_eq!(
        slot(&r).text_box().unwrap().plain_text(),
        "Line one\nLine two"
    );
    assert_eq!(slot(&r).id, src.elements[0].id);
    assert_eq!(r.elements.len(), 2);
}

#[test]
fn overrides_survive_theme_changes_and_reset_clears_them() {
    let (mut th, tid) = theme();
    let mut s = link(&song_slide("Hello"), &th, tid).unwrap();
    // The operator moves the lyrics box on this slide and makes the background blue.
    let i = s
        .elements
        .iter()
        .position(|e| e.placeholder.is_some())
        .unwrap();
    s.elements[i].bounds = Rect::new(10.0, 10.0, 500.0, 100.0);
    s.background = Some(Color::rgb(0.0, 0.0, 1.0));
    let s = refresh_overrides(&s, template(&th, tid));
    assert_eq!(
        s.elements[i].from_theme.as_ref().unwrap().overrides,
        vec!["bounds".to_string()]
    );
    assert_eq!(
        s.theme_link.as_ref().unwrap().overrides,
        vec!["background".to_string()]
    );
    // The theme's size and colour change: the moved box keeps its place but takes the size.
    let t = &mut th.slides[0].slide;
    t.elements[1].bounds = Rect::new(0.0, 0.0, 1.0, 1.0);
    t.elements[1].text_box_mut().unwrap().default_style.size = Some(90.0);
    let r = resolve(&s, template(&th, tid));
    assert_eq!(slot(&r).bounds, Rect::new(10.0, 10.0, 500.0, 100.0));
    assert_eq!(slot(&r).text_box().unwrap().default_style.size, Some(90.0));
    assert_eq!(r.background, Some(Color::rgb(0.0, 0.0, 1.0)));
    // Reset one element's bounds, then the whole slide.
    let id = slot(&r).id;
    let r2 = reset(
        &r,
        template(&th, tid),
        Some(id),
        Some(&["bounds".to_string()]),
    );
    assert_eq!(slot(&r2).bounds, Rect::new(0.0, 0.0, 1.0, 1.0));
    assert_eq!(r2.background, Some(Color::rgb(0.0, 0.0, 1.0)));
    let r3 = reset(&r2, template(&th, tid), None, None);
    assert_eq!(r3.background, Some(Color::BLACK));
    assert!(r3.theme_link.as_ref().unwrap().overrides.is_empty());
}

#[test]
fn removed_and_added_theme_elements() {
    let (mut th, tid) = theme();
    let mut s = link(&song_slide("Hi"), &th, tid).unwrap();
    // The logo is deleted on this slide: it stays away when the theme changes.
    s.elements.retain(|e| e.name != "Logo");
    let s = refresh_overrides(&s, template(&th, tid));
    th.slides[0].slide.elements[0].bounds.x = 0.0;
    let mut extra = Element::new(Rect::new(0.0, 0.0, 10.0, 10.0));
    extra.name = "Bar".into();
    th.slides[0].slide.elements.push(extra);
    let r = resolve(&s, template(&th, tid));
    assert!(!r.elements.iter().any(|e| e.name == "Logo"));
    assert!(
        r.elements.iter().any(|e| e.name == "Bar"),
        "a new theme element appears"
    );
    // Reset brings the logo back.
    let r = reset(&r, template(&th, tid), None, None);
    assert!(r.elements.iter().any(|e| e.name == "Logo"));
    // The theme loses its text slot: the text stays as the slide's own element.
    th.slides[0]
        .slide
        .elements
        .retain(|e| e.placeholder.is_none());
    let r = resolve(&r, template(&th, tid));
    let own = r
        .elements
        .iter()
        .find(|e| e.text_box().is_some_and(|t| t.plain_text() == "Hi"))
        .unwrap();
    assert!(own.from_theme.is_none());
}

#[test]
fn unlinked_slides_are_untouched() {
    let (th, tid) = theme();
    let plain = song_slide("Hello");
    assert_eq!(resolve(&plain, template(&th, tid)), plain);
    assert_eq!(refresh_overrides(&plain, template(&th, tid)), plain);
    // Copy application (as before) carries no link and writes no new fields.
    let copied = th.apply(tid, &plain).unwrap();
    assert!(copied.theme_link.is_none());
    let v = serde_json::to_value(&copied).unwrap();
    assert!(v.get("themeLink").is_none());
    assert!(!v.to_string().contains("fromTheme"));
    // Unlink keeps the look.
    let linked = link(&plain, &th, tid).unwrap();
    let u = unlink(&linked);
    assert!(u.theme_link.is_none());
    assert_eq!(u.elements.len(), linked.elements.len());
    assert_eq!(u.background, linked.background);
}

#[test]
fn a_slide_typed_without_a_theme_pours_its_text_into_the_slot() {
    let (th, tid) = theme();
    let plain = Slide {
        elements: vec![Element::text(
            Rect::new(0.0, 0.0, 800.0, 200.0),
            TextBox::plain("Typed by hand"),
        )],
        ..Slide::new()
    };
    let linked = link(&plain, &th, tid).unwrap();
    assert_eq!(linked.elements.len(), 2, "logo and lyrics, no stray copy");
    assert_eq!(
        slot(&linked).text_box().unwrap().plain_text(),
        "Typed by hand"
    );
    assert_eq!(slot(&linked).id, plain.elements[0].id);
}

#[test]
fn a_slot_that_loses_its_role_keeps_the_songs_text() {
    let (mut th, tid) = theme();
    let linked = link(&song_slide("Made-up line"), &th, tid).unwrap();
    // The theme's lyrics box becomes decoration (its role cleared).
    th.slides[0].slide.elements[1].placeholder = None;
    let r = resolve(&linked, template(&th, tid));
    let e = r
        .elements
        .iter()
        .find(|e| e.id == linked.elements[1].id)
        .unwrap();
    assert_eq!(e.text_box().unwrap().plain_text(), "Made-up line");
    assert!(e.placeholder.is_none(), "the role follows the theme");
    assert_eq!(
        e.from_theme.as_ref().unwrap().overrides,
        vec!["text.paragraphs".to_string()],
        "the text stays on later theme changes too"
    );
    // A later theme change keeps it.
    th.slides[0].slide.elements[1].bounds = Rect::new(1.0, 2.0, 300.0, 400.0);
    let r2 = resolve(&r, template(&th, tid));
    let e = r2.elements.iter().find(|x| x.id == e.id).unwrap();
    assert_eq!(e.text_box().unwrap().plain_text(), "Made-up line");
    assert_eq!(e.bounds, Rect::new(1.0, 2.0, 300.0, 400.0));
}

/// An imported slide: a copy of the template (as the importer writes it), then changed on
/// this slide: the logo moved, the lyrics coloured, its own picture on top, a blue background.
fn imported(th: &Theme, tid: TemplateSlideId) -> Slide {
    let mut s = th.apply(tid, &song_slide("Made-up words")).unwrap();
    for e in &mut s.elements {
        e.id = ElementId::new();
        e.import = Some(ImportInfo::default());
    }
    s.elements[0].bounds = Rect::new(10.0, 10.0, 100.0, 50.0);
    s.elements[1].text_box_mut().unwrap().default_style.color = Some(Color::rgb(0.2, 0.9, 0.2));
    let mut own = Element::new(Rect::new(0.0, 0.0, 300.0, 300.0));
    own.name = "Badge".into();
    own.fill = Fill::color(Color::rgb(0.1, 0.1, 0.9));
    s.elements.push(own);
    s.background = Some(Color::rgb(0.0, 0.0, 1.0));
    s
}

#[test]
fn linking_keeps_the_look_and_still_follows_the_theme() {
    use model::theme_link::link_keeping_look;
    let (mut th, tid) = theme();
    let src = imported(&th, tid);
    let k = link_keeping_look(&src, &th, tid).expect("links");
    assert_eq!(k.paired, 2);
    let s = &k.slide;
    assert!(s.theme_link.is_some());
    assert_eq!(
        s.theme_link.as_ref().unwrap().overrides,
        vec!["background".to_string()]
    );
    assert_eq!(
        s.elements[0].from_theme.as_ref().unwrap().overrides,
        vec!["bounds".to_string()]
    );
    assert_eq!(
        s.elements[1].from_theme.as_ref().unwrap().overrides,
        vec!["text.defaultStyle".to_string()]
    );
    assert!(s.elements[2].from_theme.is_none(), "the badge is its own");
    // Nothing on screen changes: resolving gives the slide back.
    let r = resolve(s, template(&th, tid));
    assert_eq!(unlink(&r).elements, src.elements);
    assert_eq!(r.background, src.background);
    // A theme change restyles what isn't overridden.
    th.slides[0].slide.elements[1].bounds = Rect::new(5.0, 6.0, 700.0, 80.0);
    let r = resolve(s, template(&th, tid));
    assert_eq!(r.elements[1].bounds, Rect::new(5.0, 6.0, 700.0, 80.0));
    assert_eq!(r.elements[0].bounds, Rect::new(10.0, 10.0, 100.0, 50.0));
    assert_eq!(
        r.elements[1].text_box().unwrap().plain_text(),
        "Made-up words"
    );
}

#[test]
fn linking_keeping_the_look_respects_z_order_and_kinds() {
    use model::theme_link::link_keeping_look;
    let (th, tid) = theme();
    // The slide's own picture under the theme's elements: nothing can follow (the theme's
    // elements would be drawn under it).
    let mut src = imported(&th, tid);
    let own = src.elements.pop().unwrap();
    src.elements.insert(0, own);
    assert!(link_keeping_look(&src, &th, tid).is_none());
    // Only the lyrics (the logo deleted on this slide): the logo is recorded as removed.
    let mut src = imported(&th, tid);
    src.elements.remove(0);
    let k = link_keeping_look(&src, &th, tid).unwrap();
    assert_eq!(k.paired, 1);
    let removed = format!("removed:{}", th.slides[0].slide.elements[0].id.0);
    assert!(
        k.slide
            .theme_link
            .as_ref()
            .unwrap()
            .overrides
            .contains(&removed),
        "{:?}",
        k.slide.theme_link
    );
    assert_eq!(
        unlink(&resolve(&k.slide, template(&th, tid))).elements,
        src.elements
    );
    // Already linked: left alone.
    assert!(link_keeping_look(&k.slide, &th, tid).is_none());
}

#[test]
fn the_closest_template_slide_wins() {
    use model::theme_link::link_to_closest;
    let (th, tid) = theme();
    // Another theme whose only template has just the logo.
    let (mut other, oid) = theme();
    other.id = ThemeId::new();
    other.slides[0].slide.elements.truncate(1);
    let mut src = imported(&th, tid);
    let k = link_to_closest(&src, [&other, &th]).unwrap();
    assert_eq!(k.slide.template_origin.as_ref().unwrap().theme.id, th.id);
    // Without provenance, the template slide with the most paired elements.
    src.template_origin = None;
    let k = link_to_closest(&src, [&other, &th]).unwrap();
    assert_eq!(k.slide.template_origin.as_ref().unwrap().theme.id, th.id);
    assert_eq!(k.paired, 2);
    // Provenance names the other theme: it is used when it links.
    src.template_origin = Some(other.template_ref(oid));
    let k = link_to_closest(&src, [&th, &other]).unwrap();
    assert_eq!(k.slide.template_origin.as_ref().unwrap().theme.id, other.id);
}
