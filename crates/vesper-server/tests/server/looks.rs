//! Looks (G3): create, update, duplicate, reorder and delete with validation and `If-Match`
//! conflicts; the live look can't be deleted; making a look live (and a saved edit of the live
//! look) plays the look's own transition (any kind, e.g. a push), a look without one cuts.

use crate::common::*;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use vesper_server::RunningServer;

async fn send(addr: std::net::SocketAddr, method: &str, path: &str, body: Value) -> Resp {
    http(addr, method, path, &[], Some(body)).await
}

async fn send_if(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    rev: &str,
    body: Option<Value>,
) -> Resp {
    let tag = format!("\"{rev}\"");
    http(addr, method, path, &[("If-Match", tag.as_str())], body).await
}

fn body(r: &Resp) -> String {
    String::from_utf8_lossy(&r.body).to_string()
}

fn look_file(fx: &Fixture, id: &str) -> std::path::PathBuf {
    fx.dir.join("portable/look").join(format!("{id}.json"))
}

fn names(looks: &Value) -> Vec<String> {
    looks
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap_or("").to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn looks_crud_with_validation_and_conflicts() {
    let fx = fixture("looks-crud");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "").await;
    let aud = fx.audience.id.0.to_string();

    // The capability is announced.
    let info = get_json(addr, "/api/v1/server").await;
    assert!(
        info["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "lookEditing"),
        "{info}"
    );

    // Create with defaults: a free name, last in the list, written to disk.
    let r = send(addr, "POST", "/api/v1/looks", json!({})).await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let created = r.json();
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["name"], "Look 1");
    let rev = created["revision"].as_str().unwrap().to_string();
    assert!(r.header("etag").unwrap().contains(&rev));
    assert!(look_file(&fx, &id).is_file());
    assert_eq!(
        names(&get_json(addr, "/api/v1/looks").await),
        ["Default", "Plain", "Look 1"]
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ws
        .events
        .iter()
        .any(|e| e["type"] == "documentChanged" && e["kind"] == "look" && e["change"] == "added")
    {
        assert!(
            Instant::now() < deadline,
            "documentChanged for the new look"
        );
        let m = ws.next_any().await;
        ws.absorb(&m);
    }

    // GET one, with its ETag.
    let r = http(addr, "GET", &format!("/api/v1/looks/{id}"), &[], None).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["revision"], rev.as_str());

    // Validation: field-tagged errors, nothing written.
    let bad = [
        (json!({"name": "  "}), "name"),
        (json!({"name": "plain"}), "name"),
        (
            json!({"name": "X", "screens": [{"screen": uuid::Uuid::new_v4().to_string()}]}),
            "screens.0",
        ),
        (
            json!({"name": "X", "screens": [{"screen": aud, "layers": {"media": false}},
                                            {"screen": aud, "layers": {"slide": false}}]}),
            "screens.1",
        ),
        (
            json!({"name": "X", "screens": [{"screen": aud, "templateOverride":
                {"theme": {"id": uuid::Uuid::new_v4().to_string()},
                 "slide": uuid::Uuid::new_v4().to_string()}}]}),
            "screens.0.templateOverride",
        ),
        (
            json!({"name": "X", "transition": {"kind": {"type": "dissolve"}, "duration": 60.0}}),
            "transition",
        ),
    ];
    for (b, field) in bad {
        let r = send_if(
            addr,
            "PUT",
            &format!("/api/v1/looks/{id}"),
            &rev,
            Some(b.clone()),
        )
        .await;
        assert_eq!(r.status, 400, "{b}: {}", body(&r));
        assert_eq!(r.json()["error"]["field"], field, "{b}");
    }
    // A stage screen can't be in a look.
    let r = send(
        addr,
        "POST",
        "/api/v1/screens",
        json!({"name": "Stage", "kind": {"type": "stage"}}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let stage = r.json()["id"].as_str().unwrap().to_string();
    let r = send(
        addr,
        "POST",
        "/api/v1/looks",
        json!({"name": "S", "screens": [{"screen": stage, "layers": {"media": false}}]}),
    )
    .await;
    assert_eq!(r.status, 400, "{}", body(&r));
    assert!(body(&r).contains("stage screen"), "{}", body(&r));

    // Update: rename, a matrix entry, a transition. An entry that changes nothing is dropped.
    let key = fx.key.id.0.to_string();
    let r = send_if(
        addr,
        "PUT",
        &format!("/api/v1/looks/{id}"),
        &rev,
        Some(json!({"name": " Lyrics only ",
                    "screens": [{"screen": aud, "layers": {"media": false, "props": false}},
                                {"screen": key}],
                    "transition": {"kind": {"type": "dissolve"}, "duration": 0.8}})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let updated = r.json();
    assert_eq!(updated["name"], "Lyrics only");
    assert_eq!(updated["screens"].as_array().unwrap().len(), 1, "{updated}");
    assert_eq!(updated["screens"][0]["layers"]["media"], false);
    assert_eq!(updated["transition"]["duration"], 0.8);
    let rev2 = updated["revision"].as_str().unwrap().to_string();
    assert_ne!(rev, rev2);
    let stored: Value =
        serde_json::from_slice(&std::fs::read(look_file(&fx, &id)).unwrap()).unwrap();
    assert_eq!(stored["name"], "Lyrics only");

    // A stale revision is a conflict with the current entry.
    let r = send_if(
        addr,
        "PUT",
        &format!("/api/v1/looks/{id}"),
        &rev,
        Some(json!({"name": "Other"})),
    )
    .await;
    assert_eq!(r.status, 409, "{}", body(&r));
    assert_eq!(r.json()["current"]["revision"], rev2.as_str());

    // Duplicate: "<name> copy", right after the original; the transition comes along.
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/looks/{id}/duplicate"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let copy = r.json();
    let copy_id = copy["id"].as_str().unwrap().to_string();
    assert_eq!(copy["name"], "Lyrics only copy");
    assert_eq!(copy["transition"]["duration"], 0.8);
    assert_eq!(copy["screens"], updated["screens"]);
    // A copy under the source's own name is refused, as a new look would be (D06-5).
    let r = send(
        addr,
        "POST",
        &format!("/api/v1/looks/{}/duplicate", fx.plain_look.id.0),
        json!({"name": "Plain"}),
    )
    .await;
    assert_eq!(r.status, 400, "{}", body(&r));
    assert_eq!(r.json()["error"]["field"], "name");
    let r = send(
        addr,
        "POST",
        &format!("/api/v1/looks/{}/duplicate", fx.plain_look.id.0),
        json!({"name": "Plain two"}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    assert_eq!(
        names(&get_json(addr, "/api/v1/looks").await),
        [
            "Default",
            "Plain",
            "Plain two",
            "Lyrics only",
            "Lyrics only copy"
        ]
    );

    // Reorder: a new sort key moves the look.
    let mut moved = copy.clone();
    moved["sortKey"] = json!("0");
    let r = send_if(
        addr,
        "PUT",
        &format!("/api/v1/looks/{copy_id}"),
        copy["revision"].as_str().unwrap(),
        Some(moved),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    assert_eq!(
        names(&get_json(addr, "/api/v1/looks").await)[0],
        "Lyrics only copy"
    );

    // The live look can't be deleted; another can (If-Match checked).
    ws.ok(json!({"type": "setLook", "look": id})).await;
    let r = send_if(addr, "DELETE", &format!("/api/v1/looks/{id}"), &rev2, None).await;
    assert_eq!(r.status, 409, "{}", body(&r));
    assert!(body(&r).contains("live"), "{}", body(&r));
    let r = send_if(
        addr,
        "DELETE",
        &format!("/api/v1/looks/{copy_id}"),
        &rev,
        None,
    )
    .await;
    assert_eq!(r.status, 409, "stale revision: {}", body(&r));
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/looks/{copy_id}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", body(&r));
    assert!(!look_file(&fx, &copy_id).exists());
    let r = http(addr, "GET", &format!("/api/v1/looks/{copy_id}"), &[], None).await;
    assert_eq!(r.status, 404);
    ws.ok(json!({"type": "setLook", "look": null})).await;
    let r = http(addr, "DELETE", &format!("/api/v1/looks/{id}"), &[], None).await;
    assert_eq!(r.status, 204, "{}", body(&r));

    // A reload sees what the server applied.
    ws.ok(json!({"type": "reloadData"})).await;
    assert_eq!(
        names(&get_json(addr, "/api/v1/looks").await),
        ["Default", "Plain", "Plain two"]
    );
    server.shutdown().await.unwrap();
}

/// Frames of screen 0 as they are published, for `dur`.
async fn record(server: &RunningServer, dur: Duration) -> Vec<image::RgbaImage> {
    let mut out = Vec::new();
    let mut last = None;
    let end = Instant::now() + dur;
    while Instant::now() < end {
        let f = server.shared.screens()[0].latest.lock().unwrap().clone();
        if let Some(f) = f
            && Some(f.sequence) != last
        {
            last = Some(f.sequence);
            out.push(f.to_straight_rgba());
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    out
}

/// The brightest red value in the text band.
fn peak(img: &image::RgbaImage) -> u8 {
    (80..280)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .map(|(x, y)| img.get_pixel(x, y).0[0])
        .max()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn making_a_look_live_plays_its_transition() {
    let fx = fixture("looks-transition");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;
    let aud = fx.audience.id.0.to_string();
    let song = fx.second.id.0.to_string();
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 0}))
        .await;
    wait_frame(&server, 0, "text up", |img| peak(img) > 240).await;

    // A look hiding the slide on the audience screen, with a 1 s dissolve.
    let r = send(
        addr,
        "POST",
        "/api/v1/looks",
        json!({"name": "No slide",
               "screens": [{"screen": aud, "layers": {"slide": false}}],
               "transition": {"kind": {"type": "dissolve"}, "duration": 1.0}}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let look = r.json();
    let id = look["id"].as_str().unwrap().to_string();
    a.ok(json!({"type": "setLook", "look": id})).await;
    let peaks: Vec<u8> = record(&server, Duration::from_millis(1300))
        .await
        .iter()
        .map(peak)
        .collect();
    assert!(peaks.len() >= 15, "{} frames", peaks.len());
    assert!(
        peaks.iter().any(|p| (120..=230).contains(p)),
        "a mid-dissolve frame: {peaks:?}"
    );
    assert!(
        peaks.windows(2).all(|w| w[1] <= w[0].saturating_add(2)),
        "{peaks:?}"
    );
    assert_eq!(*peaks.last().unwrap(), 0, "{peaks:?}");

    // Editing the live look plays its transition too: the slide comes back gradually.
    let mut edited = look.clone();
    edited["screens"] = json!([{"screen": aud, "layers": {"media": false}}]);
    let r = send_if(
        addr,
        "PUT",
        &format!("/api/v1/looks/{id}"),
        look["revision"].as_str().unwrap(),
        Some(edited),
    )
    .await;
    assert_eq!(r.status, 200, "{}", body(&r));
    let peaks: Vec<u8> = record(&server, Duration::from_millis(1300))
        .await
        .iter()
        .map(peak)
        .collect();
    assert!(
        peaks.iter().any(|p| (120..=230).contains(p)),
        "a mid-dissolve frame: {peaks:?}"
    );
    assert!(*peaks.last().unwrap() > 240, "{peaks:?}");

    // A look without a transition cuts: no frame in between.
    let r = send(
        addr,
        "POST",
        "/api/v1/looks",
        json!({"name": "Cut", "screens": [{"screen": aud, "layers": {"slide": false}}]}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    let cut = r.json()["id"].as_str().unwrap().to_string();
    a.ok(json!({"type": "setLook", "look": cut})).await;
    let peaks: Vec<u8> = record(&server, Duration::from_millis(500))
        .await
        .iter()
        .map(peak)
        .collect();
    assert!(
        peaks.iter().all(|p| *p < 20 || *p > 240),
        "a cut: {peaks:?}"
    );
    assert_eq!(*peaks.last().unwrap(), 0, "{peaks:?}");
    server.shutdown().await.unwrap();
}

/// A look's transition can be any kind with its easing: a push moves the slide out (never
/// fading it) when the look hides the slide layer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn making_a_look_live_plays_a_push() {
    let fx = fixture("looks-push");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;
    let aud = fx.audience.id.0.to_string();
    let song = fx.second.id.0.to_string();
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 0}))
        .await;
    let full = wait_frame(&server, 0, "text up", |img| peak(img) > 240).await;
    // The text's rightmost white pixel.
    let right = |img: &image::RgbaImage| {
        (80..280)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .filter(|&(x, y)| img.get_pixel(x, y).0[0] > 200)
            .map(|(x, _)| x)
            .max()
    };
    let right0 = right(&full).unwrap();

    let r = send(
        addr,
        "POST",
        "/api/v1/looks",
        json!({"name": "Push out",
               "screens": [{"screen": aud, "layers": {"slide": false}}],
               "transition": {"kind": {"type": "push", "direction": "left"},
                              "duration": 0.8, "easing": "easeInOut"}}),
    )
    .await;
    assert_eq!(r.status, 201, "{}", body(&r));
    assert_eq!(r.json()["transition"]["easing"], "easeInOut");
    let id = r.json()["id"].as_str().unwrap().to_string();
    a.ok(json!({"type": "setLook", "look": id})).await;
    let frames = record(&server, Duration::from_millis(1300)).await;
    assert!(frames.len() >= 15, "{} frames", frames.len());
    let rights: Vec<Option<u32>> = frames.iter().map(right).collect();
    let shown: Vec<u32> = rights.iter().flatten().copied().collect();
    // It moves left, never back, and some frames show it part-way out...
    assert!(shown.windows(2).all(|w| w[1] <= w[0] + 2), "{rights:?}");
    assert!(
        shown.iter().any(|x| *x + 40 < right0 && *x > 20),
        "a mid-push frame: {rights:?} (from {right0})"
    );
    // ... at full brightness (a push, not a dissolve), and it ends gone.
    for (f, r) in frames.iter().zip(&rights) {
        if r.is_some() {
            assert!(peak(f) > 240, "dimmed: {rights:?}");
        }
    }
    assert!(rights.last().unwrap().is_none(), "{rights:?}");
    server.shutdown().await.unwrap();
}
