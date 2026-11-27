//! M4.2 on a running server: stage screens render their stage layout (current / next text, a
//! slide picture, clock and timer) and never the audience layers, re-render at most once a
//! second for clocks and timers, show the stage message (banner, flash), switch layouts per
//! screen (stored in the screen document), follow the stage on/off toggle, and the stage
//! layout REST CRUD with `If-Match`.

use crate::common::*;
use model::{
    Color, DataLink, Element, Link, Rect, Screen, ScreenKind, Size, Slide, SlideTextSource,
    SlideWhich, StageLayout, Timer, TimerFormat, TimerKind,
};
use serde_json::json;
use std::time::{Duration, Instant};

const YELLOW: [u8; 3] = [255, 255, 0];
const GREEN: [u8; 3] = [0, 255, 0];
/// The stage screen's index among the running screens (Audience, Key, Stage).
const STAGE: usize = 2;

struct Stage {
    screen: Screen,
    main: StageLayout,
    green: StageLayout,
    timer: Timer,
}

fn sized(mut e: Element, size: f64) -> Element {
    if let Some(tb) = e.text_box_mut() {
        tb.default_style.size = Some(size);
    }
    e
}

/// A stage screen and two layouts: "Main" (current text in white across the top, next text in
/// yellow on the left, a picture of the current slide on the right, timer and clock along the
/// bottom) and "Green" (a full green rectangle).
fn stage_fixture(fx: &Fixture) -> Stage {
    let screen = Screen {
        name: "Stage".into(),
        sort_key: "c".into(),
        size: Size::new(W as f64, H as f64),
        frame_rate: model::FrameRate::FPS30,
        kind: ScreenKind::Stage { layout: None },
        ..Screen::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "screen",
        &screen.id.0.to_string(),
        &screen,
    );
    let timer = Timer {
        name: "Service".into(),
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

    let link = |mut e: Element, l: DataLink| {
        e.data_link = Some(l);
        e
    };
    let current = link(
        sized(
            text_element(Rect::new(0.0, 0.0, 640.0, 110.0), "Current", Color::WHITE),
            80.0,
        ),
        DataLink::SlideText {
            which: SlideWhich::Current,
            source: SlideTextSource::AllText,
            keep_formatting: false,
        },
    );
    let next = link(
        sized(
            text_element(
                Rect::new(0.0, 120.0, 320.0, 110.0),
                "Next",
                Color::rgb(1.0, 1.0, 0.0),
            ),
            70.0,
        ),
        DataLink::SlideText {
            which: SlideWhich::Next,
            source: SlideTextSource::AllText,
            keep_formatting: false,
        },
    );
    let mut picture = Element::new(Rect::new(330.0, 120.0, 300.0, 169.0));
    picture.name = "Picture".into();
    picture.data_link = Some(DataLink::SlideImage {
        which: SlideWhich::Current,
    });
    let timer_el = link(
        sized(
            text_element(Rect::new(0.0, 295.0, 320.0, 65.0), "00:00", Color::WHITE),
            50.0,
        ),
        DataLink::Timer {
            timer: Some(Link::to(timer.id)),
            format: TimerFormat::default(),
            color_triggers: vec![],
            show: Default::default(),
        },
    );
    let clock = link(
        sized(
            text_element(Rect::new(330.0, 295.0, 300.0, 65.0), "12:00", Color::WHITE),
            50.0,
        ),
        DataLink::Clock {
            format: model::ClockFormat::default(),
        },
    );
    let main = StageLayout {
        name: "Main".into(),
        sort_key: "a".into(),
        size: Size::new(W as f64, H as f64),
        slide: Slide {
            elements: vec![current, next, picture, timer_el, clock],
            ..Slide::new()
        },
        ..StageLayout::default()
    };
    let mut g = Element::new(Rect::new(0.0, 0.0, W as f64, H as f64));
    g.fill = model::Fill::color(Color::rgb(0.0, 1.0, 0.0));
    let green = StageLayout {
        name: "Green".into(),
        sort_key: "b".into(),
        size: Size::new(W as f64, H as f64),
        slide: Slide {
            elements: vec![g],
            ..Slide::new()
        },
        ..StageLayout::default()
    };
    for l in [&main, &green] {
        write_doc(&fx.dir, "portable", "stageLayout", &l.id.0.to_string(), l);
    }
    Stage {
        screen,
        main,
        green,
        timer,
    }
}

/// Opaque pixels like `c` in the rectangle `x0..x1` × `y0..y1`.
fn count_in(
    img: &image::RgbaImage,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    c: [u8; 3],
    tol: i32,
) -> usize {
    (y0..y1.min(img.height()))
        .flat_map(|y| (x0..x1.min(img.width())).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = img.get_pixel(x, y);
            p.0[3] > 200 && near(p, c, tol)
        })
        .count()
}

fn region(img: &image::RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            out.extend_from_slice(&img.get_pixel(x, y).0);
        }
    }
    out
}

fn renders(server: &vesper_server::RunningServer, i: usize) -> u64 {
    server.shared.screens()[i].stats.lock().unwrap().renders
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stage_screens_render_layouts_messages_and_toggles() {
    let fx = fixture("stage-render");
    let st = stage_fixture(&fx);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    assert_eq!(server.shared.screens()[STAGE].id, st.screen.id);
    let mut a = Client::connect(addr, "protocolVersion=1").await;
    assert_eq!(
        a.state["stageScreens"][0]["screen"],
        st.screen.id.0.to_string()
    );
    assert_eq!(
        a.state["stageScreens"][0]["layout"],
        st.main.id.0.to_string()
    );
    assert_eq!(a.state["stageScreens"][0]["layoutName"], "Main");
    assert_eq!(a.state["stageMessage"], serde_json::Value::Null);

    // ---- current / next, picture; no audience layers ---------------------------------------
    // Slide 0 has a blue background; a red prop would cover the audience screen.
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": fx.song.id, "index": 0}))
        .await;
    wait_frame(&server, 0, "audience shows the blue background", |i| {
        count(i, 0, H, BLUE, 20) > 1000
    })
    .await;
    let one = wait_frame(&server, STAGE, "stage: current, next and picture", |i| {
        count_in(i, 0, W, 0, 110, WHITE, 40) > 300
            && count_in(i, 0, 320, 120, 230, YELLOW, 40) > 200
            && count_in(i, 330, 630, 120, 289, WHITE, 40) > 80
    })
    .await;
    assert_eq!(
        count(&one, 0, H, BLUE, 30),
        0,
        "no background media on stage"
    );

    // The next slide: current, next and the picture change.
    a.ok(json!({"type": "next"})).await;
    let two = wait_frame(&server, STAGE, "stage after next", |i| {
        region(i, 0, W, 0, 110) != region(&one, 0, W, 0, 110)
            && region(i, 330, 630, 120, 289) != region(&one, 330, 630, 120, 289)
            && count_in(i, 330, 630, 120, 289, WHITE, 40) > 80
    })
    .await;
    assert_ne!(
        region(&two, 0, 320, 120, 230),
        region(&one, 0, 320, 120, 230),
        "the next text changed"
    );
    assert_eq!(count(&two, 0, H, BLUE, 30), 0);

    // ---- clock and timer: at most one render a second --------------------------------------
    a.ok(json!({"type": "timerStart", "timer": st.timer.id}))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let r0 = renders(&server, STAGE);
    let t0 = Instant::now();
    let before = latest(&server, STAGE).unwrap();
    while t0.elapsed() < Duration::from_millis(3300) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let n = renders(&server, STAGE) - r0;
    assert!(
        (2..=5).contains(&n),
        "{n} stage renders in 3.3 s of a running timer"
    );
    let after = latest(&server, STAGE).unwrap();
    assert_ne!(
        region(&before, 0, 320, 295, 360),
        region(&after, 0, 320, 295, 360),
        "the timer text changed"
    );
    a.ok(json!({"type": "timerStop", "timer": st.timer.id}))
        .await;

    // ---- stage message: banner along the top, flashing red, then steady --------------------
    let r = a
        .command(json!({"type": "stageMessage", "text": "  "}))
        .await;
    assert_eq!(r["error"]["code"], "badRequest");
    a.ok(json!({"type": "stageMessage", "text": "Wrap up", "flash": true}))
        .await;
    assert_eq!(a.state["stageMessage"]["text"], "Wrap up");
    assert_eq!(a.state["stageMessage"]["flash"], true);
    let banner_rows = (230.0 / 1080.0 * H as f64) as u32;
    wait_frame(&server, STAGE, "flashing banner (red)", |i| {
        count_in(i, 0, W, 0, banner_rows, [219, 0, 0], 30) > 20000
    })
    .await;
    wait_frame(&server, STAGE, "flashing banner (off phase)", |i| {
        count_in(i, 0, W, 0, banner_rows, [219, 0, 0], 30) < 5000
            && count_in(i, 0, W, 0, banner_rows, BLACK, 10) > 20000
    })
    .await;
    // The flash ends after a few seconds: black bar, white text.
    tokio::time::sleep(Duration::from_millis(4200)).await;
    let steady = latest(&server, STAGE).unwrap();
    assert!(count_in(&steady, 0, W, 0, banner_rows, [219, 0, 0], 30) < 5000);
    assert!(count_in(&steady, 0, W, 0, banner_rows, BLACK, 10) > 20000);
    assert!(count_in(&steady, 0, W, 0, banner_rows, WHITE, 40) > 100);
    // Never on the audience screen.
    let aud = latest(&server, 0).unwrap();
    assert_eq!(count_in(&aud, 0, W, 0, banner_rows, [219, 0, 0], 30), 0);
    a.ok(json!({"type": "stageMessageClear"})).await;
    assert_eq!(a.state["stageMessage"], serde_json::Value::Null);
    wait_frame(&server, STAGE, "banner gone", |i| {
        region(i, 0, W, 0, 110) == region(&two, 0, W, 0, 110)
    })
    .await;

    // ---- layout switching per screen, stored with the screen --------------------------------
    let r = a
        .command(json!({"type": "setStageLayout", "screen": fx.audience.id, "layout": st.green.id}))
        .await;
    assert_eq!(r["error"]["code"], "badRequest", "not a stage screen");
    let r = a
        .command(json!({"type": "setStageLayout", "screen": st.screen.id, "layout": model::StageLayoutId::new()}))
        .await;
    assert_eq!(r["error"]["code"], "notFound");
    a.ok(json!({"type": "setStageLayout", "screen": st.screen.id, "layout": st.green.id}))
        .await;
    a.wait_state("stage screen shows Green", |_, s| {
        s["stageScreens"][0]["layoutName"] == "Green"
    })
    .await;
    wait_frame(&server, STAGE, "green layout", |i| {
        count(i, 0, H, GREEN, 10) > (W * H) as usize * 9 / 10
    })
    .await;
    let doc = get_json(addr, &format!("/api/v1/screens/{}", st.screen.id.0)).await;
    assert_eq!(doc["kind"]["layout"], st.green.id.0.to_string());
    // The audience screen is untouched.
    assert_eq!(count(&latest(&server, 0).unwrap(), 0, H, GREEN, 10), 0);
    // Back to the default (the first layout).
    a.ok(json!({"type": "setStageLayout", "screen": st.screen.id, "layout": null}))
        .await;
    wait_frame(&server, STAGE, "main layout again", |i| {
        count(i, 0, H, GREEN, 10) == 0 && count_in(i, 0, W, 0, 110, WHITE, 40) > 300
    })
    .await;
    let doc = get_json(addr, &format!("/api/v1/screens/{}", st.screen.id.0)).await;
    assert!(doc["kind"].get("layout").is_none());

    // ---- the stage toggle -------------------------------------------------------------------
    a.ok(json!({"type": "setScreensEnabled", "role": "stage", "enabled": false}))
        .await;
    wait_frame(&server, STAGE, "stage off: black", |i| {
        count(i, 0, H, BLACK, 2) == (W * H) as usize
    })
    .await;
    // The audience keeps its picture.
    assert!(count(&latest(&server, 0).unwrap(), 0, H, BLUE, 30) > 1000);
    a.ok(json!({"type": "setScreensEnabled", "role": "stage", "enabled": true}))
        .await;
    wait_frame(&server, STAGE, "stage on again", |i| {
        count_in(i, 0, W, 0, 110, WHITE, 40) > 300
    })
    .await;
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stage_layout_crud_with_if_match() {
    let fx = fixture("stage-crud");
    let st = stage_fixture(&fx);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "protocolVersion=1").await;

    let list = get_json(addr, "/api/v1/stage-layouts").await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["name"], "Main");
    let tpl = get_json(addr, "/api/v1/stage-layout-templates").await;
    let ids: Vec<&str> = tpl
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["currentNext", "lyricsClockTimer", "notes", "musician"]
    );
    let png = http(
        addr,
        "GET",
        "/api/v1/stage-layout-templates/notes/thumbnail?w=160",
        &[],
        None,
    )
    .await;
    assert_eq!(png.status, 200);
    assert_eq!(&png.body[1..4], b"PNG");

    // Create from a template.
    let r = http(
        addr,
        "POST",
        "/api/v1/stage-layouts",
        &[],
        Some(json!({"template": "notes"})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let e = r.json();
    assert_eq!(e["name"], "Notes");
    assert!(e["slide"]["elements"].as_array().unwrap().len() >= 4);
    let id = e["id"].as_str().unwrap().to_string();
    let rev = e["revision"].as_str().unwrap().to_string();
    let r = http(
        addr,
        "POST",
        "/api/v1/stage-layouts",
        &[],
        Some(json!({"template": "nope"})),
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"]["field"], "template");
    // A blank one gets a name.
    let r = http(addr, "POST", "/api/v1/stage-layouts", &[], Some(json!({}))).await;
    assert_eq!(r.status, 201);
    assert_eq!(r.json()["name"], "Layout 1");

    // Update with If-Match.
    let mut body = e.clone();
    body["name"] = json!("Band notes");
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/stage-layouts/{id}"),
        &[("If-Match", "\"stale\"")],
        Some(body.clone()),
    )
    .await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["current"]["name"], "Notes");
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/stage-layouts/{id}"),
        &[("If-Match", &format!("\"{rev}\""))],
        Some(body),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let rev2 = r.json()["revision"].as_str().unwrap().to_string();
    assert_ne!(rev, rev2);
    let got = http(
        addr,
        "GET",
        &format!("/api/v1/stage-layouts/{id}"),
        &[],
        None,
    )
    .await;
    assert_eq!(got.json()["name"], "Band notes");
    assert_eq!(got.header("etag"), Some(format!("\"{rev2}\"").as_str()));

    // Edited as a one-slide document.
    let doc = get_json(addr, &format!("/api/v1/stage-layouts/{id}/document")).await;
    assert_eq!(doc["name"], "Band notes");
    let slide = doc["sections"][0]["slides"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let el = doc["sections"][0]["slides"][0]["elements"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/stage-layouts/{id}/edits"),
        &[],
        Some(json!({"ops": [{"type": "setElement", "slide": slide, "element": el, "patch": {"name": "Lyrics"}}]})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/stage-layouts/{id}/edits"),
        &[],
        Some(json!({"ops": [{"type": "addSlide", "section": id}]})),
    )
    .await;
    assert_eq!(r.status, 400, "a stage layout has one slide");
    let png = http(
        addr,
        "GET",
        &format!("/api/v1/stage-layouts/{id}/slides/{slide}/render?w=320"),
        &[],
        None,
    )
    .await;
    assert_eq!(png.status, 200);
    let png = http(
        addr,
        "GET",
        &format!("/api/v1/stage-layouts/{id}/thumbnail?w=160"),
        &[],
        None,
    )
    .await;
    assert_eq!(png.status, 200);

    // A layout chosen for the screen, then deleted: the screen falls back to the first.
    a.ok(json!({"type": "setStageLayout", "screen": st.screen.id, "layout": id}))
        .await;
    a.wait_state("chosen", |_, s| {
        s["stageScreens"][0]["layoutName"] == "Band notes"
    })
    .await;
    let cur = http(
        addr,
        "GET",
        &format!("/api/v1/stage-layouts/{id}"),
        &[],
        None,
    )
    .await
    .json();
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/stage-layouts/{id}"),
        &[("If-Match", "\"stale\"")],
        None,
    )
    .await;
    assert_eq!(r.status, 409);
    let rev3 = cur["revision"].as_str().unwrap();
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/stage-layouts/{id}"),
        &[("If-Match", &format!("\"{rev3}\""))],
        None,
    )
    .await;
    assert_eq!(r.status, 204);
    a.wait_state("fallback", |_, s| {
        s["stageScreens"][0]["layoutName"] == "Main"
    })
    .await;
    let r = http(
        addr,
        "GET",
        &format!("/api/v1/stage-layouts/{id}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 404);
    assert!(
        !content_dir(&fx.dir)
            .join("portable/stageLayout")
            .join(format!("{id}.json"))
            .exists()
    );
    let _ = st.green;
    server.shutdown().await.unwrap();
}

/// `setStageLayout` that cannot be saved (the screen folder is read-only) fails and changes
/// nothing: the layout does not show up later with an unrelated command's state.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stage_layout_that_cannot_be_saved_changes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture("stage-save-fails");
    let st = stage_fixture(&fx);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "protocolVersion=1").await;
    let screens = fx.dir.join("portable/screen");
    let set = |mode| std::fs::set_permissions(&screens, std::fs::Permissions::from_mode(mode));
    set(0o555).unwrap();
    let probe = screens.join("probe.json");
    if std::fs::write(&probe, b"{}").is_ok() {
        // Permissions don't hold (root): nothing to test.
        std::fs::remove_file(&probe).ok();
        set(0o755).unwrap();
        server.shutdown().await.unwrap();
        return;
    }
    let r = a
        .command(json!({"type": "setStageLayout", "screen": st.screen.id, "layout": st.green.id}))
        .await;
    set(0o755).unwrap();
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(a.state["stageLayouts"], json!([]), "{}", a.state);
    // Any later change publishes the state as it is: still no layout chosen.
    let before = a.revision;
    a.ok(json!({"type": "stageMessage", "text": "Wrap up"}))
        .await;
    assert!(a.revision > before);
    assert_eq!(a.state["stageLayouts"], json!([]), "{}", a.state);
    assert_ne!(a.state["stageScreens"][0]["layoutName"], "Green");
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}
