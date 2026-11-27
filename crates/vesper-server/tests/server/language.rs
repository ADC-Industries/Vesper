//! Dual-language slides (batch 16): a slide's second-language layer set with
//! `setSlideTranslation` (and undone with its inverse), drawn per screen as its look says:
//! stacked under the primary (the default), side by side, or primary only. Synthetic text.

use crate::common::*;
use serde_json::{Value, json};

/// Pale gold: the default second-language colour.
const GOLD: [u8; 3] = [255, 222, 143];

/// Mean (x, y) of the pixels like `c` in a frame, and how many there are.
fn centre(img: &image::RgbaImage, c: [u8; 3]) -> (f64, f64, usize) {
    centre_above(img, c, img.height())
}

/// [`centre`] of the rows above `y_max`.
fn centre_above(img: &image::RgbaImage, c: [u8; 3], y_max: u32) -> (f64, f64, usize) {
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0usize);
    for (x, y, p) in img.enumerate_pixels() {
        if y < y_max && p.0[3] > 200 && near(p, c, 40) {
            sx += x as f64;
            sy += y as f64;
            n += 1;
        }
    }
    let d = n.max(1) as f64;
    (sx / d, sy / d, n)
}

async fn edit(addr: std::net::SocketAddr, id: &str, body: Value) -> Resp {
    http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{id}/edits"),
        &[],
        Some(body),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_language_is_drawn_as_each_screen_look_says() {
    let fx = fixture("language");
    // "Plain": the audience screen side by side, the key screen primary only.
    let mut plain = fx.plain_look.clone();
    plain.screens = vec![
        model::ScreenLook {
            screen: fx.audience.id,
            language: model::LanguageDisplay::SideBySide,
            ..Default::default()
        },
        model::ScreenLook {
            screen: fx.key.id,
            language: model::LanguageDisplay::Primary,
            ..Default::default()
        },
    ];
    write_doc(&fx.dir, "portable", "look", &plain.id.0.to_string(), &plain);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let pid = fx.second.id.0.to_string();
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let slide = p["sections"][0]["slides"][0].clone();
    let sid = slide["id"].as_str().unwrap().to_string();
    let eid = slide["elements"][0]["id"].as_str().unwrap().to_string();

    // The second-language layer, set with an edit op; its inverse removes it.
    let r = edit(
        addr,
        &pid,
        json!({"ops": [{"type": "setSlideTranslation", "slide": sid, "translation": {
            "language": "es",
            "texts": [{"element": eid, "paragraphs": [{"runs": [{"text": "BE UNO"}]}]}]
        }}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let res = r.json();
    assert_eq!(res["inverse"][0]["type"], "setSlideTranslation");
    assert!(res["inverse"][0]["translation"].is_null());
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(
        p["sections"][0]["slides"][0]["translation"]["language"],
        "es"
    );
    let too_many: Vec<Value> = (0..65)
        .map(|_| json!({"element": uuid::Uuid::new_v4().to_string()}))
        .collect();
    let r = edit(
        addr,
        &pid,
        json!({"ops": [{"type": "setSlideTranslation", "slide": sid,
                        "translation": {"texts": too_many}}]}),
    )
    .await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));

    // The default look (audience not listed: auto → stacked): the gold second text under the
    // white primary.
    let mut ws = Client::connect(addr, "").await;
    ws.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 0}))
        .await;
    let img = wait_frame(&server, 0, "stacked", |img| {
        centre(img, GOLD).2 > 150 && centre(img, WHITE).2 > 150
    })
    .await;
    let (wx, wy, _) = centre(&img, WHITE);
    let (gx, gy, _) = centre(&img, GOLD);
    assert!(
        gy > wy + 30.0,
        "second under primary: white y {wy}, gold y {gy}"
    );
    assert!((gx - wx).abs() < 80.0, "both centred: {wx} / {gx}");

    // "Plain": side by side on the audience screen, primary only on the key screen.
    ws.ok(json!({"type": "setLook", "look": plain.id.0.to_string()}))
        .await;
    let img = wait_frame(&server, 0, "side by side", |img| {
        let (gx, _, gn) = centre(img, GOLD);
        let (wx, _, wn) = centre(img, WHITE);
        gn > 150 && wn > 150 && gx > W as f64 / 2.0 && wx < W as f64 / 2.0
    })
    .await;
    let (_, wy, _) = centre(&img, WHITE);
    let (_, gy, _) = centre(&img, GOLD);
    assert!(
        (gy - wy).abs() < 40.0,
        "side by side on one band: {wy} / {gy}"
    );
    wait_frame(&server, 1, "primary only on the key", |img| {
        centre(img, GOLD).2 == 0 && centre(img, WHITE).2 > 150
    })
    .await;

    // A look entry that only sets the language is kept (not dropped as "changes nothing").
    let aud = fx.audience.id.0.to_string();
    let r = http(
        addr,
        "POST",
        "/api/v1/looks",
        &[],
        Some(json!({"name": "Second only", "screens": [{"screen": aud, "language": "second"}]})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["screens"][0]["language"], "second");
}

/// A second made-up translation `TS2` (Spanish-tagged) beside `TST`: the same books and
/// versification, different invented words.
fn write_second_translation(dir: &std::path::Path) {
    use vesper_scripture::format::{self, Header, HeaderBook};
    let header = Header {
        format: format::FORMAT.into(),
        id: "TS2".into(),
        abbreviation: "TS2".into(),
        name: "Second Test Translation (synthetic)".into(),
        language: "es".into(),
        license: "Synthetic test data".into(),
        attribution: None,
        source: None,
        books: vec![
            HeaderBook {
                key: "GEN".into(),
                name: "Génesis".into(),
                chapters: vec![4, 3],
            },
            HeaderBook {
                key: "JHN".into(),
                name: "Juan".into(),
                chapters: vec![2, 6, 3],
            },
        ],
    };
    let mut verses = Vec::new();
    for b in &header.books {
        for (c, &n) in b.chapters.iter().enumerate() {
            for v in 1..=n {
                verses.push(format!(
                    "El tejedor de {} cosió red {} a {v}.",
                    b.name,
                    c + 1
                ));
            }
        }
    }
    let mut f = Vec::new();
    format::write(&mut f, &header, &verses).unwrap();
    std::fs::write(dir.join("scripture").join("TS2.vbible"), f).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_parallel_bible_passage_shows_both_translations_side_by_side() {
    let fx = fixture("language-bible");
    crate::scripture::write_translation(&fx.dir);
    write_second_translation(&fx.dir);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;

    // The split names both translations, verse-aligned.
    let r = http(
        addr,
        "POST",
        "/api/v1/scripture/slides",
        &[],
        Some(json!({"translation": "TST", "reference": "John 2:1-3", "secondTranslation": "TS2"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let split = r.json();
    assert_eq!(split["name"], "John 2:1–3 (TST · TS2)");
    let first = &split["slides"][0];
    assert!(
        first["second"]
            .as_str()
            .unwrap()
            .starts_with("El tejedor de Juan")
    );
    assert!(
        first["reference"].as_str().unwrap().ends_with("TST · TS2"),
        "{first}"
    );
    // An unknown second translation is refused.
    let r = http(
        addr,
        "POST",
        "/api/v1/scripture/slides",
        &[],
        Some(json!({"translation": "TST", "reference": "John 2:1", "secondTranslation": "NOPE"})),
    )
    .await;
    assert_eq!(r.status, 404, "{}", String::from_utf8_lossy(&r.body));

    // Made and shown: each slide carries the second translation, side by side by default.
    let mut a = Client::connect(addr, "").await;
    let r = http(
        addr,
        "POST",
        "/api/v1/scripture/presentations",
        &[],
        Some(
            json!({"translation": "TST", "reference": "John 2:1", "secondTranslation": "TS2",
                    "then": "show"}),
        ),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let id = r.json()["presentation"].as_str().unwrap().to_string();
    let p = get_json(addr, &format!("/api/v1/presentations/{id}")).await;
    let s = &p["sections"][0]["slides"][0];
    assert_eq!(s["translation"]["language"], "es");
    assert_eq!(s["translation"]["layout"], "sideBySide");
    assert_eq!(
        s["translation"]["texts"][0]["element"],
        s["elements"][0]["id"]
    );
    assert_eq!(p["scripture"]["generated"]["secondTranslation"], "TS2");
    a.wait_state("the passage is live", |_, st: &Value| {
        st["layers"]["slide"]["presentation"] == id.as_str()
    })
    .await;
    // The verse box only (the reference line below is right-aligned light grey).
    let band = H * 860 / 1080;
    wait_frame(&server, 0, "both translations side by side", |img| {
        let (gx, _, gn) = centre_above(img, GOLD, band);
        let (wx, _, wn) = centre_above(img, WHITE, band);
        gn > 100 && wn > 100 && gx > W as f64 / 2.0 && wx < W as f64 / 2.0
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_theme_sets_the_second_language_style() {
    let fx = fixture("language-theme");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let themes = get_json(addr, "/api/v1/themes").await;
    let id = themes[0]["id"].as_str().unwrap().to_string();
    let entry = get_json(addr, &format!("/api/v1/themes/{id}")).await;
    let rev = entry["revision"].as_str().unwrap().to_string();
    let tag = format!("\"{rev}\"");
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/themes/{id}"),
        &[("If-Match", tag.as_str())],
        Some(json!({"secondLanguage": {"sizeScale": 9.0, "italic": false,
                    "color": {"r": 0.5, "g": 0.8, "b": 1.0}}})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let t = get_json(addr, &format!("/api/v1/themes/{id}")).await;
    let sl = &t["theme"]["secondLanguage"];
    // Clamped, and the name kept.
    assert_eq!(sl["sizeScale"], 2.0);
    assert_eq!(sl["italic"], false);
    assert_eq!(t["theme"]["name"], entry["theme"]["name"]);
}
