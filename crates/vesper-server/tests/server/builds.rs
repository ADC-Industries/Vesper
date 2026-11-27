//! Object builds (G18): `next` takes a slide's build steps before the next slide, `prev` takes
//! them back, the show state reports the progress, a build dissolves on the screen clock
//! without a layer transition, and `setSlide` stores and validates the build order.

use crate::common::*;
use model::{Build, BuildMode, BuildStart, Presentation, Section, Slide, Transition};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use vesper_server::RunningServer;

/// Frames of screen 0 as they are published, for `dur`.
async fn record(server: &RunningServer, dur: Duration) -> Vec<(Instant, image::RgbaImage)> {
    let mut out = Vec::new();
    let mut last = None;
    let end = Instant::now() + dur;
    while Instant::now() < end {
        let f = server.shared.screens()[0].latest.lock().unwrap().clone();
        if let Some(f) = f
            && Some(f.sequence) != last
        {
            last = Some(f.sequence);
            out.push((Instant::now(), f.to_straight_rgba()));
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    out
}

/// A filled rectangle element of colour `c` over a third of the 1920×1080 canvas.
fn block(third: u32, c: [f64; 3]) -> model::Element {
    serde_json::from_value(json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "bounds": {"x": 640.0 * third as f64, "y": 0.0, "width": 640.0, "height": 1080.0},
        "kind": {"type": "shape", "shape": {"type": "rectangle"}},
        "fill": {"enabled": true, "kind": {"type": "color", "color": {"r": c[0], "g": c[1], "b": c[2]}}}
    }))
    .unwrap()
}

/// Mean of one colour channel over the middle of third `t` of the screen.
fn level(img: &image::RgbaImage, t: u32, ch: usize) -> f64 {
    let (x0, x1) = (W / 3 * t + 20, W / 3 * (t + 1) - 20);
    let (y0, y1) = (H / 3, H * 2 / 3);
    let mut sum = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            sum += img.get_pixel(x, y).0[ch] as f64;
        }
    }
    sum / ((x1 - x0) * (y1 - y0)) as f64
}

fn build(state: &Value) -> &Value {
    &state["build"]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn next_steps_through_builds_then_slides() {
    let fx = fixture("builds");
    // Red always shown; green builds in (dissolve), then blue builds out (cut).
    let red = block(0, [1.0, 0.0, 0.0]);
    let green = block(1, [0.0, 1.0, 0.0]);
    let blue = block(2, [0.0, 0.0, 1.0]);
    let mut s1 = Slide::new();
    s1.builds = vec![
        Build::new(green.id, BuildMode::In, Transition::dissolve(0.6)),
        Build::new(blue.id, BuildMode::Out, Transition::CUT),
    ];
    let green_id = green.id;
    s1.elements = vec![red, green, blue];
    let s2 = Slide::new();
    let mut sec = Section::new("Blocks");
    sec.slides = vec![s1.clone(), s2];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Build test")
    };
    let pid = p.id.0.to_string();
    write_doc(&fx.dir, "portable", "presentation", &pid, &p);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 0}))
        .await;
    assert_eq!(build(&a.state)["step"], 0, "{}", build(&a.state));
    assert_eq!(build(&a.state)["steps"], 2);
    assert_eq!(build(&a.state)["slide"], s1.id.0.to_string());
    // Step 0: red and blue, no green.
    wait_frame(&server, 0, "red and blue, no green", |img| {
        level(img, 0, 0) > 200.0 && level(img, 1, 1) < 30.0 && level(img, 2, 2) > 200.0
    })
    .await;
    // The cursor (stage "next") is the next slide, not the next build step.
    assert_eq!(a.state["cursor"]["slideIndex"], 1);

    // Next: green dissolves in on the screen clock; the slide stays (no layer change).
    let start = Instant::now();
    a.ok(json!({"type": "next"})).await;
    assert_eq!(build(&a.state)["step"], 1);
    assert_eq!(a.state["layers"]["slide"]["slideIndex"], 0);
    let frames = record(&server, Duration::from_millis(1000)).await;
    let greens: Vec<(f64, f64)> = frames
        .iter()
        .map(|(t, f)| ((*t - start).as_secs_f64(), level(f, 1, 1)))
        .collect();
    assert!(greens.len() >= 8, "{} frames: {greens:?}", greens.len());
    assert!(
        greens.windows(2).all(|w| w[1].1 >= w[0].1 - 3.0),
        "green only grows: {greens:?}"
    );
    assert!(
        greens.iter().any(|g| g.1 > 40.0 && g.1 < 215.0),
        "a frame mid-dissolve: {greens:?}"
    );
    let last = greens.last().unwrap();
    assert!(last.1 > 240.0, "green in full: {greens:?}");
    assert!(
        (0.45..0.9).contains(&last.0),
        "renders stop once it settles: {greens:?}"
    );
    // Red stayed put the whole time (no layer dissolve).
    assert!(frames.iter().all(|(_, f)| level(f, 0, 0) > 240.0));

    // Next: blue cuts out.
    a.ok(json!({"type": "next"})).await;
    assert_eq!(build(&a.state)["step"], 2);
    wait_frame(&server, 0, "blue gone", |img| level(img, 2, 2) < 30.0).await;
    // Prev: blue comes back (the step before, settled).
    a.ok(json!({"type": "prev"})).await;
    assert_eq!(build(&a.state)["step"], 1);
    wait_frame(&server, 0, "blue back", |img| {
        level(img, 2, 2) > 240.0 && level(img, 1, 1) > 240.0
    })
    .await;
    a.ok(json!({"type": "next"})).await;
    // Past the last step: the next slide, which has no builds.
    a.ok(json!({"type": "next"})).await;
    assert_eq!(a.state["layers"]["slide"]["slideIndex"], 1);
    assert!(build(&a.state).is_null());
    // Back to the first slide: it starts over at step 0.
    a.ok(json!({"type": "prev"})).await;
    assert_eq!(a.state["layers"]["slide"]["slideIndex"], 0);
    assert_eq!(build(&a.state)["step"], 0);
    // Clearing the slide layer ends the builds: `next` moves to the next slide.
    a.ok(json!({"type": "clearLayer", "layer": "slide"})).await;
    assert!(build(&a.state).is_null());
    a.ok(json!({"type": "next"})).await;
    assert_eq!(a.state["layers"]["slide"]["slideIndex"], 1);

    // setSlide stores the order; it refuses a build that names no element ID, and too many
    // builds (a build of an element the slide doesn't have is kept and skipped).
    let sid = s1.id.0.to_string();
    let ghost = json!({"element": uuid::Uuid::new_v4().to_string()});
    for bad in [
        json!([{"element": "not an id"}]),
        Value::Array(vec![ghost; 1001]),
    ] {
        let r = http(
            addr,
            "POST",
            &format!("/api/v1/presentations/{pid}/edits"),
            &[],
            Some(json!({"clientId": "ed", "ops": [{"type": "setSlide", "slide": sid, "builds": bad}]})),
        )
        .await;
        assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    }
    let one = Build {
        start: BuildStart::WithSlide,
        delay: 0.25,
        ..Build::new(green_id, BuildMode::In, Transition::dissolve(1.0))
    };
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{pid}/edits"),
        &[],
        Some(
            json!({"clientId": "ed", "ops": [{"type": "setSlide", "slide": sid, "builds": [one]}]}),
        ),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let doc = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let stored = &doc["sections"][0]["slides"][0]["builds"];
    assert_eq!(
        stored,
        &json!([{"element": green_id.0.to_string(), "start": "withSlide", "delay": 0.25,
                 "transition": {"kind": {"type": "dissolve"}, "duration": 1.0}}])
    );
    server.shutdown().await.unwrap();
}

/// Posts an edit batch to presentation `pid`; panics unless it is applied.
async fn edit(addr: std::net::SocketAddr, pid: &str, ops: Value) {
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{pid}/edits"),
        &[],
        Some(json!({"clientId": "ed", "ops": ops})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
}

/// D01-1: triggering a slide on the announcements layer leaves the build progress of the
/// slide live on the slide layer alone (the elements it revealed stay on screen).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slide_on_the_other_layer_keeps_the_build_progress() {
    let fx = fixture("builds-layers");
    let red = block(0, [1.0, 0.0, 0.0]);
    let green = block(1, [0.0, 1.0, 0.0]);
    let mut s1 = Slide::new();
    s1.builds = vec![Build::new(green.id, BuildMode::In, Transition::CUT)];
    s1.elements = vec![red, green];
    let mut sec = Section::new("Points");
    sec.slides = vec![s1, Slide::new()];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Build points")
    };
    // An announcement: a blue block on the right third.
    let mut ann = Slide::new();
    ann.elements = vec![block(2, [0.0, 0.0, 1.0])];
    let mut asec = Section::new("Notice");
    asec.slides = vec![ann];
    let a_p = Presentation {
        library: Some(fx.library.id),
        sections: vec![asec],
        destination: model::ContentDestination::Announcements,
        ..Presentation::new("Notice board")
    };
    let pid = p.id.0.to_string();
    let aid = a_p.id.0.to_string();
    write_doc(&fx.dir, "portable", "presentation", &pid, &p);
    write_doc(&fx.dir, "portable", "presentation", &aid, &a_p);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 0}))
        .await;
    a.ok(json!({"type": "next"})).await;
    assert_eq!(build(&a.state)["step"], 1, "{}", build(&a.state));
    wait_frame(&server, 0, "green built in", |img| level(img, 1, 1) > 200.0).await;

    a.ok(json!({"type": "triggerSlide", "presentation": aid, "index": 0}))
        .await;
    wait_frame(&server, 0, "the announcement over the built slide", |img| {
        level(img, 2, 2) > 200.0 && level(img, 1, 1) > 200.0 && level(img, 0, 0) > 200.0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let img = server.shared.screens()[0]
        .latest
        .lock()
        .unwrap()
        .clone()
        .unwrap()
        .to_straight_rgba();
    assert!(level(&img, 1, 1) > 200.0, "green stays built in");
    // The position is on the announcement (no builds): `build` reports its builds.
    assert!(build(&a.state).is_null(), "{}", build(&a.state));
    server.shutdown().await.unwrap();
}

/// D01-3: the number of build steps follows edits of the live slide: removed builds take no
/// dead `next` presses, builds added to a live slide that had none can be stepped.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn editing_the_live_slides_builds_changes_its_steps() {
    let fx = fixture("builds-edit");
    let red = block(0, [1.0, 0.0, 0.0]);
    let green = block(1, [0.0, 1.0, 0.0]);
    let blue = block(2, [0.0, 0.0, 1.0]);
    let mut s1 = Slide::new();
    s1.builds = vec![
        Build::new(red.id, BuildMode::In, Transition::CUT),
        Build::new(green.id, BuildMode::In, Transition::CUT),
        Build::new(blue.id, BuildMode::In, Transition::CUT),
    ];
    let green_id = green.id;
    s1.elements = vec![red, green, blue];
    let white = block(0, [1.0, 1.0, 1.0]);
    let white_id = white.id;
    let mut s2 = Slide::new();
    s2.elements = vec![white];
    let (s1_id, s2_id) = (s1.id.0.to_string(), s2.id.0.to_string());
    let mut sec = Section::new("Points");
    sec.slides = vec![s1, s2];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Build edits")
    };
    let pid = p.id.0.to_string();
    write_doc(&fx.dir, "portable", "presentation", &pid, &p);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 0}))
        .await;
    assert_eq!(build(&a.state)["steps"], 3);

    // Two of the three builds are deleted while the slide is live, and a build of an element
    // the slide doesn't have (a deleted object's) comes with the order: it takes no step.
    let one = Build::new(green_id, BuildMode::In, Transition::CUT);
    let ghost = Build::new(model::ElementId::new(), BuildMode::In, Transition::CUT);
    edit(
        addr,
        &pid,
        json!([{"type": "setSlide", "slide": s1_id, "builds": [ghost, one]}]),
    )
    .await;
    a.ok(json!({"type": "next"})).await;
    assert_eq!(build(&a.state)["step"], 1, "{}", build(&a.state));
    assert_eq!(build(&a.state)["steps"], 1);
    wait_frame(&server, 0, "green in", |img| level(img, 1, 1) > 200.0).await;
    // No dead presses: the next one goes to the next slide.
    a.ok(json!({"type": "next"})).await;
    assert_eq!(a.state["layers"]["slide"]["slideIndex"], 1);
    assert!(build(&a.state).is_null());
    wait_frame(&server, 0, "the white slide", |img| {
        level(img, 0, 0) > 240.0
    })
    .await;

    // A build added to the live slide: its element waits for `next`, which reveals it.
    let add = Build::new(white_id, BuildMode::In, Transition::CUT);
    edit(
        addr,
        &pid,
        json!([{"type": "setSlide", "slide": s2_id, "builds": [add]}]),
    )
    .await;
    wait_frame(&server, 0, "white waits for its build", |img| {
        level(img, 0, 0) < 30.0
    })
    .await;
    a.ok(json!({"type": "next"})).await;
    assert_eq!(a.state["layers"]["slide"]["slideIndex"], 1);
    assert_eq!(build(&a.state)["step"], 1, "{}", build(&a.state));
    assert_eq!(build(&a.state)["steps"], 1);
    wait_frame(&server, 0, "white built in", |img| level(img, 0, 0) > 240.0).await;
    server.shutdown().await.unwrap();
}

/// X3-1: a new value of live data (a running timer, a new value every second) keeps the
/// slide's clock: a 2.5 s step-0 dissolve plays through instead of starting over each second.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_data_keeps_the_slides_clock() {
    let fx = fixture("builds-live-data");
    let countdown = model::Timer {
        name: "Count".into(),
        sort_key: "a".into(),
        kind: model::TimerKind::Countdown { seconds: 120.0 },
        ..model::Timer::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "timer",
        &countdown.id.0.to_string(),
        &countdown,
    );
    let mut clock = text_element(
        model::Rect::new(0.0, 900.0, 600.0, 150.0),
        "9:99",
        model::Color::WHITE,
    );
    clock.data_link = Some(model::DataLink::Timer {
        timer: Some(model::Link::to(countdown.id)),
        format: model::TimerFormat::default(),
        color_triggers: Vec::new(),
        show: Default::default(),
    });
    let green = block(1, [0.0, 1.0, 0.0]);
    let mut s1 = Slide::new();
    s1.builds = vec![Build {
        start: BuildStart::WithSlide,
        ..Build::new(green.id, BuildMode::In, Transition::dissolve(2.5))
    }];
    s1.elements = vec![green, clock];
    let mut sec = Section::new("Countdown");
    sec.slides = vec![s1];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Countdown")
    };
    let pid = p.id.0.to_string();
    write_doc(&fx.dir, "portable", "presentation", &pid, &p);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "timerStart", "timer": countdown.id}))
        .await;
    let start = Instant::now();
    a.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 0}))
        .await;
    let frames = record(&server, Duration::from_millis(3500)).await;
    let greens: Vec<(f64, f64)> = frames
        .iter()
        .map(|(t, f)| ((*t - start).as_secs_f64(), level(f, 1, 1)))
        .collect();
    assert!(
        greens.windows(2).all(|w| w[1].1 >= w[0].1 - 3.0),
        "green only grows: {greens:?}"
    );
    assert!(
        greens.last().is_some_and(|g| g.1 > 240.0),
        "green in full: {greens:?}"
    );
    server.shutdown().await.unwrap();
}
