//! Screen configuration: CRUD with validation, `If-Match` conflicts, atomic persistence,
//! cascading deletes, live reconfiguration that leaves untouched screens alone, the identify
//! overlay and test patterns.

use crate::common::*;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vesper_server::outputs::DevOutputs;
use vesper_server::{RunningServer, ServerConfig, start};

/// Starts the fixture with a PNG development output on every screen (an always-available
/// sink whose counters show that frames keep flowing).
async fn start_with_png(fx: &Fixture) -> RunningServer {
    let dejavu = Path::new("/usr/share/fonts/truetype/dejavu");
    let renderer = if dejavu.is_dir() {
        render::RendererOptions {
            system_fonts: false,
            font_dirs: vec![dejavu.to_path_buf()],
            ..render::RendererOptions::default()
        }
    } else {
        render::RendererOptions::default()
    };
    start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        renderer,
        mdns: false,
        dev: DevOutputs {
            png_dir: Some(fx.dir.join("png-out")),
            png_every: 1_000_000,
            ..DevOutputs::default()
        },
        site: vesper_server::site::SiteConfig::off(),
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts")
}

async fn send(addr: std::net::SocketAddr, method: &str, path: &str, body: Value) -> Resp {
    http(addr, method, path, &[], Some(body)).await
}

async fn send_if(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    rev: &str,
    body: Option<Value>,
) -> Resp {
    let tag = format!("\"{rev}\"");
    http(addr, method, path, &[("If-Match", tag.as_str())], body).await
}

fn body(r: &Resp) -> String {
    String::from_utf8_lossy(&r.body).to_string()
}

fn doc_file(fx: &Fixture, section: &str, kind: &str, id: &str) -> std::path::PathBuf {
    fx.dir.join(section).join(kind).join(format!("{id}.json"))
}

fn no_temp_files(dir: &Path) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        assert!(!n.ends_with(".tmp"), "temporary file left behind: {n}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn screens_and_outputs_crud_with_validation_and_conflicts() {
    let fx = fixture("screencfg-crud");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "").await;

    // Listing: the stored documents plus a revision.
    let list = get_json(addr, "/api/v1/screens").await;
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["name"], "Audience");
    assert!(list[0]["revision"].as_str().unwrap().len() >= 8);
    let transports = get_json(addr, "/api/v1/output-transports").await;
    let kinds: Vec<&str> = transports
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["idn", "omt", "display", "sdi"]);
    // IDN Open Protocol is the network video transport, always built in.
    let idn = &transports[0];
    assert_eq!(idn["supported"], true, "{idn}");
    assert_eq!(idn["available"], true, "{idn}");
    assert!(
        idn["label"]
            .as_str()
            .unwrap()
            .contains("compatible with NDI®")
    );
    assert!(idn["detail"].as_str().unwrap().contains("Vizrt NDI AB"));
    assert!(
        idn["version"].as_str().is_some_and(|v| !v.is_empty()),
        "{idn}"
    );
    assert_eq!(transports[1]["available"], true, "OMT is built in");
    // Display outputs are supported (available where the box has a display backend).
    assert_eq!(transports[2]["supported"], true);
    assert_eq!(transports[3]["supported"], false, "SDI");

    // Create with defaults.
    let r = send(addr, "POST", "/api/v1/screens", json!({})).await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let created = r.json();
    let sid = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["name"], "Screen 1");
    // 1920 × 1080 is the model default, so it is not written out.
    assert!(
        created.get("size").is_none() || created["size"]["width"] == 1920.0,
        "{created}"
    );
    assert!(
        created.get("frameRate").is_none(),
        "30 fps is the default: {created}"
    );
    assert!(
        r.header("etag")
            .unwrap()
            .contains(created["revision"].as_str().unwrap())
    );
    let file = doc_file(&fx, "portable", "screen", &sid);
    assert!(file.is_file(), "written to {}", file.display());
    no_temp_files(file.parent().unwrap());
    // The new screen runs (it is in the show state) and announces itself.
    ws.wait_state("new screen running", |_, s| {
        s["screens"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == sid.as_str())
    })
    .await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ws
        .events
        .iter()
        .any(|e| e["type"] == "documentChanged" && e["kind"] == "screen" && e["change"] == "added")
    {
        assert!(
            Instant::now() < deadline,
            "documentChanged for the new screen"
        );
        let m = ws.next_any().await;
        ws.absorb(&m);
    }

    // Validation: field-tagged errors, nothing written.
    let bad = [
        (json!({"name": "  "}), "name"),
        (json!({"name": "Audience"}), "name"),
        (
            json!({"name": "Odd", "size": {"width": 1279, "height": 720}}),
            "size",
        ),
        (
            json!({"name": "Huge", "size": {"width": 20000, "height": 720}}),
            "size",
        ),
        (
            json!({"name": "Fast", "frameRate": {"num": 1000, "den": 1}}),
            "frameRate",
        ),
        (
            json!({"name": "Zero", "frameRate": {"num": 30, "den": 0}}),
            "frameRate",
        ),
    ];
    for (b, field) in bad {
        let r = send(addr, "POST", "/api/v1/screens", b.clone()).await;
        assert_eq!(r.status, 400, "{b} -> {}", body(&r));
        assert_eq!(r.json()["error"]["field"], field, "{b} -> {}", body(&r));
    }
    assert_eq!(
        get_json(addr, "/api/v1/screens")
            .await
            .as_array()
            .unwrap()
            .len(),
        3
    );

    // Update with If-Match.
    let rev = created["revision"].as_str().unwrap().to_string();
    let mut doc = created.clone();
    doc["name"] = json!("Side");
    doc["size"] = json!({"width": 1280, "height": 720});
    doc["frameRate"] = json!({"num": 25, "den": 1});
    let r = send_if(
        addr,
        "PUT",
        &format!("/api/v1/screens/{sid}"),
        &rev,
        Some(doc.clone()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let updated = r.json();
    assert_eq!(updated["name"], "Side");
    let rev2 = updated["revision"].as_str().unwrap().to_string();
    assert_ne!(rev, rev2);
    let on_disk: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(on_disk["name"], "Side");
    assert!(
        on_disk.get("revision").is_none(),
        "revisions are not stored"
    );

    // A second operator with the old revision gets 409 and the current document.
    doc["name"] = json!("Stale");
    let r = send_if(
        addr,
        "PUT",
        &format!("/api/v1/screens/{sid}"),
        &rev,
        Some(doc.clone()),
    )
    .await;
    assert_eq!(r.status, 409, "{}", body(&r));
    let j = r.json();
    assert_eq!(j["error"]["code"], "conflict");
    assert_eq!(j["current"]["name"], "Side");
    assert_eq!(j["current"]["revision"], rev2.as_str());
    // ... and can overwrite with the revision from the conflict.
    let r = send_if(
        addr,
        "PUT",
        &format!("/api/v1/screens/{sid}"),
        &rev2,
        Some(doc.clone()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let rev3 = r.json()["revision"].as_str().unwrap().to_string();

    // Outputs: create, duplicate IDN name refused, bad delay refused.
    let out = json!({
        "screen": sid,
        "transport": {"type": "idn", "sourceName": " Side Feed "},
        "videoDelay": 2
    });
    let r = send(addr, "POST", "/api/v1/outputs", out).await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let o = r.json();
    let oid = o["id"].as_str().unwrap().to_string();
    assert_eq!(o["transport"]["sourceName"], "Side Feed", "trimmed");
    assert_eq!(o["name"], "Side Feed", "name defaults to the source name");
    assert!(doc_file(&fx, "machine", "outputBinding", &oid).is_file());
    let r = send(
        addr,
        "POST",
        "/api/v1/outputs",
        json!({"screen": fx.audience.id, "transport": {"type": "idn", "sourceName": "side feed"}}),
    )
    .await;
    assert_eq!(r.status, 400, "{}", body(&r));
    assert_eq!(r.json()["error"]["field"], "transport.sourceName");
    assert!(body(&r).contains("Side"), "{}", body(&r));
    // A schema-1 body from an older client (`ndi` transport) is migrated to IDN, so the
    // same name is refused too.
    let r = send(
        addr,
        "POST",
        "/api/v1/outputs",
        json!({"schemaVersion": 1, "screen": fx.audience.id, "transport": {"type": "ndi", "sourceName": "SIDE FEED"}}),
    )
    .await;
    assert_eq!(r.status, 400, "{}", body(&r));
    assert_eq!(r.json()["error"]["field"], "transport.sourceName");
    assert!(
        body(&r).contains("already sends IDN source"),
        "{}",
        body(&r)
    );
    for (b, field) in [
        (
            json!({"screen": sid, "transport": {"type": "idn", "sourceName": "D"}, "videoDelay": 31}),
            "videoDelay",
        ),
        (
            json!({"screen": sid, "transport": {"type": "idn", "sourceName": ""}}),
            "transport.sourceName",
        ),
        (
            json!({"screen": sid, "transport": {"type": "display"}}),
            "transport.selector",
        ),
        (json!({"screen": sid}), "transport"),
        (
            json!({"screen": uuid::Uuid::new_v4(), "transport": {"type": "idn", "sourceName": "X"}}),
            "screen",
        ),
        (
            json!({"screen": sid, "transport": {"type": "idn", "sourceName": "R"}, "resolutionOverride": {"width": 1001, "height": 500}}),
            "resolutionOverride",
        ),
    ] {
        let r = send(addr, "POST", "/api/v1/outputs", b.clone()).await;
        assert_eq!(r.status, 400, "{b} -> {}", body(&r));
        assert_eq!(r.json()["error"]["field"], field, "{b} -> {}", body(&r));
    }
    // The output shows in the show state, matched by binding ID.
    ws.wait_state("output listed", |_, s| {
        s["screens"].as_array().unwrap().iter().any(|x| {
            x["outputs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["binding"] == oid.as_str() && o["delayFrames"] == 2)
        })
    })
    .await;
    let outs = get_json(addr, "/api/v1/outputs").await;
    assert_eq!(outs.as_array().unwrap().len(), 1);

    // Update the output (rename the source) and delete it with a stale revision first.
    let orev = o["revision"].as_str().unwrap().to_string();
    let mut od = o.clone();
    od["transport"]["sourceName"] = json!("Side Feed 2");
    let r = send_if(
        addr,
        "PUT",
        &format!("/api/v1/outputs/{oid}"),
        &orev,
        Some(od),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let r = send_if(
        addr,
        "DELETE",
        &format!("/api/v1/outputs/{oid}"),
        &orev,
        None,
    )
    .await;
    assert_eq!(r.status, 409, "{}", body(&r));
    assert_eq!(
        r.json()["current"]["transport"]["sourceName"],
        "Side Feed 2"
    );

    // A document written by a newer server is never overwritten.
    let audience_file = doc_file(&fx, "portable", "screen", &fx.audience.id.0.to_string());
    let mut newer: Value = serde_json::from_slice(&std::fs::read(&audience_file).unwrap()).unwrap();
    newer["schemaVersion"] = json!(999);
    std::fs::write(&audience_file, serde_json::to_vec(&newer).unwrap()).unwrap();
    let mut ad = serde_json::to_value(&fx.audience).unwrap();
    ad["name"] = json!("Audience 2");
    let r = send(
        addr,
        "PUT",
        &format!("/api/v1/screens/{}", fx.audience.id.0),
        ad,
    )
    .await;
    assert_eq!(r.status, 409, "{}", body(&r));
    assert!(body(&r).contains("newer"), "{}", body(&r));
    let still: Value = serde_json::from_slice(&std::fs::read(&audience_file).unwrap()).unwrap();
    assert_eq!(still["schemaVersion"], 999);

    // Delete the new screen: its output goes with it.
    let r = send_if(
        addr,
        "DELETE",
        &format!("/api/v1/screens/{sid}"),
        &rev3,
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    assert_eq!(r.json()["outputs"], json!([oid]));
    assert!(!file.exists());
    assert!(!doc_file(&fx, "machine", "outputBinding", &oid).exists());
    assert!(
        get_json(addr, "/api/v1/outputs")
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );
    ws.wait_state("screen gone", |_, s| {
        !s["screens"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == sid.as_str())
    })
    .await;
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_a_screen_removes_it_from_looks_and_clears_its_pattern() {
    let fx = fixture("screencfg-cascade");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let key = fx.key.id.0.to_string();
    let r = send(
        addr,
        "POST",
        "/api/v1/outputs",
        json!({"screen": key, "transport": {"type": "omt", "name": "Key OMT"}}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let oid = r.json()["id"].as_str().unwrap().to_string();
    let mut ws = Client::connect(addr, "").await;
    ws.ok(json!({"type": "setTestPattern", "screen": key, "pattern": {"type": "grid"}}))
        .await;
    assert!(
        ws.state["screens"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["testPattern"]["type"] == "grid")
    );

    let r = http(addr, "DELETE", &format!("/api/v1/screens/{key}"), &[], None).await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let j = r.json();
    assert_eq!(j["outputs"], json!([oid]));
    assert_eq!(j["looks"], json!([fx.default_look.id]));
    let look: Value = serde_json::from_slice(
        &std::fs::read(doc_file(
            &fx,
            "portable",
            "look",
            &fx.default_look.id.0.to_string(),
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(
        look.get("screens")
            .is_none_or(|s| s.as_array().unwrap().is_empty()),
        "{look}"
    );
    let looks = get_json(addr, "/api/v1/looks").await;
    assert!(!looks.to_string().contains(&key));
    // A test pattern on a deleted screen is gone; setting one now is refused.
    ws.wait_state("screen gone", |_, s| {
        s["screens"].as_array().unwrap().len() == 1
    })
    .await;
    let r = ws
        .command(json!({"type": "setTestPattern", "screen": key, "pattern": {"type": "grid"}}))
        .await;
    assert_eq!(r["ok"], false);
    // A reload sees the same thing the server already applied.
    ws.ok(json!({"type": "reloadData"})).await;
    assert_eq!(
        get_json(addr, "/api/v1/screens")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_reconfiguration_leaves_untouched_screens_running() {
    let fx = fixture("screencfg-hot");
    let server = start_with_png(&fx).await;
    let addr = server.addr;
    let sh = server.shared.clone();
    let audience = sh.screen(fx.audience.id).expect("audience runs");
    let png = audience.outputs()[0].clone();
    assert_eq!(png.kind, "png");
    tokio::time::sleep(Duration::from_millis(700)).await;
    let renders0 = audience.stats.lock().unwrap().renders;
    let t0 = Instant::now();
    let ticks0 = audience.stats.lock().unwrap().ticks;
    let sent0 = png.stats.sent.load(std::sync::atomic::Ordering::Relaxed);

    // Add a screen at another size and rate, change it, rename it, delete it.
    let r = send(
        addr,
        "POST",
        "/api/v1/screens",
        json!({"name": "Side", "size": {"width": 320, "height": 180}, "frameRate": {"num": 50, "den": 1}}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let side = r.json();
    let sid: model::ScreenId = serde_json::from_value(side["id"].clone()).unwrap();
    let rt = sh.screen(sid).expect("new screen running");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let f = rt.latest.lock().unwrap().clone();
        if let Some(f) = f
            && (f.width, f.height) == (320, 180)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the new screen renders at its size"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let fps = rt.stats.lock().unwrap().fps;
    assert!(
        (40.0..=55.0).contains(&fps),
        "new screen runs at 50 fps: {fps}"
    );
    let mut doc = side.clone();
    doc["size"] = json!({"width": 640, "height": 360});
    doc["frameRate"] = json!({"num": 25, "den": 1});
    doc["background"] = json!({"r": 0.0, "g": 1.0, "b": 0.0, "a": 1.0});
    let r = send(
        addr,
        "PUT",
        &format!("/api/v1/screens/{}", sid.0),
        doc.clone(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let rt2 = sh.screen(sid).unwrap();
    assert!(
        Arc::ptr_eq(&rt, &rt2),
        "a changed screen keeps its runtime and clock"
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let f = rt.latest.lock().unwrap().clone();
        if let Some(f) = f
            && (f.width, f.height) == (640, 360)
            && f.pixel(10, 10)[1] > 200
        {
            break;
        }
        assert!(Instant::now() < deadline, "resized, green background");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let fps = rt.stats.lock().unwrap().fps;
    assert!(
        (20.0..=28.0).contains(&fps),
        "rate change applied live: {fps}"
    );
    // Disable it, then delete it.
    doc["enabled"] = json!(false);
    let r = send(addr, "PUT", &format!("/api/v1/screens/{}", sid.0), doc).await;
    assert_eq!(r.status, 200, "{}", body(&r));
    assert!(sh.screen(sid).is_none(), "a disabled screen stops");
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/screens/{}", sid.0),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));

    // The untouched Audience screen: same runtime, same output, no re-render, and its clock
    // and output kept counting steadily the whole time.
    let a2 = sh.screen(fx.audience.id).unwrap();
    assert!(Arc::ptr_eq(&audience, &a2));
    assert!(Arc::ptr_eq(&png, &a2.outputs()[0]), "untouched output kept");
    let elapsed = t0.elapsed().as_secs_f64();
    let st = audience.stats.lock().unwrap().clone();
    let ticks = st.ticks - ticks0;
    let sent = png.stats.sent.load(std::sync::atomic::Ordering::Relaxed) - sent0;
    let expected = elapsed * 30.0;
    assert!(
        (ticks as f64) > expected * 0.9,
        "{ticks} ticks in {elapsed:.1}s (expected ~{expected:.0})"
    );
    assert!(
        (sent as f64) > expected * 0.9,
        "{sent} frames sent in {elapsed:.1}s"
    );
    assert_eq!(
        png.stats.dropped.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(
        st.renders, renders0,
        "edits of other screens never re-render this one"
    );
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identify_overlay_and_test_patterns() {
    let fx = fixture("screencfg-overlay");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "").await;
    let audience = fx.audience.id.0.to_string();

    // Identify: a bordered dark label box centred on every screen.
    ws.ok(json!({"type": "identifyScreens", "enabled": true}))
        .await;
    assert_eq!(ws.state["identify"], true);
    let (w, h) = (W as f64, H as f64);
    let bx = (w * 0.15) as u32;
    let by = (h * 0.5) as u32;
    let border = |img: &image::RgbaImage| {
        (bx - 3..bx + 3).any(|x| near(img.get_pixel(x, by), [255, 255, 255], 40))
    };
    wait_frame(&server, 0, "identify border", border).await;
    // The same on the preview a client sees.
    ws.ok(json!({"type": "subscribePreview", "screen": audience, "fps": 5, "width": 640}))
        .await;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let m = ws.next_any().await;
        if m.get("binary").is_some() {
            let b = ws.binary.last().unwrap().clone();
            let img = image::load_from_memory(&b[protocol::PREVIEW_HEADER_LEN..])
                .unwrap()
                .to_rgb8();
            if (bx - 3..bx + 3).any(|x| img.get_pixel(x, by).0.iter().all(|c| *c > 180)) {
                break;
            }
        } else {
            ws.absorb(&m);
        }
        assert!(Instant::now() < deadline, "identify label in the preview");
    }
    ws.ok(json!({"type": "unsubscribePreview", "screen": null}))
        .await;
    ws.ok(json!({"type": "identifyScreens", "enabled": false}))
        .await;
    wait_frame(&server, 0, "label gone", |img| !border(img)).await;

    // Colour bars replace the picture and are listed in the state.
    ws.ok(json!({"type": "setTestPattern", "screen": audience, "pattern": {"type": "colorBars"}}))
        .await;
    let s = ws.state["screens"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == audience.as_str())
        .unwrap()
        .clone();
    assert_eq!(s["testPattern"]["type"], "colorBars");
    let bw = W as f64 / 7.0;
    wait_frame(&server, 0, "colour bars", |img| {
        render::patterns::BARS_75
            .iter()
            .enumerate()
            .all(|(i, c)| near(img.get_pixel(((i as f64 + 0.5) * bw) as u32, H / 3), *c, 3))
    })
    .await;
    // A solid colour.
    ws.ok(json!({"type": "setTestPattern", "screen": audience,
        "pattern": {"type": "solid", "color": {"r": 1.0, "g": 0.0, "b": 1.0, "a": 1.0}}}))
        .await;
    wait_frame(&server, 0, "magenta", |img| {
        near(img.get_pixel(5, 5), [255, 0, 255], 3)
    })
    .await;
    // The motion pattern changes on every clock tick.
    ws.ok(json!({"type": "setTestPattern", "screen": audience, "pattern": {"type": "motion"}}))
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let rt = server.shared.screen(fx.audience.id).unwrap();
    let seq = |rt: &vesper_server::ScreenRuntime| {
        rt.latest.lock().unwrap().as_ref().map_or(0, |f| f.sequence)
    };
    let s0 = seq(&rt);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let frames = seq(&rt) - s0;
    assert!(
        frames >= 40,
        "motion re-renders every tick at 30 fps: {frames} in 2 s"
    );
    // Cleared: back to the show (nothing live: black).
    ws.ok(json!({"type": "setTestPattern", "screen": audience, "pattern": null}))
        .await;
    wait_frame(&server, 0, "black again", |img| {
        near(img.get_pixel(W / 2, H / 3), [0, 0, 0], 5)
    })
    .await;
    assert!(ws.state["screens"][0]["testPattern"].is_null());
    let r = ws
        .command(json!({"type": "setTestPattern", "screen": uuid::Uuid::new_v4(), "pattern": {"type": "grid"}}))
        .await;
    assert_eq!(r["ok"], false);
    server.shutdown().await.unwrap();
}
