//! Vesper Mosaic on the site's server (`docs/MOSAIC-PLAN.md`) against a fake IDN display
//! service: a small HTTP server answering the service's documented API (`GET /api/v1/state`,
//! `POST /api/v1/feed`, `settings`, `identify`, `test_pattern`, `GET /api/v1/thumbnail`; changes
//! need `X-IDN-Display: 1`, every request a bearer token; errors `{"ok": false, "error"}`).
//!
//! A machine joins the site by code, an admin adds it as a display box at the fake's address;
//! then: its screens are read, a feed is set, a channel is created and followed and re-pointed
//! (only the screen following it moves), a channel followed can't be deleted, a box that lost its
//! feeds (restarted) gets them again, settings go in the service's shape, identify and test
//! pattern (one screen and all), the thumbnail, `siteChanged` (`mosaic`), a Vesper box can't be
//! added, and a box that stops answering shows its screens offline.
//!
//! Video walls (round M2) on two such boxes: every tile gets the same `settings.wall` geometry
//! with its own row and column, the wall's sync group and its feed; validation; a moved or
//! removed screen; a lost tile set again; a wall following a channel; `stats.sync` into the
//! wall's state; identify; delete.

use super::*;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct Fake {
    /// (number, name, feed, test pattern).
    screens: Vec<(u32, String, String, bool)>,
    /// Every change: (command, body).
    posts: Vec<(String, Value)>,
    /// The bearer tokens seen.
    tokens: Vec<String>,
    /// Each screen's settings as it last took them (without `screen`).
    settings: HashMap<u32, Value>,
    /// Each screen's `stats.sync`.
    sync: HashMap<u32, Value>,
    /// A screen whose late count grows on every read of the state.
    late_growing: Option<u32>,
    /// The service's own error (`state.error`).
    error: String,
    /// Away: every request answers `503` (as with its LAN listener off).
    down: bool,
    /// A feed is taken, but the answer says it failed (as a change that timed out).
    feed_fails_after: bool,
}

type FakeState = Arc<Mutex<Fake>>;

fn refuse(status: StatusCode, e: &str) -> Response {
    (status, axum::Json(json!({"ok": false, "error": e}))).into_response()
}

/// `Some`: the refusal (no bearer token).
fn bearer(h: &HeaderMap, f: &FakeState) -> Option<Response> {
    match h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        Some(t) if !t.is_empty() => {
            f.lock().unwrap().tokens.push(t.to_string());
            None
        }
        _ => Some(refuse(StatusCode::FORBIDDEN, "not logged in")),
    }
}

/// `Some`: the refusal (the box is away).
fn away(f: &FakeState) -> Option<Response> {
    f.lock()
        .unwrap()
        .down
        .then(|| refuse(StatusCode::SERVICE_UNAVAILABLE, "lan off"))
}

async fn fake_state(State(f): State<FakeState>, h: HeaderMap) -> Response {
    if let Some(r) = bearer(&h, &f).or_else(|| away(&f)) {
        return r;
    }
    let mut fk = f.lock().unwrap();
    if let Some(n) = fk.late_growing
        && let Some(y) = fk.sync.get_mut(&n)
    {
        y["late"] = json!(y["late"].as_u64().unwrap_or(0) + 2);
    }
    let screens: Vec<Value> = fk
        .screens
        .iter()
        .map(|(n, name, feed, tp)| {
            let mut v = json!({
                "id": name, "name": name, "number": n, "width": 1920, "height": 1080,
                "refresh_hz": 60, "connected": true, "feed": feed, "error": "",
                "status": {"state": if feed.is_empty() { "idle" } else { "playing" }, "showing": "feed"},
                "test_pattern": tp,
                "stats": {"fps": 30},
            });
            if let Some(st) = fk.settings.get(n) {
                v["settings"] = st.clone();
            }
            if let Some(y) = fk.sync.get(n) {
                v["stats"]["sync"] = y.clone();
            }
            v
        })
        .collect();
    let error = fk.error.clone();
    drop(fk);
    axum::Json(json!({"service": "display", "version": 1, "error": error, "screens": screens, "problems": []}))
        .into_response()
}

async fn fake_post(
    State(f): State<FakeState>,
    axum::extract::Path(cmd): axum::extract::Path<String>,
    h: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Some(r) = bearer(&h, &f).or_else(|| away(&f)) {
        return r;
    }
    if h.get("x-idn-display").and_then(|v| v.to_str().ok()) != Some("1") {
        return refuse(StatusCode::FORBIDDEN, "X-IDN-Display: 1 missing");
    }
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return refuse(StatusCode::BAD_REQUEST, "bad JSON");
    };
    let mut fk = f.lock().unwrap();
    let screen = v["screen"].clone();
    let known = screen == "all"
        || screen
            .as_u64()
            .is_some_and(|n| fk.screens.iter().any(|s| u64::from(s.0) == n));
    if !known {
        return refuse(StatusCode::BAD_REQUEST, "unknown screen");
    }
    fk.posts.push((cmd.clone(), v.clone()));
    match cmd.as_str() {
        "feed" => {
            let n = screen.as_u64().unwrap() as u32;
            let src = v["source"].as_str().unwrap_or("").to_string();
            if let Some(s) = fk.screens.iter_mut().find(|s| s.0 == n) {
                s.2 = src.clone();
            }
            if fk.feed_fails_after {
                return refuse(StatusCode::INTERNAL_SERVER_ERROR, "it took too long");
            }
            axum::Json(json!({"ok": true, "screen": n, "feed": src})).into_response()
        }
        "test_pattern" => {
            let on = v["on"].as_bool().unwrap_or(false);
            for s in fk.screens.iter_mut() {
                if screen == "all" || screen.as_u64() == Some(u64::from(s.0)) {
                    s.3 = on;
                }
            }
            axum::Json(json!({"ok": true})).into_response()
        }
        "settings" => {
            let n = screen.as_u64().unwrap() as u32;
            let mut st = v.clone();
            st.as_object_mut().unwrap().remove("screen");
            fk.settings.insert(n, st);
            axum::Json(json!({"ok": true, "screen": screen})).into_response()
        }
        "identify" => axum::Json(json!({"ok": true, "screen": screen})).into_response(),
        _ => refuse(StatusCode::NOT_FOUND, "unknown page"),
    }
}

async fn fake_thumbnail(
    State(f): State<FakeState>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Some(r) = bearer(&h, &f) {
        return r;
    }
    let Some(n) = q.get("screen").and_then(|s| s.parse::<u32>().ok()) else {
        return refuse(StatusCode::BAD_REQUEST, "screen?");
    };
    let fk = f.lock().unwrap();
    match fk.screens.iter().find(|s| s.0 == n) {
        Some(s) if !s.2.is_empty() => (
            [("content-type", "image/jpeg")],
            vec![0xFFu8, 0xD8, 0xFF, 0xD9],
        )
            .into_response(),
        _ => refuse(StatusCode::NOT_FOUND, "it shows nothing"),
    }
}

/// The fake service on a free port: its state, its address and how to stop it.
async fn fake_service() -> (FakeState, SocketAddr, tokio::sync::oneshot::Sender<()>) {
    let f: FakeState = Arc::new(Mutex::new(Fake {
        screens: vec![
            (1, "HDMI-1".into(), String::new(), false),
            (2, "DP-2".into(), String::new(), false),
        ],
        ..Default::default()
    }));
    let app = Router::new()
        .route("/api/v1/state", get(fake_state))
        .route("/api/v1/thumbnail", get(fake_thumbnail))
        .route("/api/v1/{cmd}", post(fake_post))
        .with_state(f.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = axum::serve(l, app)
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await;
    });
    (f, addr, tx)
}

/// Waits until `GET /api/v1/mosaic` matches.
async fn until_state(addr: SocketAddr, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let s = get_json(addr, "/api/v1/mosaic").await;
        if pred(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "{what}: {s}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Waits until the fake matches.
async fn until_fake(f: &FakeState, what: &str, pred: impl Fn(&Fake) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !pred(&f.lock().unwrap()) {
        assert!(
            Instant::now() < deadline,
            "{what}: {:?}",
            f.lock().unwrap().posts
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn feed_of(f: &FakeState, n: u32) -> String {
    f.lock()
        .unwrap()
        .screens
        .iter()
        .find(|s| s.0 == n)
        .unwrap()
        .2
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn display_boxes_screens_and_channels_through_the_display_service() {
    let fx = fixture("site-mosaic");
    let server = start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: true,
        mdns: false,
        site: SiteConfig::loopback(),
        mosaic_poll: Duration::from_millis(200),
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts");
    let addr = server.addr;
    let info = get_json(addr, "/api/v1/server").await;
    assert!(
        info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("mosaic"))
    );
    let st = until_state(addr, "managed", |s| s["managed"] == true).await;
    assert_eq!(st["boxes"], json!([]), "no display box yet");
    let ds = get_json(addr, "/api/v1/site").await["discoveryPort"]
        .as_u64()
        .unwrap() as u16;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;

    // A machine joins by code (no Node reports a display service in a test: an admin adds it).
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    let lobby = machine(&fx, ds, "LOBBY");
    lobby
        .join_with_code(code["code"].as_str().unwrap())
        .unwrap();
    until(&lobby, "LOBBY joins", |s| {
        matches!(s, TrustState::Enrolled { .. })
    })
    .await;
    let fp = lobby.fingerprint();
    let (fake, fake_addr, stop_fake) = fake_service().await;
    let box_path = format!("/api/v1/mosaic/boxes/{fp}");
    let r = ok(
        &call(
            addr,
            "PUT",
            &box_path,
            None,
            Some(json!({"address": format!("http://{fake_addr}")})),
        )
        .await,
        200,
        "add the box",
    );
    assert_eq!(r["boxes"][0]["machine"], fp.as_str(), "{r}");
    assert_eq!(r["boxes"][0]["added"], true);
    assert_eq!(r["boxes"][0]["vesper"], false);
    let st = until_state(addr, "its screens read", |s| {
        s["boxes"][0]["reachable"] == true
            && s["boxes"][0]["screens"]
                .as_array()
                .is_some_and(|a| a.len() == 2)
    })
    .await;
    let screens = &st["boxes"][0]["screens"];
    assert_eq!(screens[0]["number"], 1);
    assert_eq!(screens[0]["name"], "HDMI-1");
    assert_eq!(screens[1]["name"], "DP-2");
    assert_eq!(screens[0]["status"], "idle");
    assert_eq!(screens[0]["feed"], json!({"kind": "none"}));
    // siteChanged: mosaic.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let m = ws.next_any().await;
        if m["type"] == "siteChanged" && m["parts"].as_array().unwrap().contains(&json!("mosaic")) {
            break;
        }
        assert!(Instant::now() < deadline, "no siteChanged mosaic");
    }

    // A source on screen 1: set at once, with the service's headers and a token.
    let screen = |n: u32, what: &str| format!("{box_path}/screens/{n}/{what}");
    let r = ok(
        &call(
            addr,
            "PUT",
            &screen(1, "feed"),
            None,
            Some(json!({"kind": "source", "source": "BOOTH (Program)"})),
        )
        .await,
        200,
        "feed",
    );
    assert_eq!(
        r["boxes"][0]["screens"][0]["feed"],
        json!({"kind": "source", "source": "BOOTH (Program)"})
    );
    assert_eq!(feed_of(&fake, 1), "BOOTH (Program)");
    assert!(
        fake.lock().unwrap().tokens.iter().all(|t| t.len() > 20),
        "a site token each time"
    );
    assert_eq!(
        call(
            addr,
            "PUT",
            &screen(7, "feed"),
            None,
            Some(json!({"kind": "none"}))
        )
        .await
        .status,
        404,
        "a screen the box doesn't have"
    );

    // A channel; screen 2 follows it, then the channel moves (screen 1 stays).
    let ch = ok(
        &call(
            addr,
            "POST",
            "/api/v1/mosaic/channels",
            None,
            Some(json!({"name": "Main", "source": "BOOTH (Stage)"})),
        )
        .await,
        201,
        "channel",
    );
    let id = ch["id"].as_str().unwrap().to_string();
    assert_eq!(
        call(
            addr,
            "POST",
            "/api/v1/mosaic/channels",
            None,
            Some(json!({"name": "main"}))
        )
        .await
        .status,
        409,
        "names are unique"
    );
    ok(
        &call(
            addr,
            "PUT",
            &screen(2, "feed"),
            None,
            Some(json!({"kind": "channel", "channel": id})),
        )
        .await,
        200,
        "follow",
    );
    assert_eq!(feed_of(&fake, 2), "BOOTH (Stage)");
    let r = ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/channels/{id}"),
            None,
            Some(json!({"source": "BOOTH (Lobby Loop)"})),
        )
        .await,
        200,
        "re-point",
    );
    assert_eq!(r["channels"][0]["screens"], 1, "{r}");
    assert_eq!(feed_of(&fake, 2), "BOOTH (Lobby Loop)");
    assert_eq!(
        feed_of(&fake, 1),
        "BOOTH (Program)",
        "only the screen following it moves"
    );
    assert_eq!(
        call(
            addr,
            "DELETE",
            &format!("/api/v1/mosaic/channels/{id}"),
            None,
            None
        )
        .await
        .status,
        409,
        "a channel screens follow stays"
    );

    // The box restarted with no feeds: Vesper sets them again.
    for s in fake.lock().unwrap().screens.iter_mut() {
        s.2.clear();
    }
    until_fake(&fake, "feeds set again", |f| {
        f.screens[0].2 == "BOOTH (Program)" && f.screens[1].2 == "BOOTH (Lobby Loop)"
    })
    .await;

    // Settings, in the service's shape; out of range refused.
    ok(
        &call(
            addr,
            "PUT",
            &screen(1, "settings"),
            None,
            Some(json!({"fit": "fill", "rotation": 90, "overscan": 2.5, "fallback": "black", "fallbackAfterSeconds": 10})),
        )
        .await,
        200,
        "settings",
    );
    let (cmd, body) = fake.lock().unwrap().posts.last().cloned().unwrap();
    assert_eq!(cmd, "settings");
    assert_eq!(body["screen"], 1);
    assert_eq!(body["fit"], "fill");
    assert_eq!(body["rotation"], 90);
    assert_eq!(body["fallback"], json!({"show": "black", "after_s": 10}));
    assert_eq!(body["audio"], json!({"device": null}));
    assert_eq!(
        call(
            addr,
            "PUT",
            &screen(1, "settings"),
            None,
            Some(json!({"rotation": 45}))
        )
        .await
        .status,
        400
    );
    let st = get_json(addr, "/api/v1/mosaic").await;
    assert_eq!(
        st["boxes"][0]["screens"][0]["settings"]["fit"], "fill",
        "{st}"
    );

    // Identify and test pattern: one screen, then all.
    ok(
        &call(addr, "POST", &screen(1, "identify"), None, None).await,
        204,
        "identify",
    );
    assert_eq!(
        fake.lock().unwrap().posts.last().cloned().unwrap(),
        ("identify".to_string(), json!({"screen": 1, "seconds": 5.0}))
    );
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/mosaic/identify",
            None,
            Some(json!({"seconds": 8})),
        )
        .await,
        204,
        "identify all",
    );
    assert_eq!(
        fake.lock().unwrap().posts.last().cloned().unwrap(),
        (
            "identify".to_string(),
            json!({"screen": "all", "seconds": 8.0})
        )
    );
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/mosaic/test-pattern",
            None,
            Some(json!({"on": true})),
        )
        .await,
        204,
        "test pattern all",
    );
    assert!(fake.lock().unwrap().screens.iter().all(|s| s.3));
    until_state(addr, "test pattern shown", |s| {
        s["boxes"][0]["screens"][1]["testPattern"] == true
    })
    .await;
    ok(
        &call(
            addr,
            "POST",
            &screen(2, "test-pattern"),
            None,
            Some(json!({"on": false})),
        )
        .await,
        204,
        "test pattern off",
    );
    assert!(!fake.lock().unwrap().screens[1].3);

    // The thumbnail of what it shows.
    let r = call(addr, "GET", &screen(1, "thumbnail"), None, None).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, vec![0xFF, 0xD8, 0xFF, 0xD9]);

    // A Vesper box is never a display box: its screens are Vesper's own outputs.
    let vesper = Enrollment::start(EnrollmentConfig {
        discovery_server: format!("127.0.0.1:{ds}"),
        state_dir: fx.dir.join("machines").join("chapel"),
        name: "Chapel (Vesper)".into(),
        role: DeviceRole::App,
        passphrase: None,
        admin_code: None,
    })
    .unwrap();
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    vesper
        .join_with_code(code["code"].as_str().unwrap())
        .unwrap();
    until(&vesper, "the Vesper box joins", |s| {
        matches!(s, TrustState::Enrolled { .. })
    })
    .await;
    assert_eq!(
        call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/boxes/{}", vesper.fingerprint()),
            None,
            Some(json!({}))
        )
        .await
        .status,
        409
    );

    // The box stops answering: its screens show offline, what they should show is kept.
    let _ = stop_fake.send(());
    let st = until_state(addr, "offline", |s| s["boxes"][0]["reachable"] == false).await;
    assert_eq!(st["boxes"][0]["screens"][0]["status"], "offline", "{st}");
    assert!(st["boxes"][0]["problem"].is_string());
    assert_eq!(
        st["boxes"][0]["screens"][0]["feed"]["source"],
        "BOOTH (Program)"
    );
    let r = call(
        addr,
        "PUT",
        &screen(1, "feed"),
        None,
        Some(json!({"kind": "none"})),
    )
    .await;
    assert_eq!(r.status, 503, "kept, but the box didn't take it");
    assert_eq!(
        get_json(addr, "/api/v1/mosaic").await["boxes"][0]["screens"][0]["feed"],
        json!({"kind": "none"})
    );

    // Kept in the machine folder.
    let kept: Value =
        serde_json::from_slice(&std::fs::read(fx.dir.join("machine/mosaic.json")).unwrap())
            .unwrap();
    assert_eq!(kept["channels"][0]["name"], "Main");

    // Forgetting the box.
    ok(
        &call(addr, "DELETE", &box_path, None, None).await,
        204,
        "forget",
    );
    until_state(addr, "forgotten", |s| s["boxes"] == json!([])).await;
}

/// Bug run 5 (H01-1, H01-5, H01-6, H01-7, H01-8, H01-9): a box set up before Vesper managed it
/// keeps its own feed and settings until Vesper sets them; settings changed while the box was
/// away reach it when it answers; the service's own error is shown; one token while the box
/// doesn't answer; a change taken although its answer failed doesn't stay an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_box_keeps_what_vesper_never_set_and_gets_what_it_missed() {
    let fx = fixture("site-mosaic-kept");
    let server = start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: true,
        mdns: false,
        site: SiteConfig::loopback(),
        mosaic_poll: Duration::from_millis(200),
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts");
    let addr = server.addr;
    until_state(addr, "managed", |s| s["managed"] == true).await;
    let ds = get_json(addr, "/api/v1/site").await["discoveryPort"]
        .as_u64()
        .unwrap() as u16;
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    let lobby = machine(&fx, ds, "FOYER");
    lobby
        .join_with_code(code["code"].as_str().unwrap())
        .unwrap();
    until(&lobby, "joins", |s| {
        matches!(s, TrustState::Enrolled { .. })
    })
    .await;
    let fp = lobby.fingerprint();
    // Set up on the box itself: screen 1 shows a loop, portrait, its sound on HDMI.
    let (fake, fake_addr, stop_fake) = fake_service().await;
    {
        let mut fk = fake.lock().unwrap();
        fk.screens[0].2 = "FOYER (Loop)".into();
        fk.settings.insert(
            1,
            json!({"fit": "fill", "rotation": 90, "overscan": 0, "idle": "desktop",
                "audio": {"device": "HDMI 1", "channels": [0, 1], "volume_db": -6.0, "mute": false},
                "fallback": {"show": "black", "after_s": 5, "logo": "", "reconnect_cap": 0},
                "wall": null}),
        );
    }
    let box_path = format!("/api/v1/mosaic/boxes/{fp}");
    let screen = |n: u32, what: &str| format!("{box_path}/screens/{n}/{what}");
    ok(
        &call(
            addr,
            "PUT",
            &box_path,
            None,
            Some(json!({"address": format!("http://{fake_addr}")})),
        )
        .await,
        200,
        "add the box",
    );
    let st = until_state(addr, "its screens read", |s| {
        s["boxes"][0]["reachable"] == true
            && s["boxes"][0]["screens"]
                .as_array()
                .is_some_and(|a| a.len() == 2)
    })
    .await;

    // H01-6: the box's own settings are reported, not defaults.
    let s1 = &st["boxes"][0]["screens"][0];
    assert_eq!(s1["showing"], "FOYER (Loop)");
    assert_eq!(s1["settings"]["rotation"], 90, "{s1}");
    assert_eq!(s1["settings"]["fit"], "fill");
    assert_eq!(s1["settings"]["fallback"], "black");
    assert_eq!(s1["settings"]["sound"]["device"], "HDMI 1");
    assert_eq!(s1["settings"]["sound"]["volumeDb"], -6.0);

    // H01-1: only its settings changed from Vesper (overscan, as Oriel sends them: whole): the
    // feed the box shows stays, poll after poll.
    let mut settings = s1["settings"].clone();
    settings["overscan"] = json!(2.0);
    ok(
        &call(addr, "PUT", &screen(1, "settings"), None, Some(settings)).await,
        200,
        "settings",
    );
    let body = posts_for(&fake, "settings", 1, 0).pop().unwrap();
    assert_eq!(body["rotation"], 90, "the box's rotation kept: {body}");
    assert_eq!(body["audio"]["device"], "HDMI 1", "its sound kept: {body}");
    let reads = fake.lock().unwrap().tokens.len();
    until_fake(&fake, "a few polls", |f| f.tokens.len() > reads + 5).await;
    assert_eq!(
        feed_of(&fake, 1),
        "FOYER (Loop)",
        "the box's own feed stays"
    );
    assert!(posts_for(&fake, "feed", 1, 0).is_empty());
    let kept: Value =
        serde_json::from_slice(&std::fs::read(fx.dir.join("machine/mosaic.json")).unwrap())
            .unwrap();
    assert!(
        kept["boxes"][fp.as_str()]["screens"]["1"]
            .get("feed")
            .is_none(),
        "{kept}"
    );

    // H01-5 and H01-8: the box away (503): one token all along; a fallback change kept, and
    // set when it answers again.
    fake.lock().unwrap().down = true;
    until_state(addr, "away", |s| s["boxes"][0]["reachable"] == false).await;
    let mut settings =
        get_json(addr, "/api/v1/mosaic").await["boxes"][0]["screens"][0]["settings"].clone();
    settings["fallback"] = json!("last");
    settings["sound"]["mute"] = json!(true);
    let r = call(addr, "PUT", &screen(1, "settings"), None, Some(settings)).await;
    assert_eq!(r.status, 503, "kept, not taken now");
    let reads = fake.lock().unwrap().tokens.len();
    until_fake(&fake, "polls while away", |f| f.tokens.len() > reads + 5).await;
    {
        let fk = fake.lock().unwrap();
        let first = &fk.tokens[0];
        assert!(
            fk.tokens.iter().all(|t| t == first),
            "no new token for every read of a box that doesn't answer"
        );
    }
    fake.lock().unwrap().down = false;
    until_fake(&fake, "the kept settings sent", |f| {
        f.settings[&1]["fallback"]["show"] == "last" && f.settings[&1]["audio"]["mute"] == true
    })
    .await;
    let from = fake.lock().unwrap().posts.len();
    let reads = fake.lock().unwrap().tokens.len();
    until_fake(&fake, "a few polls", |f| f.tokens.len() > reads + 5).await;
    assert!(
        posts_for(&fake, "settings", 1, from).is_empty(),
        "sent once, then left alone"
    );

    // H01-9: a feed taken although its answer failed: an error at first, then gone once the
    // box is read showing it.
    fake.lock().unwrap().feed_fails_after = true;
    let r = call(
        addr,
        "PUT",
        &screen(2, "feed"),
        None,
        Some(json!({"kind": "source", "source": "BOOTH (Program)"})),
    )
    .await;
    assert_eq!(r.status, 503);
    assert_eq!(feed_of(&fake, 2), "BOOTH (Program)");
    fake.lock().unwrap().feed_fails_after = false;
    until_state(addr, "the error goes", |s| {
        s["boxes"][0]["screens"][1]["status"] == "showing"
    })
    .await;

    // H01-7: a box without a desktop session: its service's error, its screens kept.
    let screens = {
        let mut fk = fake.lock().unwrap();
        fk.error = "no desktop on this computer".into();
        std::mem::take(&mut fk.screens)
    };
    let st = until_state(addr, "the service's error", |s| {
        s["boxes"][0]["problem"]
            .as_str()
            .is_some_and(|p| p.contains("no desktop"))
    })
    .await;
    assert_eq!(st["boxes"][0]["reachable"], true, "{st}");
    assert!(
        st["boxes"][0]["screens"][0]["problem"]
            .as_str()
            .unwrap()
            .contains("no desktop"),
        "{st}"
    );
    let r = call(
        addr,
        "PUT",
        &screen(1, "feed"),
        None,
        Some(json!({"kind": "source", "source": "BOOTH (Stage)"})),
    )
    .await;
    assert_ne!(r.status, 404, "its screens aren't gone, it has no desktop");
    {
        let mut fk = fake.lock().unwrap();
        fk.error.clear();
        fk.screens = screens;
    }
    until_fake(&fake, "set when it has its screens again", |f| {
        f.screens[0].2 == "BOOTH (Stage)"
    })
    .await;
    let _ = stop_fake.send(());
}

/// The posts of `cmd` for screen `n` since `from`.
fn posts_for(f: &FakeState, cmd: &str, n: u32, from: usize) -> Vec<Value> {
    f.lock().unwrap().posts[from..]
        .iter()
        .filter(|(c, v)| c == cmd && v["screen"] == n)
        .map(|(_, v)| v.clone())
        .collect()
}

fn wall_of(f: &FakeState, n: u32) -> Value {
    f.lock()
        .unwrap()
        .settings
        .get(&n)
        .map(|s| s["wall"].clone())
        .unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn video_walls_across_display_boxes() {
    let fx = fixture("site-mosaic-walls");
    let server = start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: true,
        mdns: false,
        site: SiteConfig::loopback(),
        mosaic_poll: Duration::from_millis(200),
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts");
    let addr = server.addr;
    until_state(addr, "managed", |s| s["managed"] == true).await;
    let ds = get_json(addr, "/api/v1/site").await["discoveryPort"]
        .as_u64()
        .unwrap() as u16;

    // Two display boxes (two screens each), each at its fake display service.
    let mut fakes = Vec::new();
    let mut fps = Vec::new();
    for name in ["FRONT-LEFT", "FRONT-RIGHT"] {
        let code = ok(
            &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
            201,
            "code",
        );
        let m = machine(&fx, ds, name);
        m.join_with_code(code["code"].as_str().unwrap()).unwrap();
        until(&m, "joins", |s| matches!(s, TrustState::Enrolled { .. })).await;
        let (fake, fake_addr, stop) = fake_service().await;
        ok(
            &call(
                addr,
                "PUT",
                &format!("/api/v1/mosaic/boxes/{}", m.fingerprint()),
                None,
                Some(json!({"address": format!("http://{fake_addr}")})),
            )
            .await,
            200,
            "add the box",
        );
        fps.push(m.fingerprint());
        fakes.push((fake, stop, m));
    }
    let (a, b) = (fps[0].clone(), fps[1].clone());
    let (fa, fb) = (fakes[0].0.clone(), fakes[1].0.clone());
    until_state(addr, "both boxes read", |s| {
        s["boxes"].as_array().is_some_and(|bx| {
            bx.len() == 2
                && bx
                    .iter()
                    .all(|x| x["reachable"] == true && x["screens"].as_array().unwrap().len() == 2)
        })
    })
    .await;
    let cell = |row: u16, column: u16, machine: &str, screen: u32| json!({"row": row, "column": column, "machine": machine, "screen": screen});
    let front = |cells: Vec<Value>| {
        json!({
            "name": "Front", "rows": 2, "columns": 2, "bezelMm": [10.0, 10.0],
            "screenMm": [1210.0, 680.0],
            "feed": {"kind": "source", "source": "BOOTH (Program)"}, "cells": cells,
        })
    };
    let all4 = vec![
        cell(0, 0, &a, 1),
        cell(0, 1, &a, 2),
        cell(1, 0, &b, 1),
        cell(1, 1, &b, 2),
    ];

    // Validation.
    let refused = |mut w: Value, k: &str, v: Value| {
        w[k] = v;
        w
    };
    for (what, body, status) in [
        ("no rows", refused(front(vec![]), "rows", json!(0)), 400),
        (
            "17 columns",
            refused(front(vec![]), "columns", json!(17)),
            400,
        ),
        (
            "a bezel without the screen's size",
            refused(front(vec![]), "screenMm", Value::Null),
            400,
        ),
        (
            "an empty name",
            refused(front(vec![]), "name", json!(" ")),
            400,
        ),
        (
            "a name too long for its sync group (R1-7)",
            refused(front(vec![]), "name", json!("Б".repeat(30))),
            400,
        ),
        ("a cell outside", front(vec![cell(2, 0, &a, 1)]), 400),
        (
            "two screens in a cell",
            front(vec![cell(0, 0, &a, 1), cell(0, 0, &a, 2)]),
            400,
        ),
        (
            "a screen in two cells",
            front(vec![cell(0, 0, &a, 1), cell(0, 1, &a, 1)]),
            400,
        ),
        (
            "a screen the box doesn't have",
            front(vec![cell(0, 0, &a, 7)]),
            404,
        ),
        (
            "not a display box",
            front(vec![cell(0, 0, "fp-nobody", 1)]),
            404,
        ),
        (
            "a channel that isn't",
            refused(
                front(vec![]),
                "feed",
                json!({"kind": "channel", "channel": "nope"}),
            ),
            404,
        ),
    ] {
        let r = call(addr, "POST", "/api/v1/mosaic/walls", None, Some(body)).await;
        assert_eq!(
            r.status,
            status,
            "{what}: {}",
            String::from_utf8_lossy(&r.body)
        );
    }
    assert!(get_json(addr, "/api/v1/mosaic").await["walls"] == json!([]));

    // A 2×2 wall over both boxes: the same geometry, its own row and column, one group, one
    // feed; the settings before the feed.
    let wall = ok(
        &call(
            addr,
            "POST",
            "/api/v1/mosaic/walls",
            None,
            Some(front(all4.clone())),
        )
        .await,
        201,
        "wall",
    );
    let id = wall["id"].as_str().unwrap().to_string();
    assert_eq!(wall["cells"].as_array().unwrap().len(), 4);
    for (f, n, row, col) in [
        (&fa, 1, 0, 0),
        (&fa, 2, 0, 1),
        (&fb, 1, 1, 0),
        (&fb, 2, 1, 1),
    ] {
        assert_eq!(
            wall_of(f, n),
            json!({
                "rows": 2, "cols": 2, "row": row, "col": col,
                "bezel_mm": {"h": 10.0, "v": 10.0}, "screen_mm": {"w": 1210.0, "h": 680.0},
                "group": "wall:Front",
            }),
            "screen {n}'s tile"
        );
        assert_eq!(feed_of(f, n), "BOOTH (Program)");
        let posts = f.lock().unwrap().posts.clone();
        let at = |cmd: &str| posts.iter().position(|(c, v)| c == cmd && v["screen"] == n);
        assert!(at("settings") < at("feed"), "settings before the feed");
        assert!(
            posts_for(f, "settings", n, 0)[0].get("fit").is_none(),
            "only the tile: the box's own settings are left alone"
        );
    }
    let st = get_json(addr, "/api/v1/mosaic").await;
    assert_eq!(st["walls"][0]["name"], "Front");
    let a1 = &st["boxes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["machine"] == a.as_str())
        .unwrap()["screens"][0];
    assert_eq!(a1["wall"], id.as_str());
    assert_eq!(a1["settings"]["tile"]["group"], "wall:Front");
    assert_eq!(a1["feed"]["source"], "BOOTH (Program)");

    // A tile's own feed is the wall's; a screen is in one wall; names are unique.
    assert_eq!(
        call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/boxes/{a}/screens/1/feed"),
            None,
            Some(json!({"kind": "none"}))
        )
        .await
        .status,
        409
    );
    let mut other = front(vec![cell(0, 0, &b, 2)]);
    other["name"] = json!("Side");
    assert_eq!(
        call(addr, "POST", "/api/v1/mosaic/walls", None, Some(other))
            .await
            .status,
        409
    );
    assert_eq!(
        call(
            addr,
            "POST",
            "/api/v1/mosaic/walls",
            None,
            Some(front(vec![]))
        )
        .await
        .status,
        409,
        "the name is taken"
    );

    // Sync: every tile in the wall's group → in step; the delay is the largest.
    for (f, d) in [(&fa, 38.0), (&fb, 41.4)] {
        let mut fk = f.lock().unwrap();
        for n in [1, 2] {
            fk.sync.insert(
                n,
                json!({"group": "wall:Front", "mode": "sync/1", "delay_ms": d, "late": 0}),
            );
        }
    }
    let st = until_state(addr, "the wall in step", |s| {
        s["walls"][0]["synced"] == true && s["walls"][0]["status"] == "showing"
    })
    .await;
    assert_eq!(st["walls"][0]["delayMs"], 41.0, "{st}");
    assert!(st["walls"][0]["problem"].is_null(), "{st}");
    // A tile dropping late pictures: out of step while it does.
    fb.lock().unwrap().late_growing = Some(2);
    let st = until_state(addr, "late pictures", |s| s["walls"][0]["synced"] == false).await;
    assert!(st["walls"][0]["lateRecently"].as_u64().unwrap() > 0, "{st}");
    assert!(
        st["walls"][0]["problem"].as_str().unwrap().contains("late"),
        "{st}"
    );
    fb.lock().unwrap().late_growing = None;
    until_state(addr, "in step again", |s| s["walls"][0]["synced"] == true).await;
    // A tile in another group.
    fb.lock().unwrap().sync.insert(
        1,
        json!({"group": "wall:BOOTH (Program)", "mode": "sync/1", "delay_ms": 40, "late": 0}),
    );
    let st = until_state(addr, "another group", |s| s["walls"][0]["synced"] == false).await;
    let p = st["walls"][0]["problem"].as_str().unwrap();
    assert!(
        p.contains("Row 2, column 1") && p.contains("another sync group"),
        "{p}"
    );
    fb.lock().unwrap().sync.insert(
        1,
        json!({"group": "wall:Front", "mode": "sync/1", "delay_ms": 40, "late": 0}),
    );

    // A box that lost a tile's settings (restarted with none): set again by the poll.
    fa.lock()
        .unwrap()
        .settings
        .insert(2, json!({"fit": "fit", "rotation": 0, "wall": null}));
    until_fake(&fa, "the tile set again", |f| {
        f.settings
            .get(&2)
            .is_some_and(|s| s["wall"]["col"] == 1 && s["wall"]["group"] == "wall:Front")
    })
    .await;

    // The wall, moved: one row of three; FRONT-RIGHT's screen 2 leaves it.
    let (fa_from, fb_from) = (
        fa.lock().unwrap().posts.len(),
        fb.lock().unwrap().posts.len(),
    );
    let mut row = front(vec![
        cell(0, 0, &b, 1),
        cell(0, 1, &a, 1),
        cell(0, 2, &a, 2),
    ]);
    row["rows"] = json!(1);
    row["columns"] = json!(3);
    let st = ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/walls/{id}"),
            None,
            Some(row),
        )
        .await,
        200,
        "moved",
    );
    assert_eq!(st["walls"][0]["cells"].as_array().unwrap().len(), 3);
    assert_eq!(
        (
            wall_of(&fb, 1)["row"].clone(),
            wall_of(&fb, 1)["col"].clone()
        ),
        (json!(0), json!(0))
    );
    assert_eq!(wall_of(&fa, 1)["col"], 1);
    assert_eq!(wall_of(&fa, 1)["cols"], 3);
    assert_eq!(wall_of(&fa, 2)["col"], 2);
    assert_eq!(
        wall_of(&fb, 2),
        Value::Null,
        "the screen that left has no tile"
    );
    assert!(!posts_for(&fb, "settings", 2, fb_from).is_empty());
    assert!(posts_for(&fa, "settings", 1, fa_from).len() == 1);
    assert_eq!(
        feed_of(&fb, 2),
        "BOOTH (Program)",
        "it keeps the picture, whole"
    );
    let right = st["boxes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["machine"] == b.as_str())
        .unwrap()
        .clone();
    assert!(right["screens"][1]["wall"].is_null());
    assert!(right["screens"][1]["settings"].get("tile").is_none());

    // A tile sent with a screen's settings is ignored: tiles come only from walls (H08-1).
    let stray = json!({"rows": 1, "columns": 2, "row": 0, "column": 1, "bezelMm": [0.0, 0.0], "group": "wall:Stray"});
    for (m, n) in [(&a, 1), (&b, 2)] {
        ok(
            &call(
                addr,
                "PUT",
                &format!("/api/v1/mosaic/boxes/{m}/screens/{n}/settings"),
                None,
                Some(json!({"fit": "fill", "tile": stray})),
            )
            .await,
            200,
            "settings with a tile",
        );
    }
    assert_eq!(
        wall_of(&fa, 1)["group"],
        "wall:Front",
        "the wall's tile stays"
    );
    assert_eq!(wall_of(&fa, 1)["cols"], 3);
    assert_eq!(wall_of(&fb, 2), Value::Null, "no tile outside a wall");
    let st = get_json(addr, "/api/v1/mosaic").await;
    let screen_of = |m: &str, n: usize| {
        st["boxes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["machine"] == m)
            .unwrap()["screens"][n]
            .clone()
    };
    assert_eq!(screen_of(&a, 0)["settings"]["tile"]["group"], "wall:Front");
    assert!(screen_of(&b, 1)["settings"].get("tile").is_none());
    let kept: Value =
        serde_json::from_slice(&std::fs::read(fx.dir.join("machine/mosaic.json")).unwrap())
            .unwrap();
    assert!(!kept.to_string().contains("wall:Stray"), "{kept}");

    // R1-5: a tile's screen the box doesn't list now: the rest of the wall can still be changed.
    let gone = {
        let mut fk = fa.lock().unwrap();
        let i = fk.screens.iter().position(|s| s.0 == 2).unwrap();
        fk.screens.remove(i)
    };
    until_state(addr, "screen 2 not listed", |s| {
        s["boxes"].as_array().unwrap().iter().any(|x| {
            x["machine"] == a.as_str()
                && x["screens"][1]["problem"]
                    .as_str()
                    .is_some_and(|p| p.contains("doesn't list"))
        })
    })
    .await;
    let mut row = front(vec![
        cell(0, 0, &b, 1),
        cell(0, 1, &a, 1),
        cell(0, 2, &a, 2),
    ]);
    row["rows"] = json!(1);
    row["columns"] = json!(3);
    row["bezelMm"] = json!([12.0, 10.0]);
    let r = call(
        addr,
        "PUT",
        &format!("/api/v1/mosaic/walls/{id}"),
        None,
        Some(row.clone()),
    )
    .await;
    assert_ne!(r.status, 404, "{}", String::from_utf8_lossy(&r.body));
    fa.lock().unwrap().screens.insert(1, gone);
    until_fake(&fa, "its tile when it is listed again", |f| {
        f.settings
            .get(&2)
            .is_some_and(|s| s["wall"]["bezel_mm"]["h"] == 12.0)
    })
    .await;
    row["bezelMm"] = json!([10.0, 10.0]);
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/walls/{id}"),
            None,
            Some(row),
        )
        .await,
        200,
        "the bezels back",
    );
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/boxes/{b}/screens/2/feed"),
            None,
            Some(json!({"kind": "source", "source": "BOOTH (Stage)"})),
        )
        .await,
        200,
        "its own feed again",
    );

    // The wall follows a channel; the channel moves, every tile with it.
    let ch = ok(
        &call(
            addr,
            "POST",
            "/api/v1/mosaic/channels",
            None,
            Some(json!({"name": "Main", "source": "CAM (Wide)"})),
        )
        .await,
        201,
        "channel",
    );
    let ch = ch["id"].as_str().unwrap().to_string();
    let st = ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/walls/{id}/feed"),
            None,
            Some(json!({"kind": "channel", "channel": ch})),
        )
        .await,
        200,
        "the wall's feed",
    );
    assert_eq!(st["walls"][0]["feed"]["channel"], ch.as_str());
    assert_eq!(st["channels"][0]["screens"], 3);
    for (f, n) in [(&fb, 1), (&fa, 1), (&fa, 2)] {
        assert_eq!(feed_of(f, n), "CAM (Wide)");
    }
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/channels/{ch}"),
            None,
            Some(json!({"source": "CAM (Close)"})),
        )
        .await,
        200,
        "re-point",
    );
    for (f, n) in [(&fb, 1), (&fa, 1), (&fa, 2)] {
        assert_eq!(feed_of(f, n), "CAM (Close)");
    }
    assert_eq!(feed_of(&fb, 2), "BOOTH (Stage)", "not in the wall");
    assert_eq!(
        call(
            addr,
            "DELETE",
            &format!("/api/v1/mosaic/channels/{ch}"),
            None,
            None
        )
        .await
        .status,
        409,
        "a wall follows it"
    );

    // Identify: every tile's screen.
    let from = fa.lock().unwrap().posts.len();
    ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/mosaic/walls/{id}/identify"),
            None,
            Some(json!({"seconds": 4})),
        )
        .await,
        204,
        "identify the wall",
    );
    assert_eq!(
        posts_for(&fa, "identify", 1, from),
        vec![json!({"screen": 1, "seconds": 4.0})]
    );
    assert_eq!(posts_for(&fa, "identify", 2, from).len(), 1);
    assert_eq!(posts_for(&fb, "identify", 1, 0).len(), 1);

    // Kept with the rest; audited.
    let kept: Value =
        serde_json::from_slice(&std::fs::read(fx.dir.join("machine/mosaic.json")).unwrap())
            .unwrap();
    assert_eq!(kept["walls"][0]["name"], "Front");
    assert_eq!(kept["walls"][0]["columns"], 3);
    let audit = get_json(addr, "/api/v1/site/audit?n=200").await;
    assert!(audit_has(&audit, "mosaic.wall.create", |_| true), "{audit}");

    // Deleted while FRONT-RIGHT is away: the tiles go, the screens keep showing the feed, each
    // whole; FRONT-RIGHT's when it answers again (X3-1).
    fb.lock().unwrap().down = true;
    until_state(addr, "FRONT-RIGHT away", |s| {
        s["boxes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["machine"] == b.as_str() && x["reachable"] == false)
    })
    .await;
    let r = call(
        addr,
        "DELETE",
        &format!("/api/v1/mosaic/walls/{id}"),
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 503, "{}", String::from_utf8_lossy(&r.body));
    for n in [1, 2] {
        assert_eq!(wall_of(&fa, n), Value::Null);
    }
    assert_eq!(wall_of(&fb, 1)["cols"], 3, "away: not taken yet");
    fb.lock().unwrap().down = false;
    until_fake(&fb, "the tile goes when it answers", |f| {
        f.settings.get(&1).is_some_and(|s| s["wall"].is_null())
    })
    .await;
    for (f, n) in [(&fb, 1), (&fa, 1), (&fa, 2)] {
        assert_eq!(wall_of(f, n), Value::Null);
        assert_eq!(feed_of(f, n), "CAM (Close)");
    }
    let st = get_json(addr, "/api/v1/mosaic").await;
    assert_eq!(st["walls"], json!([]));
    assert_eq!(
        st["channels"][0]["screens"], 3,
        "the screens follow the channel still"
    );
    assert_eq!(
        call(
            addr,
            "DELETE",
            &format!("/api/v1/mosaic/walls/{id}"),
            None,
            None
        )
        .await
        .status,
        404
    );
    for (_, stop, _) in fakes {
        let _ = stop.send(());
    }
}

/// The IDN Nodes Mosaic works with, faked (batch 15, M3): this server's own Node is
/// `fp-site-node`, publishing as `SITE`; every Node setting applied through the Site Manager
/// and every file sent is recorded (with `replace`); `refuse` makes the Site Manager refuse
/// settings, `old_boxes` the boxes refuse replacements (IDN before Beta14); a transfer offered is
/// `sent` unless `transfer_state` says otherwise.
#[derive(Debug, Default)]
struct FakeNodes {
    applied: Mutex<Vec<(String, Value)>>,
    sent: Mutex<Vec<(std::path::PathBuf, String)>>,
    replaced: Mutex<Vec<bool>>,
    refuse: std::sync::atomic::AtomicBool,
    old_boxes: std::sync::atomic::AtomicBool,
    transfer_state: Mutex<Option<String>>,
    media: std::path::PathBuf,
    /// Each machine's Node report, as the Site Manager would give it.
    reports: Mutex<HashMap<String, idn_open_protocol::xml::Element>>,
}

impl vesper_server::mosaic::Nodes for FakeNodes {
    fn own(&self) -> Option<String> {
        Some("fp-site-node".into())
    }
    fn source_name(&self, name: &str) -> String {
        format!("SITE ({name})")
    }
    fn apply(
        &self,
        _site: &vesper_server::site::Site,
        machine: &str,
        settings: serde_json::Map<String, Value>,
    ) -> Result<(), String> {
        if self.refuse.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("the machine isn't online".into());
        }
        self.applied
            .lock()
            .unwrap()
            .push((machine.to_string(), Value::Object(settings)));
        Ok(())
    }
    fn report(
        &self,
        _site: &vesper_server::site::Site,
        machine: &str,
    ) -> Option<idn_open_protocol::xml::Element> {
        self.reports.lock().unwrap().get(machine).cloned()
    }
    fn media_dir(&self) -> Result<std::path::PathBuf, String> {
        std::fs::create_dir_all(&self.media).map_err(|e| e.to_string())?;
        Ok(self.media.clone())
    }
    fn send_media(
        &self,
        file: &std::path::Path,
        to: &str,
        replace: bool,
    ) -> Result<Option<String>, String> {
        if !file.is_file() {
            return Err(format!("no file {}", file.display()));
        }
        if replace && self.old_boxes.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("FOYER: refused".into());
        }
        let mut sent = self.sent.lock().unwrap();
        sent.push((file.to_path_buf(), to.to_string()));
        self.replaced.lock().unwrap().push(replace);
        Ok(Some(format!(
            "media{}:{}-{}",
            if replace { "-replace" } else { "" },
            sent.len(),
            file.file_name().unwrap().to_string_lossy()
        )))
    }
    fn transfer(&self, offer: &str, _to: &str) -> Option<vesper_server::mosaic::Transfer> {
        assert!(offer.starts_with("media-replace:"), "{offer}");
        Some(vesper_server::mosaic::Transfer::new(
            self.transfer_state
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "sent".into()),
            0,
        ))
    }
}

/// A site's server with `nodes`, and a machine joined as a display box at a fake service:
/// (server, box path, the fake, the machine's fingerprint).
async fn with_a_box(
    fx: &Fixture,
    nodes: Arc<FakeNodes>,
    name: &str,
) -> (RunningServer, String, FakeState, String) {
    let server = start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: true,
        mdns: false,
        site: SiteConfig::loopback(),
        mosaic_poll: Duration::from_millis(200),
        mosaic_nodes: Some(nodes),
        mosaic_logo_settle: Duration::ZERO,
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts");
    let addr = server.addr;
    until_state(addr, "managed", |s| s["managed"] == true).await;
    let ds = get_json(addr, "/api/v1/site").await["discoveryPort"]
        .as_u64()
        .unwrap() as u16;
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    let m = machine(fx, ds, name);
    m.join_with_code(code["code"].as_str().unwrap()).unwrap();
    until(&m, "joins", |s| matches!(s, TrustState::Enrolled { .. })).await;
    let fp = m.fingerprint();
    // The machine stays in the site (online) for the whole test.
    std::mem::forget(m);
    let (fake, fake_addr, stop) = fake_service().await;
    // The fake stays up for the whole test.
    std::mem::forget(stop);
    let box_path = format!("/api/v1/mosaic/boxes/{fp}");
    ok(
        &call(
            addr,
            "PUT",
            &box_path,
            None,
            Some(json!({"address": format!("http://{fake_addr}")})),
        )
        .await,
        200,
        "add the box",
    );
    until_state(addr, "its screens read", |s| {
        s["boxes"][0]["reachable"] == true
    })
    .await;
    (server, box_path, fake, fp)
}

/// M3-3: a channel is an IDN router on the site's server's Node: screens following it show
/// the router, a new source is one `routers.route` (no screen touched), a rename re-points
/// them; channels made while the Node refused are moved onto routers once it takes them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn channels_are_routers_on_the_site_servers_node() {
    let fx = fixture("site-mosaic-routers");
    let nodes = Arc::new(FakeNodes {
        media: fx.dir.join("node-media"),
        ..Default::default()
    });
    nodes
        .refuse
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (server, box_path, fake, _fp) = with_a_box(&fx, nodes.clone(), "LOBBY").await;
    let addr = server.addr;

    // The Node refuses: the channel is resolved per screen, as before routers.
    let ch = ok(
        &call(
            addr,
            "POST",
            "/api/v1/mosaic/channels",
            None,
            Some(json!({"name": "Main", "source": "BOOTH (Stage)"})),
        )
        .await,
        201,
        "channel",
    );
    assert!(ch.get("router").is_none(), "{ch}");
    let id = ch["id"].as_str().unwrap().to_string();
    ok(
        &call(
            addr,
            "PUT",
            &format!("{box_path}/screens/2/feed"),
            None,
            Some(json!({"kind": "channel", "channel": id})),
        )
        .await,
        200,
        "follow",
    );
    assert_eq!(feed_of(&fake, 2), "BOOTH (Stage)");
    let st = get_json(addr, "/api/v1/mosaic").await;
    assert!(st["routingProblem"].is_string(), "{st}");

    // It takes them now; a second channel (the list changed) sends them at once, and the
    // screen moves onto the router by itself (migration).
    nodes
        .refuse
        .store(false, std::sync::atomic::Ordering::SeqCst);
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/mosaic/channels",
            None,
            Some(json!({"name": "Side"})),
        )
        .await,
        201,
        "second channel",
    );
    let list = nodes.applied.lock().unwrap().last().cloned().unwrap();
    assert_eq!(list.0, "fp-site-node");
    assert_eq!(
        list.1,
        json!({"routers.list": "Channel Main=BOOTH (Stage); Channel Side="})
    );
    until_fake(&fake, "the screen follows the router", |f| {
        f.screens[1].2 == "SITE (Channel Main)"
    })
    .await;
    let st = until_state(addr, "routed", |s| s["routingProblem"].is_null()).await;
    let main = st["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Main")
        .unwrap();
    assert_eq!(main["router"], "SITE (Channel Main)");

    // A new source: one change on the router, no screen touched.
    let feeds_before = posts_for(&fake, "feed", 2, 0).len();
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/channels/{id}"),
            None,
            Some(json!({"source": "BOOTH (Lobby Loop)"})),
        )
        .await,
        200,
        "re-point",
    );
    assert_eq!(
        nodes.applied.lock().unwrap().last().cloned().unwrap(),
        (
            "fp-site-node".to_string(),
            json!({"routers.route": "Channel Main=BOOTH (Lobby Loop)"})
        )
    );
    assert_eq!(feed_of(&fake, 2), "SITE (Channel Main)");
    assert_eq!(posts_for(&fake, "feed", 2, 0).len(), feeds_before);
    let st = get_json(addr, "/api/v1/mosaic").await;
    assert!(
        st["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["source"] == "BOOTH (Lobby Loop)")
    );
    // Nothing sent again on the next rounds (the list as the Node has it).
    let n = nodes.applied.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(nodes.applied.lock().unwrap().len(), n);

    // Renamed: the router's new name, the screen re-pointed.
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/channels/{id}"),
            None,
            Some(json!({"name": "Front"})),
        )
        .await,
        200,
        "rename",
    );
    assert_eq!(
        nodes.applied.lock().unwrap().last().cloned().unwrap().1,
        json!({"routers.list": "Channel Front=BOOTH (Lobby Loop); Channel Side="})
    );
    assert_eq!(feed_of(&fake, 2), "SITE (Channel Front)");

    // H01-3 (bug run 6): the Node refuses a new source (it went away): the screens get the
    // source themselves, as the routing problem says, and go back onto the router once the
    // Node takes the list (with the new source) again.
    nodes
        .refuse
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let r = call(
        addr,
        "PUT",
        &format!("/api/v1/mosaic/channels/{id}"),
        None,
        Some(json!({"source": "BOOTH (Side Cam)"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(feed_of(&fake, 2), "BOOTH (Side Cam)");
    let st = get_json(addr, "/api/v1/mosaic").await;
    assert!(st["routingProblem"].is_string(), "{st}");
    nodes
        .refuse
        .store(false, std::sync::atomic::Ordering::SeqCst);
    // (A change of the channels sends the list at once, without the retry's wait.)
    let side = st["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Side")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/channels/{side}"),
            None,
            Some(json!({"name": "Aside"})),
        )
        .await,
        200,
        "rename the other channel",
    );
    until_fake(&fake, "back on the router", |f| {
        f.screens[1].2 == "SITE (Channel Front)"
    })
    .await;
    assert_eq!(
        nodes.applied.lock().unwrap().last().cloned().unwrap().1,
        json!({"routers.list": "Channel Aside=; Channel Front=BOOTH (Side Cam)"})
    );
    assert!(get_json(addr, "/api/v1/mosaic").await["routingProblem"].is_null());

    // H01-4: a source with `;` (the list's separator) can't go into the setting: the channel
    // leaves its router and its screens get the source themselves.
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/channels/{id}"),
            None,
            Some(json!({"source": "BOOTH (Stage; Wide)"})),
        )
        .await,
        200,
        "a source with a semicolon",
    );
    assert_eq!(
        nodes.applied.lock().unwrap().last().cloned().unwrap().1,
        json!({"routers.list": "Channel Aside="})
    );
    assert_eq!(feed_of(&fake, 2), "BOOTH (Stage; Wide)");
    let st = get_json(addr, "/api/v1/mosaic").await;
    let front = st["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Front")
        .unwrap();
    assert!(front.get("router").is_none(), "{front}");
}

fn png(w: u32, h: u32, shade: u8) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([shade, 40, 90, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A raw `PUT` (an image body): its status.
async fn put_bytes(addr: SocketAddr, path: &str, body: Vec<u8>) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "PUT {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(&body).await.unwrap();
    let mut answer = Vec::new();
    s.read_to_end(&mut answer).await.unwrap();
    let text = String::from_utf8_lossy(&answer);
    text.split_whitespace().nth(1).unwrap().parse().unwrap()
}

/// M3-4: the site's logo (PNG or JPEG, kept as PNG) goes to each box's Node with `media_send`
/// (from this machine's media folder; on a Beta14 box as `logo-a.png` / `logo-b.png` in turns,
/// so a screen showing the logo gets a new path and reads it at once), again when it changes. M3-5: "Use as a
/// display box" switches the machine's display service through its Node's settings.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_logo_is_pushed_and_the_display_service_switched() {
    let fx = fixture("site-mosaic-logo");
    let nodes = Arc::new(FakeNodes {
        media: fx.dir.join("node-media"),
        ..Default::default()
    });
    let (server, box_path, fake, fp) = with_a_box(&fx, nodes.clone(), "FOYER").await;
    let addr = server.addr;

    assert_eq!(
        put_bytes(addr, "/api/v1/mosaic/logo", b"not a picture".to_vec()).await,
        400
    );
    assert_eq!(
        put_bytes(addr, "/api/v1/mosaic/logo", png(64, 32, 200)).await,
        200
    );
    // Sent as `logo-a.png`, replacing the box's (IDN Beta14), followed to `sent`; then every
    // screen of the box is given it as its fallback logo.
    let st = until_state(addr, "the logo on the box", |s| {
        s["boxes"][0]["logoState"] == "onBox"
    })
    .await;
    assert_eq!(st["logo"]["width"], 64, "{st}");
    let sent = nodes.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, fp);
    assert_eq!(*nodes.replaced.lock().unwrap(), vec![true]);
    assert_eq!(sent[0].0, fx.dir.join("node-media").join("logo-a.png"));
    let kept = call(addr, "GET", "/api/v1/mosaic/logo", None, None).await;
    assert_eq!(kept.status, 200);
    assert_eq!(std::fs::read(&sent[0].0).unwrap(), kept.body);
    for n in [1, 2] {
        assert_eq!(
            fake.lock().unwrap().settings[&n]["fallback"]["logo"],
            "logo-a.png",
            "screen {n}"
        );
    }
    // Not sent again while it is the same.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(nodes.sent.lock().unwrap().len(), 1);

    // A screen's settings changed from Vesper keep the logo (its fallback goes whole).
    ok(
        &call(
            addr,
            "PUT",
            &format!("{box_path}/screens/1/settings"),
            None,
            Some(json!({"fallback": "logo", "fallbackAfterSeconds": 9})),
        )
        .await,
        200,
        "settings",
    );
    let (_, body) = fake.lock().unwrap().posts.last().cloned().unwrap();
    assert_eq!(
        body["fallback"],
        json!({"show": "logo", "after_s": 9, "logo": "logo-a.png"})
    );

    // A new one, as a JPEG: kept as PNG, sent under the name the screens don't show
    // (`logo-b.png`, replacing what the box had under it); while the box hasn't fetched it, it
    // is "sending" and the screens keep `logo-a.png`; a failed transfer is sent again.
    *nodes.transfer_state.lock().unwrap() = Some("offered".into());
    let mut jpeg = Vec::new();
    image::RgbImage::from_pixel(40, 40, image::Rgb([10, 200, 10]))
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    assert_eq!(
        put_bytes(addr, "/api/v1/mosaic/logo", jpeg.clone()).await,
        200
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while nodes.sent.lock().unwrap().len() < 2 {
        assert!(Instant::now() < deadline, "the new logo wasn't sent");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let kept = call(addr, "GET", "/api/v1/mosaic/logo", None, None).await;
    assert_eq!(&kept.body[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        nodes.sent.lock().unwrap()[1].0,
        fx.dir.join("node-media").join("logo-b.png")
    );
    assert_eq!(
        std::fs::read(fx.dir.join("node-media").join("logo-b.png")).unwrap(),
        kept.body
    );
    until_state(addr, "the new logo sending", |s| {
        s["logo"]["width"] == 40 && s["boxes"][0]["logoState"] == "sending"
    })
    .await;
    for n in [1, 2] {
        assert_eq!(
            fake.lock().unwrap().settings[&n]["fallback"]["logo"],
            "logo-a.png",
            "screen {n} keeps the old logo until the new one is there"
        );
    }
    *nodes.transfer_state.lock().unwrap() = Some("failed: the stream stopped".into());
    until_state(addr, "the failed transfer shown", |s| {
        s["boxes"][0]["logoState"] == "failed"
    })
    .await;
    // H03-1 / X3-1 (bug run 7): until the new logo arrives, the screens keep the name that
    // arrived last (a few polls after the failure: nothing points them at logo-b.png, which the
    // box doesn't have; once it arrives, the new path makes them read it).
    tokio::time::sleep(Duration::from_millis(1200)).await;
    for n in [1, 2] {
        assert_eq!(
            fake.lock().unwrap().settings[&n]["fallback"]["logo"],
            "logo-a.png",
            "screen {n} keeps the old logo after a failed replacement"
        );
    }
    // (Tried again a minute later by itself; at once when the logo is put again.)
    *nodes.transfer_state.lock().unwrap() = None;
    assert_eq!(put_bytes(addr, "/api/v1/mosaic/logo", jpeg).await, 200);
    let deadline = Instant::now() + Duration::from_secs(30);
    while nodes.sent.lock().unwrap().len() < 3 {
        assert!(Instant::now() < deadline, "the logo wasn't sent again");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    until_state(addr, "the new logo on the box", |s| {
        s["logo"]["width"] == 40 && s["boxes"][0]["logoState"] == "onBox"
    })
    .await;
    assert_eq!(
        nodes.sent.lock().unwrap()[2].0,
        fx.dir.join("node-media").join("logo-b.png"),
        "sent again under the same name: the screens still show logo-a.png"
    );
    for n in [1, 2] {
        assert_eq!(
            fake.lock().unwrap().settings[&n]["fallback"]["logo"],
            "logo-b.png",
            "screen {n}: a new path, read at once"
        );
    }
    // And back: the next one goes as `logo-a.png` again.
    assert_eq!(
        put_bytes(addr, "/api/v1/mosaic/logo", png(56, 28, 30)).await,
        200
    );
    until_state(addr, "the third logo on the box", |s| {
        s["logo"]["width"] == 56 && s["boxes"][0]["logoState"] == "onBox"
    })
    .await;
    assert_eq!(nodes.sent.lock().unwrap().len(), 4);
    assert_eq!(
        nodes.sent.lock().unwrap()[3].0,
        fx.dir.join("node-media").join("logo-a.png")
    );
    for n in [1, 2] {
        assert_eq!(
            fake.lock().unwrap().settings[&n]["fallback"]["logo"],
            "logo-a.png",
            "screen {n}"
        );
    }
    assert!(nodes.replaced.lock().unwrap().iter().all(|r| *r));

    // A box from before Beta14 refuses a replacement: the logo goes under a name of its own
    // (its Node never overwrites a file) and its screens are given that name.
    nodes
        .old_boxes
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        put_bytes(addr, "/api/v1/mosaic/logo", png(48, 48, 90)).await,
        200
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while nodes.sent.lock().unwrap().len() < 5 {
        assert!(
            Instant::now() < deadline,
            "the logo wasn't sent to the old box"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let own = nodes.sent.lock().unwrap()[4]
        .0
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        own.starts_with("logo-") && own.ends_with(".png") && own.len() == 25,
        "{own}"
    );
    assert!(!nodes.replaced.lock().unwrap()[4]);
    until_fake(&fake, "the screens moved to the old box's logo name", |f| {
        [1, 2]
            .iter()
            .all(|n| f.settings[n]["fallback"]["logo"] == own.as_str())
    })
    .await;
    until_state(addr, "the logo on the old box", |s| {
        s["logo"]["width"] == 48 && s["boxes"][0]["logoState"] == "onBox"
    })
    .await;
    ok(
        &call(addr, "DELETE", "/api/v1/mosaic/logo", None, None).await,
        204,
        "no logo",
    );
    assert_eq!(
        call(addr, "GET", "/api/v1/mosaic/logo", None, None)
            .await
            .status,
        404
    );

    // The display service: headless, then off; a Node that doesn't take it is a 503.
    let service = format!("{box_path}/service");
    let st = ok(
        &call(
            addr,
            "PUT",
            &service,
            None,
            Some(json!({"mode": "headless"})),
        )
        .await,
        200,
        "headless",
    );
    assert_eq!(st["boxes"][0]["serviceMode"], "headless", "{st}");
    assert_eq!(
        nodes.applied.lock().unwrap().last().cloned().unwrap(),
        (
            fp.clone(),
            json!({"apps.display": "on", "apps.display_boot": "on"})
        )
    );
    ok(
        &call(
            addr,
            "PUT",
            &service,
            None,
            Some(json!({"mode": "desktop"})),
        )
        .await,
        200,
        "desktop",
    );
    assert_eq!(
        nodes.applied.lock().unwrap().last().cloned().unwrap().1,
        json!({"apps.display": "on", "apps.display_boot": "off"})
    );
    nodes
        .refuse
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        call(addr, "PUT", &service, None, Some(json!({"mode": "off"})))
            .await
            .status,
        503
    );
    let st = get_json(addr, "/api/v1/mosaic").await;
    assert_eq!(st["boxes"][0]["serviceMode"], "desktop", "kept as it was");
    assert!(st["boxes"][0].get("serviceModeReported").is_none());
    // The box's Node reports its mode (`<apps display display_boot>`): that is shown.
    nodes.reports.lock().unwrap().insert(
        fp.clone(),
        idn_open_protocol::xml::Element::new("idn_node_report").with_child(
            idn_open_protocol::xml::Element::new("apps")
                .with_attr("display", "true")
                .with_attr("display_boot", "true"),
        ),
    );
    until_state(addr, "the mode as the box has it", |s| {
        s["boxes"][0]["serviceMode"] == "headless" && s["boxes"][0]["serviceModeReported"] == true
    })
    .await;
}

/// The IDN maintainers (2026-11-26): sending `routers.list` puts every router back to the sources in
/// it. A channel's source changed (a route), then Vesper restarted: the list it sends again
/// carries the new source; and when the Node's report already shows that list, none is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn routers_keep_the_current_source_across_a_restart() {
    let fx = fixture("site-mosaic-routers-restart");
    let nodes = Arc::new(FakeNodes {
        media: fx.dir.join("node-media"),
        ..Default::default()
    });
    let (server, _box_path, _fake, _fp) = with_a_box(&fx, nodes.clone(), "LOBBY").await;
    let addr = server.addr;
    let ch = ok(
        &call(
            addr,
            "POST",
            "/api/v1/mosaic/channels",
            None,
            Some(json!({"name": "Main", "source": "BOOTH (Stage)"})),
        )
        .await,
        201,
        "channel",
    );
    let id = ch["id"].as_str().unwrap().to_string();
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/mosaic/channels/{id}"),
            None,
            Some(json!({"source": "BOOTH (Lobby Loop)"})),
        )
        .await,
        200,
        "re-point",
    );
    assert_eq!(
        nodes.applied.lock().unwrap().last().cloned().unwrap().1,
        json!({"routers.route": "Channel Main=BOOTH (Lobby Loop)"})
    );
    server.shutdown().await.unwrap();

    // Started again: the list carries the source the route set, not the one it was made with.
    let dir = fx.dir.clone();
    let restart = || {
        start(ServerConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            trust_localhost: true,
            mdns: false,
            site: SiteConfig::loopback(),
            mosaic_poll: Duration::from_millis(200),
            mosaic_nodes: Some(nodes.clone()),
            ..ServerConfig::new(&dir)
        })
    };
    let n = nodes.applied.lock().unwrap().len();
    let server = restart().await.expect("server starts");
    let deadline = Instant::now() + Duration::from_secs(30);
    while nodes.applied.lock().unwrap().len() == n {
        assert!(Instant::now() < deadline, "no list sent at the start");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        nodes.applied.lock().unwrap().last().cloned().unwrap(),
        (
            "fp-site-node".to_string(),
            json!({"routers.list": "Channel Main=BOOTH (Lobby Loop)"})
        )
    );
    server.shutdown().await.unwrap();

    // The Node's report shows that list already: nothing is sent at the start.
    nodes.reports.lock().unwrap().insert(
        "fp-site-node".into(),
        idn_open_protocol::xml::Element::new("idn_node_report").with_child(
            idn_open_protocol::xml::Element::new("routers")
                .with_attr("list", "Channel Main=BOOTH (Lobby Loop)"),
        ),
    );
    let n = nodes.applied.lock().unwrap().len();
    let server = restart().await.expect("server starts");
    let st = until_state(server.addr, "routed", |s| {
        s["channels"][0]["router"] == "SITE (Channel Main)"
    })
    .await;
    assert!(st["routingProblem"].is_null());
    assert_eq!(nodes.applied.lock().unwrap().len(), n, "nothing sent again");
    server.shutdown().await.unwrap();
}
