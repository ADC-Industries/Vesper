//! End-to-end test of the control protocol on a synthetic data directory: the server runs
//! in-process on a random port; REST and two WebSocket clients drive it over real sockets.
//! Covers documents, live-state sync between clients, playlist navigation, clear groups, looks,
//! not-implemented commands, thumbnails (ETag / caches), live previews, local pairing
//! management, protocol version checks and data reload. Runs on any adapter (lavapipe too).

use crate::common::*;
use protocol::PreviewFrameHeader;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn slide_of(state: &Value) -> &Value {
    &state["layers"]["slide"]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_protocol_end_to_end() {
    let fx = fixture("api");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let song = fx.song.id.0.to_string();
    let second = fx.second.id.0.to_string();
    let playlist = fx.playlist.id.0.to_string();
    let items: Vec<String> = fx.items().iter().map(|i| i.id.0.to_string()).collect();

    // ---- server info and documents ----------------------------------------------------------
    let info = get_json(addr, "/api/v1/server").await;
    assert_eq!(info["protocolVersion"], 1);
    assert_eq!(info["auth"], "local");
    assert_eq!(info["pairingCode"].as_str().unwrap().len(), 6, "{info}");
    assert!(
        info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("thumbnails"))
    );

    let libs = get_json(addr, "/api/v1/libraries").await;
    assert_eq!(libs.as_array().unwrap().len(), 1, "{libs}");
    assert_eq!(libs[0]["name"], "Songs");
    assert_eq!(libs[0]["presentationCount"], 2);
    let list = get_json(
        addr,
        &format!("/api/v1/libraries/{}/presentations", fx.library.id.0),
    )
    .await;
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Second Song", "Test Song"]);
    assert_eq!(list[1]["slideCount"], 4);
    let p = get_json(addr, &format!("/api/v1/presentations/{song}")).await;
    assert_eq!(p["sections"][0]["name"], "Verse 1");
    assert_eq!(p["sections"][1]["slides"][0]["enabled"], false);
    let slide3 = p["sections"][1]["slides"][1]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let slide0 = p["sections"][0]["slides"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let tree = get_json(addr, "/api/v1/playlists").await;
    assert_eq!(tree[0]["name"], "Services", "{tree}");
    assert_eq!(tree[0]["kind"], "folder");
    assert_eq!(tree[0]["children"][0]["name"], "Sunday");
    assert_eq!(tree[0]["children"][0]["itemCount"], 4);
    let pl = get_json(addr, &format!("/api/v1/playlists/{playlist}")).await;
    let shown: Vec<&str> = pl["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["displayName"].as_str().unwrap())
        .collect();
    assert_eq!(
        shown,
        ["Worship", "Test Song", "Placeholder", "Second Song"]
    );
    assert_eq!(pl["items"][1]["presentation"]["slideCount"], 4);
    assert_eq!(
        get_json(addr, "/api/v1/looks")
            .await
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(get_json(addr, "/api/v1/screens").await[1]["name"], "Key");
    assert_eq!(
        get_json(addr, "/api/v1/themes").await[0]["slides"][0]["name"],
        "Lyrics"
    );
    assert_eq!(
        get_json(addr, "/api/v1/clear-groups").await[0]["name"],
        "Clear Slide"
    );
    let m = get_json(addr, &format!("/api/v1/media/{}", fx.asset.id.0)).await;
    assert_eq!(m["online"], true);
    assert_eq!(m["asset"]["name"], "Blue background");
    let file = format!("/api/v1/media/{}/file", fx.asset.id.0);
    let whole = http(addr, "GET", &file, &[], None).await;
    assert_eq!(whole.status, 200);
    assert_eq!(&whole.body[..4], b"\x89PNG");
    let part = http(addr, "GET", &file, &[("Range", "bytes=0-9")], None).await;
    assert_eq!(part.status, 206, "Range requests are served");
    assert_eq!(part.body.len(), 10);
    let diag = get_json(addr, "/api/v1/diagnostics").await;
    assert_eq!(diag["loadIssues"].as_array().unwrap().len(), 2, "{diag}");
    for (path, status) in [
        ("/api/v1/presentations/nope", 400),
        (
            "/api/v1/presentations/00000000-0000-0000-0000-000000000001",
            404,
        ),
        ("/api/v1/nothing-here", 404),
    ] {
        let r = http(addr, "GET", path, &[], None).await;
        assert_eq!(r.status, status, "{path}");
        assert!(
            r.json()["error"]["code"].is_string(),
            "{path}: JSON error body"
        );
    }

    // ---- two clients: A triggers, both see it ------------------------------------------------
    let mut a = Client::connect(addr, "protocolVersion=1").await;
    let mut b = Client::connect(addr, "").await;
    assert_eq!(a.hello["server"]["id"], info["id"]);
    assert_eq!(a.state["activeLook"], fx.default_look.id.0.to_string());
    assert!(slide_of(&a.state).is_null());
    assert!(a.state["cursor"].is_null());
    assert_eq!(a.state["screens"].as_array().unwrap().len(), 2);
    assert_eq!(a.state["diagnostics"]["loadIssues"], 2);
    let r = a.command(json!({"type": "next"})).await;
    assert_eq!(r["ok"], false);
    assert_eq!(r["error"]["code"], "conflict", "next before any trigger");

    let start_rev = a.revision;
    let rev = a
        .ok(
            json!({"type": "triggerSlide", "presentation": song, "index": 0,
                    "playlistItem": {"playlist": playlist, "item": items[1]}}),
        )
        .await;
    assert!(rev > start_rev);
    b.wait_state("B sees A's trigger", |r, _| r >= rev).await;
    for c in [&a, &b] {
        let s = slide_of(&c.state);
        assert_eq!(s["presentationName"], "Test Song");
        assert_eq!(s["slideIndex"], 0);
        assert_eq!(s["slide"], slide0);
        assert_eq!(s["sectionName"], "Verse 1");
        assert_eq!(s["playlist"]["item"], items[1]);
        assert_eq!(c.state["layers"]["media"]["name"], "Blue background");
        assert_eq!(c.state["cursor"]["slideIndex"], 1);
    }
    if b.revision == a.revision {
        assert_eq!(b.state, a.state, "same revision, same state");
    }

    // Pixels: background + text on Audience; the look keys the red lower third on Key.
    wait_frame(&server, 0, "audience slide 0", |i| {
        count(i, 0, 40, BLUE, 12) > 1000 && count(i, 40, 230, WHITE, 30) > 200
    })
    .await;
    wait_frame(&server, 1, "key slide 0", |i| {
        let top_transparent = (0..i.width()).all(|x| i.get_pixel(x, 5).0[3] == 0);
        top_transparent && count(i, 0, 280, WHITE, 30) == 0 && count(i, 280, H, RED, 40) > 50
    })
    .await;

    // ---- next / prev, across the playlist boundary --------------------------------------------
    a.ok(json!({"type": "next"})).await;
    assert_eq!(slide_of(&a.state)["slideIndex"], 1);
    a.ok(json!({"type": "next"})).await;
    assert_eq!(
        slide_of(&a.state)["slideIndex"],
        3,
        "disabled slide 2 is skipped"
    );
    assert_eq!(slide_of(&a.state)["sectionName"], "Chorus");
    let cur = &a.state["cursor"];
    assert_eq!(cur["presentation"], second, "cursor: next playlist item");
    assert_eq!(cur["slideIndex"], 0);
    assert_eq!(cur["playlist"]["item"], items[3]);
    a.ok(json!({"type": "next"})).await;
    let s = slide_of(&a.state).clone();
    assert_eq!(
        (
            s["presentation"].as_str(),
            s["slideIndex"].as_u64(),
            s["playlist"]["item"].as_str()
        ),
        (Some(second.as_str()), Some(0), Some(items[3].as_str())),
        "next at the end moves to the next presentation item (placeholder skipped)"
    );
    a.ok(json!({"type": "next"})).await;
    assert_eq!(slide_of(&a.state)["slideIndex"], 1);
    assert!(a.state["cursor"].is_null(), "end of the playlist");
    let before = slide_of(&a.state).clone();
    a.ok(json!({"type": "next"})).await;
    assert_eq!(
        slide_of(&a.state),
        &before,
        "next at the very end changes nothing"
    );
    a.ok(json!({"type": "prev"})).await;
    assert_eq!(slide_of(&a.state)["slideIndex"], 0);
    a.ok(json!({"type": "prev"})).await;
    let s = slide_of(&a.state);
    assert_eq!(
        s["presentation"], song,
        "prev at the start goes back one item"
    );
    assert_eq!(s["slideIndex"], 3, "…to its last enabled slide");
    assert_eq!(s["playlist"]["item"], items[1]);
    // Without a playlist, next stops at the end of the presentation.
    a.ok(json!({"type": "triggerSlide", "presentation": song, "slide": slide3}))
        .await;
    assert!(slide_of(&a.state)["playlist"].is_null());
    assert!(a.state["cursor"].is_null());
    a.ok(json!({"type": "triggerPlaylistItem", "playlist": playlist, "item": items[3]}))
        .await;
    assert_eq!(slide_of(&a.state)["presentation"], second);
    let r = a
        .command(json!({"type": "triggerPlaylistItem", "playlist": playlist, "item": items[0]}))
        .await;
    assert_eq!(
        r["error"]["code"], "badRequest",
        "headers cannot be triggered"
    );
    let r = a
        .command(json!({"type": "triggerSlide", "presentation": song, "index": 99}))
        .await;
    assert_eq!(r["error"]["code"], "notFound");

    // ---- clear group, clear layer, clear all ---------------------------------------------------
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 0}))
        .await;
    a.ok(json!({"type": "clearGroup", "id": fx.clear_slide.id.0.to_string()}))
        .await;
    assert!(
        slide_of(&a.state).is_null(),
        "Clear Slide clears the slide layer"
    );
    assert_eq!(
        a.state["layers"]["media"]["name"], "Blue background",
        "…not media"
    );
    wait_frame(&server, 0, "slide cleared", |i| {
        count(i, 0, H, WHITE, 30) == 0 && count(i, 0, 270, BLUE, 12) > 100_000
    })
    .await;
    let r = a
        .command(json!({"type": "clearGroup", "id": "00000000-0000-0000-0000-000000000009"}))
        .await;
    assert_eq!(r["error"]["code"], "notFound");
    a.ok(json!({"type": "clearLayer", "layer": "media"})).await;
    assert!(a.state["layers"]["media"].is_null());
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 0}))
        .await;
    a.ok(json!({"type": "clearAll"})).await;
    assert!(slide_of(&a.state).is_null() && a.state["layers"]["media"].is_null());
    wait_frame(&server, 0, "all cleared", |i| {
        count(i, 0, H, BLACK, 4) == (i.width() * i.height()) as usize
    })
    .await;

    // ---- looks ----------------------------------------------------------------------------------
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    assert_eq!(a.state["activeLook"], fx.plain_look.id.0.to_string());
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 0}))
        .await;
    wait_frame(
        &server,
        1,
        "key under the plain look shows media and the slide",
        |i| count(i, 0, 40, BLUE, 12) > 1000 && count(i, 40, 230, WHITE, 30) > 200,
    )
    .await;
    let r = a
        .command(json!({"type": "setLook", "look": "00000000-0000-0000-0000-000000000009"}))
        .await;
    assert_eq!(r["error"]["code"], "notFound");
    a.ok(json!({"type": "setLook", "look": null})).await;
    assert!(a.state["activeLook"].is_null());
    b.wait_state("B follows the look", |_, s| s["activeLook"].is_null())
        .await;

    // ---- unknown targets, not implemented, REST commands ---------------------------------------
    let r = a
        .command(json!({"type": "timerStart", "timer": "00000000-0000-0000-0000-000000000001"}))
        .await;
    assert_eq!(r["ok"], false);
    assert_eq!(r["error"]["code"], "notFound");
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "propShow", "prop": "00000000-0000-0000-0000-000000000001"})),
    )
    .await;
    assert_eq!(r.status, 404);
    assert_eq!(r.json()["error"]["code"], "notFound");
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "triggerVideoInput", "input": "00000000-0000-0000-0000-000000000001"})),
    )
    .await;
    // An input this box doesn't have (logical inputs: `live_sources::`).
    assert_eq!(r.status, 404);
    assert_eq!(r.json()["error"]["code"], "notFound");
    // Transport with no video live: a conflict, not a crash.
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "mediaPlay"})),
    )
    .await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["error"]["code"], "conflict");
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "next"})),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["ok"], true);
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "bogus"})),
    )
    .await;
    assert_eq!(r.status, 400);
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "subscribePreview", "screen": fx.audience.id.0.to_string(), "fps": 5, "width": 100})),
    )
    .await;
    assert_eq!(r.json()["error"]["code"], "wsOnly");
    let st = get_json(addr, "/api/v1/state").await;
    assert_eq!(slide_of(&st["state"])["slideIndex"], 1, "REST next applied");

    // ---- thumbnails ---------------------------------------------------------------------------
    let thumb = format!("/api/v1/presentations/{song}/slides/{slide0}/thumbnail?w=320");
    let t0 = Instant::now();
    let t1 = http(addr, "GET", &thumb, &[], None).await;
    let cold = t0.elapsed();
    assert_eq!(t1.status, 200, "{}", String::from_utf8_lossy(&t1.body));
    assert_eq!(t1.header("content-type"), Some("image/jpeg"));
    assert_eq!(&t1.body[..2], &[0xFF, 0xD8], "JPEG magic");
    let etag = t1.header("etag").expect("ETag").to_string();
    assert_eq!(t1.header("cache-control"), Some("no-cache"));
    let img = image::load_from_memory(&t1.body).unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (320, 180));
    assert!(
        count(&img, 0, 20, BLUE, 30) > 500,
        "thumbnail has the background"
    );
    assert!(
        count(&img, 20, 115, WHITE, 60) > 50,
        "thumbnail has the text"
    );
    let t2 = http(addr, "GET", &thumb, &[("If-None-Match", &etag)], None).await;
    assert_eq!(t2.status, 304, "matching ETag");
    assert!(t2.body.is_empty());
    let t0 = Instant::now();
    let t3 = http(addr, "GET", &thumb, &[], None).await;
    let warm = t0.elapsed();
    assert_eq!(t3.body, t1.body, "cached bytes");
    eprintln!("thumbnail cold {cold:?}, cached {warm:?}");
    let disk = fx.dir.join("cache/thumbnails").join(&song);
    let files: Vec<_> = std::fs::read_dir(&disk).unwrap().flatten().collect();
    assert_eq!(files.len(), 1, "one disk cache file");
    let pf = format!("/api/v1/presentations/{song}/thumbnails?w=160");
    let r = http(addr, "POST", &pf, &[], None).await;
    assert_eq!(r.status, 202);
    assert_eq!(r.json()["slides"], 4);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let r = http(addr, "POST", &pf, &[], None).await.json();
        if r["cached"] == 4 {
            break;
        }
        assert!(Instant::now() < deadline, "prefetch never finished: {r}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ---- editor render (transparent PNG, no background media, uncached) ----------------------
    let render = format!("/api/v1/presentations/{song}/slides/{slide0}/render?w=640");
    let r = http(addr, "GET", &render, &[], None).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.header("content-type"), Some("image/png"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    let img = image::load_from_memory(&r.body).unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (640, 360));
    assert_eq!(
        img.get_pixel(2, 2)[3],
        0,
        "no background media: transparent corner"
    );
    let text_px = |img: &image::RgbaImage| {
        img.pixels()
            .filter(|p| p[3] > 200 && p[0] > 200 && p[1] > 200 && p[2] > 200)
            .count()
    };
    assert!(text_px(&img) > 100, "the slide's text is drawn");
    let doc = get_json(addr, &format!("/api/v1/presentations/{song}")).await;
    let ids: Vec<String> = doc["sections"][0]["slides"][0]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    let r = http(
        addr,
        "GET",
        &format!("{render}&hide={}", ids.join("%2C")),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    let img = image::load_from_memory(&r.body).unwrap().to_rgba8();
    assert_eq!(text_px(&img), 0, "hidden elements are left out");
    let r = http(addr, "GET", &format!("{render}&hide=nope"), &[], None).await;
    assert_eq!(r.status, 400);

    // ---- fonts --------------------------------------------------------------------------------
    let fonts = get_json(addr, "/api/v1/fonts").await;
    let fams = fonts["families"].as_array().expect("families");
    assert!(!fams.is_empty(), "the renderer reports installed fonts");
    assert!(
        fams.iter()
            .all(|f| !f["faces"].as_array().unwrap().is_empty())
    );
    assert!(
        fonts["fallbackFamily"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert!(fonts["substitutions"].is_array());

    // ---- live preview -------------------------------------------------------------------------
    let aud = fx.audience.id;
    a.ok(json!({"type": "subscribePreview", "screen": aud.0.to_string(), "fps": 10, "width": 320}))
        .await;
    let f1 = a.next_binary().await;
    let (h1, jpeg) = PreviewFrameHeader::decode(&f1).expect("preview header");
    assert_eq!(h1.screen, aud);
    assert_eq!((h1.width, h1.height), (320, 180));
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
    a.ok(json!({"type": "triggerSlide", "presentation": second, "index": 0}))
        .await;
    let f2 = loop {
        let f = a.next_binary().await;
        let (h, _) = PreviewFrameHeader::decode(&f).unwrap();
        if h.frame > h1.frame {
            break f;
        }
    };
    let (_, jpeg2) = PreviewFrameHeader::decode(&f2).unwrap();
    image::load_from_memory(jpeg2).expect("a valid JPEG");
    a.ok(json!({"type": "unsubscribePreview", "screen": null}))
        .await;
    let r = a
        .command(json!({"type": "subscribePreview", "screen": "00000000-0000-0000-0000-000000000009", "fps": 5, "width": 100}))
        .await;
    assert_eq!(r["error"]["code"], "notFound");

    // ---- protocol version check -------------------------------------------------------------
    let url = format!("ws://{addr}/api/v1/ws?protocolVersion=2");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    use futures_util::StreamExt;
    let first = ws.next().await.unwrap().unwrap();
    let v: Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(v["type"], "error");
    assert_eq!(v["error"]["code"], "incompatibleProtocol");
    match ws.next().await {
        Some(Ok(tokio_tungstenite::tungstenite::Message::Close(Some(f)))) => {
            assert_eq!(u16::from(f.code), 4001)
        }
        other => panic!("expected a close frame, got {other:?}"),
    }

    // ---- local pairing management: without a site (this server runs none) no key can be made
    let code = get_json(addr, "/api/v1/pairing/code").await;
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": code["code"], "clientName": "Booth laptop"})),
    )
    .await;
    assert_eq!(r.status, 503, "pairing needs a site");
    assert_eq!(r.json()["error"]["code"], "unavailable");
    assert_eq!(
        get_json(addr, "/api/v1/pairing/code").await["code"],
        code["code"],
        "a pairing that failed doesn't use up the code"
    );
    assert_eq!(get_json(addr, "/api/v1/clients").await, json!([]));

    // ---- reload -------------------------------------------------------------------------------
    let mut renamed = fx.second.clone();
    renamed.name = "Second Song (new key)".into();
    write_doc(&fx.dir, "portable", "presentation", &second, &renamed);
    a.ok(json!({"type": "reloadData"})).await;
    b.wait_state("B sees the reload", |_, s| {
        s["layers"]["slide"]["presentationName"] == "Second Song (new key)"
    })
    .await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !b
        .events
        .iter()
        .any(|e| e["type"] == "documentChanged" && e["id"] == second)
    {
        assert!(
            Instant::now() < deadline,
            "no documentChanged: {:?}",
            b.events
        );
        let m = b.next_any().await;
        b.absorb(&m);
    }
    let e = b
        .events
        .iter()
        .find(|e| e["type"] == "documentChanged")
        .unwrap();
    assert_eq!(
        (e["kind"].as_str(), e["change"].as_str()),
        (Some("presentation"), Some("changed"))
    );

    drop(a);
    drop(b);
    server.shutdown().await.expect("clean shutdown");
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_details_name_this_vespers_numbers() {
    let fx = fixture("server-details");
    let server = start_server(&fx, true, false).await;
    let d = get_json(server.addr, "/api/v1/server/details").await;
    let info = get_json(server.addr, "/api/v1/server").await;
    assert_eq!(d["version"], info["version"], "{d}");
    assert_eq!(d["name"], info["name"], "{d}");
    assert!(d["cpus"].as_u64().unwrap() >= 1, "{d}");
    assert!(!d["os"].as_str().unwrap().is_empty(), "{d}");
    assert!(d["uptimeSeconds"].as_u64().is_some(), "{d}");
    let products: Vec<&str> = d["products"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p.as_str())
        .collect();
    assert_eq!(products.first(), Some(&"Vesper"), "{d}");
    assert!(
        products.contains(&"Codex"),
        "a standalone box hosts its Codex: {d}"
    );
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_stats_count_ticks_and_the_process() {
    let fx = fixture("server-stats");
    let server = start_server(&fx, true, false).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let s = loop {
        let s = get_json(server.addr, "/api/v1/server/stats").await;
        let ticking = s["screens"]
            .as_array()
            .is_some_and(|a| !a.is_empty() && a.iter().all(|x| x["ticks"].as_u64() > Some(0)));
        if ticking || std::time::Instant::now() > deadline {
            break s;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let screens = s["screens"].as_array().unwrap();
    assert_eq!(screens.len(), 2, "both fixture screens run: {s}");
    for sc in screens {
        assert!(sc["ticks"].as_u64().unwrap() > 0, "{s}");
        assert!(sc["lateTicks"].as_u64().is_some(), "{s}");
        assert!(sc["framesDropped"].as_u64().is_some(), "{s}");
    }
    assert!(s["uptimeSeconds"].as_f64().unwrap() > 0.0, "{s}");
    assert!(s["videos"].as_array().unwrap().is_empty(), "{s}");
    if cfg!(target_os = "linux") {
        assert!(s["process"]["rssBytes"].as_u64().unwrap() > 1 << 20, "{s}");
        assert!(s["process"]["threads"].as_u64().unwrap() > 1, "{s}");
        assert!(s["process"]["openFiles"].as_u64().unwrap() > 3, "{s}");
    }
    if cfg!(all(target_os = "linux", target_env = "gnu")) {
        // The heap's numbers (glibc), for the soak's leak-or-fragmentation question.
        assert!(
            s["process"]["heapInUseBytes"].as_u64().unwrap() > 1 << 20,
            "{s}"
        );
        assert!(s["process"]["heapFreeBytes"].as_u64().is_some(), "{s}");
    }
    // Every cache's size: the renderer's (once it has rendered) and the thumbnails'.
    let names = |s: &Value| -> Vec<String> {
        s["caches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_string())
            .collect()
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut s = s;
    while !names(&s).iter().any(|n| n == "live slide layers (GPU)")
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(200)).await;
        s = get_json(server.addr, "/api/v1/server/stats").await;
    }
    for want in [
        "decoded images (CPU)",
        "glyphs (CPU)",
        "live slide layers (GPU)",
        "video frames (GPU)",
        "slide thumbnails (CPU)",
    ] {
        assert!(names(&s).iter().any(|n| n == want), "{want}: {s}");
    }
    let layers = s["caches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "live slide layers (GPU)")
        .unwrap();
    assert!(
        layers["entries"].as_u64().is_some() && layers["bytes"].as_u64().is_some(),
        "{s}"
    );
    let _ = std::fs::remove_dir_all(&fx.dir);
}
