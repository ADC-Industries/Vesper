//! Media bins and global groups edited over the API (G17): create, rename, move between
//! folders, reorder, delete into the trash and restore; bin item ops with their inverse;
//! the change log and `documentChanged`. Synthetic data only.

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

/// The bin tree's bins in display order, as "folder/name".
fn bin_names(tree: &Value) -> Vec<String> {
    fn walk(n: &Value, out: &mut Vec<String>) {
        for c in n.as_array().unwrap() {
            if c["kind"] == "folder" {
                walk(&c["children"], out);
            } else {
                let path: Vec<&str> = c["path"]
                    .as_array()
                    .map(|p| p.iter().map(|s| s.as_str().unwrap()).collect())
                    .unwrap_or_default();
                let mut s = path.join("/");
                if !s.is_empty() {
                    s.push('/');
                }
                s.push_str(c["name"].as_str().unwrap());
                out.push(s);
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut out);
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_bins_are_created_renamed_reordered_and_deleted() {
    let fx = fixture("media-bins");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let collection = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(get_json(addr, "/api/v1/media-bins").await, json!([]));

    // Create: at the end of their folder; a folder is a path on the bin.
    let bg = ok(
        &call(
            addr,
            "POST",
            "/api/v1/media-bins",
            Some(json!({"name": " Backgrounds "})),
        )
        .await,
        201,
        "create Backgrounds",
    );
    assert_eq!(bg["kind"], "mediaBin");
    let bg_id = bg["id"].as_str().unwrap().to_string();
    let loops = ok(
        &call(
            addr,
            "POST",
            "/api/v1/media-bins",
            Some(json!({"name": "Loops"})),
        )
        .await,
        201,
        "create Loops",
    );
    let loops_id = loops["id"].as_str().unwrap().to_string();
    let walk = ok(
        &call(
            addr,
            "POST",
            "/api/v1/media-bins",
            Some(json!({"name": "Walk-in", "folder": ["Motion"]})),
        )
        .await,
        201,
        "create Walk-in in a folder",
    );
    let walk_id = walk["id"].as_str().unwrap().to_string();
    assert_eq!(
        bin_names(&get_json(addr, "/api/v1/media-bins").await),
        ["Motion/Walk-in", "Backgrounds", "Loops"]
    );
    // documentChanged for the new bin, with the collection.
    loop {
        let m = ws.next_any().await;
        if m["type"] == "documentChanged" && m["kind"] == "mediaBin" && m["id"] == walk_id {
            assert_eq!(m["change"], "added");
            assert_eq!(m["collection"], collection.as_str());
            break;
        }
    }
    // Bad names and folders.
    for body in [
        json!({"name": "  "}),
        json!({"name": "Tab\tname"}),
        json!({"name": "Ok", "folder": [""]}),
    ] {
        let r = call(addr, "POST", "/api/v1/media-bins", Some(body.clone())).await;
        assert_eq!(
            r.status,
            400,
            "{body}: {}",
            String::from_utf8_lossy(&r.body)
        );
    }

    // Rename with If-Match; a stale revision is a conflict.
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/media-bins/{bg_id}"),
        &[("If-Match", "\"stale\"")],
        Some(json!({"name": "Stills"})),
    )
    .await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["current"]["revision"], bg["revision"]);
    let rev = format!("\"{}\"", bg["revision"].as_str().unwrap());
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/media-bins/{bg_id}"),
        &[("If-Match", &rev)],
        Some(json!({"name": "Stills"})),
    )
    .await;
    ok(&r, 200, "rename");
    // Reorder: Loops first; then move Stills into the folder after Walk-in.
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/media-bins/{loops_id}"),
            Some(json!({"after": null})),
        )
        .await,
        200,
        "Loops first",
    );
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/media-bins/{bg_id}"),
            Some(json!({"folder": ["Motion"], "after": walk_id})),
        )
        .await,
        200,
        "into the folder",
    );
    assert_eq!(
        bin_names(&get_json(addr, "/api/v1/media-bins").await),
        ["Motion/Walk-in", "Motion/Stills", "Loops"]
    );
    let r = call(
        addr,
        "PUT",
        &format!("/api/v1/media-bins/{loops_id}"),
        Some(json!({"after": "00000000-0000-4000-8000-000000000001"})),
    )
    .await;
    assert_eq!(r.status, 400, "after a bin of another folder");

    // Delete into the trash, then restore with its ID.
    let gone = ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/media-bins/{loops_id}"),
            None,
        )
        .await,
        200,
        "delete Loops",
    );
    assert_eq!(gone["kind"], "mediaBin");
    assert_eq!(gone["documents"][0]["name"], "Loops");
    assert_eq!(
        call(addr, "GET", &format!("/api/v1/media-bins/{loops_id}"), None)
            .await
            .status,
        404
    );
    let trash = get_json(addr, "/api/v1/codex/trash").await;
    let entry = trash
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "mediaBin")
        .expect("the bin in the trash")
        .clone();
    assert_eq!(entry["name"], "Loops");
    let r = call(
        addr,
        "POST",
        &format!(
            "/api/v1/codex/trash/{}/restore",
            entry["id"].as_str().unwrap()
        ),
        None,
    )
    .await;
    ok(&r, 200, "restore Loops");
    assert_eq!(
        get_json(addr, &format!("/api/v1/media-bins/{loops_id}")).await["name"],
        "Loops"
    );

    // Every change is in the collection's change log, and on disk.
    let log = get_json(
        addr,
        &format!("/api/v1/codex/collections/{collection}/changes?since=0"),
    )
    .await;
    let bins: Vec<&Value> = log["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "mediaBin")
        .collect();
    assert!(bins.len() >= 7, "{log}");
    assert!(bins.iter().any(|c| c["change"] == "removed"));
    assert!(
        fx.dir
            .join(format!(
                "codex/collections/{collection}/portable/mediaBin/{bg_id}.json"
            ))
            .is_file()
    );
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_bin_items_are_added_moved_renamed_and_removed() {
    let fx = fixture("media-bin-items");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let asset = fx.asset.id.0.to_string();
    let bin = ok(
        &call(
            addr,
            "POST",
            "/api/v1/media-bins",
            Some(json!({"name": "Backgrounds"})),
        )
        .await,
        201,
        "create",
    );
    let id = bin["id"].as_str().unwrap().to_string();
    let edits = format!("/api/v1/media-bins/{id}/edits");
    let (a, b) = (
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
    );
    let item = |iid: &str| json!({"id": iid, "trigger": {"asset": asset, "layer": "background"}});
    let r = ok(
        &call(
            addr,
            "POST",
            &edits,
            Some(json!({"clientId": "ui-1", "ops": [
                {"type": "addItem", "item": item(a)},
                {"type": "addItem", "after": a, "item": item(b)},
            ]})),
        )
        .await,
        200,
        "add two",
    );
    assert_eq!(r["inverse"][0], json!({"type": "removeItem", "item": b}));
    let d = get_json(addr, &format!("/api/v1/media-bins/{id}")).await;
    let ids: Vec<&str> = d["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [a, b]);
    assert_eq!(d["items"][0]["name"], "Blue background");
    assert_eq!(
        get_json(addr, "/api/v1/media-bins").await[0]["itemCount"],
        2
    );

    // Media that isn't in the collection, a duplicate ID, a missing item.
    let r = call(
        addr,
        "POST",
        &edits,
        Some(json!({"ops": [{"type": "addItem", "item": {"id": "33333333-3333-4333-8333-333333333333", "trigger": {"asset": "44444444-4444-4444-8444-444444444444"}}}]})),
    )
    .await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    let r = call(
        addr,
        "POST",
        &edits,
        Some(json!({"ops": [{"type": "addItem", "item": item(a)}]})),
    )
    .await;
    assert_eq!(r.status, 400);
    let r = call(
        addr,
        "POST",
        &edits,
        Some(json!({"ops": [{"type": "removeItem", "item": "55555555-5555-4555-8555-555555555555"}]})),
    )
    .await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["current"]["opIndex"], 0);

    // Move, rename, disable; the inverse undoes the batch exactly.
    let before = get_json(addr, &format!("/api/v1/media-bins/{id}")).await;
    let r = ok(
        &call(
            addr,
            "POST",
            &edits,
            Some(json!({"ops": [
                {"type": "moveItem", "item": b},
                {"type": "setItem", "item": a, "name": "Opening still", "enabled": false},
                {"type": "removeItem", "item": b},
            ]})),
        )
        .await,
        200,
        "move, rename, remove",
    );
    let d = get_json(addr, &format!("/api/v1/media-bins/{id}")).await;
    assert_eq!(d["items"].as_array().unwrap().len(), 1);
    assert_eq!(d["items"][0]["name"], "Opening still");
    assert_eq!(d["items"][0]["enabled"], false);
    let inverse = r["inverse"].clone();
    ok(
        &call(addr, "POST", &edits, Some(json!({"ops": inverse}))).await,
        200,
        "undo",
    );
    assert_eq!(
        get_json(addr, &format!("/api/v1/media-bins/{id}")).await,
        before
    );

    // A watched folder's items follow the folder.
    let watched = ok(
        &call(
            addr,
            "POST",
            "/api/v1/media-bins",
            Some(json!({"document": {
                "id": "66666666-6666-4666-8666-666666666666",
                "name": "Camera roll",
                "kind": {"type": "watched", "folder": {"root": "77777777-7777-4777-8777-777777777777"}}
            }})),
        )
        .await,
        201,
        "a watched bin",
    );
    let r = call(
        addr,
        "POST",
        &format!(
            "/api/v1/media-bins/{}/edits",
            watched["id"].as_str().unwrap()
        ),
        Some(json!({"ops": [{"type": "addItem", "item": item(a)}]})),
    )
    .await;
    assert_eq!(r.status, 400);
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_groups_are_created_edited_reordered_and_deleted() {
    let fx = fixture("global-groups");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let names = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|g| g["name"].as_str().unwrap().to_string())
            .collect()
    };
    let start = get_json(addr, "/api/v1/groups").await;
    let n0 = start.as_array().unwrap().len();
    let verse = ok(
        &call(
            addr,
            "POST",
            "/api/v1/groups",
            Some(json!({"name": "Verse", "color": {"r": 0.2, "g": 0.4, "b": 0.9, "a": 1.0}})),
        )
        .await,
        201,
        "Verse",
    );
    let chorus = ok(
        &call(
            addr,
            "POST",
            "/api/v1/groups",
            Some(json!({"name": "Chorus"})),
        )
        .await,
        201,
        "Chorus",
    );
    let (v, c) = (
        verse["id"].as_str().unwrap().to_string(),
        chorus["id"].as_str().unwrap().to_string(),
    );
    let g = get_json(addr, "/api/v1/groups").await;
    assert_eq!(names(&g)[n0..], ["Verse", "Chorus"]);
    assert!((g[n0]["color"]["b"].as_f64().unwrap() - 0.9).abs() < 1e-6);

    // Rename and clear the colour; Chorus first.
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/groups/{v}"),
            Some(json!({"name": "Verse 1", "color": null})),
        )
        .await,
        200,
        "rename",
    );
    ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/groups/{c}"),
            Some(json!({"after": null})),
        )
        .await,
        200,
        "Chorus first",
    );
    let g = get_json(addr, "/api/v1/groups").await;
    assert_eq!(names(&g)[0], "Chorus");
    let verse = g
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == v.as_str())
        .unwrap();
    assert_eq!(verse["name"], "Verse 1");
    assert!(verse.get("color").is_none(), "{verse}");

    // Delete: gone, in the trash.
    ok(
        &call(addr, "DELETE", &format!("/api/v1/groups/{c}"), None).await,
        200,
        "delete",
    );
    assert!(!names(&get_json(addr, "/api/v1/groups").await).contains(&"Chorus".to_string()));
    let trash = get_json(addr, "/api/v1/codex/trash").await;
    assert!(
        trash
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["kind"] == "globalGroup" && t["name"] == "Chorus"),
        "{trash}"
    );
    assert_eq!(
        call(
            addr,
            "PUT",
            &format!("/api/v1/groups/{c}"),
            Some(json!({"name": "x"}))
        )
        .await
        .status,
        404
    );
    let _ = std::fs::remove_dir_all(&fx.dir);
}
