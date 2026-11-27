//! Which show boxes show which collection, and how far behind they are (G14): the Codex box
//! learns it from the boxes' requests over the box-to-Codex link and answers it in `GET /codex`
//! (`boxes`) and in each collection's summary (`boxes {showing, synced}`). Synthetic fixtures.

use super::showbox::{codex_status, create_presentation, has_presentation, until, wait_client};
use crate::site::keys::two_boxes_with;
use crate::site::{call, ok};
use serde_json::Value;
use std::net::SocketAddr;
use std::time::Duration;
use vesper_server::codex::CodexOptions;

async fn collection_boxes(addr: SocketAddr, key: &str, id: &str) -> Value {
    let list = ok(
        &call(addr, "GET", "/api/v1/codex/collections", Some(key), None).await,
        200,
        "collections",
    );
    list.as_array()
        .unwrap()
        .iter()
        .find(|c| c["collection"]["id"] == id)
        .unwrap()["boxes"]
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_codex_knows_which_boxes_show_which_collection_and_how_far_behind() {
    // No pushes and a long poll: B moves on only when it asks (here: after its own write).
    let t = two_boxes_with(
        "cx-boxes",
        CodexOptions {
            push_changes: false,
            ..CodexOptions::default()
        },
        CodexOptions {
            mirror_poll: Duration::from_secs(300),
            ..CodexOptions::default()
        },
        |_| {},
    )
    .await;
    let key = t.key_a.clone();
    let b = wait_client(&t).await;
    let shown = b["collection"].as_str().unwrap().to_string();

    // A lists B on the collection it shows, up to date; the collection is shown by both.
    until("A lists B, up to date", 30, || {
        let key = key.clone();
        async move {
            let s = codex_status(t.a.addr, &key).await;
            s["boxes"]
                .as_array()
                .is_some_and(|v| v.len() == 1 && v[0]["behind"] == 0 && v[0]["online"] == true)
        }
    })
    .await;
    let s = codex_status(t.a.addr, &key).await;
    let bx = &s["boxes"][0];
    assert_eq!(bx["collection"], shown.as_str());
    assert!(!bx["fingerprint"].as_str().unwrap().is_empty());
    assert!(!bx["name"].as_str().unwrap().is_empty(), "{bx}");
    assert!(bx["lastSeen"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        collection_boxes(t.a.addr, &key, &shown).await,
        serde_json::json!({"showing": 2, "synced": 2})
    );
    // B itself (a show box) lists no boxes.
    assert!(codex_status(t.b.addr, &key).await.get("boxes").is_none());

    // An edit on A: B hasn't asked since, so it is one change behind.
    create_presentation(t.a.addr, &key, "Status check one").await;
    let s = codex_status(t.a.addr, &key).await;
    assert_eq!(s["boxes"][0]["behind"], 1, "{s}");
    assert_eq!(
        collection_boxes(t.a.addr, &key, &shown).await,
        serde_json::json!({"showing": 2, "synced": 1})
    );

    // B's own write goes through to A and B catches up at once.
    let id = create_presentation(t.b.addr, &key, "Status check two").await;
    assert!(has_presentation(&t.b, &id));
    until("B is up to date again", 10, || {
        let key = key.clone();
        async move { codex_status(t.a.addr, &key).await["boxes"][0]["behind"] == 0 }
    })
    .await;
    assert_eq!(
        collection_boxes(t.a.addr, &key, &shown).await,
        serde_json::json!({"showing": 2, "synced": 2})
    );
    t.end().await;
}
