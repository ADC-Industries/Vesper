//! Show boxes' settings backups and restores over the box-to-Codex link: two servers in this
//! test process (A, the site's server with the Codex; B, a show box of A's site), synthetic
//! fixtures.
//!
//! - B offers its settings on its own backup schedule and on demand; they land in B's set on
//!   A, bound to B's device in the site. Offers claiming another machine are refused.
//! - A restore of B's set asked on A is queued (`202`); B takes it at its next poll: its
//!   `beforeRestore` snapshot first (the undo), then the files and a reload; the queued
//!   restore reads `applied` with the result.
//! - B offline: the restore waits (`queued`), can be cancelled, and is applied once B is back.

use super::showbox::{until, wait_client};
use crate::common::*;
use crate::site::keys::{TwoBoxes, two_boxes_with};
use crate::site::{call, ok, start_with_codex};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Duration;
use vesper_server::codex::CodexOptions;
use vesper_server::codex::backups::{BackupOptions, SnapshotOffer, this_box_offer};
use vesper_server::site::SiteConfig;

const B_ID: &str = "6a1d1c8e-0000-4000-8000-0000000000b0";

/// A: no snapshots of its own while the test runs.
fn quiet() -> CodexOptions {
    CodexOptions {
        backups: BackupOptions {
            poll: Duration::from_secs(3600),
            change_delay: Duration::from_secs(3600),
            ..BackupOptions::default()
        },
        ..CodexOptions::default()
    }
}

/// B: polls the Codex every second, looks at its settings every second (the first look
/// offers them: no newest snapshot known yet), no change snapshots.
fn show_box() -> CodexOptions {
    CodexOptions {
        mirror_poll: Duration::from_secs(1),
        backups: BackupOptions {
            poll: Duration::from_secs(1),
            change_delay: Duration::from_secs(3600),
            ..BackupOptions::default()
        },
        ..CodexOptions::default()
    }
}

/// B's backup set as A lists it.
async fn box_set(a: SocketAddr, key: &str) -> Option<Value> {
    let r = call(a, "GET", "/api/v1/codex/backups", Some(key), None).await;
    ok(&r, 200, "backup sets")
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["set"]["machine"] == B_ID)
        .cloned()
}

async fn rename_screen(addr: SocketAddr, key: &str, id: &str, name: &str) {
    let list = ok(
        &call(addr, "GET", "/api/v1/screens", Some(key), None).await,
        200,
        "screens",
    );
    let mut doc = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap()
        .clone();
    let tag = format!("\"{}\"", doc["revision"].as_str().unwrap());
    doc["name"] = json!(name);
    let bearer = format!("Bearer {key}");
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/screens/{id}"),
        &[
            ("If-Match", tag.as_str()),
            ("Authorization", bearer.as_str()),
        ],
        Some(doc),
    )
    .await;
    ok(&r, 200, "rename a screen");
}

async fn screen_name(addr: SocketAddr, key: &str, id: &str) -> String {
    let list = ok(
        &call(addr, "GET", "/api/v1/screens", Some(key), None).await,
        200,
        "screens",
    );
    list.as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .map(|s| s["name"].as_str().unwrap().to_string())
        .unwrap()
}

async fn restore_state(a: SocketAddr, key: &str, id: &str) -> Value {
    ok(
        &call(
            a,
            "GET",
            &format!("/api/v1/codex/backups/restores/{id}"),
            Some(key),
            None,
        )
        .await,
        200,
        "a queued restore",
    )
}

/// B offers `offer` over its link (as its schedule does), off the async threads.
async fn offer_from(
    b: &vesper_server::RunningServer,
    offer: SnapshotOffer,
) -> Result<protocol::SnapshotTaken, protocol::ApiError> {
    let sh = b.shared.clone();
    tokio::task::spawn_blocking(move || sh.mirror.offer_backup(offer))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_show_boxs_settings_are_backed_up_and_restored_through_the_codex() {
    let t = two_boxes_with("cx-box-backups", quiet(), show_box(), |_| {}).await;
    let (a, b, key) = (t.a.addr, t.b.addr, t.key_a.clone());
    wait_client(&t).await;
    let b_fp = t.b.shared.site.membership().fingerprint.unwrap();

    // B's snapshot arrives on the Codex, in B's set, bound to B's device.
    until("B's settings reach the Codex", 30, || {
        let key = key.clone();
        async move { box_set(a, &key).await.is_some() }
    })
    .await;
    let set = box_set(a, &key).await.unwrap();
    assert_eq!(set["set"]["fingerprint"], b_fp.as_str(), "{set}");
    assert_eq!(set["set"]["name"], "BOX-B", "{set}");
    assert_eq!(set["thisBox"], false, "{set}");
    let set_id = set["set"]["id"].as_str().unwrap().to_string();
    let first = set["latest"]["id"].as_str().unwrap().to_string();
    // On demand on B: offered to the Codex (unchanged: the newest is answered).
    let r = call(
        b,
        "POST",
        "/api/v1/codex/backups/snapshots",
        Some(&key),
        None,
    )
    .await;
    let taken = ok(&r, 200, "a snapshot on demand on B");
    assert_eq!(taken["snapshot"]["id"], first.as_str(), "{taken}");
    assert_eq!(taken["created"], false, "{taken}");

    // Offers claiming another machine are refused: the Codex box's own server ID, another
    // device's fingerprint, a server ID whose set another device of the site offered.
    let by = model::ChangeBy::default();
    let mine = this_box_offer(&t.b.shared, model::SnapshotReason::Manual, by).unwrap();
    let a_id = t.a.shared.auth.identity().id.to_string();
    let e = offer_from(
        &t.b,
        SnapshotOffer {
            machine: a_id,
            ..mine.clone()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, protocol::ErrorCode::Forbidden, "{e:?}");
    let e = offer_from(
        &t.b,
        SnapshotOffer {
            fingerprint: Some("ab".repeat(32)),
            ..mine.clone()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, protocol::ErrorCode::Forbidden, "{e:?}");
    let other = "6a1d1c8e-0000-4000-8000-0000000000c0".to_string();
    t.a.shared
        .backups
        .accept(
            &t.a.shared.codex_server(),
            SnapshotOffer {
                machine: other.clone(),
                name: "BOX-C".into(),
                fingerprint: Some("cd".repeat(32)),
                ..mine.clone()
            },
        )
        .await
        .unwrap();
    let e = offer_from(
        &t.b,
        SnapshotOffer {
            machine: other,
            ..mine.clone()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, protocol::ErrorCode::Forbidden, "{e:?}");
    assert!(e.message.contains("BOX-C"), "{e:?}");

    // A restore asked on A is queued; B takes it at its poll, with the undo snapshot.
    let screen = t.fb.audience.id.0.to_string();
    let original = t.fb.audience.name.clone();
    rename_screen(b, &key, &screen, "Renamed on B").await;
    let r = call(
        a,
        "POST",
        &format!("/api/v1/codex/backups/{set_id}/snapshots/{first}/restore"),
        Some(&key),
        None,
    )
    .await;
    let q = ok(&r, 202, "a restore of B's settings");
    assert_eq!(q["target"], B_ID, "{q}");
    assert_eq!(q["targetName"], "BOX-B", "{q}");
    assert!(
        q["state"] == "queued" || q["state"] == "applying" || q["state"] == "applied",
        "{q}"
    );
    let id = q["id"].as_str().unwrap().to_string();
    until("B applies the queued restore", 30, || {
        let (key, id) = (key.clone(), id.clone());
        async move { restore_state(a, &key, &id).await["state"] == "applied" }
    })
    .await;
    assert_eq!(screen_name(b, &key, &screen).await, original);
    let done = restore_state(a, &key, &id).await;
    let result = &done["result"];
    assert!(
        result["applied"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "screen" && c["change"] == "changed"),
        "{done}"
    );
    assert_eq!(result["beforeRestore"]["reason"], "beforeRestore", "{done}");
    // The undo holds B's settings as they were: the renamed screen.
    let before = result["beforeRestore"]["id"].as_str().unwrap();
    let contents = ok(
        &call(
            a,
            "GET",
            &format!("/api/v1/codex/backups/{set_id}/snapshots/{before}"),
            Some(&key),
            None,
        )
        .await,
        200,
        "the undo snapshot",
    );
    assert!(
        contents["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["content"]["name"] == "Renamed on B"),
        "{contents}"
    );
    let list = ok(
        &call(a, "GET", "/api/v1/codex/backups/restores", Some(&key), None).await,
        200,
        "queued restores",
    );
    assert_eq!(list[0]["id"], id.as_str(), "{list}");

    // B offline: a restore waits for it, can be cancelled, and is applied once B is back.
    let TwoBoxes {
        fa,
        a: sa,
        fb,
        b: sb,
        ..
    } = t;
    sb.shutdown().await.unwrap();
    let path = format!("/api/v1/codex/backups/{set_id}/snapshots/{first}/restore");
    let items = json!({"items": [format!("portable/screen/{screen}.json")]});
    let restore = |key: String| {
        let (path, items) = (path.clone(), items.clone());
        async move { call(a, "POST", &path, Some(&key), Some(items)).await }
    };
    let q = ok(&restore(key.clone()).await, 202, "a restore while B is off");
    let id = q["id"].as_str().unwrap().to_string();
    let r = restore(key.clone()).await;
    assert_eq!(r.status, 409, "one at a time per box");
    let r = call(
        a,
        "DELETE",
        &format!("/api/v1/codex/backups/restores/{id}"),
        Some(&key),
        None,
    )
    .await;
    assert_eq!(ok(&r, 200, "cancel")["state"], "cancelled");
    let q = ok(&restore(key.clone()).await, 202, "queued again");
    let id = q["id"].as_str().unwrap().to_string();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(restore_state(a, &key, &id).await["state"], "queued");
    let sb = start_with_codex(&fb, false, SiteConfig::loopback(), show_box()).await;
    until("B takes the restore once it is back", 60, || {
        let (key, id) = (key.clone(), id.clone());
        async move { restore_state(a, &key, &id).await["state"] == "applied" }
    })
    .await;
    // (Nothing to change: the screen is as the snapshot has it.)
    let done = restore_state(a, &key, &id).await;
    assert_eq!(done["result"]["summary"], "nothing would change", "{done}");
    assert_eq!(screen_name(sb.addr, &key, &screen).await, original);
    sb.shutdown().await.unwrap();
    sa.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fa.dir).ok();
    std::fs::remove_dir_all(&fb.dir).ok();
}
