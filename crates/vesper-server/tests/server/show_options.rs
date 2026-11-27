//! Operator-facing additions to the protocol: screen on/off per role, the continuous-playlist
//! option, media bins (tree, items, thumbnails, `triggerMedia`), keyed (rolling) notices,
//! the application-level ping, CORS on `/api/v1`, serving the built control UI and the live
//! look kept across restarts.

use crate::common::*;
use futures_util::SinkExt;
use model::{
    AssetLocation, MediaAsset, MediaBin, MediaBinItem, MediaBinItemId, MediaBinKind, MediaKind,
    MediaLayer, MediaTrigger, Presentation, Section, SlideAction,
};
use serde_json::json;
use std::path::Path;
use tokio_tungstenite::tungstenite::Message;
use vesper_server::{RunningServer, ServerConfig, start};

/// Adds to the fixture: a missing image, a video, a "Backgrounds" bin in folder "Motion"
/// [blue, missing, video], and a presentation whose slide uses the missing image.
fn extras(fx: &Fixture) -> (MediaBin, MediaAsset, MediaAsset, Presentation) {
    let missing = MediaAsset {
        name: "Gone".into(),
        location: AssetLocation::Managed {
            sha256: "nothere".into(),
            extension: "png".into(),
        },
        ..MediaAsset::default()
    };
    let video = MediaAsset {
        name: "Loop".into(),
        kind: MediaKind::Video,
        location: AssetLocation::Managed {
            sha256: "bg0001".into(),
            extension: "png".into(),
        },
        ..MediaAsset::default()
    };
    for a in [&missing, &video] {
        write_doc(&fx.dir, "portable", "mediaAsset", &a.id.0.to_string(), a);
    }
    let item = |a: &MediaAsset| MediaBinItem {
        id: MediaBinItemId::new(),
        name: None,
        enabled: true,
        trigger: MediaTrigger::new(a.id, MediaLayer::Background),
    };
    let bin = MediaBin {
        name: "Backgrounds".into(),
        folder: vec!["Motion".into()],
        kind: MediaBinKind::Manual {
            items: vec![item(&fx.asset), item(&missing), item(&video)],
        },
        ..MediaBin::default()
    };
    write_doc(&fx.dir, "portable", "mediaBin", &bin.id.0.to_string(), &bin);
    let mut s = Section::new("Verse");
    let mut slide = model::Slide::new();
    slide.actions.push(SlideAction::media(MediaTrigger::new(
        missing.id,
        MediaLayer::Background,
    )));
    s.slides = vec![slide];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![s],
        ..Presentation::new("Broken Backgrounds")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    (bin, missing, video, p)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn screens_options_media_bins_and_notices() {
    let fx = fixture("options");
    let (bin, missing, video, broken) = extras(&fx);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "protocolVersion=1").await;

    // ---- defaults and ping -------------------------------------------------------------------
    assert_eq!(
        a.state["screensEnabled"],
        json!({"audience": true, "stage": true})
    );
    assert_eq!(a.state["options"], json!({"continuousPlaylist": true}));
    assert_eq!(a.state["screens"][0]["role"], "audience");
    assert_eq!(a.state["screens"][0]["enabled"], true);
    a.ws.send(Message::Text(
        json!({"type": "ping", "id": 42}).to_string().into(),
    ))
    .await
    .unwrap();
    loop {
        let m = a.next_any().await;
        if m["type"] == "pong" {
            assert_eq!(m["id"], 42);
            break;
        }
        a.absorb(&m);
    }

    // ---- audience off: outputs keep running, picture goes black / transparent ---------------
    let song = fx.song.id.0.to_string();
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 0}))
        .await;
    wait_frame(&server, 0, "blue background live", |img| {
        count(img, 0, H, BLUE, 30) > 1000
    })
    .await;
    a.ok(json!({"type": "setScreensEnabled", "role": "audience", "enabled": false}))
        .await;
    assert_eq!(a.state["screensEnabled"]["audience"], false);
    assert_eq!(a.state["screens"][0]["enabled"], false);
    assert_eq!(a.state["screens"][1]["enabled"], false);
    assert!(!a.state["layers"]["slide"].is_null(), "layers stay live");
    wait_frame(&server, 0, "audience black", |img| {
        count(img, 0, H, BLACK, 2) as u32 == W * H
    })
    .await;
    wait_frame(&server, 1, "key transparent", |img| {
        img.pixels().all(|p| p.0[3] == 0)
    })
    .await;
    a.ok(json!({"type": "setScreensEnabled", "role": "audience", "enabled": true}))
        .await;
    wait_frame(&server, 0, "blue back", |img| {
        count(img, 0, H, BLUE, 30) > 1000
    })
    .await;
    // Stage screens (none in the fixture) switch independently.
    a.ok(json!({"type": "setScreensEnabled", "role": "stage", "enabled": false}))
        .await;
    assert_eq!(
        a.state["screensEnabled"],
        json!({"audience": true, "stage": false})
    );
    assert_eq!(a.state["screens"][0]["enabled"], true);

    // ---- continuous playlist ------------------------------------------------------------------
    let playlist = fx.playlist.id.0.to_string();
    let items: Vec<String> = fx.items().iter().map(|i| i.id.0.to_string()).collect();
    a.ok(
        json!({"type": "triggerSlide", "presentation": song, "index": 3,
                "playlistItem": {"playlist": playlist, "item": items[1]}}),
    )
    .await;
    assert_eq!(
        a.state["cursor"]["presentation"],
        fx.second.id.0.to_string(),
        "continuous: the cursor is the next item's first slide"
    );
    a.ok(json!({"type": "setOption", "continuousPlaylist": false}))
        .await;
    assert_eq!(a.state["options"]["continuousPlaylist"], false);
    assert!(a.state["cursor"].is_null(), "{}", a.state["cursor"]);
    let rev = a.revision;
    a.ok(json!({"type": "next"})).await;
    assert_eq!(a.revision, rev, "next at the end stops");
    assert_eq!(a.state["layers"]["slide"]["slideIndex"], 3);
    assert_eq!(a.state["layers"]["slide"]["presentation"], song);
    a.ok(json!({"type": "prev"})).await;
    assert_eq!(
        a.state["layers"]["slide"]["slideIndex"], 1,
        "prev still works"
    );
    a.ok(json!({"type": "setOption"})).await; // nothing given: no change
    assert_eq!(a.state["options"]["continuousPlaylist"], false);
    a.ok(json!({"type": "setOption", "continuousPlaylist": true}))
        .await;
    a.ok(
        json!({"type": "triggerSlide", "presentation": song, "index": 3,
                "playlistItem": {"playlist": playlist, "item": items[1]}}),
    )
    .await;
    a.ok(json!({"type": "next"})).await;
    assert_eq!(
        a.state["layers"]["slide"]["presentation"],
        fx.second.id.0.to_string()
    );

    // ---- media bins --------------------------------------------------------------------------
    let tree = get_json(addr, "/api/v1/media-bins").await;
    assert_eq!(tree[0]["kind"], "folder", "{tree}");
    assert_eq!(tree[0]["name"], "Motion");
    assert!(tree[0]["id"].is_null());
    assert_eq!(tree[0]["itemCount"], 3);
    assert_eq!(tree[0]["children"][0]["name"], "Backgrounds");
    assert_eq!(tree[0]["children"][0]["kind"], "manual");
    assert_eq!(tree[0]["children"][0]["path"], json!(["Motion"]));
    assert_eq!(tree[0]["children"][0]["id"], bin.id.0.to_string());
    let detail = get_json(addr, &format!("/api/v1/media-bins/{}", bin.id.0)).await;
    let items = detail["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "{detail}");
    assert_eq!(items[0]["name"], "Blue background");
    assert_eq!(items[0]["kind"], "image");
    assert_eq!(items[0]["availability"], "online");
    assert_eq!(items[1]["availability"], "missing");
    assert_eq!(items[2]["kind"], "video");
    let r = http(
        addr,
        "GET",
        "/api/v1/media-bins/00000000-0000-0000-0000-000000000009",
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 404);

    // Thumbnails: decoded image, ETag, placeholder for video, 404 when missing.
    let thumb = format!("/api/v1/media/{}/thumbnail?w=160", fx.asset.id.0);
    let r = http(addr, "GET", &thumb, &[], None).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("image/jpeg"));
    let img = image::load_from_memory(&r.body).unwrap().to_rgb8();
    assert_eq!(img.width(), 64, "never upscaled past the file");
    let p = img.get_pixel(30, 18);
    assert!(
        near(&image::Rgba([p[0], p[1], p[2], 255]), BLUE, 30),
        "{p:?}"
    );
    let etag = r.header("etag").unwrap().to_string();
    let r = http(addr, "GET", &thumb, &[("If-None-Match", &etag)], None).await;
    assert_eq!(r.status, 304);
    let r = http(
        addr,
        "GET",
        &format!("/api/v1/media/{}/thumbnail?w=160", video.id.0),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    // (This "video" is a still PNG, which FFmpeg decodes as one frame: never upscaled.)
    assert_eq!(image::load_from_memory(&r.body).unwrap().width(), 64);
    let r = http(
        addr,
        "GET",
        &format!("/api/v1/media/{}/thumbnail", missing.id.0),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 404);

    // triggerMedia: an image goes to the media layer (videos: see tests/server/video.rs).
    a.ok(json!({"type": "clearAll"})).await;
    a.ok(json!({"type": "triggerMedia", "asset": fx.asset.id.0.to_string()}))
        .await;
    assert_eq!(
        a.state["layers"]["media"]["asset"],
        fx.asset.id.0.to_string()
    );
    assert!(a.state["layers"]["slide"].is_null());
    wait_frame(&server, 0, "bin image live", |img| {
        count(img, 0, H, BLUE, 30) > 1000
    })
    .await;
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "triggerMedia", "asset": missing.id.0.to_string(), "layer": "audio"})),
    )
    .await;
    // An image cannot go to the audio layer.
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "triggerMedia", "asset": fx.asset.id.0.to_string(), "layer": "slide"})),
    )
    .await;
    assert_eq!(r.status, 400);

    // ---- keyed notices: one summary per presentation ----------------------------------------
    a.ok(json!({"type": "triggerSlide", "presentation": broken.id.0.to_string(), "index": 0}))
        .await;
    let key = format!("missingMedia:{}", broken.id.0);
    let notice = loop {
        let m = a.next_any().await;
        if m["type"] == "notice" && m["key"] == key.as_str() {
            break m;
        }
        a.absorb(&m);
    };
    assert_eq!(
        notice["message"], "1 media file missing in 'Broken Backgrounds'",
        "{notice}"
    );
    assert_eq!(notice["level"], "warning");
    // A new client gets the summary once, not one notice per file.
    let mut late = Client::connect(addr, "").await;
    let mut seen = 0;
    a.ok(json!({"type": "next"})).await; // something after the replay
    late.ok(json!({"type": "clearLayer", "layer": "props"}))
        .await;
    for e in &late.events {
        if e["type"] == "notice" && e["key"] == key.as_str() {
            seen += 1;
        }
        assert!(
            !(e["type"] == "notice"
                && e["key"].is_null()
                && e["message"]
                    .as_str()
                    .is_some_and(|m| m.starts_with("media "))),
            "no per-file media notices: {e}"
        );
    }
    assert_eq!(seen, 1, "{:?}", late.events);
    let diag = get_json(addr, "/api/v1/diagnostics").await;
    assert!(
        diag["missingMedia"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["what"].as_str().unwrap().starts_with("Gone")),
        "the full list stays in diagnostics: {diag}"
    );

    // ---- CORS --------------------------------------------------------------------------------
    let r = http(
        addr,
        "OPTIONS",
        "/api/v1/state",
        &[
            ("Origin", "tauri://localhost"),
            ("Access-Control-Request-Method", "GET"),
            ("Access-Control-Request-Headers", "authorization"),
        ],
        None,
    )
    .await;
    assert!(r.status < 300, "preflight answered: {}", r.status);
    assert_eq!(r.header("access-control-allow-origin"), Some("*"));
    let allowed = r
        .header("access-control-allow-headers")
        .unwrap()
        .to_lowercase();
    assert!(allowed.contains("authorization") && allowed.contains("content-type"));
    let r = http(
        addr,
        "GET",
        "/api/v1/state",
        // (The desktop app's page: a page from elsewhere isn't the trusted server machine.)
        &[("Origin", "tauri://localhost")],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("access-control-allow-origin"), Some("*"));
    // No UI configured: `/` is a JSON 404.
    let r = http(addr, "GET", "/", &[], None).await;
    assert_eq!(r.status, 404);

    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serves_the_control_ui_with_spa_fallback() {
    let fx = fixture("ui");
    let ui = fx.dir.join("ui");
    std::fs::create_dir_all(ui.join("assets")).unwrap();
    std::fs::write(ui.join("index.html"), "<!doctype html><title>UI</title>").unwrap();
    std::fs::write(ui.join("assets/app.js"), "console.log(1)").unwrap();
    let server = start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: false,
        mdns: false,
        ui_dir: Some(ui.clone()),
        site: vesper_server::site::SiteConfig::off(),
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .unwrap();
    let addr = server.addr;
    // The UI needs no token (it pairs itself); the API still does.
    let r = http(addr, "GET", "/", &[], None).await;
    assert_eq!(r.status, 200);
    assert!(String::from_utf8_lossy(&r.body).contains("<title>UI</title>"));
    let r = http(addr, "GET", "/assets/app.js", &[], None).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"console.log(1)");
    let r = http(addr, "GET", "/show/some/route", &[], None).await;
    assert_eq!(r.status, 200, "SPA fallback");
    assert!(String::from_utf8_lossy(&r.body).contains("<title>UI</title>"));
    let r = http(addr, "GET", "/api/v1/state", &[], None).await;
    assert_eq!(r.status, 401);
    let r = http(addr, "GET", "/api/v2/anything", &[], None).await;
    assert_eq!(r.status, 401, "API paths never fall back to the UI");
    assert!(Path::new(&ui).join("index.html").is_file());
    server.shutdown().await.unwrap();
}

/// The live look after a start, as a fresh client sees it.
async fn look_after_restart(fx: &Fixture) -> (RunningServer, Client) {
    let server = start_server(fx, true, false).await;
    let a = Client::connect(server.addr, "").await;
    (server, a)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_chosen_live_look_survives_a_restart() {
    let fx = fixture("live-look");
    let plain = fx.plain_look.id.0.to_string();

    // Nothing chosen yet: the show settings' startup look.
    let (server, mut a) = look_after_restart(&fx).await;
    assert_eq!(a.state["activeLook"], fx.default_look.id.0.to_string());
    a.ok(json!({"type": "setLook", "look": plain})).await;
    server.shutdown().await.unwrap();

    // A chosen look is live again.
    let (server, mut a) = look_after_restart(&fx).await;
    assert_eq!(
        a.state["activeLook"], plain,
        "the chosen look after a restart"
    );
    a.ok(json!({"type": "setLook", "look": null})).await;
    server.shutdown().await.unwrap();

    // Every layer stays every layer (not the startup look, not the first look).
    let (server, mut a) = look_after_restart(&fx).await;
    assert!(a.state["activeLook"].is_null(), "{}", a.state["activeLook"]);
    a.ok(json!({"type": "setLook", "look": plain})).await;
    server.shutdown().await.unwrap();

    // The chosen look is gone: every layer.
    std::fs::remove_file(fx.dir.join(format!("portable/look/{plain}.json"))).unwrap();
    let (server, a) = look_after_restart(&fx).await;
    assert!(a.state["activeLook"].is_null(), "{}", a.state["activeLook"]);
    server.shutdown().await.unwrap();

    // Never chosen and no startup look: every layer, not the first look.
    std::fs::remove_file(fx.dir.join(vesper_server::show::LIVE_LOOK_FILE)).unwrap();
    let settings: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            fx.dir
                .join("portable/showSettings/00000000-0000-0000-0000-000000000000.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(settings.get("startupLook").is_some(), "{settings}");
    let mut settings = settings;
    settings.as_object_mut().unwrap().remove("startupLook");
    write_doc(
        &fx.dir,
        "portable",
        "showSettings",
        "00000000-0000-0000-0000-000000000000",
        &settings,
    );
    let (server, a) = look_after_restart(&fx).await;
    assert!(a.state["activeLook"].is_null(), "{}", a.state["activeLook"]);
    server.shutdown().await.unwrap();
}
