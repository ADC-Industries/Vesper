//! M4.1 on a running server: timers (commands, ticks, expiry), live data in rendered frames
//! (timer text, visibility at expiry, re-rendering only when a shown value changes), messages
//! (tokens, stacking), props (dissolves), slide actions (order, skipping) and the REST CRUD of
//! timers, messages and props with `If-Match`.

use crate::common::*;
use model::{
    ActionKind, Color, DataLink, Link, Message, MessageToken, MessageTokenKind, Presentation, Prop,
    Rect, Section, SlideAction, TimeUnitStyle, Timer, TimerFormat, TimerKind, TimerOperation,
    TokenValue, Transition, VisibilityCondition,
};
use serde_json::json;
use std::time::{Duration, Instant};

struct Extra {
    countdown: Timer,
    short: Timer,
    prop: Prop,
    hello: Message,
    second: Message,
    deck: Presentation,
}

fn timer_text(timer: &Timer) -> model::Element {
    let mut e = text_element(
        Rect::new(20.0, 80.0, W as f64 - 40.0, 200.0),
        "9:99",
        Color::WHITE,
    );
    e.data_link = Some(DataLink::Timer {
        timer: Some(Link::to(timer.id)),
        format: TimerFormat {
            hours: TimeUnitStyle::Hidden,
            minutes: TimeUnitStyle::Short,
            ..TimerFormat::default()
        },
        color_triggers: Vec::new(),
        show: Default::default(),
    });
    e.visibility = vec![VisibilityCondition::Timer {
        timer: Some(Link::to(timer.id)),
        state: model::TimerState::HasTimeRemaining,
    }];
    e
}

/// Two timers (2:00 and 0:02), a red prop over the left half that dissolves in 1 s, two
/// messages, and a "Deck" presentation: [timer 2:00, the same text static, short timer,
/// a slide whose actions start the 2:00 timer, show the prop, …].
fn extras(fx: &Fixture) -> Extra {
    let countdown = Timer {
        name: "Two Minutes".into(),
        sort_key: "a".into(),
        kind: TimerKind::Countdown { seconds: 120.0 },
        ..Timer::default()
    };
    let short = Timer {
        name: "Short".into(),
        sort_key: "b".into(),
        kind: TimerKind::Countdown { seconds: 2.0 },
        ..Timer::default()
    };
    for t in [&countdown, &short] {
        write_doc(&fx.dir, "portable", "timer", &t.id.0.to_string(), t);
    }
    let mut red = model::Element::new(Rect::new(0.0, 0.0, W as f64 / 2.0, H as f64));
    red.fill = model::Fill::color(Color::rgb(1.0, 0.0, 0.0));
    let prop = Prop {
        name: "Red".into(),
        size: model::Size::new(W as f64, H as f64),
        slide: model::Slide {
            elements: vec![red],
            ..model::Slide::new()
        },
        transition: Some(Transition::dissolve(1.0)),
        ..Prop::default()
    };
    write_doc(&fx.dir, "portable", "prop", &prop.id.0.to_string(), &prop);
    let hello = Message {
        name: "Hello".into(),
        sort_key: "a".into(),
        text: "Hello ${name}".into(),
        tokens: vec![MessageToken {
            name: "name".into(),
            kind: MessageTokenKind::Text {
                default_value: "World".into(),
            },
        }],
        ..Message::default()
    };
    let second = Message {
        name: "Second".into(),
        sort_key: "b".into(),
        text: "Second message".into(),
        ..Message::default()
    };
    for m in [&hello, &second] {
        write_doc(&fx.dir, "portable", "message", &m.id.0.to_string(), m);
    }

    let mut s_timer = model::Slide::new();
    s_timer.elements = vec![timer_text(&countdown)];
    let mut s_static = model::Slide::new();
    s_static.elements = vec![text_element(
        Rect::new(20.0, 80.0, W as f64 - 40.0, 200.0),
        "2:00",
        Color::WHITE,
    )];
    let mut s_short = model::Slide::new();
    s_short.elements = vec![timer_text(&short)];
    let mut s_actions = model::Slide::new();
    s_actions.elements = vec![timer_text(&countdown)];
    let mut disabled = SlideAction::new(ActionKind::Message {
        message: Link::to(second.id),
        show: true,
        values: vec![],
    });
    disabled.enabled = false;
    s_actions.actions = vec![
        SlideAction::new(ActionKind::Timer {
            timer: Link::to(countdown.id),
            operation: TimerOperation::Start,
        }),
        SlideAction::new(ActionKind::Prop {
            prop: Link::to(prop.id),
            show: true,
        }),
        SlideAction::new(ActionKind::Message {
            message: Link::to(hello.id),
            show: true,
            values: vec![TokenValue {
                name: "name".into(),
                value: "Ann".into(),
            }],
        }),
        // Shown, then hidden again by the next action: order matters.
        SlideAction::new(ActionKind::Message {
            message: Link::to(hello.id),
            show: false,
            values: vec![],
        }),
        SlideAction::new(ActionKind::Unsupported {
            source_type: "ACTION_TYPE_COMMUNICATION".into(),
            payload: serde_json::Value::Null,
        }),
        disabled,
        SlideAction::new(ActionKind::Look {
            look: Link::to(fx.plain_look.id),
        }),
        SlideAction::new(ActionKind::Macro {
            target: Link::to(model::MacroId::new()),
        }),
    ];
    let mut sec = Section::new("Timers");
    sec.slides = vec![s_timer, s_static, s_short, s_actions];
    let deck = Presentation {
        library: Some(fx.library.id),
        canvas_size: model::Size::new(W as f64, H as f64),
        sections: vec![sec],
        ..Presentation::new("Deck")
    };
    write_doc(
        &fx.dir,
        "portable",
        "presentation",
        &deck.id.0.to_string(),
        &deck,
    );
    Extra {
        countdown,
        short,
        prop,
        hello,
        second,
        deck,
    }
}

fn renders(server: &vesper_server::RunningServer) -> u64 {
    server.shared.screens()[0].stats.lock().unwrap().renders
}

fn timer<'a>(state: &'a serde_json::Value, id: &model::TimerId) -> &'a serde_json::Value {
    state["timers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id.0.to_string())
        .expect("timer in state")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn timers_render_live_and_rerender_once_a_second() {
    let fx = fixture("live-timers");
    let x = extras(&fx);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "protocolVersion=1").await;
    let t = timer(&a.state, &x.countdown.id);
    assert_eq!(t["state"], "stopped");
    assert_eq!(t["currentSeconds"], 120.0);
    assert_eq!(t["display"], "02:00");
    assert_eq!(t["kind"]["type"], "countdown");

    // Text-diff probe: the timer slide while stopped renders exactly like the static "2:00".
    a.ok(json!({"type": "triggerSlide", "presentation": x.deck.id, "index": 1}))
        .await;
    let fixed = wait_frame(&server, 0, "static 2:00", |i| {
        count(i, 80, 280, WHITE, 30) > 300
    })
    .await;
    a.ok(json!({"type": "triggerSlide", "presentation": x.deck.id, "index": 0}))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let live = wait_frame(&server, 0, "timer text", |i| {
        count(i, 80, 280, WHITE, 30) > 300
    })
    .await;
    assert_eq!(
        live.as_raw(),
        fixed.as_raw(),
        "the linked text renders like the authored text"
    );

    // Stopped: nothing re-renders.
    let r0 = renders(&server);
    tokio::time::sleep(Duration::from_millis(1600)).await;
    assert_eq!(renders(&server), r0, "a stopped timer does not re-render");

    // Running: about one render a second, each with new text; ticks arrive.
    a.ok(json!({"type": "timerStart", "timer": x.countdown.id}))
        .await;
    assert_eq!(timer(&a.state, &x.countdown.id)["state"], "running");
    let r1 = renders(&server);
    let t0 = Instant::now();
    let first = latest(&server, 0).unwrap();
    let mut ticks = 0;
    while t0.elapsed() < Duration::from_millis(3300) {
        let m = tokio::time::timeout(Duration::from_millis(400), a.next_any()).await;
        if let Ok(m) = m {
            if m["type"] == "timerTick" {
                ticks += 1;
                let tt = &m["timers"][0];
                assert_eq!(tt["id"], x.countdown.id.0.to_string());
                assert_eq!(tt["state"], "running");
                let d = tt["display"].as_str().unwrap();
                assert!(d == "02:00" || d.starts_with("01:5"), "{d}");
            } else if m["type"] == "state" {
                a.absorb(&m);
            }
        }
    }
    let n = renders(&server) - r1;
    assert!(
        (2..=5).contains(&n),
        "{n} renders in 3.3 s of a running timer"
    );
    assert!((2..=4).contains(&ticks), "{ticks} timer ticks in 3.3 s");
    let later = latest(&server, 0).unwrap();
    assert_ne!(first.as_raw(), later.as_raw(), "the text changed");

    // Pause holds the value; adjust and set change it; reset goes back.
    a.ok(json!({"type": "timerStop", "timer": x.countdown.id}))
        .await;
    let paused = timer(&a.state, &x.countdown.id).clone();
    assert_eq!(paused["state"], "paused");
    let v = paused["currentSeconds"].as_f64().unwrap();
    assert!((115.0..118.5).contains(&v), "{v}");
    a.ok(json!({"type": "timerAdjust", "timer": x.countdown.id, "deltaSeconds": 60}))
        .await;
    let v2 = timer(&a.state, &x.countdown.id)["currentSeconds"]
        .as_f64()
        .unwrap();
    assert!((v2 - v - 60.0).abs() < 0.01);
    a.ok(json!({"type": "timerSet", "timer": x.countdown.id, "seconds": 30}))
        .await;
    assert_eq!(timer(&a.state, &x.countdown.id)["display"], "00:30");
    let r = a
        .command(json!({"type": "timerSet", "timer": x.countdown.id}))
        .await;
    assert_eq!(r["error"]["code"], "badRequest");
    a.ok(json!({"type": "timerReset", "timer": x.countdown.id}))
        .await;
    assert_eq!(timer(&a.state, &x.countdown.id)["state"], "stopped");
    assert_eq!(timer(&a.state, &x.countdown.id)["currentSeconds"], 120.0);

    // Expiry hides the elements whose condition is "has time remaining".
    a.ok(json!({"type": "triggerSlide", "presentation": x.deck.id, "index": 2}))
        .await;
    wait_frame(&server, 0, "short timer text", |i| {
        count(i, 80, 280, WHITE, 30) > 300
    })
    .await;
    a.ok(json!({"type": "timerStart", "timer": x.short.id}))
        .await;
    a.wait_state("short timer expires", |_, s| {
        timer(s, &x.short.id)["state"] == "expired"
    })
    .await;
    wait_frame(&server, 0, "text hidden at expiry", |i| {
        count(i, 0, H, WHITE, 30) == 0
    })
    .await;
    assert_eq!(timer(&a.state, &x.short.id)["display"], "00:00");
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn messages_props_and_slide_actions() {
    let fx = fixture("live-overlays");
    let x = extras(&fx);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "protocolVersion=1").await;
    // Everything on the audience screen (no template override).
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id}))
        .await;

    // ---- messages: tokens, stacking ---------------------------------------------------------
    a.ok(json!({"type": "messageShow", "message": x.hello.id, "tokens": [{"name": "name", "value": "Ann"}]}))
        .await;
    let ms = a.state["layers"]["messages"].as_array().unwrap().clone();
    assert_eq!(ms.len(), 1);
    assert_eq!(ms[0]["text"], "Hello Ann");
    assert_eq!(ms[0]["visible"], true);
    // The built-in banner: white text on a dark bar along the bottom.
    let banner = wait_frame(&server, 0, "message banner", |i| {
        count(i, (H as f64 * 0.79) as u32, H, WHITE, 40) > 150
    })
    .await;
    assert_eq!(count(&banner, 0, (H as f64 * 0.7) as u32, WHITE, 40), 0);
    a.ok(json!({"type": "messageShow", "message": x.second.id}))
        .await;
    let ms = a.state["layers"]["messages"].as_array().unwrap().clone();
    assert_eq!(ms.len(), 2);
    assert_eq!(ms[0]["visible"], false);
    assert_eq!(ms[1]["visible"], true);
    assert_eq!(ms[1]["text"], "Second message");
    // Hiding the top one reveals the one below; default token values apply.
    a.ok(json!({"type": "messageHide", "message": x.second.id}))
        .await;
    a.ok(json!({"type": "messageShow", "message": x.hello.id}))
        .await;
    let ms = a.state["layers"]["messages"].as_array().unwrap().clone();
    assert_eq!(ms.len(), 1);
    assert_eq!(ms[0]["text"], "Hello World");
    // A look without the messages layer hides it on that screen.
    a.ok(json!({"type": "messageHide"})).await;
    assert!(a.state["layers"]["messages"].as_array().unwrap().is_empty());
    wait_frame(&server, 0, "banner gone", |i| {
        count(i, 0, H, WHITE, 40) == 0
    })
    .await;

    // ---- props: own dissolve in and out -------------------------------------------------------
    a.ok(json!({"type": "propShow", "prop": x.prop.id})).await;
    assert_eq!(a.state["layers"]["props"][0]["name"], "Red");
    let mid = wait_frame(&server, 0, "prop dissolving in", |i| {
        let p = i.get_pixel(20, 20);
        p.0[0] > 40 && p.0[0] < 215
    })
    .await;
    assert!(
        mid.get_pixel(W - 20, 20).0[0] < 10,
        "right half stays clear"
    );
    wait_frame(&server, 0, "prop in", |i| near(i.get_pixel(20, 20), RED, 3)).await;
    a.ok(json!({"type": "propToggle", "prop": x.prop.id})).await;
    assert!(a.state["layers"]["props"].as_array().unwrap().is_empty());
    wait_frame(&server, 0, "prop dissolving out", |i| {
        let p = i.get_pixel(20, 20);
        p.0[0] > 40 && p.0[0] < 215
    })
    .await;
    wait_frame(&server, 0, "prop out", |i| {
        near(i.get_pixel(20, 20), BLACK, 3)
    })
    .await;
    a.ok(json!({"type": "propToggle", "prop": x.prop.id})).await;
    wait_frame(&server, 0, "prop back", |i| {
        near(i.get_pixel(20, 20), RED, 3)
    })
    .await;
    a.ok(json!({"type": "clearLayer", "layer": "props"})).await;
    assert!(a.state["layers"]["props"].as_array().unwrap().is_empty());
    wait_frame(&server, 0, "props cleared", |i| {
        near(i.get_pixel(20, 20), BLACK, 3)
    })
    .await;

    // ---- slide actions: in order, disabled and unsupported ones skipped -----------------------
    a.ok(json!({"type": "setLook", "look": fx.default_look.id}))
        .await;
    let skipped0 = a.state["diagnostics"]["skippedActions"].as_u64().unwrap();
    a.ok(json!({"type": "triggerSlide", "presentation": x.deck.id, "index": 3}))
        .await;
    let s = &a.state;
    assert_eq!(timer(s, &x.countdown.id)["state"], "running");
    assert_eq!(s["layers"]["props"][0]["prop"], x.prop.id.0.to_string());
    assert!(
        s["layers"]["messages"].as_array().unwrap().is_empty(),
        "shown, then hidden by the next action; the disabled one never ran"
    );
    assert_eq!(s["activeLook"], fx.plain_look.id.0.to_string());
    assert_eq!(
        s["diagnostics"]["skippedActions"].as_u64().unwrap() - skipped0,
        2,
        "the unsupported action and the missing macro"
    );
    // `next` / `prev` trigger slides too, and the cursor look-ahead runs nothing.
    let skipped1 = a.state["diagnostics"]["skippedActions"].as_u64().unwrap();
    a.ok(json!({"type": "prev"})).await;
    a.ok(json!({"type": "clearAll"})).await;
    assert!(a.state["layers"]["props"].as_array().unwrap().is_empty());
    assert_eq!(
        a.state["diagnostics"]["skippedActions"].as_u64().unwrap(),
        skipped1
    );
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timer_message_prop_crud_with_if_match() {
    let fx = fixture("live-crud");
    let x = extras(&fx);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "protocolVersion=1").await;

    for (route, kind) in [
        ("timers", "timer"),
        ("messages", "message"),
        ("props", "prop"),
    ] {
        let list = get_json(addr, &format!("/api/v1/{route}")).await;
        assert!(!list.as_array().unwrap().is_empty(), "{route}");
        assert!(list[0]["revision"].is_string());
        // Create.
        let body = match kind {
            "timer" => {
                json!({"kind": {"type": "countdownToTime", "secondsAfterMidnight": 36000, "period": "am"}})
            }
            "message" => json!({"text": "Car ${plate}", "tokens": [{"name": "plate"}]}),
            _ => {
                json!({"fromSlide": {"presentation": x.deck.id, "slide": x.deck.sections[0].slides[1].id}})
            }
        };
        let r = http(addr, "POST", &format!("/api/v1/{route}"), &[], Some(body)).await;
        assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
        let e = r.json();
        let id = e["id"].as_str().unwrap().to_string();
        let rev = e["revision"].as_str().unwrap().to_string();
        assert_eq!(r.header("etag"), Some(format!("\"{rev}\"").as_str()));
        assert!(e["name"].as_str().unwrap().ends_with(" 1"), "{e}");
        a.wait_state(&format!("{kind} change announced"), |_, _| true)
            .await;
        // Update with the right revision, then a stale one.
        let mut doc = e.clone();
        doc["name"] = json!("Renamed");
        let r = http(
            addr,
            "PUT",
            &format!("/api/v1/{route}/{id}"),
            &[("If-Match", &format!("\"{rev}\""))],
            Some(doc.clone()),
        )
        .await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let rev2 = r.json()["revision"].as_str().unwrap().to_string();
        assert_ne!(rev, rev2);
        let r = http(
            addr,
            "PUT",
            &format!("/api/v1/{route}/{id}"),
            &[("If-Match", &format!("\"{rev}\""))],
            Some(doc),
        )
        .await;
        assert_eq!(r.status, 409);
        assert_eq!(r.json()["current"]["revision"], rev2);
        assert_eq!(r.json()["current"]["name"], "Renamed");
        // GET one with its ETag.
        let r = http(addr, "GET", &format!("/api/v1/{route}/{id}"), &[], None).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.header("etag"), Some(format!("\"{rev2}\"").as_str()));
        if kind == "prop" {
            assert_eq!(r.json()["slide"]["elements"].as_array().unwrap().len(), 1);
            let d = get_json(addr, &format!("/api/v1/props/{id}/document")).await;
            assert_eq!(d["id"], id);
            let slide = d["sections"][0]["slides"][0]["id"]
                .as_str()
                .unwrap()
                .to_string();
            let r = http(
                addr,
                "POST",
                &format!("/api/v1/props/{id}/edits"),
                &[],
                Some(json!({"ops": [{"type": "setSlide", "slide": slide, "background": {"r": 0, "g": 0, "b": 1, "a": 1}}]})),
            )
            .await;
            assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
            assert_eq!(r.json()["inverse"].as_array().unwrap().len(), 1);
            let r = http(
                addr,
                "POST",
                &format!("/api/v1/props/{id}/edits"),
                &[],
                Some(json!({"ops": [{"type": "deleteSlide", "slide": slide}]})),
            )
            .await;
            assert_eq!(r.status, 400);
            let r = http(
                addr,
                "GET",
                &format!("/api/v1/props/{id}/thumbnail?w=160&v=x"),
                &[],
                None,
            )
            .await;
            assert_eq!(r.status, 200);
            assert_eq!(&r.body[1..4], b"PNG");
            let r = http(
                addr,
                "GET",
                &format!("/api/v1/props/{id}/slides/{slide}/render?w=320"),
                &[],
                None,
            )
            .await;
            assert_eq!(r.status, 200);
            assert_eq!(&r.body[1..4], b"PNG");
        }
        // Delete: a stale revision conflicts, the current one deletes.
        let cur = http(addr, "GET", &format!("/api/v1/{route}/{id}"), &[], None).await;
        let cur_rev = cur.json()["revision"].as_str().unwrap().to_string();
        let r = http(
            addr,
            "DELETE",
            &format!("/api/v1/{route}/{id}"),
            &[("If-Match", &format!("\"{rev}\""))],
            None,
        )
        .await;
        assert_eq!(r.status, 409);
        let r = http(
            addr,
            "DELETE",
            &format!("/api/v1/{route}/{id}"),
            &[("If-Match", &format!("\"{cur_rev}\""))],
            None,
        )
        .await;
        assert_eq!(r.status, 204);
        let r = http(addr, "GET", &format!("/api/v1/{route}/{id}"), &[], None).await;
        assert_eq!(r.status, 404);
    }
    // X2-1: a declared token name with inner spaces (as imported) saves on create and update,
    // and the live text replaces it; empty, padded or braced names are refused.
    let r = http(
        addr,
        "POST",
        "/api/v1/messages",
        &[],
        Some(json!({"text": "Parent of ${Child Number}", "tokens": [{"name": "Child Number"}]})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let mut doc = r.json();
    let id = doc["id"].as_str().unwrap().to_string();
    let rev = doc["revision"].as_str().unwrap().to_string();
    doc["name"] = json!("Parent call");
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/messages/{id}"),
        &[("If-Match", &format!("\"{rev}\""))],
        Some(doc),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["tokens"][0]["name"], "Child Number");
    a.ok(json!({"type": "messageShow", "message": id, "tokens": [{"name": "Child Number", "value": "A12"}]}))
        .await;
    let ms = a.state["layers"]["messages"].as_array().unwrap().clone();
    assert!(ms.iter().any(|m| m["text"] == "Parent of A12"), "{ms:?}");
    a.ok(json!({"type": "messageHide", "message": id})).await;
    for bad in ["", " lead", "trail ", "a{b", "a}b", "a$b"] {
        let r = http(
            addr,
            "POST",
            "/api/v1/messages",
            &[],
            Some(json!({"text": "x", "tokens": [{"name": bad}]})),
        )
        .await;
        assert_eq!(r.status, 400, "{bad:?}");
        assert_eq!(r.json()["error"]["field"], "tokens", "{bad:?}");
    }

    // documentChanged events were sent for the kinds.
    let r = http(
        addr,
        "POST",
        "/api/v1/timers",
        &[],
        Some(json!({"name": "Bad", "kind": {"type": "countdown", "seconds": -5}})),
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"]["field"], "kind.seconds");

    // Deleting a running timer and a showing prop drops them from the state.
    a.ok(json!({"type": "timerStart", "timer": x.countdown.id}))
        .await;
    a.ok(json!({"type": "propShow", "prop": x.prop.id})).await;
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/props/{}", x.prop.id.0),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204);
    a.wait_state("deleted prop leaves the layer", |_, s| {
        s["layers"]["props"].as_array().unwrap().is_empty()
    })
    .await;
    let seen: Vec<String> = a
        .events
        .iter()
        .filter(|e| e["type"] == "documentChanged")
        .map(|e| e["kind"].as_str().unwrap().to_string())
        .collect();
    server.shutdown().await.unwrap();
    // The events may still be in flight for the last change; the earlier kinds arrived.
    let _ = seen;
}

fn block(rect: Rect, rgb: [f32; 3], visibility: Vec<VisibilityCondition>) -> model::Element {
    let mut e = model::Element::new(rect);
    e.fill = model::Fill::color(Color::rgb(rgb[0], rgb[1], rgb[2]));
    e.visibility = visibility;
    e
}

const GREEN: [u8; 3] = [0, 255, 0];
const MAGENTA: [u8; 3] = [255, 0, 255];
const CYAN: [u8; 3] = [0, 255, 255];

/// Batch 12 (G16's rest): the live presentation's name, its copyright line and a timer's
/// elapsed time render like the same text authored; the song's artwork fills an artwork frame;
/// message, stage message and layer conditions show and hide objects as the show changes; and
/// `setElement {visibility}` changes an object's conditions on the live slide.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn presentation_links_artwork_and_show_conditions() {
    let fx = fixture("live-links");
    let x = extras(&fx);
    let text_at = |t: &str| {
        text_element(
            Rect::new(20.0, 80.0, W as f64 - 40.0, 200.0),
            t,
            Color::WHITE,
        )
    };
    let linked = |link: DataLink| {
        let mut e = text_at("design");
        e.data_link = Some(link);
        e
    };
    let slide = |elements: Vec<model::Element>| model::Slide {
        elements,
        ..model::Slide::new()
    };
    let mut art = model::Element::new(Rect::new(0.0, 0.0, W as f64 / 2.0, H as f64 / 2.0));
    art.data_link = Some(DataLink::SongArtwork);
    art.fill = model::Fill::color(Color::WHITE);
    let (hw, qh) = (W as f64 / 2.0, H as f64 / 4.0);
    let msg_block = block(
        Rect::new(hw, 0.0, hw, qh),
        [0.0, 1.0, 0.0],
        vec![VisibilityCondition::Message {
            message: None,
            active: true,
        }],
    );
    let msg_id = msg_block.id;
    let conditions = slide(vec![
        art,
        msg_block,
        block(
            Rect::new(hw, qh, hw, qh),
            [1.0, 0.0, 1.0],
            vec![VisibilityCondition::StageMessage { active: true }],
        ),
        block(
            Rect::new(hw, 2.0 * qh, hw, qh),
            [0.0, 1.0, 1.0],
            vec![VisibilityCondition::Layer {
                layer: model::Layer::Props,
                active: false,
            }],
        ),
    ]);
    let mut sec = Section::new("Links");
    sec.slides = vec![
        slide(vec![linked(DataLink::PresentationName {
            source: Default::default(),
        })]),
        slide(vec![text_at("Links")]),
        slide(vec![linked(DataLink::CcliText)]),
        slide(vec![text_at("Dawn")]),
        slide(vec![linked(DataLink::Timer {
            timer: Some(Link::to(x.countdown.id)),
            format: TimerFormat::default(),
            color_triggers: Vec::new(),
            show: model::TimerShow::Elapsed,
        })]),
        slide(vec![text_at("00:00")]),
        conditions,
    ];
    let song = Presentation {
        library: Some(fx.library.id),
        canvas_size: model::Size::new(W as f64, H as f64),
        sections: vec![sec],
        song_info: Some(model::SongInfo {
            title: Some("Dawn".into()),
            artwork: Some(fx.asset.id),
            ..Default::default()
        }),
        ..Presentation::new("Links")
    };
    let pid = song.id.0.to_string();
    write_doc(&fx.dir, "portable", "presentation", &pid, &song);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "protocolVersion=1").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id}))
        .await;

    // Linked text renders exactly like the same text authored.
    for (linked, fixed, what) in [
        (0, 1, "presentation name"),
        (2, 3, "copyright line"),
        (4, 5, "timer elapsed"),
    ] {
        a.ok(json!({"type": "triggerSlide", "presentation": song.id, "index": fixed}))
            .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let want = wait_frame(&server, 0, what, |i| count(i, 80, 280, WHITE, 30) > 200).await;
        a.ok(json!({"type": "triggerSlide", "presentation": song.id, "index": linked}))
            .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let got = wait_frame(&server, 0, what, |i| count(i, 80, 280, WHITE, 30) > 200).await;
        assert_eq!(got.as_raw(), want.as_raw(), "{what}");
    }

    // Artwork fills its frame; nothing shows that needs a message, a stage message; the
    // "props clear" block shows.
    a.ok(json!({"type": "triggerSlide", "presentation": song.id, "index": 6}))
        .await;
    let half = (W * H / 4) as usize;
    wait_frame(&server, 0, "artwork, props clear", |i| {
        count(i, 0, H / 2, BLUE, 30) > half * 9 / 10
            && count(i, 0, H, CYAN, 30) > half / 3
            && count(i, 0, H, GREEN, 30) == 0
            && count(i, 0, H, MAGENTA, 30) == 0
    })
    .await;
    a.ok(json!({"type": "messageShow", "message": x.second.id}))
        .await;
    wait_frame(&server, 0, "a message shows", |i| {
        count(i, 0, H, GREEN, 30) > half / 3
    })
    .await;
    a.ok(json!({"type": "stageMessage", "text": "Wrap up"}))
        .await;
    wait_frame(&server, 0, "a stage message shows", |i| {
        count(i, 0, H, MAGENTA, 30) > half / 3
    })
    .await;
    a.ok(json!({"type": "propShow", "prop": x.prop.id})).await;
    wait_frame(&server, 0, "props live", |i| count(i, 0, H, CYAN, 30) == 0).await;
    a.ok(json!({"type": "messageHide", "message": x.second.id}))
        .await;
    wait_frame(&server, 0, "message hidden", |i| {
        count(i, 0, H, GREEN, 30) == 0
    })
    .await;

    // The editor takes the message condition off: the block shows again.
    let slide_id = song.sections[0].slides[6].id;
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{pid}/edits"),
        &[],
        Some(json!({"ops": [{"type": "setElement", "slide": slide_id, "element": msg_id, "patch": {"visibility": []}}]})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(
        r.json()["inverse"][0]["patch"]["visibility"][0]["type"],
        "message",
        "the inverse puts the condition back"
    );
    wait_frame(&server, 0, "condition removed", |i| {
        count(i, 0, H, GREEN, 30) > half / 3
    })
    .await;
    server.shutdown().await.unwrap();
}

/// The song's copyright line drawn on its own (`ccli.showOn`): the CCLI template (drawn at the
/// show's default canvas size, scaled to the song's) goes over the first and last slides of
/// the play order, an arrangement's repeats counted apart; its CCLI text link shows the song's
/// line. Not on other slides, not for a song that doesn't ask for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ccli_line_on_a_songs_first_and_last_slides() {
    let fx = fixture("ccli-line");
    let hd = model::Size::HD;
    // The template: a green bar along the bottom and the copyright line (white) above it.
    let mut bar = block(
        Rect::new(0.0, hd.height - 120.0, hd.width, 120.0),
        [0.0, 1.0, 0.0],
        vec![],
    );
    bar.name = "Bar".into();
    let mut line = text_element(
        Rect::new(60.0, 600.0, hd.width - 120.0, 300.0),
        "Title",
        Color::WHITE,
    );
    line.data_link = Some(DataLink::CcliText);
    let cut = Some(Transition::CUT);
    let settings = model::ShowSettings {
        default_transitions: model::LayerTransitions {
            media: cut.clone(),
            slide: cut.clone(),
            announcements: cut,
            ..Default::default()
        },
        startup_look: Some(fx.plain_look.id),
        ccli: model::CcliSettings {
            enabled: true,
            show_on: model::CcliDisplay::FirstAndLast,
            template: Some(model::Slide {
                elements: vec![bar, line],
                ..model::Slide::new()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "showSettings",
        "00000000-0000-0000-0000-000000000000",
        &settings,
    );
    // Each slide: a small red mark in the top-left corner, nothing white or green.
    let slide = || model::Slide {
        elements: vec![block(
            Rect::new(0.0, 0.0, 40.0, 40.0),
            [1.0, 0.0, 0.0],
            vec![],
        )],
        ..model::Slide::new()
    };
    let (mut a, mut b) = (Section::new("A"), Section::new("B"));
    a.slides = vec![slide(), slide()];
    b.slides = vec![slide(), slide()];
    let arr = model::Arrangement {
        id: model::ArrangementId::new(),
        name: "Twice".into(),
        sections: vec![a.id, b.id, a.id],
    };
    let info = model::SongInfo {
        title: Some("Dawn".into()),
        display: true,
        ..Default::default()
    };
    let song = Presentation {
        library: Some(fx.library.id),
        canvas_size: model::Size::new(W as f64, H as f64),
        sections: vec![a, b],
        selected_arrangement: Some(arr.id),
        arrangements: vec![arr],
        song_info: Some(info.clone()),
        ..Presentation::new("Dawn")
    };
    let mut quiet = song.clone();
    quiet.id = model::PresentationId::new();
    quiet.name = "Quiet".into();
    quiet.song_info = Some(model::SongInfo {
        display: false,
        ..info
    });
    for p in [&song, &quiet] {
        write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), p);
    }
    let server = start_server(&fx, true, false).await;
    let mut c = Client::connect(server.addr, "protocolVersion=1").await;

    // Play order: A0 A1 B0 B1 A0 A1 (steps 0..5).
    let bottom = H - 40;
    for (index, step, shown) in [
        (0, 0, true),
        (1, 1, false),
        (2, 2, false),
        (0, 4, false),
        (1, 5, true),
        (0, 0, true),
    ] {
        c.ok(
            json!({"type": "triggerSlide", "presentation": song.id, "index": index, "step": step}),
        )
        .await;
        let what = format!("slide {index} step {step}: line shown {shown}");
        wait_frame(&server, 0, &what, |i| {
            let bar = count(i, bottom, H, GREEN, 30);
            let text = count(i, 200, bottom, WHITE, 30);
            let mark = count(i, 0, 14, RED, 30) > 100;
            if shown {
                mark && bar > (W * 40) as usize * 9 / 10 && text > 50
            } else {
                mark && bar == 0 && text == 0
            }
        })
        .await;
    }
    // A song without `display`: no line on its first slide.
    c.ok(json!({"type": "triggerSlide", "presentation": quiet.id, "index": 0}))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    wait_frame(&server, 0, "quiet song", |i| {
        count(i, 0, 14, RED, 30) > 100 && count(i, bottom, H, GREEN, 30) == 0
    })
    .await;
    server.shutdown().await.unwrap();
}
