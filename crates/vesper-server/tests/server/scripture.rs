//! The scripture module over the API (G11): translations from the data folder's
//! `scripture/`, references, passages, search, slides with a theme, previews, and the
//! presentation made from a passage (into the Scripture library, a playlist, or straight live).
//! The translation is made up: invented verse text, never real scripture.

use crate::common::*;
use serde_json::{Value, json};
use vesper_scripture::format::{self, Header, HeaderBook};

/// A made-up translation `TST` in `<data>/scripture/`: Genesis (2 chapters) and John
/// (3 chapters), every verse an invented sentence.
pub(crate) fn write_translation(dir: &std::path::Path) {
    let header = Header {
        format: format::FORMAT.into(),
        id: "TST".into(),
        abbreviation: "TST".into(),
        name: "Test Translation (synthetic)".into(),
        language: "en".into(),
        license: "Synthetic test data".into(),
        attribution: Some("Synthetic text for tests.".into()),
        source: None,
        books: vec![
            HeaderBook {
                key: "GEN".into(),
                name: "Genesis".into(),
                chapters: vec![4, 3],
            },
            HeaderBook {
                key: "JHN".into(),
                name: "John".into(),
                chapters: vec![2, 6, 3],
            },
        ],
    };
    let mut verses = Vec::new();
    for b in &header.books {
        for (c, &n) in b.chapters.iter().enumerate() {
            for v in 1..=n {
                verses.push(format!(
                    "The weaver of {} mended net {} by lamp {v}.",
                    b.name,
                    c + 1
                ));
            }
        }
    }
    let d = dir.join("scripture");
    std::fs::create_dir_all(&d).unwrap();
    let mut f = Vec::new();
    format::write(&mut f, &header, &verses).unwrap();
    std::fs::write(d.join("TST.vbible"), f).unwrap();
}

fn q(s: &str) -> String {
    s.replace(' ', "%20")
        .replace(':', "%3A")
        .replace(',', "%2C")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn passages_slides_previews_and_presentations() {
    let fx = fixture("scripture");
    write_translation(&fx.dir);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;

    let info = get_json(addr, "/api/v1/server").await;
    assert!(
        info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("scripture"))
    );

    // Translations, books, parsing feedback, passages, search.
    let list = get_json(addr, "/api/v1/scripture/translations").await;
    assert_eq!(list[0]["id"], "TST", "{list}");
    assert_eq!(list[0]["origin"], "local");
    assert_eq!(list[0]["attribution"], "Synthetic text for tests.");
    let books = get_json(addr, "/api/v1/scripture/translations/TST/books").await;
    assert_eq!(books[1]["key"], "JHN");
    assert_eq!(books[1]["testament"], "new");
    assert_eq!(books[1]["chapters"], json!([2, 6, 3]));
    let r = http(
        addr,
        "GET",
        "/api/v1/scripture/translations/NOPE/books",
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 404);

    let p = get_json(
        addr,
        &format!("/api/v1/scripture/parse?translation=TST&q={}", q("jn 2:9")),
    )
    .await;
    assert_eq!(p["ok"], false);
    assert_eq!(p["bookName"], "John");
    assert_eq!(p["error"], "John 2 has 6 verses");
    let p = get_json(
        addr,
        &format!(
            "/api/v1/scripture/parse?translation=tst&q={}",
            q("Jn 2:1-3, 5")
        ),
    )
    .await;
    assert_eq!(p["ok"], true, "{p}");
    assert_eq!(p["label"], "John 2:1–3, 5");

    let passage = get_json(
        addr,
        &format!(
            "/api/v1/scripture/passage?translation=TST&ref={}",
            q("John 2:5-3:1")
        ),
    )
    .await;
    assert_eq!(passage["label"], "John 2:5–3:1");
    assert_eq!(passage["verses"].as_array().unwrap().len(), 3);
    assert_eq!(
        passage["verses"][2]["text"],
        "The weaver of John mended net 3 by lamp 1."
    );
    let r = http(
        addr,
        "GET",
        &format!(
            "/api/v1/scripture/passage?translation=TST&ref={}",
            q("John 9")
        ),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"]["message"], "John has 3 chapters");

    let s = get_json(
        addr,
        &format!(
            "/api/v1/scripture/search?translation=TST&q={}&limit=2",
            q("weaver LAMP 4")
        ),
    )
    .await;
    // Genesis 1:4 and John 2:4.
    assert_eq!(s["total"], 2, "{s}");
    assert_eq!(s["hits"][1]["book"], "JHN");

    // The split, with a theme read through today's theme API.
    let themes = get_json(addr, "/api/v1/themes").await;
    let theme = themes[0]["id"].as_str().unwrap().to_string();
    let req = json!({
        "translation": "TST", "reference": "John 2:1-6", "theme": theme,
        "versesPerSlide": 2, "verseNumbers": true, "referenceOn": "last"
    });
    let r = http(
        addr,
        "POST",
        "/api/v1/scripture/slides",
        &[],
        Some(req.clone()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let split = r.json();
    assert_eq!(split["name"], "John 2 (TST)");
    let slides = split["slides"].as_array().unwrap().clone();
    // The theme's slot is a lower third (one 40 px line, 640 × 70): measured with the
    // renderer's text layout, two verses never fit, so each verse breaks between words and
    // continues on the next slide ("…"), whatever `versesPerSlide` allows.
    assert!(slides.len() > 6, "{split}");
    let text = |s: &Value| s["text"].as_str().unwrap().to_string();
    assert!(text(&slides[0]).starts_with("1 The weaver") && text(&slides[0]).ends_with(" …"));
    assert!(text(&slides[1]).starts_with("… "), "{split}");
    assert!(slides[0].get("reference").is_none());
    let last = slides.last().unwrap();
    assert!(last["reference"].as_str().unwrap().starts_with("John 2:6"));
    assert_eq!(last["attribution"], "Synthetic text for tests.");
    // Nothing is lost or repeated: the six verses, in order, once the marks are taken out.
    let passage = get_json(
        addr,
        &format!(
            "/api/v1/scripture/passage?translation=TST&ref={}",
            q("John 2:1-6")
        ),
    )
    .await;
    let want: Vec<String> = passage["verses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| format!("{} {}", v["verse"], v["text"].as_str().unwrap()))
        .collect();
    let got: Vec<String> = slides.iter().map(text).collect();
    assert_eq!(
        got.join(" ").replace(" …", "").replace("… ", ""),
        want.join(" ")
    );

    // The plain layout (no theme) fits two verses a slide, as asked.
    let mut plain = req.clone();
    plain.as_object_mut().unwrap().remove("theme");
    let r = http(addr, "POST", "/api/v1/scripture/slides", &[], Some(plain)).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let split = r.json();
    let plain_slides = split["slides"].as_array().unwrap();
    assert_eq!(plain_slides.len(), 3, "{split}");
    assert_eq!(plain_slides[2]["reference"], "John 2:5–6 TST");
    assert!(text(&plain_slides[1]).starts_with("3 The weaver"));

    // A preview of a generated slide: a PNG of the requested width.
    let r = http(
        addr,
        "GET",
        &format!(
            "/api/v1/scripture/preview?translation=TST&reference={}&theme={theme}&versesPerSlide=2&slide=1&w=160",
            q("John 2:1-6")
        ),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.header("content-type"), Some("image/png"));
    let img = image::load_from_memory(&r.body).unwrap();
    assert_eq!(img.width(), 160);
    let r = http(
        addr,
        "GET",
        &format!(
            "/api/v1/scripture/preview?translation=TST&reference={}&slide=9",
            q("John 2:1-2")
        ),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 404);

    // Added to the playlist: a scripture presentation in a new "Scripture" library, themed,
    // at the end of the playlist.
    let plid = fx.playlist.id.0.to_string();
    let mut body = req.clone();
    body["then"] = json!("playlist");
    body["playlist"] = json!(plid);
    let r = http(
        addr,
        "POST",
        "/api/v1/scripture/presentations",
        &[],
        Some(body),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let created = r.json();
    assert_eq!(created["slideCount"], slides.len());
    let pid = created["presentation"].as_str().unwrap().to_string();
    let doc = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(doc["scripture"]["label"], "John 2 (TST)");
    assert_eq!(doc["templateOrigin"]["theme"]["id"], theme.as_str());
    let libs = get_json(addr, "/api/v1/libraries").await;
    let lib = libs
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == "Scripture")
        .unwrap_or_else(|| panic!("no Scripture library: {libs}"))
        .clone();
    assert_eq!(doc["library"], lib["id"]);
    let summaries = get_json(addr, "/api/v1/presentations").await;
    let summary = summaries
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == pid.as_str())
        .unwrap()
        .clone();
    assert_eq!(summary["kind"], "scripture");
    let pl = get_json(addr, &format!("/api/v1/playlists/{plid}")).await;
    let items = pl["items"].as_array().unwrap();
    assert_eq!(items.last().unwrap()["id"], created["playlistItem"], "{pl}");

    // Shown now: no playlist; the Scripture library is reused.
    let mut a = Client::connect(addr, "").await;
    let r = http(
        addr,
        "POST",
        "/api/v1/scripture/presentations",
        &[],
        Some(json!({"translation": "TST", "reference": "Gen 1:2", "then": "show"})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["name"], "Genesis 1:2 (TST)");
    a.wait_state("the passage is live", |_, s: &Value| {
        s["layers"]["slide"]["presentationName"] == "Genesis 1:2 (TST)"
    })
    .await;
    let libs = get_json(addr, "/api/v1/libraries").await;
    let n = libs
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["name"] == "Scripture")
        .count();
    assert_eq!(n, 1);
    let pl2 = get_json(addr, &format!("/api/v1/playlists/{plid}")).await;
    assert_eq!(pl2["items"].as_array().unwrap().len(), items.len());

    // Mistakes are refused before anything is written.
    for (body, status) in [
        (
            json!({"translation": "TST", "reference": "Gen 1:2", "then": "playlist"}),
            400,
        ),
        (
            json!({"translation": "TST", "reference": "Gen 9", "then": "none"}),
            400,
        ),
        (
            json!({"translation": "XYZ", "reference": "Gen 1", "then": "none"}),
            404,
        ),
        (
            json!({"translation": "TST", "reference": "Gen 1", "theme": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}),
            404,
        ),
    ] {
        let r = http(
            addr,
            "POST",
            "/api/v1/scripture/presentations",
            &[],
            Some(body.clone()),
        )
        .await;
        assert_eq!(
            r.status,
            status,
            "{body}: {}",
            String::from_utf8_lossy(&r.body)
        );
    }
    server.shutdown().await.unwrap();
}

/// Passages created at the same time (a double click, two operators) share one Scripture
/// library (A05-3); a custom name follows the naming rule (A05-4).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_passages_share_one_library() {
    let fx = fixture("scripture-race");
    write_translation(&fx.dir);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let refs = [
        "Gen 1:1", "Gen 1:2", "Gen 1:3", "John 1:1", "John 2:1", "John 3:1",
    ];
    let creates = refs.iter().map(|r| {
        http(
            addr,
            "POST",
            "/api/v1/scripture/presentations",
            &[],
            Some(json!({"translation": "TST", "reference": r, "then": "none"})),
        )
    });
    for r in futures_util::future::join_all(creates).await {
        assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    }
    let libs = get_json(addr, "/api/v1/libraries").await;
    let scripture: Vec<&Value> = libs
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["name"] == "Scripture")
        .collect();
    assert_eq!(scripture.len(), 1, "{libs}");
    let summaries = get_json(addr, "/api/v1/presentations").await;
    let in_lib = summaries
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["library"] == scripture[0]["id"])
        .count();
    assert_eq!(in_lib, refs.len());

    for name in ["two\nlines".to_string(), "x".repeat(201)] {
        let r = http(
            addr,
            "POST",
            "/api/v1/scripture/presentations",
            &[],
            Some(
                json!({"translation": "TST", "reference": "Gen 1:1", "then": "none", "name": name}),
            ),
        )
        .await;
        assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    }
    server.shutdown().await.unwrap();
}

/// Batch 9: a passage made with a theme is split again when the theme changes, unless the
/// operator edited it (then it is marked `stale`, and `resplit` with `force` splits it again).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn passages_split_again_when_their_theme_changes() {
    let fx = fixture("scripture-resplit");
    write_translation(&fx.dir);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let post = |path: &str, body: Value| {
        let path = path.to_string();
        async move { http(addr, "POST", &path, &[], Some(body)).await }
    };
    let r = post("/api/v1/themes", json!({"name": "Readings"})).await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let tid = r.json()["id"].as_str().unwrap().to_string();
    let entry = get_json(addr, &format!("/api/v1/themes/{tid}")).await;
    let tpl = entry["theme"]["slides"][2].clone();
    assert_eq!(tpl["name"], "Scripture");
    let tpl_slide = tpl["slide"]["id"].as_str().unwrap().to_string();
    let slot = tpl["slide"]["elements"][0].clone();
    let slot_id = slot["id"].as_str().unwrap().to_string();
    let r = post(
        "/api/v1/scripture/presentations",
        json!({"translation": "TST", "reference": "John 2:1-6", "theme": tid}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let pid = r.json()["presentation"].as_str().unwrap().to_string();
    let first = r.json()["slideCount"].as_u64().unwrap();
    let doc = || async { get_json(addr, &format!("/api/v1/presentations/{pid}")).await };
    let count = |p: &Value| -> usize {
        p["sections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["slides"].as_array().map_or(0, |a| a.len()))
            .sum()
    };
    let p = doc().await;
    assert_eq!(count(&p) as u64, first);
    let g = p["scripture"]["generated"].clone();
    assert_eq!(g["reference"], "John 2:1-6");
    assert_eq!(g["theme"], tid.as_str());
    assert!(g["slides"].is_string());

    // The theme's passage box shrinks to one short line: the passage is split again.
    let small =
        json!({"x": slot["bounds"]["x"], "y": slot["bounds"]["y"], "width": 200.0, "height": 40.0});
    let r = post(
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "setElement", "slide": tpl_slide, "element": slot_id, "patch": {"bounds": small}}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let mut p = doc().await;
    for _ in 0..100 {
        if count(&p) as u64 > first {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        p = doc().await;
    }
    let second = count(&p);
    assert!(second as u64 > first, "split again: {first} → {second}");
    assert_eq!(p["id"], pid.as_str());
    assert_ne!(p["scripture"]["generated"]["slides"], g["slides"]);
    assert!(p["scripture"]["generated"]["stale"].is_null());

    // The operator edits the passage; the box grows again: the edit stays, the passage is
    // marked stale.
    let s0 = p["sections"][0]["slides"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = post(
        &format!("/api/v1/presentations/{pid}/edits"),
        json!({"ops": [{"type": "setSlideText", "slide": s0, "text": "An operator's made-up note"}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let r = post(
        &format!("/api/v1/themes/{tid}/edits"),
        json!({"ops": [{"type": "setElement", "slide": tpl_slide, "element": slot_id, "patch": {"bounds": slot["bounds"]}}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let mut p = doc().await;
    for _ in 0..100 {
        if p["scripture"]["generated"]["stale"] == true {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        p = doc().await;
    }
    assert_eq!(
        p["scripture"]["generated"]["stale"], true,
        "{}",
        p["scripture"]
    );
    assert_eq!(count(&p), second, "the edited passage keeps its split");
    assert!(p.to_string().contains("An operator's made-up note"));

    // Splitting it again is the operator's choice: refused without force.
    let path = format!("/api/v1/scripture/presentations/{pid}/resplit");
    let r = post(&path, json!({})).await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    let r = post(&path, json!({"force": true})).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["changed"], true);
    assert_eq!(r.json()["slideCount"].as_u64(), Some(first));
    let p = doc().await;
    assert_eq!(count(&p) as u64, first);
    assert!(p["scripture"]["generated"]["stale"].is_null());
    assert!(!p.to_string().contains("An operator's made-up note"));
    // Unchanged: nothing written.
    let r = post(&path, json!({})).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["changed"], false);
    // A presentation that wasn't made on the Bible page can't be split again.
    let r = post(
        &format!("/api/v1/scripture/presentations/{}/resplit", fx.song.id.0),
        json!({}),
    )
    .await;
    assert_eq!(r.status, 400);
}

/// A theme with a "Scripture" template and a passage of John 2:1-6 made with it, one verse a
/// slide: `(theme id, template slide id, its slot element, passage id)`.
async fn passage_with_theme(addr: std::net::SocketAddr) -> (String, String, Value, String) {
    let r = http(
        addr,
        "POST",
        "/api/v1/themes",
        &[],
        Some(json!({"name": "Readings"})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let tid = r.json()["id"].as_str().unwrap().to_string();
    let entry = get_json(addr, &format!("/api/v1/themes/{tid}")).await;
    let tpl = entry["theme"]["slides"][2].clone();
    assert_eq!(tpl["name"], "Scripture");
    let tpl_slide = tpl["slide"]["id"].as_str().unwrap().to_string();
    let slot = tpl["slide"]["elements"][0].clone();
    let r = http(
        addr,
        "POST",
        "/api/v1/scripture/presentations",
        &[],
        Some(json!({"translation": "TST", "reference": "John 2:1-6", "theme": tid, "versesPerSlide": 1})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let pid = r.json()["presentation"].as_str().unwrap().to_string();
    (tid, tpl_slide, slot, pid)
}

fn slide_ids(p: &Value) -> Vec<String> {
    p["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["slides"].as_array().unwrap().iter())
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect()
}

/// D06-1: a theme tweak re-splits a live passage in place: its slides keep their IDs, the
/// screen stays on its slide and Next goes on to the following one. D06-4: the re-split's
/// edit names the revision it was made from (`If-Match`), so an edit made meanwhile isn't
/// lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_passage_keeps_its_place_when_its_theme_changes() {
    let fx = fixture("scripture-resplit-live");
    write_translation(&fx.dir);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let (tid, tpl_slide, slot, pid) = passage_with_theme(addr).await;
    let doc = || async { get_json(addr, &format!("/api/v1/presentations/{pid}")).await };
    let p = doc().await;
    let ids = slide_ids(&p);
    assert_eq!(ids.len(), 6, "one verse a slide");
    let digest = p["scripture"]["generated"]["slides"].clone();

    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    ws.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 2}))
        .await;
    assert_eq!(ws.state["layers"]["slide"]["slide"], ids[2].as_str());

    // A colour-like tweak of the theme's passage box: split again, nothing moves.
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/themes/{tid}/edits"),
        &[],
        Some(json!({"ops": [{"type": "setElement", "slide": tpl_slide, "element": slot["id"], "patch": {"opacity": 0.9}}]})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let mut p = doc().await;
    for _ in 0..100 {
        if p["scripture"]["generated"]["slides"] != digest {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        p = doc().await;
    }
    assert_ne!(p["scripture"]["generated"]["slides"], digest, "split again");
    assert_eq!(slide_ids(&p), ids, "the slides keep their IDs");
    assert!(p.to_string().contains("0.9"), "with the theme's change");
    ws.ok(json!({"type": "next"})).await;
    assert_eq!(
        ws.state["layers"]["slide"]["slide"],
        ids[3].as_str(),
        "Next goes on from the live slide"
    );

    // An edit naming an old revision is refused; the current one applies.
    let path = format!("/api/v1/presentations/{pid}/edits");
    let body =
        json!({"ops": [{"type": "setSlideText", "slide": ids[0], "text": "A made-up note"}]});
    let r = http(
        addr,
        "POST",
        &path,
        &[("If-Match", "0123456789abcdef")],
        Some(body.clone()),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    let rev = r.json()["current"]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let r = http(
        addr,
        "POST",
        &path,
        &[("If-Match", rev.as_str())],
        Some(body),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    server.shutdown().await.unwrap();
}

/// D05-1: splitting a passage again respects its edit lock: refused for another client, done
/// for the lock's holder; the theme job leaves a locked passage as it is and marks it stale.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resplit_respects_the_edit_lock() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let fx = fixture("scripture-resplit-lock");
    write_translation(&fx.dir);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let (tid, tpl_slide, slot, pid) = passage_with_theme(addr).await;
    let doc = || async { get_json(addr, &format!("/api/v1/presentations/{pid}")).await };
    let ids = slide_ids(&doc().await);

    // Oriel A opens the passage in the editor and edits it: A holds the lock.
    let mut a = Client::connect(addr, "protocolVersion=1").await;
    let m = json!({"type": "presence", "clientId": "oriel-a", "computer": "Booth laptop",
        "open": [{"kind": "presentation", "id": pid, "place": "edit"}]});
    a.ws.send(Message::Text(m.to_string().into()))
        .await
        .unwrap();
    let path = format!("/api/v1/presentations/{pid}/edits");
    let mut locked = false;
    for _ in 0..100 {
        let r = http(
            addr,
            "POST",
            &path,
            &[("X-Client-Id", "oriel-a")],
            Some(json!({"ops": [{"type": "setSlideText", "slide": ids[0], "text": "A made-up note"}]})),
        )
        .await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        if get_json(addr, "/api/v1/presence").await["locks"][0]["clientId"] == "oriel-a" {
            locked = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(locked, "A holds the lock");

    // The theme changes: the locked passage is marked stale, not split again.
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/themes/{tid}/edits"),
        &[],
        Some(json!({"ops": [{"type": "setElement", "slide": tpl_slide, "element": slot["id"], "patch": {"opacity": 0.9}}]})),
    )
    .await;
    assert_eq!(r.status, 200);
    let mut p = doc().await;
    for _ in 0..100 {
        if p["scripture"]["generated"]["stale"] == true {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        p = doc().await;
    }
    assert_eq!(
        p["scripture"]["generated"]["stale"], true,
        "{}",
        p["scripture"]
    );
    assert!(p.to_string().contains("A made-up note"));

    // B can't split it again, even with force; A can.
    let rs = format!("/api/v1/scripture/presentations/{pid}/resplit");
    let r = http(
        addr,
        "POST",
        &rs,
        &[("X-Client-Id", "oriel-b")],
        Some(json!({"force": true})),
    )
    .await;
    assert_eq!(r.status, 423, "{}", String::from_utf8_lossy(&r.body));
    let r = http(addr, "POST", &rs, &[], Some(json!({"force": true}))).await;
    assert_eq!(r.status, 423, "nor can a client without an ID");
    assert!(doc().await.to_string().contains("A made-up note"));
    let r = http(
        addr,
        "POST",
        &rs,
        &[("X-Client-Id", "oriel-a")],
        Some(json!({"force": true})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let p = doc().await;
    assert!(!p.to_string().contains("A made-up note"));
    assert!(p["scripture"]["generated"]["stale"].is_null());
    server.shutdown().await.unwrap();
}

/// E06-1 / E06-2: a forced re-split replaces the operator's edits: slides the operator moved
/// go back into verse order (and the passage then counts as unedited), and a passage the
/// operator gave a second group splits again (kept slide IDs don't clash with the old ones).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forced_resplit_restores_the_order_and_handles_added_groups() {
    let fx = fixture("scripture-resplit-force");
    write_translation(&fx.dir);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let (_tid, _tpl_slide, _slot, pid) = passage_with_theme(addr).await;
    let doc = || async { get_json(addr, &format!("/api/v1/presentations/{pid}")).await };
    let edit = |ops: Value| {
        let path = format!("/api/v1/presentations/{pid}/edits");
        async move { http(addr, "POST", &path, &[], Some(json!({ "ops": ops }))).await }
    };
    let resplit = |force: bool| {
        let path = format!("/api/v1/scripture/presentations/{pid}/resplit");
        async move { http(addr, "POST", &path, &[], Some(json!({ "force": force }))).await }
    };
    let p = doc().await;
    let ids = slide_ids(&p);
    assert_eq!(ids.len(), 6, "one verse a slide");
    let section = p["sections"][0]["id"].as_str().unwrap().to_string();

    // The operator moves the last verse to the front.
    let r = edit(json!([{"type": "moveSlide", "slide": ids[5], "toSection": section}])).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(slide_ids(&doc().await)[0], ids[5]);
    let r = resplit(false).await;
    assert_eq!(
        r.status,
        409,
        "edited: {}",
        String::from_utf8_lossy(&r.body)
    );
    let r = resplit(true).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(
        slide_ids(&doc().await),
        ids,
        "back in verse order, IDs kept"
    );
    let r = resplit(false).await;
    assert_eq!(
        r.status,
        200,
        "no longer edited: {}",
        String::from_utf8_lossy(&r.body)
    );
    assert_eq!(r.json()["changed"], false);

    // The operator adds a group: the forced re-split replaces both groups.
    let extra = uuid::Uuid::new_v4().to_string();
    let r = edit(json!([{"type": "addSection", "after": section,
        "section": {"id": extra, "name": "Extra"}}]))
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(doc().await["sections"].as_array().unwrap().len(), 2);
    let r = resplit(true).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let p = doc().await;
    assert_eq!(
        p["sections"].as_array().unwrap().len(),
        1,
        "{}",
        p["sections"]
    );
    assert_eq!(slide_ids(&p), ids, "IDs kept");
    let r = resplit(false).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["changed"], false);
    server.shutdown().await.unwrap();
}
