//! Content editing over the API: presentation edit ops (concurrency, conflicts, live
//! re-render, deleted live slide), documentChanged with ops, library / presentation /
//! playlist CRUD with If-Match, and persistence. Synthetic data only.

use crate::common::*;
use serde_json::{Value, json};

fn slide_ids(p: &Value) -> Vec<String> {
    p["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["slides"].as_array().cloned().unwrap_or_default())
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect()
}

async fn edit(addr: std::net::SocketAddr, id: &str, body: Value) -> Resp {
    http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{id}/edits"),
        &[],
        Some(body),
    )
    .await
}

/// Number of white pixels (text) in a frame.
fn white(img: &image::RgbaImage) -> usize {
    count(img, 0, img.height(), WHITE, 30)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn presentation_edits() {
    let fx = fixture("edits");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let pid = fx.song.id.0.to_string();
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let slides = slide_ids(&p);

    let mut ws = Client::connect(addr, "protocolVersion=1").await;

    // Two operators edit different slides from the same base revision: both succeed.
    let r1 = edit(
        addr,
        &pid,
        json!({"clientId": "op-a", "baseRevision": "old", "ops": [
            {"type": "setSlideText", "slide": slides[1], "text": "TWO EDITED", "expectText": "TWO"}
        ]}),
    )
    .await;
    assert_eq!(r1.status, 200, "{}", String::from_utf8_lossy(&r1.body));
    let r2 = edit(
        addr,
        &pid,
        json!({"clientId": "op-b", "ops": [
            {"type": "setSlide", "slide": slides[3], "label": {"text": "END"}}
        ]}),
    )
    .await;
    assert_eq!(r2.status, 200, "{}", String::from_utf8_lossy(&r2.body));
    let res = r1.json();
    assert_eq!(res["applied"][0]["type"], "setSlideText");
    assert!(
        res["applied"][0]["element"].is_string(),
        "element filled in"
    );
    assert_eq!(res["inverse"][0]["type"], "replaceTextBox");
    assert_eq!(
        r2.header("etag"),
        Some(format!("\"{}\"", r2.json()["revision"].as_str().unwrap()).as_str())
    );

    // documentChanged carries the revision, the client and the applied ops.
    let ev = loop {
        let m = ws.next_any().await;
        if m["type"] == "documentChanged" && m["clientId"] == "op-a" {
            break m;
        }
    };
    assert_eq!(ev["kind"], "presentation");
    assert_eq!(ev["id"], pid);
    assert_eq!(ev["ops"][0]["type"], "setSlideText");
    assert_eq!(ev["ops"][0]["text"], "TWO EDITED");
    assert!(ev["revision"].is_string());

    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(
        p["sections"][0]["slides"][1]["elements"][0]["kind"]["text"]["paragraphs"][0]["runs"][0]["text"],
        "TWO EDITED"
    );
    assert_eq!(p["sections"][1]["slides"][1]["label"]["text"], "END");
    // Persisted atomically to the data folder (the Codex's active collection).
    let on_disk = std::fs::read_to_string(
        content_dir(&fx.dir).join(format!("portable/presentation/{pid}.json")),
    )
    .unwrap();
    assert!(on_disk.contains("TWO EDITED") && on_disk.contains("\"modifiedAt\""));

    // expectText mismatch: 409 with the current text.
    let r = edit(
        addr,
        &pid,
        json!({"ops": [{"type": "setSlideText", "slide": slides[1], "text": "mine", "expectText": "TWO"}]}),
    )
    .await;
    assert_eq!(r.status, 409);
    let j = r.json();
    assert_eq!(j["error"]["code"], "conflict");
    assert_eq!(j["current"]["text"], "TWO EDITED");
    assert_eq!(j["current"]["opIndex"], 0);
    assert!(j["current"]["revision"].is_string());

    // A target that no longer exists: 409, nothing applied (the first op neither).
    let r = edit(
        addr,
        &pid,
        json!({"ops": [
            {"type": "setSlide", "slide": slides[0], "enabled": false},
            {"type": "deleteSlide", "slide": "00000000-0000-4000-8000-000000000000"}
        ]}),
    )
    .await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["current"]["opIndex"], 1);
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert!(
        p["sections"][0]["slides"][0].get("enabled").is_none(),
        "batch is all or nothing"
    );

    // Undo with the inverse, then redo with the inverse's inverse.
    let undo = edit(addr, &pid, json!({"ops": res["inverse"]})).await;
    assert_eq!(undo.status, 200);
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(
        p["sections"][0]["slides"][1]["elements"][0]["kind"]["text"]["paragraphs"][0]["runs"][0]["text"],
        "TWO"
    );
    let redo = edit(addr, &pid, json!({"ops": undo.json()["inverse"]})).await;
    assert_eq!(redo.status, 200);

    // The live slide is edited: the screens re-render with the new text.
    ws.ok(json!({"type": "setLook", "look": fx.plain_look.id}))
        .await;
    ws.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 1}))
        .await;
    let before = wait_frame(&server, 0, "TWO EDITED live", |img| white(img) > 500).await;
    let r = edit(
        addr,
        &pid,
        json!({"ops": [{"type": "setSlideText", "slide": slides[1], "text": "I"}]}),
    )
    .await;
    assert_eq!(r.status, 200);
    let n0 = white(&before);
    wait_frame(&server, 0, "re-rendered with the short text", |img| {
        white(img) < n0 / 3
    })
    .await;

    // Thumbnails follow: the ETag of the edited slide changes.
    let t1 = http(
        addr,
        "GET",
        &format!(
            "/api/v1/presentations/{pid}/slides/{}/thumbnail?w=160",
            slides[1]
        ),
        &[],
        None,
    )
    .await;
    let r = edit(
        addr,
        &pid,
        json!({"ops": [{"type": "setSlideText", "slide": slides[1], "text": "IIII"}]}),
    )
    .await;
    assert_eq!(r.status, 200);
    let t2 = http(
        addr,
        "GET",
        &format!(
            "/api/v1/presentations/{pid}/slides/{}/thumbnail?w=160",
            slides[1]
        ),
        &[],
        None,
    )
    .await;
    assert_eq!(t1.status, 200);
    assert_ne!(t1.header("etag"), t2.header("etag"));

    // The live slide is deleted: it stays on screen until the next trigger; the cursor moves
    // on to the slide that took its place.
    let shown = wait_frame(&server, 0, "IIII live", |img| white(img) > 0).await;
    let r = edit(
        addr,
        &pid,
        json!({"ops": [{"type": "deleteSlide", "slide": slides[1]}]}),
    )
    .await;
    assert_eq!(r.status, 200);
    ws.wait_state("held live slide", |_, s| {
        s["layers"]["slide"]["slide"] == slides[1] && s["cursor"]["slide"] == slides[3]
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let still = latest(&server, 0).unwrap();
    assert_eq!(
        white(&still),
        white(&shown),
        "the deleted slide stays on screen"
    );
    // A configuration change (here a screen renamed) keeps the held copy and the cursor.
    let mut ad = serde_json::to_value(&fx.audience).unwrap();
    ad["name"] = json!("Audience renamed");
    let renamed = http(
        addr,
        "PUT",
        &format!("/api/v1/screens/{}", fx.audience.id.0),
        &[],
        Some(ad),
    )
    .await;
    assert_eq!(
        renamed.status,
        200,
        "{}",
        String::from_utf8_lossy(&renamed.body)
    );
    ws.wait_state("screen renamed", |_, s| {
        s["screens"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == "Audience renamed")
    })
    .await;
    assert_eq!(ws.state["layers"]["slide"]["slide"], slides[1]);
    assert_eq!(ws.state["cursor"]["slide"], slides[3]);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        white(&latest(&server, 0).unwrap()),
        white(&shown),
        "the held slide stays on screen after a configuration change"
    );
    // `next` goes to the slide that took its place (slide 2 "SKIP" is disabled → "FOUR").
    ws.ok(json!({"type": "next"})).await;
    assert_eq!(ws.state["layers"]["slide"]["slide"], slides[3]);
    // Undo the delete: the slide comes back where it was.
    let r = edit(addr, &pid, json!({"ops": r.json()["inverse"]})).await;
    assert_eq!(r.status, 200);
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(slide_ids(&p), slides);

    // Split / merge through the API.
    let r = edit(
        addr,
        &pid,
        json!({"ops": [
            {"type": "setSlideText", "slide": slides[0], "text": "A\nB"},
            {"type": "splitSlide", "slide": slides[0], "atParagraph": 1}
        ]}),
    )
    .await;
    assert_eq!(r.status, 200);
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(slide_ids(&p).len(), 5);
    let new = slide_ids(&p)[1].clone();
    let r = edit(
        addr,
        &pid,
        json!({"ops": [{"type": "mergeSlides", "first": slides[0], "second": new}]}),
    )
    .await;
    assert_eq!(r.status, 200);

    // Bad requests.
    let r = edit(addr, &pid, json!({"ops": [{"type": "nope"}]})).await;
    assert_eq!(r.status, 400);
    let r = http(
        addr,
        "POST",
        "/api/v1/presentations/00000000-0000-4000-8000-000000000000/edits",
        &[],
        Some(json!({"ops": []})),
    )
    .await;
    assert_eq!(r.status, 404);

    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn library_and_playlist_crud() {
    let fx = fixture("crud");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let cid = [("X-Client-Id", "ui-1")];

    // Libraries: create, rename (If-Match), delete-if-empty.
    let r = http(
        addr,
        "POST",
        "/api/v1/libraries",
        &cid,
        Some(json!({"name": "Hymns"})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let lib = r.json();
    let lid = lib["id"].as_str().unwrap().to_string();
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/libraries/{lid}"),
        &[("If-Match", "\"stale\"")],
        Some(json!({"name": "X"})),
    )
    .await;
    assert_eq!(r.status, 409);
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/libraries/{lid}"),
        &[(
            "If-Match",
            &format!("\"{}\"", lib["revision"].as_str().unwrap()),
        )],
        Some(json!({"name": "Old Hymns"})),
    )
    .await;
    assert_eq!(r.status, 200);
    let libs = get_json(addr, "/api/v1/libraries").await;
    assert!(
        libs.as_array()
            .unwrap()
            .iter()
            .any(|l| l["name"] == "Old Hymns")
    );
    let ev = loop {
        let m = ws.next_any().await;
        if m["type"] == "documentChanged" && m["kind"] == "library" {
            break m;
        }
    };
    assert_eq!(ev["clientId"], "ui-1");

    // Presentations: create blank, rename, duplicate, move, delete.
    let r = http(
        addr,
        "POST",
        "/api/v1/presentations",
        &[],
        Some(json!({"name": "Reflow Test", "library": lid})),
    )
    .await;
    assert_eq!(r.status, 201);
    let pid = r.json()["id"].as_str().unwrap().to_string();
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(p["sections"].as_array().unwrap().len(), 1);
    assert_eq!(slide_ids(&p).len(), 1);
    assert_eq!(
        p["sections"][0]["slides"][0]["elements"][0]["kind"]["type"],
        "text"
    );
    let r = edit(
        addr,
        &pid,
        json!({"ops": [{"type": "setPresentation", "name": "Renamed"}]}),
    )
    .await;
    assert_eq!(r.status, 200);
    let list = get_json(addr, &format!("/api/v1/libraries/{lid}/presentations")).await;
    assert_eq!(list[0]["name"], "Renamed");
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/libraries/{lid}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 409, "not empty");
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{pid}/duplicate"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 201);
    let dup = r.json()["id"].as_str().unwrap().to_string();
    let list = get_json(addr, &format!("/api/v1/libraries/{lid}/presentations")).await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{dup}/move"),
        &[],
        Some(json!({"library": fx.library.id})),
    )
    .await;
    assert_eq!(r.status, 200);
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/presentations/{pid}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    let deleted = r.json();
    assert_eq!(deleted["kind"], "presentation");
    assert_eq!(deleted["documents"][0]["name"], "Renamed");
    assert!(
        !content_dir(&fx.dir)
            .join(format!("portable/presentation/{pid}.json"))
            .exists()
    );
    // Undo of a delete: create from the document (same ID).
    let r = http(
        addr,
        "POST",
        "/api/v1/presentations",
        &[],
        Some(json!({"document": deleted["documents"][0]})),
    )
    .await;
    assert_eq!(r.status, 201);
    assert_eq!(r.json()["id"], pid);
    http(
        addr,
        "DELETE",
        &format!("/api/v1/presentations/{pid}"),
        &[],
        None,
    )
    .await;
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/libraries/{lid}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));

    // Playlists and folders.
    let r = http(
        addr,
        "POST",
        "/api/v1/playlists",
        &[],
        Some(json!({"name": "Sunday Test"})),
    )
    .await;
    assert_eq!(r.status, 201);
    let plid = r.json()["id"].as_str().unwrap().to_string();
    let r = http(
        addr,
        "POST",
        "/api/v1/playlists",
        &[],
        Some(json!({"name": "Archive", "kind": "folder"})),
    )
    .await;
    let folder = r.json()["id"].as_str().unwrap().to_string();
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/playlists/{plid}"),
        &[],
        Some(json!({"name": "Sunday", "parent": folder})),
    )
    .await;
    assert_eq!(r.status, 200);
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/playlists/{folder}"),
        &[],
        Some(json!({"parent": plid})),
    )
    .await;
    assert_eq!(r.status, 400, "a playlist is not a folder");
    let tree = get_json(addr, "/api/v1/playlists").await;
    let arch = tree
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == folder)
        .unwrap()
        .clone();
    assert_eq!(arch["children"][0]["name"], "Sunday");

    // Item ops: add a presentation and a header, reorder, rename, remove; with inverses.
    let song = fx.song.id;
    let r = http(addr, "POST", &format!("/api/v1/playlists/{plid}/edits"), &[], Some(json!({"clientId": "ui-1", "ops": [
        {"type": "addItem", "item": {"id": "11111111-1111-4111-8111-111111111111",
            "kind": {"type": "presentation", "presentation": {"id": song, "name": "Test Song"}}}},
        {"type": "addItem", "after": "11111111-1111-4111-8111-111111111111",
            "item": {"id": "22222222-2222-4222-8222-222222222222", "kind": {"type": "header", "name": "Worship", "color": {"r": 1, "g": 0.5, "b": 0}}}},
        {"type": "moveItem", "item": "22222222-2222-4222-8222-222222222222"},
        {"type": "setItem", "item": "22222222-2222-4222-8222-222222222222", "name": "Welcome"}
    ]}))).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let inverse = r.json()["inverse"].clone();
    let d = get_json(addr, &format!("/api/v1/playlists/{plid}")).await;
    assert_eq!(d["items"][0]["displayName"], "Welcome");
    assert_eq!(d["items"][1]["displayName"], "Test Song");
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/playlists/{plid}/edits"),
        &[],
        Some(json!({"ops": inverse})),
    )
    .await;
    assert_eq!(r.status, 200);
    let d = get_json(addr, &format!("/api/v1/playlists/{plid}")).await;
    assert_eq!(d["items"].as_array().unwrap().len(), 0);
    let r = http(addr, "POST", &format!("/api/v1/playlists/{plid}/edits"), &[], Some(json!({"ops": [{"type": "removeItem", "item": "22222222-2222-4222-8222-222222222222"}]}))).await;
    assert_eq!(r.status, 409);

    // Duplicate and delete (a folder takes its subtree).
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/playlists/{plid}/duplicate"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 201);
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/playlists/{folder}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["documents"].as_array().unwrap().len(), 3);
    let tree = get_json(addr, "/api/v1/playlists").await;
    assert!(!tree.to_string().contains(&plid));

    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// Names: an edit can't store a name creating would refuse, and a long name can still be
/// duplicated with the default "copy" name (A05-4).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn names_stay_valid_through_edits_and_copies() {
    let fx = fixture("names");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let pid = fx.song.id.0.to_string();
    for bad in ["two\nlines".to_string(), "y".repeat(201)] {
        let r = edit(
            addr,
            &pid,
            json!({"ops": [{"type": "setPresentation", "name": bad}]}),
        )
        .await;
        assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    }
    let long = "z".repeat(199);
    let r = edit(
        addr,
        &pid,
        json!({"ops": [{"type": "setPresentation", "name": long}]}),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{pid}/duplicate"),
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let copy = r.json()["id"].as_str().unwrap().to_string();
    let doc = get_json(addr, &format!("/api/v1/presentations/{copy}")).await;
    let name = doc["name"].as_str().unwrap();
    assert!(
        name.ends_with(" copy") && name.chars().count() <= 200,
        "{name}"
    );
    server.shutdown().await.unwrap();
}
