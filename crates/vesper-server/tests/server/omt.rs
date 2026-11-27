//! OMT (Open Media Transport, `docs/OMT-PLAN.md`) end to end: an OMT output of a screen seen
//! by a receiver (picture at the screen's size, embedded program audio, status), and an OMT
//! source on the video-input layer, by key (`omt:host:port`) and through a logical input
//! bound to it (`InputSource::Omt`).
//!
//! ```text
//! sg render -c "cargo test -p vesper-server --test server omt:: -- --nocapture"
//! ```

use crate::common::*;
use output::{FrameSink, LiveEvent, LiveInputConfig, OmtInput, OmtSenderConfig, OmtSink};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_omt_output_sends_the_screen_with_its_audio() {
    let fx = fixture("omt-out");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let name = format!("E2E OMT {}", std::process::id());

    // OMT is built in.
    let transports = get_json(addr, "/api/v1/output-transports").await;
    let omt = transports
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "omt")
        .cloned()
        .unwrap();
    assert_eq!(omt["available"], true, "{omt}");
    assert!(omt["version"].as_str().is_some_and(|v| !v.is_empty()));

    let r = http(
        addr,
        "POST",
        "/api/v1/outputs",
        &[],
        Some(json!({"screen": fx.audience.id, "embedAudio": true,
            "transport": {"type": "omt", "name": name}})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let id = r.json()["id"].as_str().unwrap().to_string();

    // Running, embedding audio.
    let state = get_json(addr, "/api/v1/state").await;
    let out = state["state"]["screens"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["outputs"].as_array().unwrap().iter())
        .find(|o| o["binding"] == id.as_str())
        .cloned()
        .expect("output in the show state");
    assert_eq!(out["kind"], "omt", "{out}");
    assert_eq!(out["status"], "ok", "{out}");
    assert_eq!(out["embedAudio"], true, "{out}");

    // Found on the network by name (mDNS), then received.
    let full = tokio::task::spawn_blocking({
        let name = name.clone();
        move || {
            let end = Instant::now() + Duration::from_secs(20);
            while Instant::now() < end {
                if let Some(n) = output::omt::sources(Duration::from_secs(1))
                    .into_iter()
                    .find(|n| output::recv::source_part(n) == name)
                {
                    return Some(n);
                }
            }
            None
        }
    })
    .await
    .unwrap()
    .expect("the OMT output is announced");
    let mut a = Client::connect(addr, "").await;
    a.ok(json!({"type": "playTestTone", "seconds": 3.0})).await;
    let (size, loud) = tokio::task::spawn_blocking(move || {
        let input = OmtInput::open(&LiveInputConfig {
            source: full,
            sound: true,
            ..LiveInputConfig::default()
        })
        .unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        let (mut size, mut loud) = (None, 0.0f32);
        while Instant::now() < end && (size.is_none() || loud < 0.05) {
            match input.next(Duration::from_millis(200)) {
                Some(LiveEvent::Picture(p)) => size = Some((p.width, p.height)),
                Some(LiveEvent::Sound(s)) => {
                    let peak = s.channels[0].iter().fold(0.0f32, |m, v| m.max(v.abs()));
                    loud = loud.max(peak);
                }
                _ => {}
            }
        }
        (size, loud)
    })
    .await
    .unwrap();
    assert_eq!(
        size,
        Some((
            fx.audience.size.width as u32,
            fx.audience.size.height as u32
        )),
        "a picture at the screen's size"
    );
    assert!(loud >= 0.05, "the test tone arrives (peak {loud})");

    let r = http(addr, "DELETE", &format!("/api/v1/outputs/{id}"), &[], None).await;
    assert!(r.status == 200 || r.status == 204, "{}", r.status);
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// An OMT camera on this machine sending a solid colour, by address; returns its port.
fn omt_camera(rgb: [u8; 3], stop: Arc<AtomicBool>) -> (std::thread::JoinHandle<()>, u16) {
    let mut sink = OmtSink::new(OmtSenderConfig {
        announce: false,
        ..OmtSenderConfig::new(format!("OMT Cam {}", std::process::id()))
    })
    .unwrap();
    let port = sink.port();
    let h = std::thread::spawn(move || {
        let (w, h) = (320u32, 180u32);
        let px = [rgb[0], rgb[1], rgb[2], 255].repeat((w * h) as usize);
        let mut n = 0i64;
        while !stop.load(Ordering::Relaxed) {
            let _ = sink.send(&output::Frame {
                width: w,
                height: h,
                pixels: &px,
                alpha: output::AlphaMode::Opaque,
                frame_rate: output::FrameRate::FPS30,
                timecode: n * 333_333,
            });
            n += 1;
            std::thread::sleep(Duration::from_millis(33));
        }
    });
    (h, port)
}

fn corner_is(img: &image::RgbaImage, c: [u8; 3]) -> bool {
    near(img.get_pixel(4, 4), c, 40) && near(img.get_pixel(img.width() - 5, 4), c, 40)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_omt_source_goes_on_the_video_input_layer() {
    let fx = fixture("omt-in");
    let stop = Arc::new(AtomicBool::new(false));
    let (cam, port) = omt_camera([0x20, 0xC0, 0x30], stop.clone());
    let address = format!("127.0.0.1:{port}");
    let input = model::VideoInput {
        name: "OMT Camera".into(),
        ..model::VideoInput::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "videoInput",
        &input.id.0.to_string(),
        &input,
    );
    let binding = model::InputBinding {
        input: input.id,
        source: model::InputSource::Omt {
            name: address.clone(),
        },
        ..model::InputBinding::default()
    };
    write_doc(
        &fx.dir,
        "machine",
        "inputBinding",
        &input.id.0.to_string(),
        &binding,
    );
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;

    // By key.
    let key = format!("omt:{address}");
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "triggerLiveSource", "source": key})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    wait_frame(&server, 0, "the OMT camera's green", |i| {
        corner_is(i, [0x20, 0xC0, 0x30])
    })
    .await;
    // Listed as a received OMT source, live at its size.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let list = get_json(addr, "/api/v1/sources").await;
        let s = list["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == json!(key))
            .cloned()
            .expect("the received source is listed");
        assert_eq!(s["omt"], json!(true), "{s}");
        if s["status"] == json!("live") {
            assert_eq!(
                (s["width"].clone(), s["height"].clone()),
                (json!(320), json!(180))
            );
            break;
        }
        assert!(Instant::now() < deadline, "never live: {s}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // A thumbnail comes from it.
    let r = http(
        addr,
        "GET",
        &format!(
            "/api/v1/sources/thumbnail?source={}",
            key.replace(':', "%3A")
        ),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));

    // Through the logical input bound to it.
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "clearLayer", "layer": "videoInput"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "triggerVideoInput", "input": input.id.0.to_string()})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let snap = get_json(addr, "/api/v1/state").await;
    let layer = &snap["state"]["layers"]["videoInput"];
    assert_eq!(layer["source"], json!(key), "{layer}");
    assert_eq!(layer["name"], json!("OMT Camera"));
    wait_frame(&server, 0, "the OMT camera through its input", |i| {
        corner_is(i, [0x20, 0xC0, 0x30])
    })
    .await;

    stop.store(true, Ordering::Relaxed);
    cam.join().unwrap();
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}
