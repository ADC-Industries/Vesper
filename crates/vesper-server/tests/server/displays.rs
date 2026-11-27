//! Display outputs with simulated displays: the display list (API and state), binding a screen
//! to a display with a mode, validation, persistence, frames reaching the display, hot-plug,
//! restarting with the bindings in place, and a Fill/Key pair (the fill and key pixels of a
//! half-transparent picture). Real monitors are the owner's hardware checklist
//! (`docs/DISPLAY-OUTPUTS-PLAN.md`).

use crate::common::*;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vesper_server::displays::FakeDisplays;
use vesper_server::{RunningServer, ServerConfig, start};

async fn start_with(fx: &Fixture, displays: Arc<FakeDisplays>) -> RunningServer {
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
        site: vesper_server::site::SiteConfig::off(),
        displays: Some(displays),
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts")
}

async fn state(addr: std::net::SocketAddr) -> Value {
    get_json(addr, "/api/v1/state").await["state"].clone()
}

/// Polls the state until `pred` holds (10 s).
async fn wait_state(
    addr: std::net::SocketAddr,
    what: &str,
    pred: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = state(addr).await;
        if pred(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "{what}: {s:#}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn output<'a>(s: &'a Value, screen: &str, kind: &str) -> Option<&'a Value> {
    s["screens"]
        .as_array()?
        .iter()
        .find(|x| x["id"] == screen)?["outputs"]
        .as_array()?
        .iter()
        .find(|o| o["kind"] == kind)
}

fn display<'a>(s: &'a Value, id: &str) -> &'a Value {
    s["displays"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id)
        .unwrap_or(&Value::Null)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn screens_go_to_displays_and_follow_hot_plug() {
    let fx = fixture("displays");
    let fake = Arc::new(FakeDisplays::default());
    fake.set(vec![
        FakeDisplays::display("HDMI-A-1", "Test Panel", true),
        FakeDisplays::display("DP-1", "", false),
    ]);
    let server = start_with(&fx, fake.clone()).await;
    let addr = server.addr;
    let (aud, key) = (fx.audience.id.0.to_string(), fx.key.id.0.to_string());

    // The list: connector, model, modes (largest first), preferred mode.
    let list = get_json(addr, "/api/v1/displays").await;
    assert_eq!(list.as_array().unwrap().len(), 2, "{list}");
    assert_eq!(list[0]["id"], "HDMI-A-1");
    assert_eq!(list[0]["name"], "Test Panel");
    assert_eq!(list[0]["connected"], true);
    assert_eq!(list[0]["modes"].as_array().unwrap().len(), 3);
    assert_eq!(
        list[0]["preferred"],
        json!({"width": 1920, "height": 1080, "refreshMhz": 60000})
    );
    assert_eq!(list[1]["name"], "DP-1", "no EDID: the connector");
    assert_eq!(list[0]["source"], "fake", "where the display comes from");
    assert!(list[0].get("primary").is_none() && list[0].get("position").is_none());
    assert_eq!(list[1]["connected"], false);
    let t = get_json(addr, "/api/v1/output-transports").await;
    let d = t
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "display")
        .unwrap();
    assert_eq!(
        (d["supported"].clone(), d["available"].clone()),
        (json!(true), json!(true))
    );
    let server_info = get_json(addr, "/api/v1/server").await;
    assert!(
        server_info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("displays")),
        "{server_info}"
    );

    // Bind the audience screen to HDMI-A-1 at 720p60.
    let r = http(
        addr,
        "POST",
        "/api/v1/outputs",
        &[],
        Some(json!({
            "screen": aud,
            "transport": {"type": "display", "selector": {"connector": "HDMI-A-1", "name": "Test Panel"},
                          "mode": {"width": 1280, "height": 720, "refreshMhz": 60000}}
        })),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let hdmi_out = r.json()["id"].as_str().unwrap().to_string();
    let stored: Value = serde_json::from_slice(
        &std::fs::read(
            fx.dir
                .join("machine")
                .join("outputBinding")
                .join(format!("{hdmi_out}.json")),
        )
        .expect("stored as a machine document"),
    )
    .unwrap();
    assert_eq!(stored["transport"]["selector"]["connector"], "HDMI-A-1");
    assert_eq!(stored["transport"]["mode"]["refreshMhz"], 60000);
    // The same display twice is refused; a broken mode too.
    for (body, needle) in [
        (
            json!({"screen": key, "transport": {"type": "display", "selector": {"connector": "hdmi-a-1"}}}),
            "already uses display",
        ),
        // The same connector with its card (an ID of a box with several cards) is the same
        // display as a bare one saved before (R2-3).
        (
            json!({"screen": key, "transport": {"type": "display", "selector": {"connector": "card1-HDMI-A-1"}}}),
            "already uses display",
        ),
        (
            json!({"screen": key, "transport": {"type": "display", "selector": {"connector": "DP-1"},
                   "mode": {"width": 0, "height": 720, "refreshMhz": 60000}}}),
            "invalid display mode",
        ),
    ] {
        let r = http(addr, "POST", "/api/v1/outputs", &[], Some(body)).await;
        assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
        assert!(
            String::from_utf8_lossy(&r.body).contains(needle),
            "{}",
            String::from_utf8_lossy(&r.body)
        );
    }

    // Frames reach the display in the chosen mode; the state says which output uses it.
    let deadline = Instant::now() + Duration::from_secs(10);
    while fake.shown("HDMI-A-1").frames < 3 {
        assert!(Instant::now() < deadline, "no frames on HDMI-A-1");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let shown = fake.shown("HDMI-A-1");
    assert_eq!(
        shown.mode.map(|m| (m.width, m.height, m.refresh_mhz)),
        Some((1280, 720, 60000))
    );
    assert_eq!(
        shown.fullscreen,
        Some(true),
        "full screen unless asked otherwise"
    );
    let size = (
        fx.audience.size.width as u32,
        fx.audience.size.height as u32,
    );
    assert_eq!(
        shown.last_size, size,
        "the screen's frames (the presenter scales them to the mode)"
    );
    let s = wait_state(addr, "display output ok", |s| {
        output(s, &aud, "display").is_some_and(|o| o["status"] == "ok")
    })
    .await;
    assert_eq!(
        output(&s, &aud, "display").unwrap()["name"],
        "Display: HDMI-A-1"
    );
    assert_eq!(display(&s, "HDMI-A-1")["boundTo"], json!([hdmi_out]));

    // A rehearsal window instead of full screen: the output is reopened as a window, and the
    // flag is stored (`fullscreen: false`; true is the default and is left out).
    let mut doc = r.json();
    doc["transport"]["fullscreen"] = json!(false);
    let opened = fake.shown("HDMI-A-1").opened;
    let put = http(
        addr,
        "PUT",
        &format!("/api/v1/outputs/{hdmi_out}"),
        &[],
        Some(doc.clone()),
    )
    .await;
    assert_eq!(put.status, 200, "{}", String::from_utf8_lossy(&put.body));
    let deadline = Instant::now() + Duration::from_secs(10);
    while fake.shown("HDMI-A-1").opened == opened {
        assert!(Instant::now() < deadline, "not reopened as a window");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(fake.shown("HDMI-A-1").fullscreen, Some(false));
    doc["transport"]
        .as_object_mut()
        .unwrap()
        .remove("fullscreen");
    let put = http(
        addr,
        "PUT",
        &format!("/api/v1/outputs/{hdmi_out}"),
        &[],
        Some(doc),
    )
    .await;
    assert_eq!(put.status, 200, "{}", String::from_utf8_lossy(&put.body));
    let deadline = Instant::now() + Duration::from_secs(10);
    while fake.shown("HDMI-A-1").fullscreen != Some(true) {
        assert!(Instant::now() < deadline, "not back to full screen");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // A display that is not plugged in: configured, unavailable.
    let r = http(
        addr,
        "POST",
        "/api/v1/outputs",
        &[],
        Some(json!({"screen": key, "transport": {"type": "display", "selector": {"connector": "DP-1"}}})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let s = wait_state(addr, "DP-1 unavailable", |s| {
        output(s, &key, "display").is_some_and(|o| o["status"] == "unavailable")
    })
    .await;
    let detail = output(&s, &key, "display").unwrap()["detail"].clone();
    assert!(
        detail.as_str().unwrap().contains("not connected"),
        "{detail}"
    );

    // Hot-plug: HDMI out, DP in. The state follows, DP starts, HDMI stops.
    fake.set(vec![
        FakeDisplays::display("HDMI-A-1", "Test Panel", false),
        FakeDisplays::display("DP-1", "Other Panel", true),
    ]);
    let s = wait_state(addr, "hot-plug", |s| {
        display(s, "DP-1")["connected"] == true
            && output(s, &key, "display").is_some_and(|o| o["status"] == "ok")
            && output(s, &aud, "display").is_some_and(|o| o["status"] == "unavailable")
    })
    .await;
    assert_eq!(display(&s, "HDMI-A-1")["connected"], false);
    let deadline = Instant::now() + Duration::from_secs(10);
    while fake.shown("DP-1").frames < 3 {
        assert!(Instant::now() < deadline, "no frames on DP-1");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Plugged back in: the HDMI output restarts by itself.
    let opened = fake.shown("HDMI-A-1").opened;
    fake.set(vec![
        FakeDisplays::display("HDMI-A-1", "Test Panel", true),
        FakeDisplays::display("DP-1", "Other Panel", true),
    ]);
    wait_state(addr, "replugged", |s| {
        output(s, &aud, "display").is_some_and(|o| o["status"] == "ok")
    })
    .await;
    assert_eq!(fake.shown("HDMI-A-1").opened, opened + 1);
    server.shutdown().await.unwrap();

    // The bindings are machine documents: a restart drives the same displays.
    let fake2 = Arc::new(FakeDisplays::default());
    fake2.set(vec![
        FakeDisplays::display("HDMI-A-1", "Test Panel", true),
        FakeDisplays::display("DP-1", "Other Panel", true),
    ]);
    let server = start_with(&fx, fake2.clone()).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while fake2.shown("HDMI-A-1").frames < 1 || fake2.shown("DP-1").frames < 1 {
        assert!(Instant::now() < deadline, "no frames after the restart");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        fake2.shown("HDMI-A-1").mode.map(|m| m.width),
        Some(1280),
        "the stored mode"
    );
    server.shutdown().await.unwrap();
}

/// Waits until `pred` holds for what `connector` shows (10 s).
async fn wait_shown(
    fake: &FakeDisplays,
    connector: &str,
    what: &str,
    pred: impl Fn(&vesper_server::displays::FakeShown) -> bool,
) -> vesper_server::displays::FakeShown {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = fake.shown(connector);
        if pred(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "{what}: {s:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fill_and_key_outputs_split_a_half_transparent_picture() {
    let fx = fixture("displays-fill-key");
    let fake = Arc::new(FakeDisplays::default());
    fake.set(vec![
        FakeDisplays::display("HDMI-A-1", "Fill Panel", true),
        FakeDisplays::display("HDMI-A-2", "Key Panel", true),
    ]);
    let server = start_with(&fx, fake.clone()).await;
    let addr = server.addr;
    let key = fx.key.id.0.to_string();
    assert_eq!(fx.key.alpha, model::AlphaMode::Straight);
    let server_info = get_json(addr, "/api/v1/server").await;
    assert!(
        server_info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("fillKey")),
        "{server_info}"
    );

    // A target the server doesn't know is refused.
    let r = http(
        addr,
        "POST",
        "/api/v1/outputs",
        &[],
        Some(json!({"screen": key, "target": "middle",
            "transport": {"type": "display", "selector": {"connector": "HDMI-A-1"}}})),
    )
    .await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    assert!(String::from_utf8_lossy(&r.body).contains("target"));

    // The pair: two outputs of the screen, the fill on HDMI-A-1, the key on HDMI-A-2.
    let mut ids = Vec::new();
    for (target, connector) in [("fill", "HDMI-A-1"), ("key", "HDMI-A-2")] {
        let r = http(
            addr,
            "POST",
            "/api/v1/outputs",
            &[],
            Some(json!({"screen": key, "target": target,
                "transport": {"type": "display", "selector": {"connector": connector}}})),
        )
        .await;
        assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
        assert_eq!(r.json()["target"], target, "stored and returned");
        ids.push(r.json());
    }
    // Both halves can't share one display.
    let r = http(
        addr,
        "POST",
        "/api/v1/outputs",
        &[],
        Some(json!({"screen": key, "target": "key",
            "transport": {"type": "display", "selector": {"connector": "HDMI-A-1"}}})),
    )
    .await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));

    // Red at half alpha: premultiplied (128, 0, 0, 128) on the screen.
    let mut ws = Client::connect(addr, "").await;
    ws.ok(json!({"type": "setTestPattern", "screen": key,
        "pattern": {"type": "solid", "color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 0.5}}}))
        .await;
    // The fill: the straight colour over black; the key: the alpha as grey. Both opaque.
    let fill = wait_shown(&fake, "HDMI-A-1", "fill shows straight red", |s| {
        near(s.last_centre, [255, 0, 0, 255])
    })
    .await;
    let k = wait_shown(&fake, "HDMI-A-2", "key shows half grey", |s| {
        near(s.last_centre, [128, 128, 128, 255])
    })
    .await;
    assert_eq!(fill.last_alpha, Some(model::AlphaMode::Opaque));
    assert_eq!(k.last_alpha, Some(model::AlphaMode::Opaque));
    // Each display knows which half of which screen's pair it shows (KMS flips the two
    // together).
    let pair = |is_key: bool| {
        Some(vesper_server::displays::Pairing {
            screen: key.clone(),
            part: String::new(),
            key: is_key,
        })
    };
    assert_eq!(fill.pair, pair(false));
    assert_eq!(k.pair, pair(true));
    let s = wait_state(addr, "both halves ok", |s| {
        s["screens"].as_array().unwrap().iter().any(|x| {
            x["id"] == key
                && x["outputs"]
                    .as_array()
                    .is_some_and(|o| o.len() == 2 && o.iter().all(|o| o["status"] == "ok"))
        })
    })
    .await;
    let names: Vec<&str> = s["screens"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == key)
        .unwrap()["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Display: HDMI-A-1 (fill)"), "{names:?}");
    assert!(names.contains(&"Display: HDMI-A-2 (key)"), "{names:?}");
    assert_eq!(display(&s, "HDMI-A-2")["boundTo"], json!([ids[1]["id"]]));

    // A premultiplied ("shaped") fill: the premultiplied colour over black.
    let mut doc = ids[0].clone();
    doc["alphaOverride"] = json!({"type": "premultiplied"});
    let put = http(
        addr,
        "PUT",
        &format!("/api/v1/outputs/{}", doc["id"].as_str().unwrap()),
        &[],
        Some(doc),
    )
    .await;
    assert_eq!(put.status, 200, "{}", String::from_utf8_lossy(&put.body));
    wait_shown(&fake, "HDMI-A-1", "shaped fill", |s| {
        near(s.last_centre, [128, 0, 0, 255])
    })
    .await;
    // The key is unchanged.
    assert!(near(
        fake.shown("HDMI-A-2").last_centre,
        [128, 128, 128, 255]
    ));
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_data_folder_becomes_a_data_directory() {
    let fx = fixture("empty-data");
    let empty = fx.dir.join("fresh-install");
    std::fs::create_dir_all(&empty).unwrap();
    let server = start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        mdns: false,
        site: vesper_server::site::SiteConfig::off(),
        displays: Some(Arc::new(FakeDisplays::default())),
        ..ServerConfig::new(&empty)
    })
    .await
    .expect("an empty folder is accepted");
    assert!(empty.join("portable").is_dir());
    server.shutdown().await.unwrap();
    // A folder that does not exist is still refused (a mistyped --data).
    let missing = fx.dir.join("no-such-folder");
    let e = start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        mdns: false,
        site: vesper_server::site::SiteConfig::off(),
        ..ServerConfig::new(&missing)
    })
    .await
    .err()
    .expect("a missing folder is refused");
    assert!(e.to_string().contains("does not exist"), "{e}");
    assert!(!missing.exists());
}

/// E06-3: on a box with two cards that both have an `HDMI-A-1`, an output saved with the bare
/// connector (before card-prefixed IDs) is the display it resolves to, not both: the other
/// card's `HDMI-A-1` can be used by another output, and both stay editable. The display it
/// resolves to stays refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bare_connector_is_only_the_display_it_resolves_to() {
    let fx = fixture("displays-bare");
    let fake = Arc::new(FakeDisplays::default());
    fake.set(vec![
        FakeDisplays::display("card0-HDMI-A-1", "Panel", true),
        FakeDisplays::display("card1-HDMI-A-1", "Projector", true),
    ]);
    let server = start_with(&fx, fake.clone()).await;
    let addr = server.addr;
    let (aud, key) = (fx.audience.id.0.to_string(), fx.key.id.0.to_string());
    let create = |screen: &str, connector: &str, name: Option<&str>| {
        json!({"screen": screen, "transport": {"type": "display",
               "selector": {"connector": connector, "name": name}}})
    };
    let r = http(
        addr,
        "POST",
        "/api/v1/outputs",
        &[],
        Some(create(&aud, "HDMI-A-1", Some("Projector"))),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let a = r.json();
    // The other card's HDMI-A-1: another display.
    let r = http(
        addr,
        "POST",
        "/api/v1/outputs",
        &[],
        Some(create(&key, "card0-HDMI-A-1", None)),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let b = r.json();
    // Both stay editable.
    for (doc, name) in [(a, "Projector out"), (b, "Panel out")] {
        let mut doc = doc;
        doc["name"] = json!(name);
        let id = doc["id"].as_str().unwrap().to_string();
        let put = http(
            addr,
            "PUT",
            &format!("/api/v1/outputs/{id}"),
            &[],
            Some(doc),
        )
        .await;
        assert_eq!(put.status, 200, "{}", String::from_utf8_lossy(&put.body));
    }
    // The display the bare one resolves to is taken.
    let r = http(
        addr,
        "POST",
        "/api/v1/outputs",
        &[],
        Some(create(&key, "card1-HDMI-A-1", None)),
    )
    .await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    assert!(String::from_utf8_lossy(&r.body).contains("already uses display"));
}

/// F03-1 and R1-2: an identify label holds one of the card's display controllers for a few
/// seconds. An output assigned to the identified display takes it over at once (the label is
/// taken down first), and an output on another display that found no free controller while
/// the label showed opens as soon as the label is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outputs_win_over_identify_labels() {
    let fx = fixture("displays-identify");
    let fake = Arc::new(FakeDisplays::default());
    fake.set(vec![
        FakeDisplays::display("DP-7", "Left Panel", true),
        FakeDisplays::display("DP-8", "Right Panel", true),
    ]);
    // A card with a single display controller. (Connectors no other test uses: identify
    // labels are tracked process-wide, by connector.)
    fake.set_controllers(Some(1));
    let server = start_with(&fx, fake.clone()).await;
    let addr = server.addr;
    let (aud, key) = (fx.audience.id.0.to_string(), fx.key.id.0.to_string());
    async fn identify(addr: std::net::SocketAddr, d: &str) -> Resp {
        let path = format!("/api/v1/displays/{d}/identify");
        http(addr, "POST", &path, &[], None).await
    }
    async fn add(addr: std::net::SocketAddr, screen: &str, connector: &str) -> Resp {
        let b = json!({"screen": screen,
            "transport": {"type": "display", "selector": {"connector": connector}}});
        http(addr, "POST", "/api/v1/outputs", &[], Some(b)).await
    }
    let status_of = |s: &Value, screen: &str| {
        output(s, screen, "display").map(|o| o["status"].as_str().unwrap_or("").to_string())
    };

    // F03-1: identify DP-8, then assign it at once: the output gets the display.
    let r = identify(addr, "DP-8").await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let r = add(addr, &aud, "DP-8").await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let s = wait_state(addr, "the output took the identified display", |s| {
        status_of(s, &aud).as_deref() == Some("ok")
    })
    .await;
    assert_eq!(status_of(&s, &aud).as_deref(), Some("ok"));
    // Used by an output now: it can't be identified.
    assert_eq!(identify(addr, "DP-8").await.status, 409);
    let out = r.json()["id"].as_str().unwrap().to_string();
    let r = http(addr, "DELETE", &format!("/api/v1/outputs/{out}"), &[], None).await;
    assert!(r.status < 300, "{}", String::from_utf8_lossy(&r.body));
    wait_state(addr, "the output is gone", |s| status_of(s, &aud).is_none()).await;

    // R1-2: identify DP-8 (it takes the only controller), then assign DP-7: it can't
    // open while the label shows, and opens once the label is gone (about 5 s).
    let r = identify(addr, "DP-8").await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let r = add(addr, &key, "DP-7").await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let s = state(addr).await;
    assert_eq!(
        status_of(&s, &key).as_deref(),
        Some("unavailable"),
        "no free controller while the label shows: {s:#}"
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let s = state(addr).await;
        if status_of(&s, &key).as_deref() == Some("ok") {
            break;
        }
        assert!(Instant::now() < deadline, "never retried: {s:#}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    server.shutdown().await.unwrap();
}
