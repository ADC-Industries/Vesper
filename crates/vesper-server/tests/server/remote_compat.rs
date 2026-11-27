//! Control surfaces: `triggerMacro` and `GET /macros`, and the Remote API compatibility layer
//! (`/api/v1/remote-compat`; its own port answering `/version` and `/v1/...`, behind the
//! API's authentication).

use crate::common::*;
use model::{ActionKind, Link, Macro, SlideAction, Timer, TimerKind, TimerOperation};
use serde_json::{Value, json};
use std::net::SocketAddr;
use vesper_server::site::SiteConfig;

/// Adds a "Walk-in" countdown and a "Start service" macro (the Plain look, start Walk-in).
fn extras(fx: &Fixture) -> (Timer, Macro) {
    let timer = Timer {
        name: "Walk-in".into(),
        sort_key: "a".into(),
        kind: TimerKind::Countdown { seconds: 600.0 },
        ..Timer::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "timer",
        &timer.id.0.to_string(),
        &timer,
    );
    let m = Macro {
        name: "Start service".into(),
        actions: vec![
            SlideAction::new(ActionKind::Look {
                look: Link::to(fx.plain_look.id),
            }),
            SlideAction::new(ActionKind::Timer {
                timer: Link::to(timer.id),
                operation: TimerOperation::Start,
            }),
        ],
        ..Macro::default()
    };
    write_doc(&fx.dir, "portable", "macro", &m.id.0.to_string(), &m);
    (timer, m)
}

async fn state(addr: SocketAddr) -> Value {
    get_json(addr, "/api/v1/state").await["state"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn macros_are_listed_and_triggered() {
    let fx = fixture("rc-macro");
    let (timer, m) = extras(&fx);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;

    let info = get_json(addr, "/api/v1/server").await;
    let caps = info["capabilities"].as_array().unwrap();
    for c in ["macros", "remoteCompat"] {
        assert!(caps.iter().any(|x| x == c), "{c}: {info}");
    }
    let list = get_json(addr, "/api/v1/macros").await;
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list[0]["name"], "Start service");
    assert_eq!(list[0]["id"], m.id.0.to_string());

    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "triggerMacro", "macro": m.id.0.to_string()})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let s = state(addr).await;
    assert_eq!(s["activeLook"], fx.plain_look.id.0.to_string(), "{s}");
    let t = s["timers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == timer.id.0.to_string())
        .unwrap();
    assert_eq!(t["state"], "running", "{t}");

    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "triggerMacro", "macro": "00000000-0000-0000-0000-000000000001"})),
    )
    .await;
    assert_eq!(r.status, 404);
    server.shutdown().await.unwrap();
}

/// Switches the layer on at a free port; its address.
async fn enable(addr: SocketAddr, headers: &[(&str, &str)]) -> SocketAddr {
    let r = http(
        addr,
        "PUT",
        "/api/v1/remote-compat",
        headers,
        Some(json!({"enabled": true, "port": 0})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let s = r.json();
    assert_eq!(s["enabled"], true, "{s}");
    assert_eq!(s["listening"], true, "{s}");
    let port = s["listeningPort"].as_u64().unwrap() as u16;
    SocketAddr::from(([127, 0, 0, 1], port))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_compat_maps_the_common_endpoints() {
    let fx = fixture("rc-compat");
    let (timer, _) = extras(&fx);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;

    // Off by default.
    let s = get_json(addr, "/api/v1/remote-compat").await;
    assert_eq!(
        s,
        json!({"enabled": false, "port": 50001, "listening": false}),
        "{s}"
    );
    let rc = enable(addr, &[]).await;
    let get = |path: &'static str| async move { http(rc, "GET", path, &[], None).await };

    let v = get("/version").await;
    assert_eq!(v.status, 200);
    assert!(
        v.json()["host_description"]
            .as_str()
            .unwrap()
            .starts_with("Vesper "),
        "{}",
        v.json()
    );

    // Slides: by presentation and index, next, the live presentation's index.
    let song = fx.song.id.0.to_string();
    let r = http(
        rc,
        "GET",
        &format!("/v1/presentation/{song}/0/trigger"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let s = state(addr).await;
    assert_eq!(s["layers"]["slide"]["presentation"], song);
    assert_eq!(s["layers"]["slide"]["slideIndex"], 0);
    assert_eq!(get("/v1/trigger/next").await.status, 204);
    let i = state(addr).await["layers"]["slide"]["slideIndex"]
        .as_u64()
        .unwrap();
    assert!(i > 0, "next moved on");
    assert_eq!(get("/v1/presentation/active/3/trigger").await.status, 204);
    assert_eq!(state(addr).await["layers"]["slide"]["slideIndex"], 3);
    assert_eq!(
        get("/v1/presentation/focused/previous/trigger")
            .await
            .status,
        204
    );
    assert_eq!(
        get("/v1/presentation/slide_index").await.json()["presentation_index"]["presentation_id"]["uuid"],
        song
    );
    let slide = get("/v1/status/slide").await.json();
    assert!(
        !slide["current"]["text"].as_str().unwrap().is_empty(),
        "{slide}"
    );

    // Looks by name; the live look.
    let looks = get("/v1/looks").await.json();
    assert_eq!(looks.as_array().unwrap().len(), 2, "{looks}");
    assert_eq!(get("/v1/look/plain/trigger").await.status, 204);
    assert_eq!(
        state(addr).await["activeLook"],
        fx.plain_look.id.0.to_string()
    );
    assert_eq!(get("/v1/look/current").await.json()["id"]["name"], "Plain");

    // Timers by index, name and UUID.
    assert_eq!(get("/v1/timers").await.json()[0]["id"]["name"], "Walk-in");
    assert_eq!(get("/v1/timer/0/start").await.status, 204);
    assert_eq!(
        get("/v1/timers/current").await.json()[0]["state"],
        "running"
    );
    let r = http(
        rc,
        "PUT",
        "/v1/timer/walk-in/increment",
        &[],
        Some(json!(60)),
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let r = http(
        rc,
        "GET",
        &format!("/v1/timer/{}/reset", timer.id.0),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204);
    assert_eq!(
        get("/v1/timers/current").await.json()[0]["state"],
        "stopped"
    );
    assert_eq!(get("/v1/timer/0/explode").await.status, 400);
    assert_eq!(get("/v1/timer/nope/start").await.status, 404);

    // Clear.
    assert_eq!(get("/v1/clear/layer/slide").await.status, 204);
    assert!(state(addr).await["layers"]["slide"].is_null());
    assert_eq!(get("/v1/clear/layer/bogus").await.status, 400);
    assert_eq!(
        get("/v1/clear/groups").await.json()[0]["id"]["name"],
        "Clear Slide"
    );

    // A playlist item (item 1: the song after the header).
    assert_eq!(get("/v1/playlist/sunday/1/trigger").await.status, 204);
    let s = state(addr).await;
    assert_eq!(s["layers"]["slide"]["presentation"], song, "{s}");
    assert!(!s["layers"]["slide"]["playlist"].is_null(), "{s}");

    // Macros and the stage message.
    assert_eq!(
        get("/v1/macros").await.json()[0]["id"]["name"],
        "Start service"
    );
    assert_eq!(get("/v1/macro/0/trigger").await.status, 204);
    let r = http(rc, "PUT", "/v1/stage/message", &[], Some(json!("Wrap up"))).await;
    assert_eq!(r.status, 204);
    assert_eq!(state(addr).await["stageMessage"]["text"], "Wrap up");
    assert_eq!(get("/v1/stage/message").await.json(), json!("Wrap up"));
    assert_eq!(
        http(rc, "DELETE", "/v1/stage/message", &[], None)
            .await
            .status,
        204
    );
    assert!(state(addr).await["stageMessage"].is_null());

    // Not covered; and a request that isn't the server machine's needs a key.
    assert_eq!(get("/v1/library/x").await.status, 404);
    let r = http(rc, "GET", "/version", &[("Host", "booth.example")], None).await;
    assert_eq!(r.status, 401);

    // Off again: the port closes. On again after a restart (the setting is kept).
    let r = http(
        addr,
        "PUT",
        "/api/v1/remote-compat",
        &[],
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(r.json()["listening"], false);
    let mut closed = false;
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(rc).await.is_err() {
            closed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(closed, "the port closes");
    enable(addr, &[]).await;
    server.shutdown().await.unwrap();
    let server = start_server(&fx, true, false).await;
    let s = get_json(server.addr, "/api/v1/remote-compat").await;
    assert_eq!(s["enabled"], true, "{s}");
    assert_eq!(s["listening"], true, "{s}");
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_compat_takes_a_site_key() {
    let fx = fixture("rc-key");
    let server = start_with_site(&fx, false, SiteConfig::loopback()).await;
    let addr = server.addr;
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": server.pairing_code(), "clientName": "Companion"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let key = r.json()["token"].as_str().unwrap().to_string();
    let bearer = format!("Bearer {key}");
    let auth = [("Authorization", bearer.as_str())];

    // Changing the setting needs a key like the rest of the API.
    let r = http(
        addr,
        "PUT",
        "/api/v1/remote-compat",
        &[],
        Some(json!({"enabled": true})),
    )
    .await;
    assert_eq!(r.status, 401);
    let rc = enable(addr, &auth).await;

    assert_eq!(http(rc, "GET", "/version", &[], None).await.status, 401);
    assert_eq!(http(rc, "GET", "/version", &auth, None).await.status, 200);
    let r = http(rc, "GET", &format!("/v1/looks?token={key}"), &[], None).await;
    assert_eq!(r.status, 200);
    assert_eq!(
        http(rc, "GET", "/v1/look/Plain/trigger", &auth, None)
            .await
            .status,
        204
    );
    server.shutdown().await.unwrap();
}

/// The triggers are plain `GET`s, which any web page open in a browser on the server machine
/// can send (`<img src="http://127.0.0.1:50001/v1/trigger/next">` carries no `Origin`): a
/// browser-made request is never the server machine here, only a key counts for it. Button
/// panels and scripts (no browser headers) and a URL typed into the address bar stay trusted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_web_page_cannot_trigger_through_the_compat_layer() {
    let fx = fixture("rc-browser");
    let server = start_with_site(&fx, true, SiteConfig::loopback()).await;
    let addr = server.addr;
    let rc = enable(addr, &[]).await;
    let song = fx.song.id.0.to_string();
    let trigger = format!("/v1/presentation/{song}/0/trigger");
    let pages: [&[(&str, &str)]; 7] = [
        // An `<img>` or `<script>` from another site (Chromium, Firefox, Safari 16.4+).
        &[
            ("Sec-Fetch-Site", "cross-site"),
            ("Sec-Fetch-Mode", "no-cors"),
            ("Sec-Fetch-Dest", "image"),
        ],
        // A page served from another port of this machine.
        &[
            ("Sec-Fetch-Site", "same-site"),
            ("Sec-Fetch-Mode", "no-cors"),
        ],
        // `fetch()` from a page: it names itself, even a page of this machine.
        &[("Origin", "http://attacker.example")],
        &[("Origin", "http://localhost:8080")],
        &[("Origin", "null")],
        // An older browser without `Sec-Fetch-*`: the page sends its address.
        &[("Referer", "http://attacker.example/lyrics")],
        &[("Referer", "http://127.0.0.1:8080/")],
    ];
    for h in pages {
        let r = http(rc, "GET", &trigger, h, None).await;
        assert_eq!(r.status, 401, "{h:?}: {}", String::from_utf8_lossy(&r.body));
        let r = http(rc, "GET", "/v1/trigger/next", h, None).await;
        assert_eq!(r.status, 401, "{h:?}");
    }
    assert!(
        state(addr).await["layers"]["slide"]["presentation"].is_null(),
        "nothing went live"
    );
    // A button panel or script (no browser headers), or a URL typed into the address bar.
    let r = http(rc, "GET", &trigger, &[], None).await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let typed = [("Sec-Fetch-Site", "none"), ("Sec-Fetch-Mode", "navigate")];
    assert_eq!(
        http(rc, "GET", "/v1/trigger/next", &typed, None)
            .await
            .status,
        204
    );
    assert_eq!(state(addr).await["layers"]["slide"]["slideIndex"], 1);

    // A paired browser client sends its key.
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": server.pairing_code(), "clientName": "Browser remote"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let key = r.json()["token"].as_str().unwrap().to_string();
    let r = http(
        rc,
        "GET",
        &format!("/v1/trigger/next?token={key}"),
        &[
            ("Sec-Fetch-Site", "cross-site"),
            ("Origin", "http://192.0.2.20:8000"),
        ],
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    server.shutdown().await.unwrap();
}

/// Port 0 gives the layer a free port; typing that port in keeps it listening there (the
/// old listener's socket is closed before the new one opens on the same port).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_port_it_was_given_can_be_kept() {
    let fx = fixture("rc-keep-port");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    for _ in 0..3 {
        let rc = enable(addr, &[]).await;
        let r = http(
            addr,
            "PUT",
            "/api/v1/remote-compat",
            &[],
            Some(json!({"port": rc.port()})),
        )
        .await;
        assert_eq!(r.status, 200);
        let s = r.json();
        assert_eq!(s["listening"], true, "{s}");
        assert_eq!(s["listeningPort"], rc.port(), "{s}");
        assert!(s.get("problem").is_none(), "{s}");
        assert_eq!(http(rc, "GET", "/version", &[], None).await.status, 200);
    }
    server.shutdown().await.unwrap();
}

/// Changes sent at the same time (two Oriel windows) are applied one after the other: none
/// is lost, and what is saved is what runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_at_the_same_time_save_what_runs() {
    let fx = fixture("rc-concurrent");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let file = fx.dir.join("machine/remote-compat.json");
    for round in 0..8 {
        let off = http(
            addr,
            "PUT",
            "/api/v1/remote-compat",
            &[],
            Some(json!({"enabled": false, "port": 50001})),
        );
        assert_eq!(off.await.status, 200);
        let (a, b) = tokio::join!(
            http(
                addr,
                "PUT",
                "/api/v1/remote-compat",
                &[],
                Some(json!({"enabled": true}))
            ),
            http(
                addr,
                "PUT",
                "/api/v1/remote-compat",
                &[],
                Some(json!({"port": 0}))
            ),
        );
        assert_eq!((a.status, b.status), (200, 200));
        let s = get_json(addr, "/api/v1/remote-compat").await;
        assert_eq!(s["enabled"], true, "round {round}: {s}");
        assert_eq!(s["port"], 0, "round {round}: {s}");
        assert_eq!(s["listening"], true, "round {round}: {s}");
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved["enabled"], true, "round {round}: {saved}");
        assert_eq!(saved["port"], 0, "round {round}: {saved}");
    }
    server.shutdown().await.unwrap();
}
