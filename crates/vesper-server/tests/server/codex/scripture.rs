//! Scripture passages of a show box's collection (D06-2): a theme edited through a show box
//! splits again the passages of the collection the show box shows, on the Codex, not those of
//! the Codex box's own active collection. Synthetic fixtures and translation.

use super::showbox::{until, wait_client};
use crate::site::keys::two_boxes_prepared;
use crate::site::{call, ok};
use serde_json::{Value, json};
use std::time::Duration;
use vesper_server::codex::CodexOptions;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_theme_edited_through_a_show_box_resplits_its_collections_passages() {
    let t = two_boxes_prepared(
        "cx-scripture",
        CodexOptions::default(),
        CodexOptions {
            mirror_poll: Duration::from_secs(600),
            ..CodexOptions::default()
        },
        |f| crate::scripture::write_translation(&f.dir),
        |f| crate::scripture::write_translation(&f.dir),
    )
    .await;
    let (a, b, key) = (t.a.addr, t.b.addr, t.key_a.clone());
    wait_client(&t).await;
    // B shows a second collection of A's Codex.
    let r = call(
        a,
        "POST",
        "/api/v1/codex/collections",
        Some(&key),
        Some(json!({"name": "Youth"})),
    )
    .await;
    let youth = ok(&r, 201, "a second collection on A")["collection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = call(
        b,
        "PUT",
        "/api/v1/codex/assignment",
        Some(&key),
        Some(json!({"collection": youth})),
    )
    .await;
    ok(&r, 200, "B shows Youth");

    // On B: a theme and a passage made with it (both written to Youth on A).
    let r = call(
        b,
        "POST",
        "/api/v1/themes",
        Some(&key),
        Some(json!({"name": "Readings"})),
    )
    .await;
    let tid = ok(&r, 201, "a theme through B")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = call(b, "GET", &format!("/api/v1/themes/{tid}"), Some(&key), None).await;
    let tpl = ok(&r, 200, "the theme on B")["theme"]["slides"][2].clone();
    assert_eq!(tpl["name"], "Scripture");
    let r = call(
        b,
        "POST",
        "/api/v1/scripture/presentations",
        Some(&key),
        Some(json!({"translation": "TST", "reference": "John 2:1-6", "theme": tid})),
    )
    .await;
    let pid = ok(&r, 201, "a passage through B")["presentation"]
        .as_str()
        .unwrap()
        .to_string();
    let file = t.fa.dir.join(format!(
        "codex/collections/{youth}/portable/presentation/{pid}.json"
    ));
    let digest = move || -> Value {
        let p: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        p["scripture"]["generated"]["slides"].clone()
    };
    let before = digest();
    assert!(before.is_string(), "the passage is in Youth on A");

    // The theme's passage box shrinks, edited on B: A splits the passage in Youth again.
    let slot = &tpl["slide"]["elements"][0];
    let small =
        json!({"x": slot["bounds"]["x"], "y": slot["bounds"]["y"], "width": 200.0, "height": 40.0});
    let r = call(
        b,
        "POST",
        &format!("/api/v1/themes/{tid}/edits"),
        Some(&key),
        Some(json!({"ops": [{"type": "setElement", "slide": tpl["slide"]["id"], "element": slot["id"], "patch": {"bounds": small}}]})),
    )
    .await;
    ok(&r, 200, "the theme edited through B");
    until("the passage in Youth is split again", 30, || {
        let (digest, before) = (digest.clone(), before.clone());
        async move { digest() != before }
    })
    .await;
}
