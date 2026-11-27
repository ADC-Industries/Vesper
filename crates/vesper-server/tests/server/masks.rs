//! Masks (G3): CRUD with validation, looks that name them, deleting a mask in use (refused, or
//! detached), and the picture: an overlay mask drawn over every layer, a keyhole mask that
//! shows the screen only through its shapes, and a feathered edge.

use crate::common::*;
use serde_json::{Value, json};

async fn send(addr: std::net::SocketAddr, method: &str, path: &str, body: Value) -> Resp {
    http(addr, method, path, &[], Some(body)).await
}

fn body(r: &Resp) -> String {
    String::from_utf8_lossy(&r.body).to_string()
}

/// A mask slide with one red rectangle over the left half of a W×H canvas.
fn left_half() -> Value {
    json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "elements": [{
            "id": uuid::Uuid::new_v4().to_string(),
            "name": "Left",
            "bounds": {"x": 0.0, "y": 0.0, "width": (W / 2) as f64, "height": H as f64},
            "kind": {"type": "shape", "shape": {"type": "rectangle"}},
            "fill": {"enabled": true, "kind": {"type": "color", "color": {"r": 1.0, "g": 0.0, "b": 0.0}}}
        }]
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn masks_crud_and_deleting_a_mask_in_use() {
    let fx = fixture("masks-crud");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let aud = fx.audience.id.0.to_string();

    let info = get_json(addr, "/api/v1/server").await;
    assert!(
        info["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "masks")
    );
    assert_eq!(get_json(addr, "/api/v1/masks").await, json!([]));

    // Create with defaults: a name, an empty slide, overlay.
    let r = send(addr, "POST", "/api/v1/masks", json!({})).await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let m = r.json();
    let id = m["id"].as_str().unwrap().to_string();
    assert_eq!(m["name"], "Mask 1");
    assert!(m.get("mode").is_none(), "overlay is the default: {m}");
    assert!(fx.dir.join(format!("portable/mask/{id}.json")).is_file());

    // Validation.
    let rev = m["revision"].as_str().unwrap().to_string();
    let tag = format!("\"{rev}\"");
    for (b, field) in [
        (
            json!({"name": "M", "slide": left_half(), "feather": 900.0}),
            "feather",
        ),
        (
            json!({"name": "M", "slide": left_half(), "mode": "sideways"}),
            "mode",
        ),
        (
            json!({"name": "M", "slide": left_half(), "color": {"r": 2.0, "g": 0.0, "b": 0.0}}),
            "color",
        ),
        (json!({"name": "", "slide": left_half()}), "name"),
    ] {
        let r = http(
            addr,
            "PUT",
            &format!("/api/v1/masks/{id}"),
            &[("If-Match", tag.as_str())],
            Some(b.clone()),
        )
        .await;
        assert_eq!(r.status, 400, "{b}: {}", body(&r));
        assert_eq!(r.json()["error"]["field"], field, "{b}");
    }
    // Update: a keyhole with a shape and a feather.
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/masks/{id}"),
        &[("If-Match", tag.as_str())],
        Some(json!({"name": "Window", "slide": left_half(), "mode": "keyhole", "feather": 12.0})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    assert_eq!(r.json()["mode"], "keyhole");
    // Duplicate.
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/masks/{id}/duplicate"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    assert_eq!(r.json()["name"], "Window copy");
    assert_eq!(r.json()["feather"], 12.0);
    // A copy under the source's own name is refused, as a new mask would be (D06-5).
    let r = send(
        addr,
        "POST",
        &format!("/api/v1/masks/{id}/duplicate"),
        json!({"name": "Window"}),
    )
    .await;
    assert_eq!(r.status, 400, "{}", body(&r));
    assert_eq!(r.json()["error"]["field"], "name");

    // A look names a mask that exists, or nothing.
    let r = send(
        addr,
        "POST",
        "/api/v1/looks",
        json!({"name": "Bad", "screens": [{"screen": aud, "mask": uuid::Uuid::new_v4().to_string()}]}),
    )
    .await;
    assert_eq!(r.status, 400, "{}", body(&r));
    assert_eq!(r.json()["error"]["field"], "screens.0.mask");
    let r = send(
        addr,
        "POST",
        "/api/v1/looks",
        json!({"name": "Windowed", "screens": [{"screen": aud, "mask": id}]}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let look = r.json()["id"].as_str().unwrap().to_string();

    // Deleting a mask in use is refused, naming the looks; with detach they forget it.
    let r = http(addr, "DELETE", &format!("/api/v1/masks/{id}"), &[], None).await;
    assert_eq!(r.status, 409, "{}", body(&r));
    assert_eq!(r.json()["current"]["looks"], json!([look]));
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/masks/{id}?detach=true"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    assert_eq!(r.json()["looks"], json!([look]));
    let l = get_json(addr, &format!("/api/v1/looks/{look}")).await;
    assert!(
        l.get("screens")
            .is_none_or(|s| s.as_array().unwrap().is_empty()),
        "{l}"
    );
    assert!(!fx.dir.join(format!("portable/mask/{id}.json")).exists());
    assert_eq!(
        get_json(addr, "/api/v1/masks")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    server.shutdown().await.unwrap();
}

/// Mean (r, g, b) over a rectangle of the frame.
fn mean(img: &image::RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32) -> [f64; 3] {
    let mut s = [0f64; 3];
    let mut n = 0f64;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.get_pixel(x, y).0;
            for c in 0..3 {
                s[c] += p[c] as f64;
            }
            n += 1.0;
        }
    }
    s.map(|v| v / n)
}

/// White text pixels in the text band, left and right halves.
fn text_halves(img: &image::RgbaImage) -> (usize, usize) {
    let mut l = 0;
    let mut r = 0;
    for y in 80..280 {
        for x in 0..W {
            let p = img.get_pixel(x, y).0;
            if p[0] > 200 && p[1] > 200 && p[2] > 200 {
                if x < W / 2 { l += 1 } else { r += 1 }
            }
        }
    }
    (l, r)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn masks_are_drawn_over_the_screen() {
    let fx = fixture("masks-render");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;
    let aud = fx.audience.id.0.to_string();
    // A slide with white text on both halves, under a look without a mask.
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": fx.second.id.0.to_string(), "index": 0}))
        .await;
    let plain = wait_frame(&server, 0, "text on both halves", |img| {
        let (l, r) = text_halves(img);
        l > 50 && r > 50
    })
    .await;
    let (l0, r0) = text_halves(&plain);

    // Overlay: the red rectangle covers the left half, text and all.
    let r = send(
        addr,
        "POST",
        "/api/v1/masks",
        json!({"name": "Red left", "size": {"width": W, "height": H}, "slide": left_half()}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let overlay = r.json()["id"].as_str().unwrap().to_string();
    let r = send(
        addr,
        "POST",
        "/api/v1/looks",
        json!({"name": "Masked", "screens": [{"screen": aud, "mask": overlay}]}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let look = r.json();
    let look_id = look["id"].as_str().unwrap().to_string();
    a.ok(json!({"type": "setLook", "look": look_id})).await;
    let img = wait_frame(&server, 0, "red left half", |img| {
        mean(img, 0, W / 2, 0, H)[0] > 245.0
    })
    .await;
    // Kept next to the fixture for a look (`masks-render-*/overlay.png`).
    let _ = img.save(fx.dir.join("overlay.png"));
    let left = mean(&img, 0, W / 2, 0, H);
    assert!(left[1] < 5.0 && left[2] < 5.0, "pure red: {left:?}");
    let (l, r) = text_halves(&img);
    assert_eq!(l, 0, "the text under the mask is covered");
    assert!(
        r.abs_diff(r0) < r0 / 20 + 5,
        "the right half is untouched: {r} vs {r0}"
    );

    // Keyhole: the screen shows only through the rectangle; the rest is black.
    let r = send(
        addr,
        "POST",
        "/api/v1/masks",
        json!({"name": "Window", "size": {"width": W, "height": H}, "slide": left_half(),
               "mode": "keyhole"}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let keyhole = r.json()["id"].as_str().unwrap().to_string();
    let mut edited = look.clone();
    edited["screens"] = json!([{"screen": aud, "mask": keyhole}]);
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/looks/{look_id}"),
        &[],
        Some(edited.clone()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let img = wait_frame(&server, 0, "keyhole", |img| {
        let (l, r) = text_halves(img);
        r == 0 && l > 50
    })
    .await;
    let (l, _) = text_halves(&img);
    assert!(
        l.abs_diff(l0) < l0 / 20 + 5,
        "the left half shows through: {l} vs {l0}"
    );
    let _ = img.save(fx.dir.join("keyhole.png"));
    let right = mean(&img, W / 2 + 2, W, 0, H);
    assert!(right.iter().all(|c| *c < 2.0), "black outside: {right:?}");

    // Feather: a white keyhole colour with a 40 px feather fades in over many pixels
    // around the edge (row y = 40, above the text), instead of a hard step.
    let r = http(addr, "GET", &format!("/api/v1/masks/{keyhole}"), &[], None).await;
    let mut m = r.json();
    m["feather"] = json!(40.0);
    m["color"] = json!({"r": 1.0, "g": 1.0, "b": 1.0});
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/masks/{keyhole}"),
        &[],
        Some(m),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let soft = wait_frame(&server, 0, "white outside", |img| {
        img.get_pixel(W - 5, 40).0[0] > 250
    })
    .await;
    let _ = soft.save(fx.dir.join("feather.png"));
    let row: Vec<u32> = (W / 2 - 40..W / 2 + 40)
        .map(|x| {
            let p = soft.get_pixel(x, 40).0;
            p[0] as u32 + p[1] as u32 + p[2] as u32
        })
        .collect();
    let between = row.iter().filter(|v| (60..700).contains(*v)).count();
    assert!(between >= 15, "a gradual edge: {row:?}");
    assert!(
        row.windows(2).all(|w| w[1] + 6 >= w[0]),
        "brighter towards the outside: {row:?}"
    );
    server.shutdown().await.unwrap();
}

/// A stage screen's own mask (`Screen::mask`): validated (stage screens only, an existing
/// mask), drawn over its layout, detached by a mask delete; and a keyhole with a transparent
/// colour cuts the picture out to alpha on a screen with alpha.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stage_masks_and_transparent_keyholes() {
    use model::{Screen, ScreenKind};
    let fx = fixture("masks-stage");
    let stage = Screen {
        name: "Stage".into(),
        sort_key: "c".into(),
        size: model::Size::new(W as f64, H as f64),
        frame_rate: model::FrameRate::FPS30,
        kind: ScreenKind::Stage { layout: None },
        ..Screen::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "screen",
        &stage.id.0.to_string(),
        &stage,
    );
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    assert_eq!(server.shared.screens()[2].id, stage.id);
    let mut a = Client::connect(addr, "").await;
    let sid = stage.id.0.to_string();

    let r = send(
        addr,
        "POST",
        "/api/v1/masks",
        json!({"name": "Red left", "size": {"width": W, "height": H}, "slide": left_half()}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let red = r.json()["id"].as_str().unwrap().to_string();

    // Only a stage screen takes a mask of its own, and only one that exists.
    let aud = fx.audience.id.0.to_string();
    let mut doc = get_json(addr, &format!("/api/v1/screens/{aud}")).await;
    doc["mask"] = json!(red);
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/screens/{aud}"),
        &[],
        Some(doc),
    )
    .await;
    assert_eq!(r.status, 400, "{}", body(&r));
    assert_eq!(r.json()["error"]["field"], "mask");
    let mut doc = get_json(addr, &format!("/api/v1/screens/{sid}")).await;
    doc["mask"] = json!(uuid::Uuid::new_v4().to_string());
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/screens/{sid}"),
        &[],
        Some(doc.clone()),
    )
    .await;
    assert_eq!(r.status, 400, "{}", body(&r));
    doc["mask"] = json!(red);
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/screens/{sid}"),
        &[],
        Some(doc),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let img = wait_frame(&server, 2, "the stage screen's left half is red", |img| {
        mean(img, 0, W / 2, 0, H)[0] > 245.0
    })
    .await;
    let _ = img.save(fx.dir.join("stage.png"));
    let left = mean(&img, 0, W / 2, 0, H);
    assert!(left[1] < 5.0 && left[2] < 5.0, "pure red: {left:?}");
    assert!(
        mean(&img, W / 2 + 2, W, 0, H)[0] < 200.0,
        "only the left half"
    );

    // A delete is refused while the stage screen uses it, unless detached.
    let r = http(addr, "DELETE", &format!("/api/v1/masks/{red}"), &[], None).await;
    assert_eq!(r.status, 409, "{}", body(&r));
    assert_eq!(r.json()["current"]["screens"], json!([sid]));
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/masks/{red}?detach=true"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    assert_eq!(r.json()["screens"], json!([sid]));
    assert!(
        get_json(addr, &format!("/api/v1/screens/{sid}"))
            .await
            .get("mask")
            .is_none()
    );
    wait_frame(&server, 2, "the stage mask is gone", |img| {
        mean(img, 0, W / 2, 0, H)[0] < 200.0
    })
    .await;

    // A stage screen with a mask becomes an audience screen: the mask, kept as it was, goes
    // with the edit instead of refusing it (X2-3).
    let r = send(
        addr,
        "POST",
        "/api/v1/masks",
        json!({"name": "Blue left", "size": {"width": W, "height": H}, "slide": left_half()}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let blue = r.json()["id"].as_str().unwrap().to_string();
    let mut doc = get_json(addr, &format!("/api/v1/screens/{sid}")).await;
    doc["mask"] = json!(blue);
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/screens/{sid}"),
        &[],
        Some(doc),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let mut doc = get_json(addr, &format!("/api/v1/screens/{sid}")).await;
    assert_eq!(doc["mask"], blue.as_str());
    // (An audience screen is the default kind: no `kind`.)
    assert!(
        get_json(addr, &format!("/api/v1/screens/{aud}"))
            .await
            .get("kind")
            .is_none()
    );
    doc.as_object_mut().unwrap().remove("kind");
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/screens/{sid}"),
        &[],
        Some(doc),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    assert!(r.json().get("mask").is_none(), "{}", body(&r));
    assert!(
        get_json(addr, &format!("/api/v1/screens/{sid}"))
            .await
            .get("mask")
            .is_none()
    );

    // A transparent keyhole on the Key screen (straight alpha): the picture shows through
    // the rectangle; outside it the screen is transparent, not black.
    let r = send(
        addr,
        "POST",
        "/api/v1/masks",
        json!({"name": "Cut out", "size": {"width": W, "height": H}, "slide": left_half(),
               "mode": "keyhole", "color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 0.0}}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let cut = r.json()["id"].as_str().unwrap().to_string();
    let key = fx.key.id.0.to_string();
    let r = send(
        addr,
        "POST",
        "/api/v1/looks",
        json!({"name": "Cut", "screens": [{"screen": key, "mask": cut}]}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    a.ok(json!({"type": "setLook", "look": r.json()["id"]}))
        .await;
    // Slide 0 of the song has the blue background: opaque everywhere without a mask.
    a.ok(json!({"type": "triggerSlide", "presentation": fx.song.id.0.to_string(), "index": 0}))
        .await;
    let alpha = |img: &image::RgbaImage, x0: u32, x1: u32| -> f64 {
        let mut s = 0.0;
        for y in 0..H {
            for x in x0..x1 {
                s += img.get_pixel(x, y).0[3] as f64;
            }
        }
        s / ((x1 - x0) * H) as f64
    };
    let img = wait_frame(
        &server,
        1,
        "blue on the left, transparent on the right",
        |img| {
            alpha(img, 0, W / 2 - 2) > 250.0
                && alpha(img, W / 2 + 2, W) < 3.0
                && mean(img, 0, W / 2 - 2, 0, H)[2] > 200.0
        },
    )
    .await;
    let _ = img.save(fx.dir.join("cut.png"));
    server.shutdown().await.unwrap();
}
