//! A screen's colour correction and corner pin: the narrow live routes, validation,
//! persistence, the picture changing without restarting the screen or its outputs, and the
//! corner-pinned picture's transparency reaching an IDN receiver.
//!
//! ```text
//! sg render -c "cargo test -p vesper-server --test server screen_adjust::"
//! ```

use crate::common::*;
use output::recv::source_part;
use output::{IdnFinder, IdnReceiver};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vesper_server::outputs::DevOutputs;
use vesper_server::{RunningServer, ServerConfig, start};

async fn start_with_png(fx: &Fixture) -> RunningServer {
    start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
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

fn body(r: &Resp) -> String {
    String::from_utf8_lossy(&r.body).to_string()
}

/// The latest frame of `screen`, as stored (premultiplied), polled until `ok` accepts it.
async fn wait_screen(
    server: &RunningServer,
    screen: model::ScreenId,
    what: &str,
    ok: impl Fn(&render::RenderedFrame) -> bool,
) -> Arc<render::RenderedFrame> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let f = server
            .shared
            .screen(screen)
            .and_then(|rt| rt.latest.lock().unwrap().clone());
        if let Some(f) = f
            && ok(&f)
        {
            return f;
        }
        assert!(Instant::now() < deadline, "{what}: frame never matched");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

fn close(p: [u8; 4], want: [u8; 4], tol: i32) -> bool {
    (0..4).all(|i| (p[i] as i32 - want[i] as i32).abs() <= tol)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn color_and_corner_pin_apply_live_and_persist() {
    let fx = fixture("screen-adjust");
    let server = start_with_png(&fx).await;
    let addr = server.addr;
    let sh = server.shared.clone();
    let id = fx.audience.id;
    let path = format!("/api/v1/screens/{}", id.0);
    let rt = sh.screen(id).expect("audience runs");
    let png = rt.outputs()[0].clone();
    assert_eq!(png.kind, "png");

    // A known picture: solid mid grey (sRGB 0.5).
    let mut ws = Client::connect(addr, "").await;
    ws.ok(json!({"type": "setTestPattern", "screen": id.0.to_string(),
        "pattern": {"type": "solid", "color": {"r": 0.5, "g": 0.5, "b": 0.5, "a": 1.0}}}))
        .await;
    let grey = wait_screen(&server, id, "grey", |f| {
        close(f.pixel(10, 10), [128, 128, 128, 255], 2)
    })
    .await;
    let (w, h) = (grey.width, grey.height);
    let rev0 = get_json(addr, &path).await["revision"].clone();

    // Colour: red gain 2, blue gain 0 → (255, 128, 0).
    let r = http(
        addr,
        "PUT",
        &format!("{path}/color"),
        &[],
        Some(json!({"red": 2.0, "blue": 0.0})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let e = r.json();
    assert_eq!(e["color"]["red"], 2.0);
    assert_eq!(
        e["color"]["green"], 1.0,
        "missing settings are their defaults"
    );
    assert_ne!(e["revision"], rev0, "a new revision");
    wait_screen(&server, id, "red gain", |f| {
        close(f.pixel(10, 10), [255, 128, 0, 255], 2)
    })
    .await;
    let stored: Value = serde_json::from_slice(
        &std::fs::read(fx.dir.join(format!("portable/screen/{}.json", id.0))).unwrap(),
    )
    .unwrap();
    assert_eq!(stored["color"]["blue"], 0.0, "saved: {stored}");
    assert!(stored.get("cornerPin").is_none(), "defaults are not stored");

    // Corner pin: every corner 25 % of the way in → black around a grey-orange middle.
    let (dx, dy) = (w as f64 * 0.25, h as f64 * 0.25);
    let inward = json!({"x": -dx, "y": -dy});
    let pin = json!({"enabled": true, "topLeft": inward, "topRight": inward,
        "bottomRight": inward, "bottomLeft": inward});
    let r = http(
        addr,
        "PUT",
        &format!("{path}/corner-pin"),
        &[],
        Some(pin.clone()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    assert_eq!(r.json()["color"]["red"], 2.0, "the colour is kept");
    wait_screen(&server, id, "pinned", |f| {
        close(f.pixel(10, 10), [0, 0, 0, 255], 0)
            && close(f.pixel(w / 2, h / 2), [255, 128, 0, 255], 2)
            && close(f.pixel(w / 4 - 3, h / 2), [0, 0, 0, 255], 0)
            && close(f.pixel(w / 4 + 3, h / 2), [255, 128, 0, 255], 2)
    })
    .await;

    // Neither the screen nor its output restarted.
    assert!(Arc::ptr_eq(&rt, &sh.screen(id).unwrap()));
    assert!(Arc::ptr_eq(&png, &sh.screen(id).unwrap().outputs()[0]));
    assert!(png.active);

    // The full screen document carries both; a full PUT can change them as well.
    let mut doc = get_json(addr, &path).await;
    assert_eq!(doc["cornerPin"]["enabled"], true);
    doc["cornerPin"]["enabled"] = json!(false);
    let r = http(addr, "PUT", &path, &[], Some(doc)).await;
    assert_eq!(r.status, 200, "{}", body(&r));
    wait_screen(&server, id, "pin off", |f| {
        close(f.pixel(10, 10), [255, 128, 0, 255], 2)
    })
    .await;

    // Validation: out of range, crossed corners, unknown screen. Nothing changes.
    for (route, v, field) in [
        ("color", json!({"gamma": 0.0}), "color.gamma"),
        ("color", json!({"brightness": 3.0}), "color.brightness"),
        ("color", json!({"red": "much"}), "color"),
        (
            "corner-pin",
            json!({"enabled": true, "topLeft": {"x": -(w as f64), "y": 0.0}, "topRight": {"x": -(w as f64), "y": 0.0}}),
            "cornerPin",
        ),
        (
            "corner-pin",
            json!({"enabled": true, "topLeft": {"x": 1e9, "y": 0.0}}),
            "cornerPin.topLeft",
        ),
    ] {
        let r = http(
            addr,
            "PUT",
            &format!("{path}/{route}"),
            &[],
            Some(v.clone()),
        )
        .await;
        assert_eq!(r.status, 400, "{v}: {}", body(&r));
        assert_eq!(r.json()["error"]["field"], field, "{v}: {}", body(&r));
    }
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/screens/{}/color", uuid::Uuid::new_v4()),
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, 404, "{}", body(&r));

    // Reset both: the plain picture, bit for bit, and nothing stored.
    for route in ["color", "corner-pin"] {
        let r = http(
            addr,
            "PUT",
            &format!("{path}/{route}"),
            &[],
            Some(json!({})),
        )
        .await;
        assert_eq!(r.status, 200, "{}", body(&r));
    }
    let f = wait_screen(&server, id, "reset", |f| {
        close(f.pixel(10, 10), [128, 128, 128, 255], 2)
    })
    .await;
    assert_eq!(
        f.data, grey.data,
        "identity settings: the same frame as before"
    );
    let stored: Value = serde_json::from_slice(
        &std::fs::read(fx.dir.join(format!("portable/screen/{}.json", id.0))).unwrap(),
    )
    .unwrap();
    assert!(
        stored.get("color").is_none() && stored.get("cornerPin").is_none(),
        "{stored}"
    );

    // The settings come back after a restart.
    let r = http(
        addr,
        "PUT",
        &format!("{path}/color"),
        &[],
        Some(json!({"gamma": 2.0})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    server.shutdown().await.unwrap();
    let server = start_with_png(&fx).await;
    let doc = get_json(server.addr, &path).await;
    assert_eq!(doc["color"]["gamma"], 2.0);
    server.shutdown().await.unwrap();
}

/// The transparency a corner pin leaves around a picture on a screen with alpha reaches an
/// IDN receiver: outside the quad alpha 0, inside the picture's alpha.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn corner_pin_transparency_reaches_an_idn_receiver() {
    let fx = fixture("screen-adjust-idn");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let key = fx.key.id;
    assert_eq!(fx.key.alpha, model::AlphaMode::Straight);
    let name = format!("Adjust IDN {}", std::process::id());
    let r = http(
        addr,
        "POST",
        "/api/v1/outputs",
        &[],
        Some(json!({"screen": key, "transport": {"type": "idn", "sourceName": name}})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let mut ws = Client::connect(addr, "").await;
    ws.ok(
        json!({"type": "setTestPattern", "screen": key.0.to_string(),
        "pattern": {"type": "solid", "color": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}}}),
    )
    .await;
    let (w, h) = (fx.key.size.width, fx.key.size.height);
    let inward = json!({"x": -w * 0.25, "y": -h * 0.25});
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/screens/{}/corner-pin", key.0),
        &[],
        Some(
            json!({"enabled": true, "topLeft": inward, "topRight": inward,
            "bottomRight": inward, "bottomLeft": inward}),
        ),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let (w, h) = (w as u32, h as u32);
    wait_screen(&server, key, "pinned key", |f| {
        f.pixel(5, 5) == [0, 0, 0, 0] && f.pixel(w / 2, h / 2) == [255, 255, 255, 255]
    })
    .await;

    let frame = tokio::task::block_in_place(|| {
        let finder = IdnFinder::new().unwrap();
        let end = Instant::now() + Duration::from_secs(20);
        let src = loop {
            if let Some(s) = finder
                .sources(Duration::from_millis(300))
                .into_iter()
                .find(|s| source_part(&s.name) == name)
            {
                break s;
            }
            assert!(Instant::now() < end, "the IDN output is listed");
        };
        let mut rx = IdnReceiver::connect(&src.name).unwrap();
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(f) = rx.capture_video(Duration::from_millis(500)).unwrap()
                && f.pixel_rgba(5, 5)[3] < 128
            {
                break f;
            }
            assert!(Instant::now() < end, "a pinned frame from the IDN output");
        }
    });
    eprintln!(
        "received {}x{} {}",
        frame.width,
        frame.height,
        frame.fourcc_str()
    );
    assert_eq!(frame.fourcc_str(), "RGBA", "frames with an alpha plane");
    let corner = frame.pixel_rgba(5, 5);
    let middle = frame.pixel_rgba(frame.width / 2, frame.height / 2);
    assert!(
        corner[3] <= 2,
        "outside the quad: transparent, got {corner:?}"
    );
    assert!(middle[3] >= 253, "inside: opaque, got {middle:?}");
    assert!(
        middle[..3].iter().all(|c| *c >= 245),
        "inside: white, got {middle:?}"
    );
    server.shutdown().await.unwrap();
}
