//! IDN Open Protocol output end to end: an IDN output created through the screen-config API
//! on a synthetic data folder appears to an IDN receiver with the screen's picture; renaming
//! it makes the old name disappear and the new one appear; deleting it removes it.
//!
//! ```text
//! sg render -c "cargo test -p vesper-server --test server idn_e2e:: -- --nocapture"
//! ```

use crate::common::*;
use output::recv::source_part;
use output::{IdnFinder, IdnReceiver};
use serde_json::json;
use std::time::{Duration, Instant};

/// Waits until the finder's list does (`present`) or does not contain `name`; returns the
/// time it took.
fn wait_listed(finder: &IdnFinder, name: &str, present: bool, timeout: Duration) -> Option<f64> {
    let t = Instant::now();
    while t.elapsed() < timeout {
        let listed = finder
            .sources(Duration::from_millis(200))
            .iter()
            .any(|s| source_part(&s.name) == name);
        if listed == present {
            return Some(t.elapsed().as_secs_f64());
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idn_output_lifecycle_seen_by_a_receiver() {
    let fx = fixture("idn-e2e");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let tag = std::process::id();
    let first = format!("E2E IDN {tag}");
    let second = format!("E2E IDN renamed {tag}");

    // Create.
    let r = http(
        addr,
        "POST",
        "/api/v1/outputs",
        &[],
        Some(json!({"screen": fx.audience.id, "transport": {"type": "idn", "sourceName": first}})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let created = r.json();
    let id = created["id"].as_str().unwrap().to_string();
    let finder = IdnFinder::new().unwrap();
    let t =
        tokio::task::block_in_place(|| wait_listed(&finder, &first, true, Duration::from_secs(20)))
            .expect("the finder lists the new IDN output");
    eprintln!("created: listed by the finder after {t:.2} s");

    // The picture arrives at the screen's size.
    let src = finder.find_by_name(&first, Duration::from_secs(5)).unwrap();
    let frame = tokio::task::block_in_place(|| {
        let mut rx = IdnReceiver::connect(&src.name).unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        let mut got = None;
        while got.is_none() && Instant::now() < end {
            got = rx.capture_video(Duration::from_millis(500)).unwrap();
        }
        got
    })
    .expect("video from the IDN output");
    assert_eq!(
        (frame.width as f64, frame.height as f64),
        (fx.audience.size.width, fx.audience.size.height)
    );
    eprintln!(
        "received {}x{} {} at {}/{}",
        frame.width,
        frame.height,
        frame.fourcc_str(),
        frame.frame_rate.0,
        frame.frame_rate.1
    );
    // The show state reports it running.
    let state = get_json(addr, "/api/v1/state").await;
    let out = state["state"]["screens"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["outputs"].as_array().unwrap().iter())
        .find(|o| o["binding"] == id.as_str())
        .cloned()
        .expect("output in the show state");
    assert_eq!(out["kind"], "idn", "{out}");
    assert_eq!(out["status"], "ok", "{out}");

    // Rename: the old name goes, the new one comes.
    let mut doc = created.clone();
    doc["transport"]["sourceName"] = json!(second);
    doc["name"] = json!(second);
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/outputs/{id}"),
        &[],
        Some(doc),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let (gone, came) = tokio::task::block_in_place(|| {
        (
            wait_listed(&finder, &first, false, Duration::from_secs(20)),
            wait_listed(&finder, &second, true, Duration::from_secs(20)),
        )
    });
    let gone = gone.expect("the old name disappears after the rename");
    let came = came.expect("the new name appears after the rename");
    eprintln!("renamed: old name gone after {gone:.2} s, new name listed {came:.2} s later");

    // Delete.
    let r = http(addr, "DELETE", &format!("/api/v1/outputs/{id}"), &[], None).await;
    assert!(r.status == 200 || r.status == 204, "{}", r.status);
    let gone = tokio::task::block_in_place(|| {
        wait_listed(&finder, &second, false, Duration::from_secs(20))
    })
    .expect("the deleted output disappears");
    eprintln!("deleted: gone after {gone:.2} s");
    drop(server);
}
