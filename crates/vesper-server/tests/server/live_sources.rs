//! Live network sources (G7) end to end, with an in-process IDN source as the camera: the
//! source list lists it, `triggerLiveSource` puts it on the video-input layer of every screen
//! (newest picture at the screen rate), a thumbnail comes from it, clearing and restoring the
//! layer work, a look hiding the layer hides it, and a source that goes away is no longer
//! drawn once its hold time is over.
//!
//! ```text
//! sg render -c "cargo test -p vesper-server --test server live_sources:: -- --nocapture"
//! ```

use crate::common::*;
use output::{FrameSink, IdnSenderConfig, IdnSink};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// A camera: an IDN source sending a solid colour (`0xRRGGBB` in `color`) at 30 fps until
/// `stop`.
fn camera(name: &str, color: Arc<AtomicU32>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    let name = name.to_string();
    std::thread::spawn(move || {
        let mut sink = IdnSink::new(IdnSenderConfig::new(name)).unwrap();
        let (w, h) = (320u32, 180u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        let mut n = 0i64;
        while !stop.load(Ordering::Relaxed) {
            let c = color.load(Ordering::Relaxed);
            for p in px.as_chunks_mut::<4>().0 {
                p.copy_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
            }
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
    })
}

fn corner_is(img: &image::RgbaImage, c: [u8; 3]) -> bool {
    near(img.get_pixel(4, 4), c, 40) && near(img.get_pixel(img.width() - 5, 4), c, 40)
}

async fn command(addr: std::net::SocketAddr, body: serde_json::Value) {
    let r = http(addr, "POST", "/api/v1/commands", &[], Some(body)).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_source_goes_on_the_video_input_layer() {
    let fx = fixture("live-sources");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let short = format!("Live Cam {}", std::process::id());
    let color = Arc::new(AtomicU32::new(0xD02020));
    let stop = Arc::new(AtomicBool::new(false));
    let cam = camera(&short, color.clone(), stop.clone());

    // The list finds it (mDNS on this machine).
    let deadline = Instant::now() + Duration::from_secs(20);
    let full = loop {
        let list = get_json(addr, "/api/v1/sources").await;
        let found = list["sources"].as_array().unwrap().iter().find(|s| {
            s["name"]
                .as_str()
                .is_some_and(|n| output::recv::source_part(n) == short)
        });
        if let Some(s) = found {
            assert_eq!(s["listed"], json!(true));
            assert_eq!(s["status"], json!("idle"));
            break s["name"].as_str().unwrap().to_string();
        }
        assert!(
            Instant::now() < deadline,
            "the source list never had {short}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    // Live: every screen draws it under everything else.
    command(addr, json!({"type": "triggerLiveSource", "source": full})).await;
    let st = server.shared.state();
    assert_eq!(st.video_input.as_ref().unwrap().source, full);
    let snap = get_json(addr, "/api/v1/state").await;
    let layer = &snap["state"]["layers"]["videoInput"];
    assert_eq!(layer["source"], json!(full));
    assert_eq!(layer["name"], json!(short));
    wait_frame(&server, 0, "the camera's red", |i| {
        corner_is(i, [0xD0, 0x20, 0x20])
    })
    .await;

    // The newest picture follows the source.
    color.store(0x2040D0, Ordering::Relaxed);
    wait_frame(&server, 0, "the camera's blue", |i| {
        corner_is(i, [0x20, 0x40, 0xD0])
    })
    .await;

    // Its status and size are listed.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let list = get_json(addr, "/api/v1/sources").await;
        let s = list["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == json!(full))
            .cloned()
            .unwrap();
        if s["status"] == json!("live") {
            assert_eq!(s["width"], json!(320));
            assert_eq!(s["height"], json!(180));
            break;
        }
        assert!(Instant::now() < deadline, "never live: {s}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // A thumbnail of it (from the receiver in use).
    let q = format!(
        "/api/v1/sources/thumbnail?source={}&w=160",
        full.replace(' ', "%20")
            .replace('(', "%28")
            .replace(')', "%29")
    );
    let r = http(addr, "GET", &q, &[], None).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-type"), Some("image/jpeg"));
    let img = image::load_from_memory(&r.body).unwrap().to_rgb8();
    assert_eq!(img.width(), 160);
    let p = img.get_pixel(80, 45);
    assert!(
        (p.0[2] as i32 - 0xD0).abs() < 40 && p.0[0] < 80,
        "thumbnail colour {p:?}"
    );

    // Clear and restore.
    command(addr, json!({"type": "clearLayer", "layer": "videoInput"})).await;
    wait_frame(&server, 0, "cleared", |i| !corner_is(i, [0x20, 0x40, 0xD0])).await;
    let snap = get_json(addr, "/api/v1/state").await;
    assert!(snap["state"]["layers"]["videoInput"].is_null());
    assert!(
        snap["state"]["restorableLayers"]
            .as_array()
            .unwrap()
            .contains(&json!("videoInput"))
    );
    command(addr, json!({"type": "restoreLayer", "layer": "videoInput"})).await;
    wait_frame(&server, 0, "restored", |i| corner_is(i, [0x20, 0x40, 0xD0])).await;

    // Clear all takes it off too.
    command(addr, json!({"type": "clearAll"})).await;
    assert!(server.shared.state().video_input.is_none());
    command(addr, json!({"type": "triggerLiveSource", "source": full})).await;
    wait_frame(&server, 0, "live again", |i| {
        corner_is(i, [0x20, 0x40, 0xD0])
    })
    .await;

    // A look that hides the video-input layer on the audience screen hides it there.
    let r = http(
        addr,
        "POST",
        "/api/v1/looks",
        &[],
        Some(json!({"name": "No camera", "screens": [{"screen": fx.audience.id, "layers": {"videoInput": false}}]})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let look = r.json()["id"].as_str().unwrap().to_string();
    command(addr, json!({"type": "setLook", "look": look})).await;
    wait_frame(&server, 0, "hidden by the look", |i| {
        !corner_is(i, [0x20, 0x40, 0xD0])
    })
    .await;
    command(addr, json!({"type": "setLook", "look": null})).await;
    wait_frame(&server, 0, "shown again", |i| {
        corner_is(i, [0x20, 0x40, 0xD0])
    })
    .await;

    // The camera goes away: the last picture holds a moment, then nothing is drawn.
    stop.store(true, Ordering::Relaxed);
    cam.join().unwrap();
    let gone = Instant::now();
    wait_frame(&server, 0, "lost source not drawn", |i| {
        !corner_is(i, [0x20, 0x40, 0xD0])
    })
    .await;
    eprintln!(
        "lost source cleared from the screen after {:?}",
        gone.elapsed()
    );
    assert!(
        gone.elapsed() >= Duration::from_secs(2),
        "held before going"
    );

    // Bad requests.
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "triggerLiveSource", "source": "  "})),
    )
    .await;
    assert_eq!(r.status, 400);
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// The full name of the camera `short` once the source list has it.
async fn find_source(addr: std::net::SocketAddr, short: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let list = get_json(addr, "/api/v1/sources").await;
        let found = list["sources"].as_array().unwrap().iter().find(|s| {
            s["name"]
                .as_str()
                .is_some_and(|n| output::recv::source_part(n) == short)
        });
        if let Some(s) = found {
            return s["name"].as_str().unwrap().to_string();
        }
        assert!(
            Instant::now() < deadline,
            "the source list never had {short}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// D02-1: a test pattern up longer than the idle time closes the receiver nobody draws; once
/// the pattern is off the source is received again (live), not drawn frozen.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_source_comes_back_live_after_a_test_pattern() {
    let fx = fixture("live-sources-pattern");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let short = format!("Pattern Cam {}", std::process::id());
    let color = Arc::new(AtomicU32::new(0xD02020));
    let stop = Arc::new(AtomicBool::new(false));
    let cam = camera(&short, color.clone(), stop.clone());
    let full = find_source(addr, &short).await;
    command(addr, json!({"type": "triggerLiveSource", "source": full})).await;
    wait_frame(&server, 0, "the camera's red", |i| {
        corner_is(i, [0xD0, 0x20, 0x20])
    })
    .await;

    // Every screen shows a pattern until the registry has closed the receiver.
    let screens: Vec<String> = [&fx.audience, &fx.key]
        .iter()
        .map(|s| s.id.0.to_string())
        .collect();
    for s in &screens {
        command(
            addr,
            json!({"type": "setTestPattern", "screen": s,
                "pattern": {"type": "solid", "color": {"r": 0.5, "g": 0.5, "b": 0.5, "a": 1.0}}}),
        )
        .await;
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let list = get_json(addr, "/api/v1/sources").await;
        let s = list["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == json!(full))
            .cloned()
            .unwrap();
        if s["status"] == json!("idle") {
            break;
        }
        assert!(Instant::now() < deadline, "never closed: {s}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    color.store(0x2040D0, Ordering::Relaxed);
    for s in &screens {
        command(
            addr,
            json!({"type": "setTestPattern", "screen": s, "pattern": null}),
        )
        .await;
    }
    wait_frame(&server, 0, "the camera live again (blue)", |i| {
        corner_is(i, [0x20, 0x40, 0xD0])
    })
    .await;
    stop.store(true, Ordering::Relaxed);
    cam.join().unwrap();
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// D11-4: `RunningServer::shutdown` stops the live-source receivers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_stops_the_live_receivers() {
    let fx = fixture("live-sources-shutdown");
    let server = start_server(&fx, true, false).await;
    let rx = server.shared.live_sources.receiver("192.0.2.1:5961", None);
    assert!(rx.frame().is_none());
    let shared = server.shared.clone();
    server.shutdown().await.unwrap();
    assert!(rx.is_stopped(), "the receiver still runs after shutdown");
    // A screen still planning gets a closed receiver, not a new one.
    assert!(
        shared
            .live_sources
            .receiver("192.0.2.1:5961", None)
            .is_stopped()
    );
    let _ = std::fs::remove_dir_all(&fx.dir);
}

// ---- Stage 3 (video): fills, logical inputs, alpha, compressed video ----------------------

/// A camera reachable at `127.0.0.1:<port>` (the port comes back): sends `color` at 30 fps,
/// with `alpha` (255: opaque; else an alpha plane, the colour premultiplied as the renderer's
/// frames are) until `stop`.
fn camera_at(
    name: &str,
    color: Arc<AtomicU32>,
    alpha: u8,
    stop: Arc<AtomicBool>,
) -> (std::thread::JoinHandle<()>, u16) {
    let name = name.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    let h = std::thread::spawn(move || {
        // Started with the alpha intent of its frames (a change would restart it on
        // another port).
        let mut cfg = IdnSenderConfig::new(name);
        cfg.alpha = alpha != 255;
        let mut sink = IdnSink::new(cfg).unwrap();
        let _ = tx.send(sink.port());
        let (w, h) = (320u32, 180u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        let mut n = 0i64;
        let pre = |v: u32| ((v & 0xFF) * alpha as u32 / 255) as u8;
        while !stop.load(Ordering::Relaxed) {
            let c = color.load(Ordering::Relaxed);
            for p in px.as_chunks_mut::<4>().0 {
                p.copy_from_slice(&[pre(c >> 16), pre(c >> 8), pre(c), alpha]);
            }
            let _ = sink.send(&output::Frame {
                width: w,
                height: h,
                pixels: &px,
                alpha: if alpha == 255 {
                    output::AlphaMode::Opaque
                } else {
                    output::AlphaMode::Premultiplied
                },
                frame_rate: output::FrameRate::FPS30,
                timecode: n * 333_333,
            });
            n += 1;
            std::thread::sleep(Duration::from_millis(33));
        }
    });
    let port = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("camera port");
    // The sink reports its port once its source runs (`port()` is 0 before).
    assert_ne!(port, 0, "the camera's source did not start");
    (h, port)
}

fn px_is(img: &image::RgbaImage, x: u32, y: u32, c: [u8; 3], tol: i32) -> bool {
    near(img.get_pixel(x, y), c, tol)
}

/// A slide element whose fill is a live source shows the camera inside its box (newest
/// picture), and nothing of it outside.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_source_fills_a_slide_element() {
    use model::{Presentation, Rect, Section, Slide};
    let fx = fixture("live-fill");
    let color = Arc::new(AtomicU32::new(0xD02020));
    let stop = Arc::new(AtomicBool::new(false));
    let (cam, port) = camera_at(
        &format!("Fill Cam {}", std::process::id()),
        color.clone(),
        255,
        stop.clone(),
    );
    // The left half of the canvas.
    let mut e = model::Element::media(Rect::new(0.0, 0.0, 960.0, 1080.0), fx.asset.id);
    e.fill = model::Fill::enabled(model::FillKind::LiveSource {
        source: format!("127.0.0.1:{port}"),
        input: None,
        display: model::MediaDisplay {
            scale: model::ScaleMode::Stretch,
            ..model::MediaDisplay::default()
        },
    });
    let mut slide = Slide::new();
    slide.elements = vec![e];
    let mut sec = Section::new("Camera");
    sec.slides = vec![slide];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Camera box")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    command(
        addr,
        json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}),
    )
    .await;
    command(
        addr,
        json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 0}),
    )
    .await;
    let img = wait_frame(&server, 0, "the camera in its box", |i| {
        px_is(i, i.width() / 8, i.height() / 2, [0xD0, 0x20, 0x20], 40)
    })
    .await;
    let (w, h) = (img.width(), img.height());
    assert!(
        !px_is(&img, w * 7 / 8, h / 2, [0xD0, 0x20, 0x20], 60),
        "the camera is drawn outside its box: {:?}",
        img.get_pixel(w * 7 / 8, h / 2)
    );
    assert!(
        server.shared.state().video_input.is_none(),
        "a fill is not the video-input layer"
    );
    // The newest picture follows the source.
    color.store(0x2040D0, Ordering::Relaxed);
    wait_frame(&server, 0, "the camera's blue in the box", |i| {
        px_is(i, i.width() / 8, i.height() / 2, [0x20, 0x40, 0xD0], 40)
    })
    .await;
    stop.store(true, Ordering::Relaxed);
    cam.join().unwrap();
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// `triggerVideoInput`: a logical input goes live through this machine's binding, with its
/// name and ID in the state; the source's alpha plane composites (half-transparent over the
/// screen's black), and an input set to ignore alpha draws it opaque. A playlist's
/// video-input item works the same; unknown and unbound inputs are refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_video_input_goes_live_through_its_binding() {
    let mut fx = fixture("live-input");
    let color = Arc::new(AtomicU32::new(0xD02020));
    let stop = Arc::new(AtomicBool::new(false));
    // Half transparent.
    let (cam, port) = camera_at(
        &format!("Key Cam {}", std::process::id()),
        color.clone(),
        128,
        stop.clone(),
    );
    let source = format!("127.0.0.1:{port}");
    let input = |name: &str, alpha| model::VideoInput {
        name: name.into(),
        alpha,
        ..model::VideoInput::default()
    };
    let keyed = input("Camera 1", model::SourceAlpha::Auto);
    let opaque = input("Camera 1 opaque", model::SourceAlpha::Opaque);
    let unbound = input("Spare", model::SourceAlpha::Auto);
    for v in [&keyed, &opaque, &unbound] {
        write_doc(&fx.dir, "portable", "videoInput", &v.id.0.to_string(), v);
    }
    for v in [&keyed, &opaque] {
        let b = model::InputBinding {
            input: v.id,
            source: model::InputSource::Idn {
                source_name: source.clone(),
            },
            ..model::InputBinding::default()
        };
        write_doc(&fx.dir, "machine", "inputBinding", &v.id.0.to_string(), &b);
    }
    // A playlist item that puts the opaque input live.
    let item = model::PlaylistItem::new(model::PlaylistItemKind::VideoInput(
        model::VideoInputTrigger {
            input: model::Link::to(opaque.id),
            display: model::MediaDisplay::default(),
            audio: Default::default(),
            transition: None,
        },
    ));
    if let model::PlaylistNodeKind::Playlist { items, .. } = &mut fx.playlist.kind {
        items.push(item.clone());
    }
    write_doc(
        &fx.dir,
        "portable",
        "playlistNode",
        &fx.playlist.id.0.to_string(),
        &fx.playlist,
    );
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;

    // Alpha 128 of 0xD0 red over black: clearly darker than the opaque red (about 135–150 in
    // sRGB, blended in linear light), not drawn opaque nor dropped.
    command(
        addr,
        json!({"type": "triggerVideoInput", "input": keyed.id.0.to_string()}),
    )
    .await;
    let snap = get_json(addr, "/api/v1/state").await;
    let layer = &snap["state"]["layers"]["videoInput"];
    assert_eq!(layer["input"], json!(keyed.id.0.to_string()));
    assert_eq!(layer["name"], json!("Camera 1"));
    assert_eq!(layer["source"], json!(source));
    let img = wait_frame(&server, 0, "the keyed camera over black", |i| {
        let p = i.get_pixel(4, 4).0;
        (100..185).contains(&p[0]) && p[1] < 50 && p[2] < 50
    })
    .await;
    eprintln!("keyed: {:?}", img.get_pixel(4, 4));

    // The same source through an input that ignores alpha: opaque.
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "triggerPlaylistItem",
            "playlist": fx.playlist.id.0.to_string(), "item": item.id.0.to_string()})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(
        server.shared.state().video_input.as_ref().unwrap().input,
        Some(opaque.id)
    );
    wait_frame(&server, 0, "the camera drawn opaque", |i| {
        px_is(i, 4, 4, [0xD0, 0x20, 0x20], 22)
    })
    .await;

    // Refused: an unknown input, an input without a binding here.
    for (id, status) in [(model::VideoInputId::new(), 404), (unbound.id, 409)] {
        let r = http(
            addr,
            "POST",
            "/api/v1/commands",
            &[],
            Some(json!({"type": "triggerVideoInput", "input": id.0.to_string()})),
        )
        .await;
        assert_eq!(r.status, status, "{}", String::from_utf8_lossy(&r.body));
    }
    stop.store(true, Ordering::Relaxed);
    cam.join().unwrap();
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// An H.264 source (here an IDN output forcing compressed video from this machine's CPU
/// encoder, as an NDI|HX camera sends it) is decoded through the system FFmpeg and shown.
/// Skipped where FFmpeg has no H.264 encoder.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_h264_source_is_decoded() {
    use idn_open_protocol::api::{Output, OutputConfig, Publisher, PublisherConfig, VideoFrame};
    use idn_open_protocol::codec::{CodecOutput, CodecPolicy, EncoderFactory};
    use idn_open_protocol::node::video::{Format, Processor};
    // The process's FFmpeg (no second probe and its Vulkan instance: M-4).
    let ff = media::machine_ffmpeg().for_output(Processor::Cpu, Format::H264);
    if ff.codecs().is_empty() {
        eprintln!("skipped: FFmpeg has no CPU H.264 encoder here");
        return;
    }
    let fx = fixture("live-h264");
    let publisher = Publisher::new(PublisherConfig::default()).unwrap();
    let mut cfg = OutputConfig::new(&format!("HX Cam {}", std::process::id()));
    cfg.codec = Some(CodecOutput {
        policy: CodecPolicy::Force,
        ..CodecOutput::new(Arc::new(ff))
    });
    let out = Arc::new(Output::new(&publisher, cfg).unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let sender = {
        let (out, stop) = (out.clone(), stop.clone());
        std::thread::spawn(move || {
            let (w, h) = (320usize, 180usize);
            let px: Arc<[u8]> = [0x20u8, 0xB0, 0x30, 255].repeat(w * h).into();
            while !stop.load(Ordering::Relaxed) {
                let _ = out.send(VideoFrame::rgba(
                    w,
                    h,
                    px.clone(),
                    idn_open_protocol::color::AlphaMode::Opaque,
                ));
                std::thread::sleep(Duration::from_millis(33));
            }
        })
    };
    let server = start_server(&fx, true, false).await;
    let source = format!("127.0.0.1:{}", out.port());
    command(
        server.addr,
        json!({"type": "triggerLiveSource", "source": source}),
    )
    .await;
    wait_frame(&server, 0, "the H.264 camera's green", |i| {
        corner_is(i, [0x20, 0xB0, 0x30])
    })
    .await;
    let st = out.stats();
    eprintln!("H.264 source: {} pictures encoded", st.codec_encoded);
    assert!(st.codec_encoded > 0, "the source sent SpeedHQ, not H.264");
    stop.store(true, Ordering::Relaxed);
    sender.join().unwrap();
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// "Video input X is live" visibility conditions follow the video-input layer: an element
/// shown while the input is live appears when it goes live (by ID, or by its bound source),
/// one shown while it isn't disappears, and both swap back when the layer clears. No camera
/// is needed: the condition is about the layer, not the picture.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn video_input_conditions_follow_the_live_input() {
    use model::{Presentation, Rect, Section, Slide, VisibilityCondition};
    let fx = fixture("live-input-cond");
    let source = "127.0.0.1:9".to_string();
    let cam = model::VideoInput {
        name: "Camera 1".into(),
        ..model::VideoInput::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "videoInput",
        &cam.id.0.to_string(),
        &cam,
    );
    let binding = model::InputBinding {
        input: cam.id,
        source: model::InputSource::Idn {
            source_name: source.clone(),
        },
        ..model::InputBinding::default()
    };
    write_doc(
        &fx.dir,
        "machine",
        "inputBinding",
        &cam.id.0.to_string(),
        &binding,
    );
    // Green on the left while the input is live, blue on the right while it isn't.
    let shape = |x: f64, c: model::Color, active: bool| {
        let mut e = model::Element::new(Rect::new(x, 0.0, 960.0, 1080.0));
        e.fill = model::Fill::color(c);
        e.visibility = vec![VisibilityCondition::VideoInput {
            input: model::Link::to(cam.id),
            active,
        }];
        e
    };
    let mut slide = Slide::new();
    slide.elements = vec![
        shape(0.0, model::Color::rgb(0.0, 1.0, 0.0), true),
        shape(960.0, model::Color::rgb(0.0, 0.0, 1.0), false),
    ];
    let mut sec = Section::new("Camera cue");
    sec.slides = vec![slide];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Camera cue")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    command(
        addr,
        json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}),
    )
    .await;
    command(
        addr,
        json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 0}),
    )
    .await;
    let off = |i: &image::RgbaImage| {
        let (w, h) = (i.width(), i.height());
        px_is(i, w * 3 / 4, h / 2, [0, 0, 0xFF], 30) && px_is(i, w / 4, h / 2, [0, 0, 0], 30)
    };
    let on = |i: &image::RgbaImage| {
        let (w, h) = (i.width(), i.height());
        px_is(i, w / 4, h / 2, [0, 0xFF, 0], 30) && px_is(i, w * 3 / 4, h / 2, [0, 0, 0], 30)
    };
    wait_frame(&server, 0, "the not-live element only", off).await;

    command(
        addr,
        json!({"type": "triggerVideoInput", "input": cam.id.0.to_string()}),
    )
    .await;
    wait_frame(&server, 0, "the live element once the input is live", on).await;

    command(addr, json!({"type": "clearLayer", "layer": "videoInput"})).await;
    wait_frame(&server, 0, "the not-live element after clearing", off).await;

    // The bound source triggered by address is the same input.
    command(addr, json!({"type": "triggerLiveSource", "source": source})).await;
    wait_frame(&server, 0, "the live element for its bound source", on).await;
    command(addr, json!({"type": "clearLayer", "layer": "videoInput"})).await;
    wait_frame(&server, 0, "the not-live element again", off).await;

    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// `/video-inputs`: logical inputs with this box's binding. Create (a default "Camera N" name;
/// a name and a source), rename and bind with `If-Match` (a stale revision is refused), a
/// live input follows its new binding and goes off the layer when unbound, bad names and
/// sources are refused, everything survives a restart, and delete removes the input and its
/// binding.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn video_inputs_crud_and_bindings() {
    let fx = fixture("live-input-crud");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let list = get_json(addr, "/api/v1/video-inputs").await;
    assert_eq!(list, json!([]));

    let r = http(addr, "POST", "/api/v1/video-inputs", &[], Some(json!({}))).await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let first = r.json();
    assert_eq!(first["name"], json!("Camera 1"));
    assert!(first.get("source").is_none());
    let id = first["id"].as_str().unwrap().to_string();

    let r = http(
        addr,
        "POST",
        "/api/v1/video-inputs",
        &[],
        Some(
            json!({"name": "  Pulpit  ", "source": {"type": "idn", "sourceName": " 127.0.0.1:9 "}}),
        ),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let pulpit = r.json();
    assert_eq!(pulpit["name"], json!("Pulpit"));
    assert_eq!(
        pulpit["source"],
        json!({"type": "idn", "sourceName": "127.0.0.1:9"})
    );
    let list = get_json(addr, "/api/v1/video-inputs").await;
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Camera 1", "Pulpit"]);

    // Rename and bind with the current revision; the old one is refused afterwards.
    let rev = first["revision"].as_str().unwrap().to_string();
    let mut body = first.clone();
    body["name"] = json!("Wide");
    body["source"] = json!({"type": "idn", "sourceName": "127.0.0.1:10"});
    let path = format!("/api/v1/video-inputs/{id}");
    let r = http(
        addr,
        "PUT",
        &path,
        &[("If-Match", &rev)],
        Some(body.clone()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let wide = r.json();
    assert_eq!(wide["name"], json!("Wide"));
    assert_ne!(wide["revision"], json!(rev));
    let r = http(
        addr,
        "PUT",
        &path,
        &[("If-Match", &rev)],
        Some(body.clone()),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        fx.dir
            .join("machine/inputBinding")
            .join(format!("{id}.json"))
            .exists()
    );

    // Live: follows its binding; unbinding takes it off the layer.
    command(addr, json!({"type": "triggerVideoInput", "input": id})).await;
    let layer = |s: serde_json::Value| s["state"]["layers"]["videoInput"].clone();
    assert_eq!(
        layer(get_json(addr, "/api/v1/state").await)["source"],
        json!("127.0.0.1:10")
    );
    let rev = wide["revision"].as_str().unwrap().to_string();
    body["source"] = json!({"type": "idn", "sourceName": "127.0.0.1:11"});
    let r = http(
        addr,
        "PUT",
        &path,
        &[("If-Match", &rev)],
        Some(body.clone()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let l = layer(get_json(addr, "/api/v1/state").await);
    assert_eq!(l["source"], json!("127.0.0.1:11"));
    assert_eq!(l["name"], json!("Wide"));
    body["source"] = serde_json::Value::Null;
    let r = http(addr, "PUT", &path, &[], Some(body.clone())).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert!(r.json().get("source").is_none());
    assert!(layer(get_json(addr, "/api/v1/state").await).is_null());
    assert!(
        !fx.dir
            .join("machine/inputBinding")
            .join(format!("{id}.json"))
            .exists()
    );

    // Refused: an empty or too long name, an empty source, an unknown input.
    for bad in [
        json!({"name": "x".repeat(65)}),
        json!({"name": "Ok", "source": {"type": "idn", "sourceName": "  "}}),
        json!({"name": "Ok", "source": {"type": "carrier-pigeon"}}),
    ] {
        let r = http(addr, "POST", "/api/v1/video-inputs", &[], Some(bad.clone())).await;
        assert_eq!(r.status, 400, "{bad}: {}", String::from_utf8_lossy(&r.body));
    }
    let mut blank = body.clone();
    blank["name"] = json!(" ");
    let r = http(addr, "PUT", &path, &[], Some(blank)).await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    let unknown = format!("/api/v1/video-inputs/{}", model::VideoInputId::new().0);
    assert_eq!(http(addr, "GET", &unknown, &[], None).await.status, 404);
    assert_eq!(http(addr, "DELETE", &unknown, &[], None).await.status, 404);

    // A restart reads them back.
    server.shutdown().await.unwrap();
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let list = get_json(addr, "/api/v1/video-inputs").await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    let p = &list[1];
    assert_eq!(p["name"], json!("Pulpit"));
    assert_eq!(p["source"]["sourceName"], json!("127.0.0.1:9"));

    // Delete: a stale revision is refused; then the input and its binding go.
    let pid = p["id"].as_str().unwrap().to_string();
    let ppath = format!("/api/v1/video-inputs/{pid}");
    let r = http(addr, "DELETE", &ppath, &[("If-Match", "0000")], None).await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    let rev = p["revision"].as_str().unwrap().to_string();
    let r = http(addr, "DELETE", &ppath, &[("If-Match", &rev)], None).await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(http(addr, "GET", &ppath, &[], None).await.status, 404);
    for section in ["portable/videoInput", "machine/inputBinding"] {
        assert!(!fx.dir.join(section).join(format!("{pid}.json")).exists());
    }
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "triggerVideoInput", "input": pid})),
    )
    .await;
    assert_eq!(r.status, 404);
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// F02-2 / F02-3: a live logical input renamed through `/video-inputs` shows its new name on
/// the layer; restored after a clear, it comes back on its binding as it is now (rebound: the
/// new source), and not at all once unbound.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restored_video_input_follows_its_binding_and_name() {
    let fx = fixture("live-input-restore");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let r = http(
        addr,
        "POST",
        "/api/v1/video-inputs",
        &[],
        Some(json!({"name": "Camera 1", "source": {"type": "idn", "sourceName": "127.0.0.1:10"}})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let mut body = r.json();
    let id = body["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/video-inputs/{id}");
    let layer =
        || async { get_json(addr, "/api/v1/state").await["state"]["layers"]["videoInput"].clone() };
    let put = |body: serde_json::Value| {
        let path = path.clone();
        async move {
            let r = http(addr, "PUT", &path, &[], Some(body)).await;
            assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        }
    };
    command(addr, json!({"type": "triggerVideoInput", "input": id})).await;
    assert_eq!(layer().await["name"], json!("Camera 1"));
    // Renamed while live: the layer shows the new name.
    body["name"] = json!("Pulpit");
    put(body.clone()).await;
    let l = layer().await;
    assert_eq!(l["name"], json!("Pulpit"));
    assert_eq!(l["source"], json!("127.0.0.1:10"));
    // Cleared, then rebound: restored on the new source.
    command(addr, json!({"type": "clearLayer", "layer": "videoInput"})).await;
    assert!(layer().await.is_null());
    body["source"] = json!({"type": "idn", "sourceName": "127.0.0.1:11"});
    put(body.clone()).await;
    command(addr, json!({"type": "restoreLayer", "layer": "videoInput"})).await;
    let l = layer().await;
    assert_eq!(l["source"], json!("127.0.0.1:11"));
    assert_eq!(l["name"], json!("Pulpit"));
    // Cleared, then unbound: nothing to restore.
    command(addr, json!({"type": "clearLayer", "layer": "videoInput"})).await;
    body["source"] = serde_json::Value::Null;
    put(body.clone()).await;
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "restoreLayer", "layer": "videoInput"})),
    )
    .await;
    assert_eq!(r.status, 404, "{}", String::from_utf8_lossy(&r.body));
    assert!(layer().await.is_null());
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}
