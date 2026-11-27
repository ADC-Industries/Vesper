//! Screen layouts (batch 12): the pixels each output of a Grouped and an Edge Blend screen
//! gets (simulated displays, the same output worker every transport uses), arrangement /
//! region / tile validation, placeholders, and identifying a display no output uses.

use crate::common::*;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vesper_server::displays::{FakeDisplays, FakeShown};
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

fn body(r: &Resp) -> String {
    String::from_utf8_lossy(&r.body).into_owned()
}

/// Waits until `pred` holds for what `connector` shows (10 s).
async fn wait_shown(
    fake: &FakeDisplays,
    connector: &str,
    what: &str,
    pred: impl Fn(&FakeShown) -> bool,
) -> FakeShown {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = fake.shown(connector);
        if pred(&s) {
            return s;
        }
        let summary = (s.frames, s.last_size, s.opened);
        assert!(Instant::now() < deadline, "{what}: {summary:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Polls the state until `pred` holds (10 s).
async fn wait_state(addr: std::net::SocketAddr, what: &str, pred: impl Fn(&Value) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = get_json(addr, "/api/v1/state").await["state"].clone();
        if pred(&s) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: {s:#}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn outputs_of(s: &Value, screen: &str) -> Vec<Value> {
    s["screens"]
        .as_array()
        .and_then(|l| l.iter().find(|x| x["id"] == screen))
        .and_then(|x| x["outputs"].as_array().cloned())
        .unwrap_or_default()
}

/// Puts the screen's document with `patch` merged in; returns the response.
async fn put_screen(addr: std::net::SocketAddr, id: &str, patch: Value) -> Resp {
    let path = format!("/api/v1/screens/{id}");
    let mut doc = get_json(addr, &path).await;
    for (k, v) in patch.as_object().unwrap() {
        doc[k] = v.clone();
    }
    http(addr, "PUT", &path, &[], Some(doc)).await
}

async fn add_output(addr: std::net::SocketAddr, body: Value) -> Resp {
    http(addr, "POST", "/api/v1/outputs", &[], Some(body)).await
}

fn display_output(screen: &str, connector: &str, tile: Option<u32>) -> Value {
    let mut b = json!({"screen": screen,
        "transport": {"type": "display", "selector": {"connector": connector}}});
    if let Some(t) = tile {
        b["tile"] = json!(t);
    }
    b
}

/// The `(x, y, w, h)` part of a `width`-wide RGBA frame.
fn crop(px: &[u8], width: u32, (x, y, w, h): (u32, u32, u32, u32)) -> Vec<u8> {
    let mut out = Vec::new();
    for row in y..y + h {
        let a = ((row * width + x) * 4) as usize;
        out.extend_from_slice(&px[a..a + (w * 4) as usize]);
    }
    out
}

fn pixel(s: &FakeShown, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * s.last_size.0 + x) * 4) as usize;
    s.last_pixels[i..i + 4].try_into().unwrap()
}

/// Saves what a display shows as a PNG (`VESPER_LAYOUT_PNGS=<dir>`), to look at.
fn save(s: &FakeShown, name: &str) {
    if let Some(dir) = std::env::var_os("VESPER_LAYOUT_PNGS") {
        let path = Path::new(&dir).join(format!("{name}.png"));
        let _ = std::fs::create_dir_all(&dir);
        let frame = output::Frame {
            width: s.last_size.0,
            height: s.last_size.1,
            pixels: &s.last_pixels,
            alpha: model::AlphaMode::Opaque,
            frame_rate: model::FrameRate::default(),
            timecode: 0,
        };
        output::write_png(&path, &frame).expect("png written");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grouped_and_edge_blend_outputs_get_their_tiles() {
    let fx = fixture("screen-layouts-pixels");
    let fake = Arc::new(FakeDisplays::default());
    fake.set(vec![
        FakeDisplays::display("HDMI-A-1", "Left", true),
        FakeDisplays::display("HDMI-A-2", "Right", true),
        FakeDisplays::display("HDMI-A-3", "Confidence", true),
    ]);
    let server = start_with(&fx, fake.clone()).await;
    let addr = server.addr;
    let id = fx.audience.id.0.to_string();
    let caps = get_json(addr, "/api/v1/server").await["capabilities"].clone();
    assert!(
        caps.as_array().unwrap().contains(&json!("screenLayouts")),
        "{caps}"
    );

    // A tile on a screen that isn't tiled is refused.
    let r = add_output(addr, display_output(&id, "HDMI-A-1", Some(0))).await;
    assert_eq!(r.status, 400, "{}", body(&r));
    assert!(body(&r).contains("Grouped"), "{}", body(&r));

    // Grouped, 2 × 1 tiles of 320 × 180 with a 64 px bezel: a 704 × 180 canvas.
    let r = put_screen(
        addr,
        &id,
        json!({"size": {"width": 704.0, "height": 180.0},
            "arrangement": {"kind": "grouped", "columns": 2, "bezel": {"x": 64.0, "y": 0.0}}}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    assert_eq!(r.json()["arrangement"]["kind"], "grouped");
    for (connector, tile) in [
        ("HDMI-A-1", Some(0)),
        ("HDMI-A-2", Some(1)),
        ("HDMI-A-3", None),
    ] {
        let r = add_output(addr, display_output(&id, connector, tile)).await;
        assert_eq!(r.status, 201, "{}", body(&r));
        assert_eq!(r.json()["tile"], json!(tile));
    }
    // A tile past the grid is refused.
    let r = add_output(
        addr,
        json!({"screen": id, "tile": 2,
        "transport": {"type": "placeholder"}, "name": "Third"}),
    )
    .await;
    assert_eq!(r.status, 400, "{}", body(&r));
    assert!(body(&r).contains("2 tiles"), "{}", body(&r));

    // Colour bars: different colours across the canvas.
    let mut ws = Client::connect(addr, "").await;
    ws.ok(json!({"type": "setTestPattern", "screen": id, "pattern": {"type": "colorBars"}}))
        .await;
    wait_shown(&fake, "HDMI-A-3", "the whole canvas", |s| {
        s.last_size == (704, 180) && s.last_centre != [0, 0, 0, 0]
    })
    .await;
    wait_shown(&fake, "HDMI-A-1", "the left tile", |s| {
        s.last_size == (320, 180)
    })
    .await;
    wait_shown(&fake, "HDMI-A-2", "the right tile", |s| {
        s.last_size == (320, 180)
    })
    .await;
    // Let a frame of the pattern reach all three, then compare exactly.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (full, left, right) = (
        fake.shown("HDMI-A-3"),
        fake.shown("HDMI-A-1"),
        fake.shown("HDMI-A-2"),
    );
    save(&full, "grouped-full");
    save(&left, "grouped-left");
    save(&right, "grouped-right");
    assert!(
        left.last_pixels == crop(&full.last_pixels, 704, (0, 0, 320, 180)),
        "the left display shows the canvas's first 320 columns"
    );
    assert!(
        right.last_pixels == crop(&full.last_pixels, 704, (384, 0, 320, 180)),
        "the right display skips the 64 px behind the bezels"
    );
    assert_ne!(left.last_pixels, right.last_pixels);
    wait_state(addr, "outputs report their tile size", |s| {
        let o = outputs_of(s, &id);
        o.len() == 3
            && o.iter().filter(|o| o["width"] == 320).count() == 2
            && o.iter().any(|o| o["width"] == 704)
    })
    .await;

    // Edge Blend: the same two tiles overlap by 64 px: a 576 × 180 canvas, all white.
    let r = put_screen(
        addr,
        &id,
        json!({"size": {"width": 576.0, "height": 180.0},
            "arrangement": {"kind": "edgeBlend", "columns": 2, "overlap": {"x": 64.0, "y": 0.0},
                "blend": {"gamma": 2.2, "curve": 2.0}}}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    ws.ok(json!({"type": "setTestPattern", "screen": id,
        "pattern": {"type": "solid", "color": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}}}))
        .await;
    let white = |s: &FakeShown, x: u32| pixel(s, x, 90) == [255, 255, 255, 255];
    let left = wait_shown(&fake, "HDMI-A-1", "the left projector, blended", |s| {
        s.last_size == (320, 180) && white(s, 0) && !white(s, 319)
    })
    .await;
    let right = wait_shown(&fake, "HDMI-A-2", "the right projector, blended", |s| {
        s.last_size == (320, 180) && white(s, 319) && !white(s, 0)
    })
    .await;
    let full = wait_shown(&fake, "HDMI-A-3", "the whole canvas, unblended", |s| {
        s.last_size == (576, 180) && white(s, 0) && white(s, 288) && white(s, 575)
    })
    .await;
    save(&left, "blend-left");
    save(&right, "blend-right");
    save(&full, "blend-full");
    // Outside the overlap: untouched. Inside: the two projectors' light adds up to one
    // (gamma 2.2), the left one fading out, the right one fading in.
    assert!(white(&left, 255) && white(&right, 64));
    let light = |v: u8| (v as f64 / 255.0).powf(2.2);
    let mut last = 256u32;
    for k in 0..64 {
        let a = pixel(&left, 256 + k, 90);
        let b = pixel(&right, k, 90);
        let sum = light(a[0]) + light(b[0]);
        assert!(
            (sum - 1.0).abs() < 0.03,
            "column {k} of the overlap: {a:?} + {b:?} = {sum}"
        );
        assert_eq!(a[3], 255, "alpha untouched");
        assert!(a[0] as u32 <= last, "the left ramp falls");
        last = a[0] as u32;
        // Every row of a column is the same.
        assert_eq!(pixel(&left, 256 + k, 3), a);
    }
    assert!(pixel(&left, 319, 90)[0] < 60, "nearly dark at the far edge");
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn layouts_are_validated_and_placeholders_and_unused_displays_show() {
    let fx = fixture("screen-layouts-config");
    let fake = Arc::new(FakeDisplays::default());
    fake.set(vec![
        FakeDisplays::display("HDMI-A-1", "Used", true),
        FakeDisplays::display("HDMI-A-2", "Spare", true),
        FakeDisplays::display("DP-1", "", false),
    ]);
    let server = start_with(&fx, fake.clone()).await;
    let addr = server.addr;
    let id = fx.audience.id.0.to_string();

    // Arrangement validation: nothing changes.
    for (arrangement, size, field) in [
        (
            json!({"kind": "grouped", "columns": 9}),
            [1920.0, 1080.0],
            "arrangement.columns",
        ),
        (
            json!({"kind": "mirror", "columns": 2}),
            [1920.0, 1080.0],
            "arrangement.columns",
        ),
        (
            json!({"kind": "edgeBlend", "columns": 2, "overlap": {"x": 2000.0}}),
            [1920.0, 1080.0],
            "arrangement.overlap",
        ),
        // Three tiles across, tile 2080 px: a 1200 px overlap would let three projectors
        // light one strip (F08-3).
        (
            json!({"kind": "edgeBlend", "columns": 3, "overlap": {"x": 1200.0}}),
            [3840.0, 1080.0],
            "arrangement.overlap",
        ),
        (
            json!({"kind": "grouped", "columns": 2, "bezel": {"x": 2000.0}}),
            [2000.0, 1080.0],
            "arrangement.columns",
        ),
        (
            json!({"kind": "edgeBlend", "columns": 2, "blend": {"gamma": 5.0}}),
            [1920.0, 1080.0],
            "arrangement.blend.gamma",
        ),
        (
            json!({"kind": "sideways"}),
            [1920.0, 1080.0],
            "arrangement.kind",
        ),
    ] {
        let r = put_screen(
            addr,
            &id,
            json!({"size": {"width": size[0], "height": size[1]}, "arrangement": arrangement}),
        )
        .await;
        assert_eq!(r.status, 400, "{arrangement}: {}", body(&r));
        assert!(body(&r).contains(field), "{arrangement}: {}", body(&r));
    }
    let doc = get_json(addr, &format!("/api/v1/screens/{id}")).await;
    assert!(
        doc.get("arrangement").is_none(),
        "Single is not stored: {doc}"
    );

    // A 2 × 2 Grouped wall with two displays and two placeholders.
    let r = put_screen(
        addr,
        &id,
        json!({"size": {"width": 3840.0, "height": 2160.0},
            "arrangement": {"kind": "grouped", "columns": 2, "rows": 2}}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let r = add_output(addr, display_output(&id, "HDMI-A-1", Some(0))).await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let mut placeholders = Vec::new();
    for (tile, name) in [(2, "Bottom left"), (3, "Bottom right")] {
        let r = add_output(
            addr,
            json!({"screen": id, "tile": tile, "name": name,
            "transport": {"type": "placeholder"}}),
        )
        .await;
        assert_eq!(r.status, 201, "{}", body(&r));
        assert_eq!(r.json()["transport"]["type"], "placeholder");
        placeholders.push(r.json());
    }
    wait_state(addr, "placeholders listed, never run", |s| {
        let o = outputs_of(s, &id);
        let ph: Vec<&Value> = o.iter().filter(|o| o["kind"] == "placeholder").collect();
        ph.len() == 2
            && ph
                .iter()
                .all(|o| o["status"] == "unavailable" && o["detail"] == "No hardware yet")
            && o.iter()
                .any(|o| o["kind"] == "display" && o["width"] == 1920)
    })
    .await;
    // Regions: inside the canvas and not tiny.
    for region in [
        json!({"x": 3000.0, "y": 0.0, "width": 1920.0, "height": 1080.0}),
        json!({"x": 0.0, "y": 0.0, "width": 8.0, "height": 8.0}),
    ] {
        let r = add_output(
            addr,
            json!({"screen": id, "region": region, "name": "Crop",
            "transport": {"type": "placeholder"}}),
        )
        .await;
        assert_eq!(r.status, 400, "{region}: {}", body(&r));
        assert!(body(&r).contains("region"), "{}", body(&r));
    }
    // A wall made smaller leaves the tiles past its end unavailable, and says why.
    let r = put_screen(
        addr,
        &id,
        json!({"size": {"width": 3840.0, "height": 1080.0},
            "arrangement": {"kind": "grouped", "columns": 2, "rows": 1}}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let r = add_output(
        addr,
        json!({"screen": id, "tile": 1, "name": "Right",
        "transport": {"type": "display", "selector": {"connector": "HDMI-A-2"}}}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let right = r.json();
    let r = put_screen(
        addr,
        &id,
        json!({"size": {"width": 1920.0, "height": 1080.0},
            "arrangement": {"kind": "grouped", "columns": 1, "rows": 1}}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    wait_state(addr, "tile 2 is gone", |s| {
        outputs_of(s, &id).iter().any(|o| {
            o["binding"] == right["id"]
                && o["status"] == "unavailable"
                && o["detail"]
                    .as_str()
                    .is_some_and(|d| d.contains("outside the 1×1"))
        })
    })
    .await;
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/outputs/{}", right["id"].as_str().unwrap()),
        &[],
        None,
    )
    .await;
    assert!(r.status < 300, "{}", body(&r));

    // Identify a display no output uses: it shows a picture at its preferred mode for a
    // few seconds, then closes again.
    let opened = fake.shown("HDMI-A-2").opened;
    let r = http(
        addr,
        "POST",
        "/api/v1/displays/HDMI-A-2/identify",
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", body(&r));
    let shown = wait_shown(&fake, "HDMI-A-2", "the identify label", |s| {
        s.frames > 0 && s.last_size == (1920, 1080)
    })
    .await;
    save(&shown, "identify");
    assert_eq!(shown.fullscreen, Some(true));
    // Dark box in the middle, the screen around it black.
    assert_eq!(pixel(&shown, 20, 20), [0, 0, 0, 255]);
    let r = http(
        addr,
        "POST",
        "/api/v1/displays/HDMI-A-2/identify",
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 409, "already showing: {}", body(&r));
    for (display, status) in [("HDMI-A-1", 409), ("DP-1", 409), ("HDMI-A-9", 404)] {
        let r = http(
            addr,
            "POST",
            &format!("/api/v1/displays/{display}/identify"),
            &[],
            None,
        )
        .await;
        assert_eq!(r.status, status, "{display}: {}", body(&r));
    }
    // After five seconds the display is free again.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let r = http(
            addr,
            "POST",
            "/api/v1/displays/HDMI-A-2/identify",
            &[],
            None,
        )
        .await;
        if r.status == 204 {
            break;
        }
        assert!(Instant::now() < deadline, "never freed: {}", body(&r));
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(fake.shown("HDMI-A-2").opened, opened + 2);
    let _ = placeholders;
    server.shutdown().await.unwrap();
}
