//! Write-through edits from a show box (M10): two servers in this test process (A, the site's
//! server with the Codex; B, a show box of A's site), synthetic fixtures.
//!
//! - An edit on B goes to A's Codex as B's client did it (the change log names the client and
//!   B) and is in B's copy when B answers (B's poll is far away).
//! - The same document edited through both boxes: the stale one gets `409` with `current`.
//! - The trash through B; uploads and collection administration on B answer where the Codex
//!   box is; search on B (its own collection here, `all` asked of A).
//! - A restore of B's settings queued on A reaches B at once (the Codex tells B; B's poll is
//!   far away).
//! - A down: edits and searches beyond B's collection answer `503`; B keeps its copy.

use super::showbox::{codex_status, create_presentation, has_presentation, until, wait_client};
use crate::common::*;
use crate::site::keys::{TwoBoxes, two_boxes_with};
use crate::site::{call, ok};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Duration;
use vesper_server::codex::CodexOptions;
use vesper_server::codex::backups::BackupOptions;

const B_ID: &str = "6a1d1c8e-0000-4000-8000-0000000000b0";

async fn call_h(
    addr: SocketAddr,
    method: &str,
    path: &str,
    key: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Resp {
    let bearer = format!("Bearer {key}");
    let mut h: Vec<(&str, &str)> = vec![("Authorization", bearer.as_str())];
    h.extend_from_slice(headers);
    http(addr, method, path, &h, body).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn show_box_edits_go_through_to_the_codex() {
    // B asks the Codex only when it writes: its poll is far away (pushes don't reach show
    // boxes yet).
    let t = two_boxes_with(
        "cx-write-through",
        CodexOptions::default(),
        CodexOptions {
            mirror_poll: Duration::from_secs(600),
            backups: BackupOptions {
                poll: Duration::from_secs(1),
                change_delay: Duration::from_secs(3600),
                ..BackupOptions::default()
            },
            ..CodexOptions::default()
        },
        |_| {},
    )
    .await;
    let (a, b, key) = (t.a.addr, t.b.addr, t.key_a.clone());
    wait_client(&t).await;
    let col = codex_status(a, &key).await["collection"]
        .as_str()
        .unwrap()
        .to_string();

    // An edit on B lands on A and is in B's copy when B answers.
    let id = create_presentation(b, &key, "Written on B").await;
    assert!(has_presentation(&t.a, &id), "on the Codex");
    assert!(has_presentation(&t.b, &id), "in B's copy at once");
    let rel = format!("collections/{col}/portable/presentation/{id}.json");
    assert_eq!(
        std::fs::read(t.fb.dir.join("codex-cache").join(&rel)).unwrap(),
        std::fs::read(t.fa.dir.join("codex").join(&rel)).unwrap()
    );
    // The change log names B's client and B.
    let changes = ok(
        &call(
            a,
            "GET",
            &format!("/api/v1/codex/collections/{col}/changes?since=0"),
            Some(&key),
            None,
        )
        .await,
        200,
        "A's change log",
    );
    let last = changes["changes"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|c| c["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(last["by"]["client"], "Oriel – Booth", "{last}");
    assert_eq!(last["by"]["machine"], B_ID, "{last}");
    assert!(last["by"]["clientId"].as_str().is_some(), "{last}");
    // Validation is the Codex's: the same 400 as there.
    let r = call(
        b,
        "POST",
        "/api/v1/timers",
        Some(&key),
        Some(json!({"name": "Bad", "kind": {"type": "countdown", "seconds": -5}})),
    )
    .await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["error"]["field"], "kind.seconds");

    // The same document edited through both boxes: the stale write gets 409 with `current`.
    let r = call(
        b,
        "POST",
        "/api/v1/timers",
        Some(&key),
        Some(json!({"name": "Walk-in", "kind": {"type": "countdown", "seconds": 300}})),
    )
    .await;
    let timer = ok(&r, 201, "a timer through B");
    let tid = timer["id"].as_str().unwrap().to_string();
    let tag = format!("\"{}\"", timer["revision"].as_str().unwrap());
    let mut doc = timer.clone();
    doc["name"] = json!("Walk-in (A)");
    let r = call_h(
        a,
        "PUT",
        &format!("/api/v1/timers/{tid}"),
        &key,
        &[("If-Match", &tag)],
        Some(doc.clone()),
    )
    .await;
    ok(&r, 200, "the edit on A");
    doc["name"] = json!("Walk-in (B)");
    let r = call_h(
        b,
        "PUT",
        &format!("/api/v1/timers/{tid}"),
        &key,
        &[("If-Match", &tag)],
        Some(doc),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["current"]["name"], "Walk-in (A)", "{:?}", r.json());

    // The trash through B: the Codex's trash, restored into B's copy at once.
    let r = call(
        b,
        "DELETE",
        &format!("/api/v1/presentations/{id}"),
        Some(&key),
        None,
    )
    .await;
    assert!(r.status < 300, "{}", String::from_utf8_lossy(&r.body));
    assert!(!has_presentation(&t.b, &id));
    let trash = ok(
        &call(b, "GET", "/api/v1/codex/trash", Some(&key), None).await,
        200,
        "the trash through B",
    );
    let entry = trash
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["docId"] == id.as_str())
        .unwrap_or_else(|| panic!("{trash}"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = call(
        b,
        "POST",
        &format!("/api/v1/codex/trash/{entry}/restore"),
        Some(&key),
        None,
    )
    .await;
    ok(&r, 200, "restore through B");
    assert!(has_presentation(&t.b, &id) && has_presentation(&t.a, &id));

    // Uploads and collection administration on B: where the Codex box is.
    let a_id = t.a.shared.auth.identity().id.to_string();
    for (method, path, body) in [
        (
            "POST",
            "/api/v1/codex/uploads",
            Some(json!({"collection": col, "name": "x.png", "size": 1, "sha256": "00"})),
        ),
        ("GET", "/api/v1/codex/uploads", None),
        (
            "POST",
            "/api/v1/codex/collections",
            Some(json!({"name": "Not here"})),
        ),
        // Settings backups (but offering B's own snapshot) and export/import.
        ("GET", "/api/v1/codex/backups", None),
        ("GET", "/api/v1/codex/backups/oriel", None),
        (
            "PUT",
            "/api/v1/codex/backups/oriel",
            Some(json!({"schemaVersion": 1, "preferences": {}})),
        ),
        ("GET", "/api/v1/codex/backups/restores", None),
        (
            "POST",
            "/api/v1/codex/export",
            Some(json!({"passphrase": "long enough"})),
        ),
        ("GET", "/api/v1/codex/export/settings", None),
        ("GET", "/api/v1/codex/export/scheduled", None),
        ("POST", "/api/v1/codex/export/imports", Some(json!({}))),
        (
            "POST",
            "/api/v1/codex/export/imports/x/apply",
            Some(json!({"passphrase": "long enough"})),
        ),
    ] {
        let r = call(b, method, path, Some(&key), body).await;
        assert_eq!(
            r.status,
            409,
            "{method} {path}: {}",
            String::from_utf8_lossy(&r.body)
        );
        let e = r.json();
        assert_eq!(e["error"]["code"], "codexElsewhere", "{e}");
        assert_eq!(e["codex"]["serverId"], a_id.as_str(), "{e}");
        assert!(
            e["codex"]["addresses"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| *x == format!("http://{a}").as_str()),
            "{e}"
        );
    }

    // Search on B: its own collection here; `all` asked of the Codex.
    let r = ok(
        &call(b, "GET", "/api/v1/codex/search?q=written", Some(&key), None).await,
        200,
        "search on B",
    );
    assert!(r["hits"][0]["id"] == id.as_str(), "{r}");
    let all_b = ok(
        &call(
            b,
            "GET",
            "/api/v1/codex/search?q=written&collection=all",
            Some(&key),
            None,
        )
        .await,
        200,
        "search all on B",
    );
    let all_a = ok(
        &call(
            a,
            "GET",
            "/api/v1/codex/search?q=written&collection=all",
            Some(&key),
            None,
        )
        .await,
        200,
        "search all on A",
    );
    assert_eq!(all_b, all_a);

    // A restore queued on A reaches B at once: A tells B (B's poll is far away).
    until("B's settings reach the Codex", 30, || {
        let key = key.clone();
        async move {
            let r = call(a, "GET", "/api/v1/codex/backups", Some(&key), None).await;
            ok(&r, 200, "sets")
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["set"]["machine"] == B_ID)
        }
    })
    .await;
    let sets = ok(
        &call(a, "GET", "/api/v1/codex/backups", Some(&key), None).await,
        200,
        "sets",
    );
    let set = sets
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["set"]["machine"] == B_ID)
        .unwrap()
        .clone();
    let r = call(
        a,
        "POST",
        &format!(
            "/api/v1/codex/backups/{}/snapshots/{}/restore",
            set["set"]["id"].as_str().unwrap(),
            set["latest"]["id"].as_str().unwrap()
        ),
        Some(&key),
        None,
    )
    .await;
    let rid = ok(&r, 202, "a restore of B")["id"]
        .as_str()
        .unwrap()
        .to_string();
    until("B takes the restore at once", 20, || {
        let (key, rid) = (key.clone(), rid.clone());
        async move {
            let r = call(
                a,
                "GET",
                &format!("/api/v1/codex/backups/restores/{rid}"),
                Some(&key),
                None,
            )
            .await;
            ok(&r, 200, "the restore")["state"] == "applied"
        }
    })
    .await;

    // A down: 503 for edits (the show keeps playing from B's copy).
    let TwoBoxes {
        fa,
        a: sa,
        fb,
        b: sb,
        ..
    } = t;
    sa.shutdown().await.unwrap();
    let libs = ok(
        &call(b, "GET", "/api/v1/libraries", Some(&key), None).await,
        200,
        "B's libraries",
    );
    let r = call(
        b,
        "POST",
        "/api/v1/presentations",
        Some(&key),
        Some(json!({"name": "Offline", "library": libs[0]["id"]})),
    )
    .await;
    assert_eq!(r.status, 503, "{}", String::from_utf8_lossy(&r.body));
    let e = r.json();
    assert_eq!(e["error"]["code"], "unavailable");
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("site's server"),
        "{e}"
    );
    let r = call(
        b,
        "GET",
        "/api/v1/codex/search?q=written&collection=all",
        Some(&key),
        None,
    )
    .await;
    assert_eq!(r.status, 503, "{}", String::from_utf8_lossy(&r.body));
    let r = ok(
        &call(b, "GET", "/api/v1/codex/search?q=written", Some(&key), None).await,
        200,
        "B's own search without the Codex",
    );
    assert_eq!(r["hits"][0]["id"], id.as_str(), "{r}");
    assert!(has_presentation(&sb, &id));
    sb.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fa.dir).ok();
    std::fs::remove_dir_all(&fb.dir).ok();
}

/// A forwarded write while a long job holds the Codex's writer is refused before the show box
/// gives up, and never applied later (A06-4): the client's "failed" is true, a retry can't
/// make a second copy.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_write_the_show_box_gave_up_on_is_never_applied() {
    let t = two_boxes_with(
        "cx-write-busy",
        CodexOptions::default(),
        CodexOptions {
            mirror_poll: Duration::from_secs(600),
            ..CodexOptions::default()
        },
        |_| {},
    )
    .await;
    let (b, key) = (t.b.addr, t.key_a.clone());
    wait_client(&t).await;
    let libs = ok(
        &call(b, "GET", "/api/v1/libraries", Some(&key), None).await,
        200,
        "libraries",
    );
    let lib = libs[0]["id"].as_str().unwrap().to_string();
    let count = |s: &vesper_server::RunningServer| {
        s.shared
            .data()
            .content
            .presentations
            .iter()
            .filter(|p| p.name == "While busy")
            .count()
    };

    // A long job (an import, an export) holds A's writer past the box's wait.
    let held = t.a.shared.clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let (got_tx, got_rx) = tokio::sync::oneshot::channel::<()>();
    let job = tokio::spawn(async move {
        let _w = held.codex_writer.exclusive().await;
        let _ = got_tx.send(());
        let _ = rx.await;
    });
    got_rx.await.unwrap();
    let started = std::time::Instant::now();
    let r = call(
        b,
        "POST",
        "/api/v1/presentations",
        Some(&key),
        Some(json!({"name": "While busy", "library": lib})),
    )
    .await;
    assert_eq!(r.status, 503, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        String::from_utf8_lossy(&r.body).contains("busy"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert!(
        started.elapsed() < vesper_server::codex::write_through::TIMEOUT,
        "answered by the Codex, before the box gave up"
    );

    // The job ends: the refused write is not applied after all.
    tx.send(()).unwrap();
    job.await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(count(&t.a), 0, "never applied on the Codex");
    // A retry now goes through, once.
    let r = call(
        b,
        "POST",
        "/api/v1/presentations",
        Some(&key),
        Some(json!({"name": "While busy", "library": lib})),
    )
    .await;
    ok(&r, 201, "the retry");
    assert_eq!(count(&t.a), 1);
    t.end().await;
}

/// A long job that queues for the Codex's writer while a forwarded write waits for it doesn't
/// slip in ahead of the write (D04-3): the write goes in when the writer is free, answered
/// before the box gives up, and is never applied after a timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_job_queued_behind_a_forwarded_write_doesnt_jump_ahead() {
    let t = two_boxes_with(
        "cx-write-queue",
        CodexOptions::default(),
        CodexOptions {
            mirror_poll: Duration::from_secs(600),
            ..CodexOptions::default()
        },
        |_| {},
    )
    .await;
    let (b, key) = (t.b.addr, t.key_a.clone());
    wait_client(&t).await;
    let libs = ok(
        &call(b, "GET", "/api/v1/libraries", Some(&key), None).await,
        200,
        "libraries",
    );
    let lib = libs[0]["id"].as_str().unwrap().to_string();
    let count = |s: &vesper_server::RunningServer| {
        s.shared
            .data()
            .content
            .presentations
            .iter()
            .filter(|p| p.name == "Queued")
            .count()
    };

    // A short write holds A's writer while the forwarded write comes in.
    let held = t.a.shared.clone();
    let (tx1, rx1) = tokio::sync::oneshot::channel::<()>();
    let (got_tx, got_rx) = tokio::sync::oneshot::channel::<()>();
    let short = tokio::spawn(async move {
        let _w = held.codex_writer.exclusive().await;
        let _ = got_tx.send(());
        let _ = rx1.await;
    });
    got_rx.await.unwrap();
    let started = std::time::Instant::now();
    let write = tokio::spawn(async move {
        call(
            b,
            "POST",
            "/api/v1/presentations",
            Some(&key),
            Some(json!({"name": "Queued", "library": lib})),
        )
        .await
    });
    tokio::time::sleep(Duration::from_secs(2)).await;
    // A long job (an export) queues for the writer after it, and holds it a long time.
    let held = t.a.shared.clone();
    let (tx2, rx2) = tokio::sync::oneshot::channel::<()>();
    let long = tokio::spawn(async move {
        let _w = held.codex_writer.exclusive().await;
        let _ = rx2.await;
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    tx1.send(()).unwrap();
    short.await.unwrap();

    let r = write.await.unwrap();
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        started.elapsed() < vesper_server::codex::write_through::TIMEOUT,
        "answered before the box gave up"
    );
    assert_eq!(count(&t.a), 1, "applied once, ahead of the long job");
    tx2.send(()).unwrap();
    long.await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(count(&t.a), 1);
    t.end().await;
}
