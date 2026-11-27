//! Linked themes over the API: create, apply to a song, edit the theme (songs follow, slide
//! overrides stay), reset to theme, template thumbnails, the theme document's edit rules,
//! duplicate, delete refused while in use, reassign and unlink. Synthetic data only.

use crate::common::*;
use serde_json::{Value, json};

async fn post(addr: std::net::SocketAddr, path: &str, body: Value) -> Resp {
    http(addr, "POST", path, &[], Some(body)).await
}

fn slides(p: &Value) -> Vec<Value> {
    p["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["slides"].as_array().cloned().unwrap_or_default())
        .collect()
}

/// The element of a slide that follows template element `te`.
fn follower<'a>(slide: &'a Value, te: &str) -> &'a Value {
    slide["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["fromTheme"]["element"] == te)
        .unwrap_or_else(|| panic!("no element follows {te}: {slide}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn linked_themes() {
    let fx = fixture("themes");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let pid = fx.song.id.0.to_string();

    // A new theme has three starter template slides and nobody using it.
    let r = post(addr, "/api/v1/themes", json!({"name": "Evening"})).await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let tid = r.json()["id"].as_str().unwrap().to_string();
    let list = get_json(addr, "/api/v1/themes").await;
    let summary = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == tid.as_str())
        .unwrap()
        .clone();
    assert_eq!(summary["slides"].as_array().unwrap().len(), 3);
    assert_eq!(summary["slides"][0]["name"], "Lyrics");
    assert_eq!(summary["usedBy"], 0);
    let entry = get_json(addr, &format!("/api/v1/themes/{tid}")).await;
    let lyrics_tpl = entry["theme"]["slides"][0].clone();
    let lyrics_el = lyrics_tpl["slide"]["elements"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let tpl_slide = lyrics_tpl["slide"]["id"].as_str().unwrap().to_string();

    // Template thumbnails (PNG, ETag, 304).
    let thumb = format!("/api/v1/themes/{tid}/slides/{tpl_slide}/thumbnail?w=160");
    let t1 = http(addr, "GET", &thumb, &[], None).await;
    assert_eq!(t1.status, 200);
    assert_eq!(t1.header("content-type"), Some("image/png"));
    let etag = t1.header("etag").unwrap().to_string();
    let t2 = http(addr, "GET", &thumb, &[("If-None-Match", &etag)], None).await;
    assert_eq!(t2.status, 304);

    // Apply it to the song: every slide follows its Lyrics template slide, text kept.
    let before = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let texts_before: Vec<String> = slides(&before)
        .iter()
        .map(|s| {
            s["elements"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|e| e["kind"]["text"]["paragraphs"][0]["runs"][0]["text"].as_str())
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect();
    let r = post(
        addr,
        &format!("/api/v1/presentations/{pid}/edits"),
        json!({"ops": [{"type": "applyTheme", "theme": tid}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["applied"][0]["type"], "replaceSlide");
    assert_eq!(r.json()["inverse"][0]["type"], "replaceSlide");
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let ss = slides(&p);
    for (s, text) in ss.iter().zip(&texts_before) {
        assert_eq!(s["templateOrigin"]["theme"]["id"], tid.as_str());
        assert!(s["themeLink"].is_object(), "{s}");
        let f = follower(s, &lyrics_el);
        assert_eq!(
            f["kind"]["text"]["paragraphs"][0]["runs"][0]["text"].as_str(),
            Some(text.as_str()),
            "the lyrics stay"
        );
    }
    let summaries = get_json(addr, "/api/v1/presentations").await;
    let song = summaries
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == pid.as_str())
        .unwrap();
    assert_eq!(song["theme"], tid.as_str());
    assert_eq!(song["themeName"], "Evening");
    let used = get_json(addr, &format!("/api/v1/themes/{tid}/used-by")).await;
    assert_eq!(used[0]["id"], pid.as_str());

    // The operator moves the lyrics box on slide 0 only: an override.
    let s0 = ss[0]["id"].as_str().unwrap().to_string();
    let s1 = ss[1]["id"].as_str().unwrap().to_string();
    let el0 = follower(&ss[0], &lyrics_el)["id"]
        .as_str()
        .unwrap()
        .to_string();
    let moved = json!({"x": 10.0, "y": 20.0, "width": 300.0, "height": 100.0});
    let r = post(
        addr,
        &format!("/api/v1/presentations/{pid}/edits"),
        json!({"ops": [{"type": "setElement", "slide": s0, "element": el0, "patch": {"bounds": moved}}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(
        follower(&slides(&p)[0], &lyrics_el)["fromTheme"]["overrides"],
        json!(["bounds"])
    );
    let song_thumb = format!("/api/v1/presentations/{pid}/slides/{s1}/thumbnail?w=160");
    let thumb_before = http(addr, "GET", &song_thumb, &[], None)
        .await
        .header("etag")
        .unwrap()
        .to_string();

    // Editing the theme restyles the song: new placement and colour everywhere, except the
    // overridden placement on slide 0. Open songs hear documentChanged.
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let doc = get_json(addr, &format!("/api/v1/themes/{tid}/document")).await;
    assert_eq!(doc["sections"][0]["slides"][0]["label"]["text"], "Lyrics");
    let theme_bounds = json!({"x": 40.0, "y": 200.0, "width": 560.0, "height": 120.0});
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [
            {"type": "setElement", "slide": tpl_slide, "element": lyrics_el, "patch": {"bounds": theme_bounds}},
            {"type": "setTextStyle", "slide": tpl_slide, "element": lyrics_el, "character": {"color": {"r": 1.0, "g": 1.0, "b": 0.0, "a": 1.0}}}
        ]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let ev = loop {
        let m = ws.next_any().await;
        if m["type"] == "documentChanged" && m["kind"] == "presentation" && m["id"] == pid.as_str()
        {
            break m;
        }
    };
    assert!(ev["revision"].is_string());
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let ss = slides(&p);
    let f0 = follower(&ss[0], &lyrics_el);
    let f1 = follower(&ss[1], &lyrics_el);
    assert_eq!(f0["bounds"], moved, "the override stays");
    assert_eq!(f1["bounds"], theme_bounds, "the others follow");
    for f in [f0, f1] {
        assert_eq!(
            f["kind"]["text"]["defaultStyle"]["color"],
            json!({"r": 1.0, "g": 1.0, "b": 0.0})
        );
    }
    let thumb_after = http(addr, "GET", &song_thumb, &[], None)
        .await
        .header("etag")
        .unwrap()
        .to_string();
    assert_ne!(thumb_before, thumb_after, "the song's thumbnail re-renders");

    // Reset to theme on slide 0's element: back to the theme's placement.
    let r = post(
        addr,
        &format!("/api/v1/presentations/{pid}/edits"),
        json!({"ops": [{"type": "resetToTheme", "slide": s0, "element": el0}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let ss = slides(&p);
    let f0 = follower(&ss[0], &lyrics_el);
    assert_eq!(f0["bounds"], theme_bounds);
    assert!(f0["fromTheme"]["overrides"].is_null());

    // The theme's document takes slide ops but not sections; a duplicated slide is a new
    // template slide.
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "addSection", "section": {"id": uuid::Uuid::new_v4(), "name": "X"}}]}),
    )
    .await;
    assert_eq!(r.status, 400);
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "duplicateSlide", "slide": tpl_slide}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let entry = get_json(addr, &format!("/api/v1/themes/{tid}")).await;
    assert_eq!(entry["theme"]["slides"].as_array().unwrap().len(), 4);

    // Renaming the theme renames the songs' reference to it.
    let rev = get_json(addr, &format!("/api/v1/themes/{tid}")).await["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/themes/{tid}"),
        &[("If-Match", &format!("\"{rev}\""))],
        Some(json!({"name": "Evening Service"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(
        slides(&p)[0]["templateOrigin"]["theme"]["name"],
        "Evening Service"
    );

    // Delete is refused while songs follow it; duplicate, reassign, then unlink.
    let r = http(addr, "DELETE", &format!("/api/v1/themes/{tid}"), &[], None).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["current"]["usedBy"][0]["id"], pid.as_str());
    let r = post(addr, &format!("/api/v1/themes/{tid}/duplicate"), json!({})).await;
    assert_eq!(r.status, 201);
    let copy = r.json()["id"].as_str().unwrap().to_string();
    let list = get_json(addr, "/api/v1/themes").await;
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Evening Service copy"), "{names:?}");
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/themes/{tid}?reassign={copy}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    for s in slides(&p) {
        assert_eq!(s["templateOrigin"]["theme"]["id"], copy.as_str());
    }
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/themes/{copy}?unlink=true"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    for s in slides(&p) {
        assert!(s["themeLink"].is_null(), "unlinked: {s}");
        assert!(s["background"].is_object(), "the look stays");
    }
    let r = http(addr, "GET", &format!("/api/v1/themes/{copy}"), &[], None).await;
    assert_eq!(r.status, 404);
    // Deleted themes are in the item trash.
    let trash = get_json(addr, "/api/v1/codex/trash").await;
    assert!(
        trash.to_string().contains(&copy),
        "the theme is in the trash: {trash}"
    );
}

/// Undoing a template slide's delete brings back the same template slide (its ID, which
/// messages and songs name), while a duplicate still gets a new one (A05-1). The template
/// thumbnail's ETag follows the theme's canvas size (A05-5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn theme_slide_delete_undo_and_canvas_thumbnails() {
    let fx = fixture("themes-undo");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let r = post(addr, "/api/v1/themes", json!({"name": "Morning"})).await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let tid = r.json()["id"].as_str().unwrap().to_string();
    let template_ids = |entry: &Value| -> Vec<(String, String)> {
        entry["theme"]["slides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                (
                    t["id"].as_str().unwrap().to_string(),
                    t["slide"]["id"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    let before = template_ids(&get_json(addr, &format!("/api/v1/themes/{tid}")).await);
    let (tpl_id, slide_id) = before[1].clone();

    // Delete the second template slide, then send back the answer's inverse (Oriel's undo).
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "deleteSlide", "slide": slide_id}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let inverse = r.json()["inverse"].clone();
    let mid = template_ids(&get_json(addr, &format!("/api/v1/themes/{tid}")).await);
    assert_eq!(mid.len(), before.len() - 1);
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({ "ops": inverse }),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let after = template_ids(&get_json(addr, &format!("/api/v1/themes/{tid}")).await);
    assert_eq!(
        after, before,
        "the same template slide, same place, same ID"
    );

    // A duplicate is a new template slide.
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "duplicateSlide", "slide": slide_id}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let dup = template_ids(&get_json(addr, &format!("/api/v1/themes/{tid}")).await);
    assert_eq!(dup.len(), before.len() + 1);
    let ids: std::collections::HashSet<&String> = dup.iter().map(|(t, _)| t).collect();
    assert_eq!(ids.len(), dup.len(), "template IDs stay unique: {dup:?}");
    assert!(ids.contains(&tpl_id));

    // A canvas change gives the template thumbnail a new ETag.
    let thumb = format!("/api/v1/themes/{tid}/slides/{slide_id}/thumbnail?w=160");
    let etag = http(addr, "GET", &thumb, &[], None)
        .await
        .header("etag")
        .unwrap()
        .to_string();
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "setPresentation", "canvasSize": {"width": 1080, "height": 1920}}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let t = http(addr, "GET", &thumb, &[("If-None-Match", &etag)], None).await;
    assert_eq!(t.status, 200, "a new picture, not 304");
    assert_ne!(t.header("etag").unwrap(), etag);
}

/// The theme editor's template slides (batch 9): add one with its roles, rename and reorder
/// it, change a box's role, and delete it while a song follows it: the song keeps its look,
/// unlinked. `used-by?template=` names who follows one template slide.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn template_slides_roles_and_delete() {
    let fx = fixture("themes-templates");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let pid = fx.song.id.0.to_string();
    let r = post(addr, "/api/v1/themes", json!({"name": "Midweek"})).await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let tid = r.json()["id"].as_str().unwrap().to_string();
    let doc = get_json(addr, &format!("/api/v1/themes/{tid}/document")).await;
    let section = doc["sections"][0]["id"].as_str().unwrap().to_string();
    let last = doc["sections"][0]["slides"][2]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Add a "Lower third" template slide: a lyrics slot and a decoration bar.
    let (new_slide, slot_el, bar_el) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    let text = |runs: &str| json!({"paragraphs": [{"runs": [{"text": runs}]}], "defaultStyle": {"fontFamily": "DejaVu Sans", "size": 40.0, "color": {"r": 1.0, "g": 1.0, "b": 1.0}}});
    let slide = json!({
        "id": new_slide,
        "label": {"text": "Lower third"},
        "background": {"r": 0.0, "g": 0.0, "b": 0.2},
        "elements": [
            {"id": bar_el, "name": "Bar", "bounds": {"x": 0.0, "y": 260.0, "width": 640.0, "height": 100.0},
             "fill": {"enabled": true, "kind": {"type": "color", "color": {"r": 0.6, "g": 0.1, "b": 0.1}}}},
            {"id": slot_el, "name": "Lyrics", "bounds": {"x": 20.0, "y": 270.0, "width": 600.0, "height": 80.0},
             "placeholder": {"slot": "lyrics"}, "kind": {"type": "text", "text": text("")}}
        ]
    });
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "addSlide", "section": section, "after": last, "slide": slide}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    // Reorder it first, rename it, and give the bar a role and take it back.
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [
            {"type": "moveSlide", "slide": new_slide, "toSection": section},
            {"type": "setSlide", "slide": new_slide, "label": {"text": "Lower Third"}},
            {"type": "setElement", "slide": new_slide, "element": bar_el, "patch": {"placeholder": {"slot": "title"}}}
        ]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let inverse = r.json()["inverse"].clone();
    assert!(
        inverse.to_string().contains("\"placeholder\":null"),
        "the inverse clears the role again: {inverse}"
    );
    let entry = get_json(addr, &format!("/api/v1/themes/{tid}")).await;
    let t0 = entry["theme"]["slides"][0].clone();
    assert_eq!(t0["name"], "Lower Third");
    assert_eq!(t0["slide"]["id"], new_slide.as_str());
    assert_eq!(entry["theme"]["slides"].as_array().unwrap().len(), 4);
    assert_eq!(t0["slide"]["elements"][0]["placeholder"]["slot"], "title");
    let template = t0["id"].as_str().unwrap().to_string();
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "setElement", "slide": new_slide, "element": bar_el, "patch": {"placeholder": null}}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let entry = get_json(addr, &format!("/api/v1/themes/{tid}")).await;
    assert!(entry["theme"]["slides"][0]["slide"]["elements"][0]["placeholder"].is_null());

    // The song follows the new template slide.
    let r = post(
        addr,
        &format!("/api/v1/presentations/{pid}/edits"),
        json!({"ops": [{"type": "applyTheme", "theme": tid, "template": template}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let used = get_json(
        addr,
        &format!("/api/v1/themes/{tid}/used-by?template={template}"),
    )
    .await;
    assert_eq!(used[0]["id"], pid.as_str());
    let other = entry["theme"]["slides"][1]["id"].as_str().unwrap();
    let used = get_json(
        addr,
        &format!("/api/v1/themes/{tid}/used-by?template={other}"),
    )
    .await;
    assert_eq!(used, json!([]));

    // The lyrics box stops being a slot: the songs keep their own text.
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let texts = |p: &Value| -> Vec<String> {
        slides(p)
            .iter()
            .map(|s| {
                s["elements"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|e| e["kind"]["text"]["paragraphs"][0]["runs"][0]["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .collect()
    };
    let before = texts(&p);
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "setElement", "slide": new_slide, "element": slot_el, "patch": {"placeholder": null}}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(texts(&p), before, "no song text is lost");

    // Delete the template slide: the song keeps its look, unlinked.
    let looks: Vec<Value> = slides(&p)
        .iter()
        .map(|s| {
            json!([
                s["background"],
                s["elements"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e["bounds"].clone())
                    .collect::<Vec<_>>()
            ])
        })
        .collect();
    let r = post(
        addr,
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "deleteSlide", "slide": new_slide}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    for (s, look) in slides(&p).iter().zip(&looks) {
        assert!(s["themeLink"].is_null(), "unlinked: {s}");
        let now = json!([
            s["background"],
            s["elements"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["bounds"].clone())
                .collect::<Vec<_>>()
        ]);
        assert_eq!(&now, look, "the look stays");
    }
    assert_eq!(texts(&p), before);
    let used = get_json(addr, &format!("/api/v1/themes/{tid}/used-by")).await;
    assert_eq!(used, json!([]));
}

/// Golden test for "link, keeping the look" (batch 9): an imported song (copies of a theme's
/// template slide, changed per slide) is linked with `linkTheme`; every slide renders the same
/// bytes before and after, the slide whose own picture sits under the theme's elements stays
/// unlinked, and a later theme change restyles what the songs didn't override.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn link_keeping_the_look_renders_identically() {
    use model::{
        CharStyle, Color, Element, ElementId, Fill, ImportInfo, Paragraph, Placeholder,
        Presentation, Rect, Section, Size, Slide, TemplateSlide, TextBox, Theme,
    };
    let fx = fixture("themes-keep-look");
    let canvas = Size::new(W as f64, H as f64);
    let mut bar = Element::new(Rect::new(0.0, 250.0, W as f64, 110.0));
    bar.name = "Bar".into();
    bar.fill = Fill::color(Color::rgb(0.5, 0.1, 0.4));
    let mut slot = Element::text(
        Rect::new(20.0, 260.0, W as f64 - 40.0, 90.0),
        TextBox {
            default_style: CharStyle {
                font_family: Some("DejaVu Sans".into()),
                size: Some(36.0),
                color: Some(Color::WHITE),
                ..CharStyle::default()
            },
            ..TextBox::default()
        },
    );
    slot.name = "Lyrics".into();
    slot.placeholder = Some(Placeholder::text("lyrics"));
    let template = TemplateSlide {
        name: "Lyrics".into(),
        slide: Slide {
            background: Some(Color::rgb(0.05, 0.05, 0.1)),
            elements: vec![bar, slot],
            ..Slide::new()
        },
        ..TemplateSlide::default()
    };
    let tpl_id = template.id;
    let theme = Theme {
        name: "Imported Evening".into(),
        canvas_size: canvas,
        slides: vec![template],
        ..Theme::default()
    };
    // The song as an importer writes it: the template copied into each slide.
    let copy = |text: &str| -> Slide {
        let mut s = theme.template(tpl_id).unwrap().slide.clone();
        s.id = model::SlideId::new();
        s.template_origin = Some(theme.template_ref(tpl_id));
        for e in &mut s.elements {
            e.id = ElementId::new();
            e.import = Some(ImportInfo::default());
        }
        s.elements[1].text_box_mut().unwrap().paragraphs = Paragraph::from_lines(text);
        s
    };
    let a = copy("Made-up line one");
    let mut b = copy("Made-up line two");
    b.elements[0].bounds = Rect::new(0.0, 0.0, W as f64, 80.0);
    b.elements[1].text_box_mut().unwrap().default_style.color = Some(Color::rgb(1.0, 0.9, 0.2));
    let mut badge = Element::new(Rect::new(500.0, 20.0, 100.0, 100.0));
    badge.name = "Badge".into();
    badge.fill = Fill::color(Color::rgb(0.1, 0.6, 0.2));
    b.elements.push(badge.clone());
    let mut c = copy("Made-up line three");
    c.elements.insert(0, badge);
    let mut sec = Section::new("Verse");
    sec.slides = vec![a, b, c];
    let song = Presentation {
        canvas_size: canvas,
        sections: vec![sec],
        ..Presentation::new("Imported Song")
    };
    let pid = song.id.0.to_string();
    let theme_id = theme.id.0.to_string();
    write_doc(&fx.dir, "portable", "theme", &theme_id, &theme);
    write_doc(&fx.dir, "portable", "presentation", &pid, &song);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let ids: Vec<String> = song
        .slides()
        .map(|s| s.id.0.to_string())
        .collect::<Vec<_>>();
    let render = |sid: &str| format!("/api/v1/presentations/{pid}/slides/{sid}/render?w=320");
    let mut before = Vec::new();
    for sid in &ids {
        let r = http(addr, "GET", &render(sid), &[], None).await;
        assert_eq!(r.status, 200);
        before.push(r.body.clone());
    }

    let r = post(
        addr,
        &format!("/api/v1/presentations/{pid}/edits"),
        json!({"ops": [{"type": "linkTheme"}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let applied = r.json()["applied"].as_array().unwrap().clone();
    assert_eq!(applied.len(), 2, "two slides link: {applied:?}");
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let ss = slides(&p);
    assert!(ss[0]["themeLink"].is_object());
    assert_eq!(ss[0]["templateOrigin"]["theme"]["id"], theme_id.as_str());
    assert!(ss[1]["themeLink"].is_object());
    assert_eq!(
        ss[1]["elements"][0]["fromTheme"]["overrides"],
        json!(["bounds"])
    );
    assert_eq!(
        ss[1]["elements"][1]["fromTheme"]["overrides"],
        json!(["text.defaultStyle"])
    );
    assert!(ss[1]["elements"][2]["fromTheme"].is_null(), "its own badge");
    assert!(
        ss[2]["themeLink"].is_null(),
        "the badge under the theme: unlinked"
    );
    for (sid, b) in ids.iter().zip(&before) {
        let r = http(addr, "GET", &render(sid), &[], None).await;
        assert_eq!(r.status, 200);
        assert!(&r.body == b, "slide {sid} renders the same after linking");
    }
    let used = get_json(addr, &format!("/api/v1/themes/{theme_id}/used-by")).await;
    assert_eq!(used[0]["id"], pid.as_str());

    // The theme's bar turns blue: slide 0 follows; slide 1 keeps its moved bar but the colour
    // follows; slide 2 (unlinked) doesn't change.
    let bar_id = theme.slides[0].slide.elements[0].id.0.to_string();
    let tpl_slide = theme.slides[0].slide.id.0.to_string();
    let r = post(
        addr,
        &format!("/api/v1/themes/{theme_id}/edits"),
        json!({"ops": [{"type": "setElement", "slide": tpl_slide, "element": bar_id, "patch": {"fill": {"enabled": true, "kind": {"type": "color", "color": {"r": 0.1, "g": 0.2, "b": 0.9}}}}}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let ss = slides(&p);
    for s in &ss[..2] {
        assert_eq!(s["elements"][0]["fill"]["kind"]["color"]["b"], json!(0.9));
    }
    assert_eq!(ss[1]["elements"][0]["bounds"]["height"], json!(80.0));
    for (i, sid) in ids.iter().enumerate() {
        let r = http(addr, "GET", &render(sid), &[], None).await;
        assert_eq!(r.body != before[i], i < 2, "slide {i}");
    }
}
