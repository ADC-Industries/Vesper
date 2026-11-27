//! Codex collections: CRUD with `If-Match`, the collection trash (delete, restore, delete for
//! good, never expiring), this box's assignment (what the content routes act on, confirmed
//! while something is live), the WebSocket events (`codexChanged`,
//! `documentChanged.collection`), and access at each site access level.

use crate::common::*;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

async fn call(
    addr: SocketAddr,
    method: &str,
    path: &str,
    auth: Option<&str>,
    if_match: Option<&str>,
    body: Option<Value>,
) -> Resp {
    let bearer = auth.map(|t| format!("Bearer {t}"));
    let quoted = if_match.map(|r| format!("\"{r}\""));
    let mut headers: Vec<(&str, &str)> = Vec::new();
    if let Some(b) = bearer.as_deref() {
        headers.push(("Authorization", b));
    }
    if let Some(m) = quoted.as_deref() {
        headers.push(("If-Match", m));
    }
    http(addr, method, path, &headers, body).await
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

/// The next message matching `pred` (seen before or arriving within 30 s).
async fn event(c: &mut Client, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    if let Some(i) = c.events.iter().position(&pred) {
        return c.events.remove(i);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "{what}: no such event");
        let m = c.next_any().await;
        if pred(&m) {
            return m;
        }
        c.absorb(&m);
    }
}

fn names(v: &Value) -> Vec<String> {
    let mut n: Vec<String> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect();
    n.sort();
    n
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collections_crud_with_if_match_and_the_collection_trash() {
    let fx = fixture("codex-crud");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;

    // The migrated install: one collection, active, holding the fixture's content.
    let info = get_json(addr, "/api/v1/server").await;
    assert!(
        info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("codex")),
        "{info}"
    );
    let st = get_json(addr, "/api/v1/codex").await;
    assert_eq!(st["role"], "host");
    assert_eq!(st["collections"], 1);
    let main = st["collection"].as_str().unwrap().to_string();
    assert!(st["documents"].as_u64().unwrap() >= 5, "{st}");
    let list = get_json(addr, "/api/v1/codex/collections").await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let m = &list[0];
    assert_eq!(m["collection"]["id"], main.as_str());
    assert_eq!(m["active"], true);
    assert_eq!(m["readOnly"], false);
    assert_eq!(m["counts"]["presentations"], 2, "{m}");
    assert_eq!(m["counts"]["libraries"], 1);
    assert_eq!(m["counts"]["mediaAssets"], 1);
    assert!(m["documentBytes"].as_u64().unwrap() > 0);
    let png = std::fs::metadata(
        server
            .shared
            .data()
            .content
            .asset_path(fx.asset.id)
            .unwrap(),
    )
    .unwrap()
    .len();
    assert_eq!(m["mediaBytes"], png);

    // Create: 201 with the summary and its revision as the ETag.
    let r = call(
        addr,
        "POST",
        "/api/v1/codex/collections",
        None,
        None,
        Some(json!({"name": "  Youth  ", "description": "Wednesday nights"})),
    )
    .await;
    let youth = ok(&r, 201, "create");
    let yid = youth["collection"]["id"].as_str().unwrap().to_string();
    assert_eq!(youth["collection"]["name"], "Youth");
    assert_eq!(youth["collection"]["description"], "Wednesday nights");
    assert_eq!(youth["active"], false);
    assert_eq!(youth["counts"]["presentations"], 0);
    assert_eq!(youth["documentBytes"], 0);
    assert_eq!(
        r.header("etag").unwrap().trim_matches('"'),
        youth["revision"]
    );
    assert!(
        youth["collection"]["sortKey"].as_str().unwrap()
            > m["collection"]["sortKey"].as_str().unwrap(),
        "a new collection goes last"
    );
    let e = event(&mut ws, "codexChanged on create", |m| {
        m["type"] == "codexChanged" && m["collection"] == yid.as_str()
    })
    .await;
    assert_eq!(e["parts"], json!(["status", "collections"]));
    assert!(
        fx.dir
            .join(format!("codex/collections/{yid}/collection.json"))
            .is_file()
    );
    assert_eq!(get_json(addr, "/api/v1/codex").await["collections"], 2);
    let list = get_json(addr, "/api/v1/codex/collections").await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[1]["collection"]["id"], yid.as_str());

    // Validation: 400 with the field.
    for (body, field) in [
        (json!({"name": " "}), "name"),
        (json!({"name": "x".repeat(201)}), "name"),
        (json!({"name": "A", "sortKey": "not a key!"}), "sortKey"),
    ] {
        let r = call(
            addr,
            "POST",
            "/api/v1/codex/collections",
            None,
            None,
            Some(body),
        )
        .await;
        assert_eq!(ok(&r, 400, "invalid")["error"]["field"], field);
    }

    // Update with If-Match: a stale revision is 409 with the current summary.
    let path = format!("/api/v1/codex/collections/{yid}");
    let one = call(addr, "GET", &path, None, None, None).await;
    assert_eq!(ok(&one, 200, "get")["revision"], youth["revision"]);
    assert_eq!(
        one.header("etag").unwrap().trim_matches('"'),
        youth["revision"]
    );
    let rev = youth["revision"].as_str().unwrap();
    let r = call(
        addr,
        "PUT",
        &path,
        None,
        Some(rev),
        Some(json!({"name": "Students", "description": null, "color": {"r": 1.0, "g": 0.5, "b": 0.0, "a": 1.0}})),
    )
    .await;
    let renamed = ok(&r, 200, "rename");
    assert_eq!(renamed["collection"]["name"], "Students");
    assert!(
        renamed["collection"].get("description").is_none(),
        "null clears"
    );
    assert_eq!(renamed["collection"]["color"]["g"], 0.5);
    assert_ne!(renamed["revision"], youth["revision"]);
    event(&mut ws, "codexChanged on rename", |m| {
        m["type"] == "codexChanged" && m["collection"] == yid.as_str()
    })
    .await;
    let r = call(
        addr,
        "PUT",
        &path,
        None,
        Some(rev),
        Some(json!({"name": "Stale"})),
    )
    .await;
    let c = ok(&r, 409, "stale rename");
    assert_eq!(c["error"]["code"], "conflict");
    assert_eq!(c["current"]["revision"], renamed["revision"]);
    assert_eq!(c["current"]["collection"]["name"], "Students");
    let r = call(addr, "DELETE", &path, None, Some(rev), None).await;
    assert_eq!(
        ok(&r, 409, "stale delete")["current"]["revision"],
        renamed["revision"]
    );
    assert_eq!(
        call(
            addr,
            "GET",
            "/api/v1/codex/collections/0b3c5a51-7f63-4d1c-9a33-3a7c0e1f8d2e",
            None,
            None,
            None
        )
        .await
        .status,
        404
    );

    // The active collection can't be deleted.
    let main_rev = m["revision"].as_str().unwrap();
    let r = call(
        addr,
        "DELETE",
        &format!("/api/v1/codex/collections/{main}"),
        None,
        Some(main_rev),
        None,
    )
    .await;
    assert_eq!(
        ok(&r, 409, "delete the active one")["error"]["code"],
        "conflict"
    );

    // Give Students a presentation, and a timer deleted into the item trash.
    ok(
        &call(
            addr,
            "PUT",
            "/api/v1/codex/assignment",
            None,
            None,
            Some(json!({"collection": yid})),
        )
        .await,
        200,
        "assign Students",
    );
    let r = call(
        addr,
        "POST",
        "/api/v1/presentations",
        None,
        None,
        Some(json!({"name": "Welcome"})),
    )
    .await;
    let created = ok(&r, 201, "create a presentation");
    let pid = created["id"].as_str().unwrap().to_string();
    let e = event(&mut ws, "documentChanged of the new presentation", |m| {
        m["type"] == "documentChanged" && m["id"] == pid.as_str()
    })
    .await;
    assert_eq!(e["collection"], yid.as_str(), "{e}");
    assert_eq!(e["change"], "added");
    assert!(
        fx.dir
            .join(format!(
                "codex/collections/{yid}/portable/presentation/{pid}.json"
            ))
            .is_file()
    );
    let c = event(&mut ws, "codexChanged: the counts changed", |m| {
        m["type"] == "codexChanged" && m["collection"] == yid.as_str()
    })
    .await;
    assert_eq!(c["parts"], json!(["collections"]));
    let r = call(
        addr,
        "POST",
        "/api/v1/timers",
        None,
        None,
        Some(json!({"name": "Countdown"})),
    )
    .await;
    let tid = ok(&r, 201, "create a timer")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = call(
        addr,
        "DELETE",
        &format!("/api/v1/timers/{tid}"),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 204);
    let trash = get_json(addr, "/api/v1/codex/trash").await;
    assert_eq!(trash.as_array().unwrap().len(), 1, "{trash}");
    let timer_entry = trash[0]["id"].as_str().unwrap().to_string();
    ok(
        &call(
            addr,
            "PUT",
            "/api/v1/codex/assignment",
            None,
            None,
            Some(json!({"collection": main})),
        )
        .await,
        200,
        "assign the main collection again",
    );
    let s = get_json(addr, &path).await;
    assert_eq!(s["counts"]["presentations"], 1, "{s}");
    assert!(s["documentBytes"].as_u64().unwrap() > 0);

    // Deleting a collection with documents moves it to the collection trash, intact: hidden
    // from the list, its item trash entries waiting with it.
    let rev = s["revision"].as_str().unwrap().to_string();
    let r = call(addr, "DELETE", &path, None, Some(&rev), None).await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(call(addr, "GET", &path, None, None, None).await.status, 404);
    let list = get_json(addr, "/api/v1/codex/collections").await;
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
    let st = get_json(addr, "/api/v1/codex").await;
    assert_eq!(
        (&st["collections"], &st["deletedCollections"], &st["trash"]),
        (&json!(1), &json!(1), &json!(0)),
        "{st}"
    );
    assert_eq!(get_json(addr, "/api/v1/codex/trash").await, json!([]));
    assert!(!fx.dir.join(format!("codex/collections/{yid}")).exists());
    let kept = fx.dir.join(format!("codex/collection-trash/{yid}"));
    assert!(
        kept.join(format!("portable/presentation/{pid}.json"))
            .is_file()
    );
    assert!(kept.join("collection.json").is_file() && kept.join("deleted.json").is_file());
    event(&mut ws, "codexChanged on delete", |m| {
        m["type"] == "codexChanged"
            && m["collection"] == yid.as_str()
            && m["parts"]
                .as_array()
                .unwrap()
                .contains(&json!("collectionTrash"))
    })
    .await;
    let bin = get_json(addr, "/api/v1/codex/collection-trash").await;
    assert_eq!(bin.as_array().unwrap().len(), 1, "{bin}");
    let d = &bin[0];
    assert_eq!(d["collection"]["id"], yid.as_str());
    assert_eq!(d["collection"]["name"], "Students");
    assert_eq!(d["counts"]["presentations"], 1);
    assert_eq!(d["trashItems"], 1);
    assert_eq!(d["by"]["client"], "server machine");
    assert!(!d["deletedAt"].as_str().unwrap().is_empty());
    assert!(d["documentBytes"].as_u64().unwrap() > 0);
    assert_eq!(
        call(
            addr,
            "PUT",
            "/api/v1/codex/assignment",
            None,
            None,
            Some(json!({"collection": yid}))
        )
        .await
        .status,
        400,
        "a deleted collection can't be assigned"
    );
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/trash/{timer_entry}/restore"),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 409, "its collection is deleted");

    // Restored: back as it was, its trash entry listed again.
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/collection-trash/{yid}/restore"),
        None,
        None,
        None,
    )
    .await;
    let back = ok(&r, 200, "restore the collection");
    assert_eq!(back["collection"]["id"], yid.as_str());
    assert_eq!(back["counts"]["presentations"], 1);
    assert_eq!(back["revision"], s["revision"]);
    assert_eq!(
        get_json(addr, "/api/v1/codex/collections")
            .await
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        get_json(addr, "/api/v1/codex/collection-trash").await,
        json!([])
    );
    let trash = get_json(addr, "/api/v1/codex/trash").await;
    assert_eq!(trash[0]["id"], timer_entry.as_str(), "{trash}");
    assert!(
        fx.dir
            .join(format!(
                "codex/collections/{yid}/portable/presentation/{pid}.json"
            ))
            .is_file()
    );
    assert!(!kept.exists());

    // Deleted again, then for good: its documents, change log and trash entries go.
    let r = call(addr, "DELETE", &path, None, None, None).await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let bin_path = format!("/api/v1/codex/collection-trash/{yid}");
    let r = call(addr, "DELETE", &bin_path, None, Some("stale"), None).await;
    let c = ok(&r, 409, "a stale If-Match");
    assert_eq!(c["current"]["collection"]["id"], yid.as_str());
    let r = call(addr, "DELETE", &bin_path, None, Some(&rev), None).await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    assert!(!kept.exists());
    assert!(!fx.dir.join(format!("codex/collections/{yid}")).exists());
    assert!(
        !fx.dir
            .join(format!("codex/trash/{timer_entry}.json"))
            .exists()
    );
    let index = std::fs::read_to_string(fx.dir.join("codex/trash.jsonl")).unwrap_or_default();
    assert!(!index.contains(&yid), "{index}");
    assert_eq!(
        get_json(addr, "/api/v1/codex/collection-trash").await,
        json!([])
    );
    assert_eq!(
        call(addr, "DELETE", &bin_path, None, None, None)
            .await
            .status,
        404
    );
    let st = get_json(addr, "/api/v1/codex").await;
    assert_eq!(
        (&st["collections"], &st["deletedCollections"]),
        (&json!(1), &json!(0))
    );

    // Content edits name their collection.
    let lib = fx.library.id.0.to_string();
    let r = call(
        addr,
        "PUT",
        &format!("/api/v1/libraries/{lib}"),
        None,
        None,
        Some(json!({"name": "Songs"})),
    )
    .await;
    ok(&r, 200, "rename the library");
    let e = event(&mut ws, "documentChanged of the library", |m| {
        m["type"] == "documentChanged" && m["id"] == lib.as_str()
    })
    .await;
    assert_eq!(e["collection"], main.as_str());
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_assignment_switches_what_the_box_shows_and_survives_a_restart() {
    let fx = fixture("codex-assign");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let before = names(&get_json(addr, "/api/v1/presentations").await);
    assert_eq!(before.len(), 2);

    let r = call(
        addr,
        "POST",
        "/api/v1/codex/collections",
        None,
        None,
        Some(json!({"name": "Chapel"})),
    )
    .await;
    let chapel = ok(&r, 201, "create")["collection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        call(
            addr,
            "PUT",
            "/api/v1/codex/assignment",
            None,
            None,
            Some(json!({"collection": "0b3c5a51-7f63-4d1c-9a33-3a7c0e1f8d2e"}))
        )
        .await
        .status,
        400,
        "an unknown collection"
    );

    // Assigned: the content routes act on the (empty) Chapel collection.
    let r = call(
        addr,
        "PUT",
        "/api/v1/codex/assignment",
        None,
        None,
        Some(json!({"collection": chapel})),
    )
    .await;
    let st = ok(&r, 200, "assign");
    assert_eq!(st["collection"], chapel.as_str());
    assert_eq!(st["collectionName"], "Chapel");
    assert!(
        get_json(addr, "/api/v1/presentations")
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        get_json(addr, "/api/v1/libraries")
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        call(
            addr,
            "GET",
            &format!("/api/v1/presentations/{}", fx.song.id.0),
            None,
            None,
            None
        )
        .await
        .status,
        404
    );
    // The screens stay: they are the box's.
    assert_eq!(
        get_json(addr, "/api/v1/screens")
            .await
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // Clients hear what left the box's content (from the collection it was in).
    let e = event(&mut ws, "documentChanged: the song left", |m| {
        m["type"] == "documentChanged" && m["id"] == fx.song.id.0.to_string().as_str()
    })
    .await;
    assert_eq!(e["change"], "removed");
    assert_eq!(e["collection"], main.as_str());
    event(&mut ws, "codexChanged: status", |m| {
        m["type"] == "codexChanged" && m["parts"] == json!(["status", "collections"])
    })
    .await;
    let list = get_json(addr, "/api/v1/codex/collections").await;
    for c in list.as_array().unwrap() {
        assert_eq!(c["active"], c["collection"]["id"] == chapel.as_str(), "{c}");
    }
    let config: Value =
        serde_json::from_slice(&std::fs::read(fx.dir.join("machine/codex.json")).unwrap()).unwrap();
    assert_eq!(config["collection"], chapel.as_str());

    // A new presentation lands in Chapel; the main collection still has its two.
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/presentations",
            None,
            None,
            Some(json!({"name": "Vespers"})),
        )
        .await,
        201,
        "create",
    );
    assert_eq!(
        names(&get_json(addr, "/api/v1/presentations").await),
        ["Vespers"]
    );
    let main_summary = get_json(addr, &format!("/api/v1/codex/collections/{main}")).await;
    assert_eq!(main_summary["counts"]["presentations"], 2);

    // A reload keeps the assignment.
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/commands",
            None,
            None,
            Some(json!({"type": "reloadData"})),
        )
        .await,
        200,
        "reload",
    );
    assert_eq!(
        names(&get_json(addr, "/api/v1/presentations").await),
        ["Vespers"]
    );

    // So does a restart.
    drop(ws);
    server.shutdown().await.unwrap();
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    assert_eq!(
        get_json(addr, "/api/v1/codex").await["collection"],
        chapel.as_str()
    );
    assert_eq!(
        names(&get_json(addr, "/api/v1/presentations").await),
        ["Vespers"]
    );

    // And back: the main collection's content is shown again, unchanged.
    ok(
        &call(
            addr,
            "PUT",
            "/api/v1/codex/assignment",
            None,
            None,
            Some(json!({"collection": main})),
        )
        .await,
        200,
        "assign back",
    );
    assert_eq!(
        names(&get_json(addr, "/api/v1/presentations").await),
        before
    );
    let song = get_json(addr, &format!("/api/v1/presentations/{}", fx.song.id.0)).await;
    assert_eq!(song["name"], fx.song.name.as_str());
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_collection_trash_never_expires_and_survives_a_restart() {
    let fx = fixture("codex-ctrash");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let r = call(
        addr,
        "POST",
        "/api/v1/codex/collections",
        None,
        None,
        Some(json!({"name": "Archive"})),
    )
    .await;
    let id = ok(&r, 201, "create")["collection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = call(
        addr,
        "DELETE",
        &format!("/api/v1/codex/collections/{id}"),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 204);

    // The hourly upkeep, a year on: items expire, deleted collections never do.
    let later = std::time::SystemTime::now() + Duration::from_secs(400 * 86_400);
    server
        .shared
        .codex_writer
        .tick(&server.shared.codex_server(), later)
        .await;
    let bin = get_json(addr, "/api/v1/codex/collection-trash").await;
    assert_eq!(bin[0]["collection"]["id"], id.as_str(), "{bin}");
    assert!(
        fx.dir
            .join(format!("codex/collection-trash/{id}/collection.json"))
            .is_file()
    );

    // A restart finds it in the collection trash still.
    server.shutdown().await.unwrap();
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let bin = get_json(addr, "/api/v1/codex/collection-trash").await;
    assert_eq!(bin.as_array().unwrap().len(), 1);
    assert_eq!(bin[0]["collection"]["name"], "Archive");
    assert_eq!(bin[0]["by"]["client"], "server machine");
    assert_eq!(get_json(addr, "/api/v1/codex").await["collections"], 1);
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/collection-trash/{id}/restore"),
        None,
        None,
        None,
    )
    .await;
    ok(&r, 200, "restore after the restart");
    assert_eq!(get_json(addr, "/api/v1/codex").await["collections"], 2);
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_collections_while_live_needs_a_confirmation() {
    let fx = fixture("codex-live");
    let message = model::Message {
        name: "Parking reminder".into(),
        text: "Please move your car".into(),
        ..model::Message::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "message",
        &message.id.0.to_string(),
        &message,
    );
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let r = call(
        addr,
        "POST",
        "/api/v1/codex/collections",
        None,
        None,
        Some(json!({"name": "Chapel"})),
    )
    .await;
    let chapel = ok(&r, 201, "create")["collection"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Nothing live: no confirmation needed.
    let assign = |collection: &str, confirm: Option<bool>| {
        let mut body = json!({"collection": collection});
        if let Some(c) = confirm {
            body["confirm"] = json!(c);
        }
        call(
            addr,
            "PUT",
            "/api/v1/codex/assignment",
            None,
            None,
            Some(body),
        )
    };
    ok(&assign(&chapel, None).await, 200, "switch, nothing live");
    ok(&assign(&main, None).await, 200, "and back");

    // A message showing counts (messages are collection items).
    ws.ok(json!({"type": "messageShow", "message": message.id}))
        .await;
    let e = ok(
        &assign(&chapel, None).await,
        409,
        "switch with a message up",
    );
    assert_eq!(e["error"]["code"], "liveShow", "{e}");
    let msg = e["error"]["message"].as_str().unwrap();
    assert!(msg.contains("the message “Parking reminder”"), "{msg}");
    assert_eq!(
        e["error"]["live"],
        json!(["the message “Parking reminder”"]),
        "what is live, as a list: {e}"
    );
    assert!(!msg.contains("\"confirm\""), "plain words: {msg}");
    ws.ok(json!({"type": "messageHide"})).await;
    ok(
        &assign(&chapel, None).await,
        200,
        "switch, the message down",
    );
    ok(&assign(&main, None).await, 200, "and back again");

    // A live slide and live media: refused with liveShow, naming them.
    let song = fx.song.id.0.to_string();
    ws.ok(json!({"type": "triggerSlide", "presentation": song, "index": 0}))
        .await;
    ws.ok(json!({"type": "triggerMedia", "asset": fx.asset.id.0.to_string()}))
        .await;
    let r = assign(&chapel, None).await;
    let e = ok(&r, 409, "switch while live");
    assert_eq!(e["error"]["code"], "liveShow", "{e}");
    let msg = e["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains(&fx.song.name) && msg.contains("Blue background"),
        "{msg}"
    );
    let r = assign(&chapel, Some(false)).await;
    assert_eq!(r.status, 409, "confirm: false is no confirmation");
    assert_eq!(
        get_json(addr, "/api/v1/codex").await["collection"],
        main.as_str()
    );
    let state = get_json(addr, "/api/v1/state").await;
    assert_eq!(
        state["state"]["layers"]["slide"]["presentation"],
        song.as_str()
    );

    // Confirmed: switched, the live slide cleared.
    let st = ok(&assign(&chapel, Some(true)).await, 200, "confirmed switch");
    assert_eq!(st["collection"], chapel.as_str());
    let state = get_json(addr, "/api/v1/state").await;
    assert!(state["state"]["layers"]["slide"].is_null(), "{state}");
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// Pairs a client with the server's pairing code; its token.
async fn pair(server: &vesper_server::RunningServer, name: &str) -> String {
    let r = http(
        server.addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": server.pairing_code(), "clientName": name})),
    )
    .await;
    ok(&r, 200, "pair")["token"].as_str().unwrap().to_string()
}

async fn set_access(addr: SocketAddr, auth: &str, access: &str, pin: Option<&str>) {
    let mut body = json!({"enabled": true, "joinMode": "code_or_approval", "access": access});
    if let Some(p) = pin {
        body["pin"] = json!(p);
    }
    let r = call(
        addr,
        "PUT",
        "/api/v1/site/settings",
        Some(auth),
        None,
        Some(body),
    )
    .await;
    ok(&r, 200, &format!("access {access}"));
}

async fn create(addr: SocketAddr, auth: &str, name: &str) -> Resp {
    call(
        addr,
        "POST",
        "/api/v1/codex/collections",
        Some(auth),
        None,
        Some(json!({"name": name})),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collection_administration_follows_the_site_access_level() {
    let fx = fixture("codex-access");
    let server = start_with_site(&fx, false, vesper_server::site::SiteConfig::loopback()).await;
    let addr = server.addr;
    let admin_token = pair(&server, "Booth").await;
    let other_token = pair(&server, "Tablet").await;
    let (a, b) = (admin_token.as_str(), other_token.as_str());
    assert_eq!(
        call(addr, "GET", "/api/v1/codex", None, None, None)
            .await
            .status,
        401,
        "the Codex needs pairing like the rest"
    );

    // "open": every paired client administers collections.
    let open = ok(&create(addr, b, "Open").await, 201, "open: create");
    let oid = open["collection"]["id"].as_str().unwrap().to_string();

    // "pin": locked clients read, but only an unlocked one administers.
    set_access(addr, a, "pin", Some("2468")).await;
    let r = create(addr, b, "Locked").await;
    let e = ok(&r, 403, "pin, locked: create");
    assert!(
        e["error"]["message"].as_str().unwrap().contains("unlock"),
        "{e}"
    );
    for (method, path, body) in [
        (
            "PUT",
            format!("/api/v1/codex/collections/{oid}"),
            Some(json!({"name": "X"})),
        ),
        ("DELETE", format!("/api/v1/codex/collections/{oid}"), None),
        (
            "PUT",
            "/api/v1/codex/assignment".to_string(),
            Some(json!({"collection": oid})),
        ),
        ("DELETE", "/api/v1/codex/trash".to_string(), None),
        (
            "POST",
            format!("/api/v1/codex/collection-trash/{oid}/restore"),
            None,
        ),
        (
            "DELETE",
            format!("/api/v1/codex/collection-trash/{oid}"),
            None,
        ),
    ] {
        let r = call(addr, method, &path, Some(b), None, body).await;
        assert_eq!(r.status, 403, "pin, locked: {method} {path}");
    }
    ok(
        &call(addr, "GET", "/api/v1/codex", Some(b), None, None).await,
        200,
        "pin, locked: read",
    );
    ok(
        &call(
            addr,
            "GET",
            "/api/v1/codex/collections",
            Some(b),
            None,
            None,
        )
        .await,
        200,
        "pin, locked: list",
    );
    // Content edits stay open to every paired client.
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/libraries",
            Some(b),
            None,
            Some(json!({"name": "Hymns"})),
        )
        .await,
        201,
        "pin, locked: content edit",
    );
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/unlock",
            Some(b),
            None,
            Some(json!({"pin": "2468"})),
        )
        .await,
        200,
        "unlock",
    );
    ok(
        &create(addr, b, "Unlocked").await,
        201,
        "pin, unlocked: create",
    );
    ok(
        &call(addr, "DELETE", "/api/v1/codex/trash", Some(b), None, None).await,
        200,
        "pin, unlocked: purge the trash",
    );

    // "users": a signed-in admin only; a viewer reads.
    set_access(addr, b, "open", None).await;
    for (name, role) in [("root", "admin"), ("vic", "viewer")] {
        let r = call(
            addr,
            "PUT",
            &format!("/api/v1/site/users/{name}"),
            Some(a),
            None,
            Some(json!({"role": role, "password": format!("{name}-password")})),
        )
        .await;
        ok(&r, 200, "add user");
    }
    set_access(addr, a, "users", None).await;
    for (token, user) in [(a, "root"), (b, "vic")] {
        let r = call(
            addr,
            "POST",
            "/api/v1/site/session",
            Some(token),
            None,
            Some(json!({"user": user, "password": format!("{user}-password")})),
        )
        .await;
        ok(&r, 200, "sign in");
    }
    let e = ok(
        &create(addr, b, "Viewer").await,
        403,
        "users, viewer: create",
    );
    assert!(
        e["error"]["message"].as_str().unwrap().contains("viewer"),
        "{e}"
    );
    ok(
        &call(addr, "GET", "/api/v1/codex", Some(b), None, None).await,
        200,
        "users, viewer: read",
    );
    ok(&create(addr, a, "Admin").await, 201, "users, admin: create");

    // The collection trash: deleting, restoring and deleting for good are admin actions.
    let open_path = format!("/api/v1/codex/collections/{oid}");
    let bin_path = format!("/api/v1/codex/collection-trash/{oid}");
    let r = call(addr, "DELETE", &open_path, Some(b), None, None).await;
    assert_eq!(r.status, 403, "users, viewer: delete");
    let r = call(addr, "DELETE", &open_path, Some(a), None, None).await;
    assert_eq!(r.status, 204, "users, admin: delete");
    ok(
        &call(
            addr,
            "GET",
            "/api/v1/codex/collection-trash",
            Some(b),
            None,
            None,
        )
        .await,
        200,
        "users, viewer: list the collection trash",
    );
    let r = call(
        addr,
        "POST",
        &format!("{bin_path}/restore"),
        Some(b),
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 403, "users, viewer: restore");
    ok(
        &call(
            addr,
            "POST",
            &format!("{bin_path}/restore"),
            Some(a),
            None,
            None,
        )
        .await,
        200,
        "users, admin: restore",
    );
    let r = call(addr, "DELETE", &open_path, Some(a), None, None).await;
    assert_eq!(r.status, 204, "users, admin: delete again");
    let r = call(addr, "DELETE", &bin_path, Some(b), None, None).await;
    assert_eq!(r.status, 403, "users, viewer: delete for good");
    let r = call(addr, "DELETE", &bin_path, Some(a), None, None).await;
    assert_eq!(r.status, 204, "users, admin: delete for good");

    // The audit names the admin actions, refused ones too.
    let audit = ok(
        &call(addr, "GET", "/api/v1/site/audit?n=500", Some(a), None, None).await,
        200,
        "audit",
    );
    let entries: Vec<&Value> = audit
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "codex.createCollection")
        .collect();
    let ok_names: Vec<&str> = entries
        .iter()
        .filter(|e| e["ok"] == true)
        .map(|e| e["detail"]["name"].as_str().unwrap_or_default())
        .collect();
    for n in ["Open", "Unlocked", "Admin"] {
        assert!(ok_names.contains(&n), "{n} in {ok_names:?}");
    }
    assert!(
        entries
            .iter()
            .filter(|e| e["ok"] == false)
            .any(|e| e["detail"]["name"] == "Viewer" && e["user"] == "vic"),
        "{entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|e| e["ok"] == false && e["detail"]["name"] == "Locked"),
        "{entries:?}"
    );
    let all = audit.as_array().unwrap();
    let outcomes = |action: &str| -> Vec<(bool, String)> {
        all.iter()
            .filter(|e| e["action"] == action && e["detail"]["collection"] == oid.as_str())
            .map(|e| {
                (
                    e["ok"] == true,
                    e["user"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    };
    for action in [
        "codex.deleteCollection",
        "codex.restoreCollection",
        "codex.purgeCollection",
    ] {
        let o = outcomes(action);
        assert!(o.contains(&(true, "root".into())), "{action}: {o:?}");
        assert!(o.contains(&(false, "vic".into())), "{action}: {o:?}");
    }
    let _ = std::fs::remove_dir_all(&fx.dir);
}
