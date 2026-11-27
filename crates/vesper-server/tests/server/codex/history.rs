//! A collection's change log (`changes?since=`) after edits and reloads, and the trash:
//! deleted documents come back with their IDs; purging needs an admin.

use crate::common::*;
use serde_json::{Value, json};
use std::net::SocketAddr;

async fn call(addr: SocketAddr, method: &str, path: &str, body: Option<Value>) -> Resp {
    http(addr, method, path, &[], body).await
}

fn ok(r: &Resp, status: u16, what: &str) -> Value {
    assert_eq!(
        r.status,
        status,
        "{what}: {}",
        String::from_utf8_lossy(&r.body)
    );
    r.json()
}

async fn changes(addr: SocketAddr, collection: &str, since: u64) -> Value {
    get_json(
        addr,
        &format!("/api/v1/codex/collections/{collection}/changes?since={since}"),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changes_since_follow_edits_and_reloads() {
    let fx = fixture("codex-changes");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();

    // A fresh Codex: an empty log.
    let c = changes(addr, &main, 0).await;
    assert_eq!(
        c,
        json!({"from": 0, "to": 0, "changes": [], "resync": false})
    );

    // Create, edit, delete: three entries, in order, with who did it.
    let r = call(
        addr,
        "POST",
        "/api/v1/presentations",
        Some(json!({"name": "Welcome"})),
    )
    .await;
    let created = ok(&r, 201, "create");
    let pid = created["id"].as_str().unwrap().to_string();
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/presentations/{pid}/edits"),
        Some(json!({"ops": [{"type": "setPresentation", "name": "Welcome!"}]})),
    )
    .await;
    let edited = ok(&r, 200, "edit");
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/presentations/{pid}"),
            None,
        )
        .await,
        200,
        "delete",
    );
    let c = changes(addr, &main, 0).await;
    assert_eq!(
        (c["from"].as_u64(), c["to"].as_u64()),
        (Some(0), Some(3)),
        "{c}"
    );
    assert_eq!(c["resync"], false);
    let list = c["changes"].as_array().unwrap();
    let seen: Vec<(u64, &str, &str, &str)> = list
        .iter()
        .map(|e| {
            (
                e["seq"].as_u64().unwrap(),
                e["kind"].as_str().unwrap(),
                e["id"].as_str().unwrap(),
                e["change"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (1, "presentation", pid.as_str(), "added"),
            (2, "presentation", pid.as_str(), "changed"),
            (3, "presentation", pid.as_str(), "removed"),
        ]
    );
    assert_eq!(list[0]["revision"], created["revision"]);
    assert_eq!(list[1]["revision"], edited["revision"]);
    assert!(list[2].get("revision").is_none());
    let server_id = get_json(addr, "/api/v1/server").await["id"].clone();
    for e in list {
        assert_eq!(e["by"]["client"], "server machine", "{e}");
        assert_eq!(e["by"]["machine"], server_id);
        assert!(e["at"].as_str().unwrap().ends_with('Z'));
    }
    let on_disk = std::fs::read_to_string(content_dir(&fx.dir).join("changes.jsonl")).unwrap();
    assert_eq!(on_disk.lines().count(), 3);

    // Since a seq: only what follows; from beyond the head: resync.
    let c = changes(addr, &main, 2).await;
    assert_eq!(c["changes"].as_array().unwrap().len(), 1);
    assert_eq!(c["changes"][0]["seq"], 3);
    let c = changes(addr, &main, 3).await;
    assert_eq!(
        (c["to"].as_u64(), c["changes"].as_array().unwrap().len()),
        (Some(3), 0)
    );
    let c = changes(addr, &main, 50).await;
    assert_eq!(c["resync"], true);
    assert_eq!(c["to"], 3);
    let r = call(
        addr,
        "GET",
        &format!("/api/v1/codex/collections/{main}/changes?since=x"),
        None,
    )
    .await;
    assert_eq!(r.status, 400);
    let r = call(
        addr,
        "GET",
        "/api/v1/codex/collections/0b3c5a51-7f63-4d1c-9a33-3a7c0e1f8d2e/changes?since=0",
        None,
    )
    .await;
    assert_eq!(r.status, 404);

    // A document changed on disk, picked up by a reload, is logged too.
    let song = fx.song.id.0.to_string();
    let path = content_dir(&fx.dir).join(format!("portable/presentation/{song}.json"));
    let mut doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    doc["name"] = json!("Renamed on disk");
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/commands",
            Some(json!({"type": "reloadData"})),
        )
        .await,
        200,
        "reload",
    );
    let c = changes(addr, &main, 3).await;
    let list = c["changes"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{c}");
    assert_eq!(list[0]["id"], song.as_str());
    assert_eq!(list[0]["change"], "changed");
    assert_eq!(list[0]["by"]["client"], "reload");

    // Each collection has its own log.
    let r = call(
        addr,
        "POST",
        "/api/v1/codex/collections",
        Some(json!({"name": "Chapel"})),
    )
    .await;
    let chapel = ok(&r, 201, "create a collection")["collection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(changes(addr, &chapel, 0).await["to"], 0);
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_trash_restores_documents_with_their_ids() {
    let fx = fixture("codex-trash");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(get_json(addr, "/api/v1/codex/trash").await, json!([]));

    // A presentation: deleted, listed in the trash, restored with its ID and content.
    let song = fx.song.id.0.to_string();
    let before = get_json(addr, &format!("/api/v1/presentations/{song}")).await;
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/presentations/{song}"),
            None,
        )
        .await,
        200,
        "delete the song",
    );
    assert_eq!(
        call(addr, "GET", &format!("/api/v1/presentations/{song}"), None)
            .await
            .status,
        404
    );
    let trash = get_json(addr, "/api/v1/codex/trash").await;
    let items = trash.as_array().unwrap();
    assert_eq!(items.len(), 1, "{trash}");
    let item = &items[0];
    assert_eq!(item["kind"], "presentation");
    assert_eq!(item["docId"], song.as_str());
    assert_eq!(item["name"], fx.song.name.as_str());
    assert_eq!(item["collection"], main.as_str());
    assert_eq!(item["by"]["client"], "server machine");
    assert!(item["expiresAt"].as_str().unwrap() > item["deletedAt"].as_str().unwrap());
    let entry = item["id"].as_str().unwrap().to_string();
    assert!(fx.dir.join(format!("codex/trash/{entry}.json")).is_file());
    assert_eq!(get_json(addr, "/api/v1/codex").await["trash"], 1);
    loop {
        let m = ws.next_any().await;
        if m["type"] == "codexChanged" && m["parts"].as_array().unwrap().contains(&json!("trash")) {
            break;
        }
    }

    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/trash/{entry}/restore"),
        None,
    )
    .await;
    let restored = ok(&r, 200, "restore");
    assert_eq!(
        restored,
        json!({"collection": main, "kind": "presentation", "documents": [song]})
    );
    let after = get_json(addr, &format!("/api/v1/presentations/{song}")).await;
    assert_eq!(after, before, "the same document, with its ID");
    assert_eq!(get_json(addr, "/api/v1/codex/trash").await, json!([]));
    assert!(!fx.dir.join(format!("codex/trash/{entry}.json")).exists());
    assert_eq!(
        call(
            addr,
            "POST",
            &format!("/api/v1/codex/trash/{entry}/restore"),
            None
        )
        .await
        .status,
        404,
        "restored once"
    );
    // The log has the removal and the restore.
    let c = changes(addr, &main, 0).await;
    let last: Vec<&str> = c["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["change"].as_str().unwrap())
        .collect();
    assert_eq!(last, ["removed", "added"]);

    // A folder: its subtree goes into one entry and comes back with it.
    let folder = fx.folder.id.0.to_string();
    let tree_before = get_json(addr, "/api/v1/playlists").await;
    let playlist_before = get_json(addr, &format!("/api/v1/playlists/{}", fx.playlist.id.0)).await;
    ok(
        &call(addr, "DELETE", &format!("/api/v1/playlists/{folder}"), None).await,
        200,
        "delete the folder",
    );
    let trash = get_json(addr, "/api/v1/codex/trash").await;
    assert_eq!(trash[0]["docId"], folder.as_str());
    assert_eq!(trash[0]["subtree"], 1, "{trash}");
    let entry = trash[0]["id"].as_str().unwrap().to_string();
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/trash/{entry}/restore"),
        None,
    )
    .await;
    let restored = ok(&r, 200, "restore the folder");
    assert_eq!(
        restored["documents"],
        json!([folder, fx.playlist.id.0.to_string()])
    );
    assert_eq!(get_json(addr, "/api/v1/playlists").await, tree_before);
    assert_eq!(
        get_json(addr, &format!("/api/v1/playlists/{}", fx.playlist.id.0)).await,
        playlist_before
    );

    // A timer (a show document) too.
    let r = call(
        addr,
        "POST",
        "/api/v1/timers",
        Some(json!({"name": "Countdown"})),
    )
    .await;
    let timer = ok(&r, 201, "create a timer");
    let tid = timer["id"].as_str().unwrap().to_string();
    assert_eq!(
        call(addr, "DELETE", &format!("/api/v1/timers/{tid}"), None)
            .await
            .status,
        204
    );
    let trash = get_json(addr, "/api/v1/codex/trash").await;
    assert_eq!(trash[0]["kind"], "timer");
    let entry = trash[0]["id"].as_str().unwrap().to_string();
    ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/codex/trash/{entry}/restore"),
            None,
        )
        .await,
        200,
        "restore the timer",
    );
    assert_eq!(
        get_json(addr, &format!("/api/v1/timers/{tid}")).await,
        timer
    );

    // Undo through the create route (the document sent back) takes it out of the trash too.
    let second = fx.second.id.0.to_string();
    let r = call(
        addr,
        "DELETE",
        &format!("/api/v1/presentations/{second}"),
        None,
    )
    .await;
    let deleted = ok(&r, 200, "delete the second song");
    assert_eq!(
        get_json(addr, "/api/v1/codex/trash")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let r = call(
        addr,
        "POST",
        "/api/v1/presentations",
        Some(json!({"document": deleted["documents"][0]})),
    )
    .await;
    ok(&r, 201, "undo the delete");
    assert_eq!(get_json(addr, "/api/v1/codex/trash").await, json!([]));

    // A presentation whose library went too: 409 until the library is back; the entry stays.
    let r = call(
        addr,
        "POST",
        "/api/v1/libraries",
        Some(json!({"name": "Extra"})),
    )
    .await;
    let lib = ok(&r, 201, "create a library")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = call(
        addr,
        "POST",
        "/api/v1/presentations",
        Some(json!({"name": "Offering", "library": lib})),
    )
    .await;
    let pid = ok(&r, 201, "create a presentation")["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/presentations/{pid}"),
            None,
        )
        .await,
        200,
        "delete the presentation",
    );
    let p_entry = get_json(addr, "/api/v1/codex/trash").await[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &call(addr, "DELETE", &format!("/api/v1/libraries/{lib}"), None).await,
        200,
        "delete the library",
    );
    let trash = get_json(addr, "/api/v1/codex/trash").await;
    assert_eq!(trash[0]["kind"], "library", "newest first: {trash}");
    let l_entry = trash[0]["id"].as_str().unwrap().to_string();
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/trash/{p_entry}/restore"),
        None,
    )
    .await;
    let e = ok(&r, 409, "restore without its library");
    assert!(
        e["error"]["message"].as_str().unwrap().contains("Offering"),
        "{e}"
    );
    assert_eq!(
        get_json(addr, "/api/v1/codex/trash")
            .await
            .as_array()
            .unwrap()
            .len(),
        2
    );
    ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/codex/trash/{l_entry}/restore"),
            None,
        )
        .await,
        200,
        "restore the library",
    );
    ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/codex/trash/{p_entry}/restore"),
            None,
        )
        .await,
        200,
        "then the presentation",
    );
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(p["library"], lib.as_str());

    // Purge: everything goes for good.
    for id in [&song, &second] {
        ok(
            &call(addr, "DELETE", &format!("/api/v1/presentations/{id}"), None).await,
            200,
            "delete",
        );
    }
    let r = call(addr, "DELETE", "/api/v1/codex/trash", None).await;
    assert_eq!(ok(&r, 200, "purge"), json!({"purged": 2}));
    assert_eq!(get_json(addr, "/api/v1/codex/trash").await, json!([]));
    let left: Vec<_> = std::fs::read_dir(fx.dir.join("codex/trash"))
        .unwrap()
        .flatten()
        .collect();
    assert!(left.is_empty(), "{left:?}");
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// Changes noted as pending but not yet in the log (a stop between a document write and its
/// log entry; old-layout documents moved in at a start) are in the log after the next start,
/// so show boxes following it fetch them (A06-2, R2-4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_changes_reach_the_log_at_the_next_start() {
    let fx = fixture("codex-pending");
    let server = start_server(&fx, true, false).await;
    let main = get_json(server.addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let head = changes(server.addr, &main, 0).await["to"].as_u64().unwrap();
    server.shutdown().await.unwrap();
    let dir = fx.dir.join("codex/collections").join(&main);

    // The stop: a change noted, its document written, the log not appended.
    let song = fx.song.id.0.to_string();
    std::fs::write(
        dir.join("changes.pending.jsonl"),
        format!(
            "{}\n",
            json!({"kind": "presentation", "id": song, "change": "changed",
                   "by": {"client": "Booth", "machine": "m1"}})
        ),
    )
    .unwrap();
    // Old-layout documents that show up while the server is stopped.
    let timer = model::Timer::default();
    let t = timer.id.0.to_string();
    std::fs::create_dir_all(fx.dir.join("portable/timer")).unwrap();
    std::fs::write(
        fx.dir.join(format!("portable/timer/{t}.json")),
        serde_json::to_vec(&timer).unwrap(),
    )
    .unwrap();

    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut log = Value::Null;
    for _ in 0..50 {
        log = changes(addr, &main, head).await;
        if log["changes"].as_array().is_some_and(|c| c.len() >= 2) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let entries = log["changes"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["id"] == song.as_str() && e["by"]["client"] == "Booth"),
        "{log}"
    );
    assert!(
        entries
            .iter()
            .any(|e| e["id"] == t.as_str() && e["change"] == "added"),
        "{log}"
    );
    assert!(!dir.join("changes.pending.jsonl").exists());
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// E04-1: while the pending file can't be read, every write's change is kept for the next
/// append; one appended already is not queued again, so once the file reads again each
/// change is in the log exactly once.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changes_held_while_the_pending_file_cant_be_read_are_logged_once() {
    use std::os::unix::fs::PermissionsExt as _;
    let fx = fixture("codex-held-once");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let head = changes(addr, &main, 0).await["to"].as_u64().unwrap();
    let pending = fx
        .dir
        .join("codex/collections")
        .join(&main)
        .join("changes.pending.jsonl");
    std::fs::write(&pending, b"").unwrap();
    std::fs::set_permissions(&pending, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&pending).is_ok() {
        // (root reads it anyway)
        server.shutdown().await.unwrap();
        return;
    }
    let mut ids = Vec::new();
    for n in 0..3 {
        let r = call(
            addr,
            "POST",
            "/api/v1/presentations",
            Some(json!({"name": format!("Held {n}")})),
        )
        .await;
        ids.push(ok(&r, 201, "create")["id"].as_str().unwrap().to_string());
    }
    std::fs::remove_file(&pending).unwrap();
    let r = call(
        addr,
        "POST",
        "/api/v1/presentations",
        Some(json!({"name": "After"})),
    )
    .await;
    ids.push(ok(&r, 201, "create")["id"].as_str().unwrap().to_string());
    let log = changes(addr, &main, head).await;
    let entries = log["changes"].as_array().unwrap();
    for id in &ids {
        let n = entries.iter().filter(|e| e["id"] == id.as_str()).count();
        assert_eq!(n, 1, "{id} logged once: {log:#}");
    }
    assert_eq!(entries.len(), ids.len(), "{log:#}");
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}
