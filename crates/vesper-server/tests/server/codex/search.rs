//! Codex search (`GET /codex/search`): built at start off the request path, kept current
//! through edits, the trash and the collection trash, scoped to the active collection, one
//! collection or all, and its query checks.

use crate::common::*;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

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

async fn search(addr: SocketAddr, query: &str) -> Value {
    let r = call(addr, "GET", &format!("/api/v1/codex/search?{query}"), None).await;
    ok(&r, 200, query)
}

fn titles(v: &Value) -> Vec<String> {
    v["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["title"].as_str().unwrap().to_string())
        .collect()
}

/// The index (as the background thread keeps it, without a search's own sync) finds `word` in
/// `collection` within 10 s.
async fn indexed(server: &vesper_server::RunningServer, collection: &str, word: &str, n: u32) {
    let id: model::CollectionId = collection.parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let r = server.shared.search.search(word, &[id], None, 10);
        if r.total == n {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{word}: {} hits in the index, not {n}",
            r.total
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_follows_edits_the_trash_and_collections() {
    let fx = fixture("codex-search");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();

    // Built at start by the indexing thread.
    indexed(&server, &main, "four", 1).await;

    // Slide text: where it is, the match marked.
    let r = search(addr, "q=four").await;
    assert_eq!(r["total"], 1, "{r}");
    let h = &r["hits"][0];
    assert_eq!(h["kind"], "presentation");
    assert_eq!(h["id"], fx.song.id.0.to_string());
    assert_eq!(h["collection"], main.as_str());
    assert_eq!(h["title"], "Test Song");
    assert_eq!(h["field"], "text");
    assert_eq!(h["slideIndex"], 3);
    assert_eq!(h["slide"], fx.song.sections[1].slides[1].id.0.to_string());
    assert_eq!(h["section"], fx.song.sections[1].id.0.to_string());
    assert_eq!(h["snippet"], json!([{"text": "FOUR", "mark": true}]));
    // Names: the presentations and the "Songs" library (it starts with the word, first);
    // percent-encoded, accents and case folded; kinds.
    let r = search(addr, "q=S%C3%96NG").await;
    assert_eq!(titles(&r), ["Songs", "Second Song", "Test Song"], "{r}");
    assert_eq!(r["hits"][0]["kind"], "library");
    let r = search(addr, "q=song&kinds=presentation,timer").await;
    assert_eq!(titles(&r), ["Second Song", "Test Song"]);
    let r = search(addr, "q=%22test%20song%22").await;
    assert_eq!(titles(&r), ["Test Song"]);
    let r = search(addr, "q=%22song+test%22").await;
    assert_eq!(r["total"], 0, "a phrase keeps its order: {r}");
    let r = search(addr, "q=song&limit=1").await;
    assert_eq!(
        (r["hits"].as_array().unwrap().len(), &r["total"]),
        (1, &json!(3))
    );
    assert_eq!(
        search(addr, "q=services").await["hits"][0]["kind"],
        "playlistNode"
    );
    assert_eq!(
        search(addr, "q=blue").await["hits"][0]["kind"],
        "mediaAsset"
    );
    assert_eq!(search(addr, "q=lower").await["hits"][0]["kind"], "theme");
    assert_eq!(search(addr, "q=chorus").await["hits"][0]["field"], "group");
    assert_eq!(search(addr, "q=%20").await, json!({"hits": [], "total": 0}));

    // Added: found at once (a search syncs first), and by the indexing thread.
    let r = call(
        addr,
        "POST",
        "/api/v1/presentations",
        Some(json!({"name": "Lantern Hymn"})),
    )
    .await;
    let pid = ok(&r, 201, "create")["id"].as_str().unwrap().to_string();
    assert_eq!(titles(&search(addr, "q=lantern").await), ["Lantern Hymn"]);
    indexed(&server, &main, "lantern", 1).await;
    // Changed.
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/presentations/{pid}/edits"),
        Some(json!({"ops": [{"type": "setPresentation", "name": "Candle Hymn"}]})),
    )
    .await;
    ok(&r, 200, "rename");
    assert_eq!(search(addr, "q=lantern").await["total"], 0);
    assert_eq!(titles(&search(addr, "q=candle").await), ["Candle Hymn"]);
    // Removed, and back from the trash.
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
    assert_eq!(search(addr, "q=candle").await["total"], 0);
    indexed(&server, &main, "candle", 0).await;
    let trash = get_json(addr, "/api/v1/codex/trash").await;
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
        "restore",
    );
    assert_eq!(titles(&search(addr, "q=candle").await), ["Candle Hymn"]);

    // Another collection, with a presentation of its own.
    let r = call(
        addr,
        "POST",
        "/api/v1/codex/collections",
        Some(json!({"name": "Youth"})),
    )
    .await;
    let yid = ok(&r, 201, "create Youth")["collection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let assign = |to: String| async move {
        let r = call(
            addr,
            "PUT",
            "/api/v1/codex/assignment",
            Some(json!({"collection": to, "confirm": true})),
        )
        .await;
        ok(&r, 200, "assign");
    };
    assign(yid.clone()).await;
    let r = call(
        addr,
        "POST",
        "/api/v1/presentations",
        Some(json!({"name": "Firefly Anthem"})),
    )
    .await;
    ok(&r, 201, "create in Youth");
    // The default scope is the active collection.
    assert_eq!(search(addr, "q=firefly").await["total"], 1);
    assert_eq!(search(addr, "q=candle").await["total"], 0);
    assign(main.clone()).await;
    assert_eq!(search(addr, "q=firefly").await["total"], 0);
    let r = search(addr, "q=firefly&collection=all").await;
    assert_eq!(r["total"], 1);
    assert_eq!(r["hits"][0]["collection"], yid.as_str());
    assert_eq!(r["hits"][0]["collectionName"], "Youth");
    assert_eq!(
        search(addr, &format!("q=firefly&collection={yid}")).await["total"],
        1
    );
    assert_eq!(
        search(addr, &format!("q=candle&collection={yid}")).await["total"],
        0
    );
    let r = search(addr, "q=hymn%20OR%20anthem&collection=all").await;
    assert_eq!(r["total"], 0, "every word must match: {r}");

    // Into the collection trash: not searched (and not a scope); restored: searched again;
    // deleted for good: gone.
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/codex/collections/{yid}"),
            None,
        )
        .await,
        204,
        "delete Youth",
    );
    assert_eq!(search(addr, "q=firefly&collection=all").await["total"], 0);
    let r = call(
        addr,
        "GET",
        &format!("/api/v1/codex/search?q=firefly&collection={yid}"),
        None,
    )
    .await;
    assert_eq!(r.status, 404);
    ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/codex/collection-trash/{yid}/restore"),
            None,
        )
        .await,
        200,
        "restore Youth",
    );
    assert_eq!(search(addr, "q=firefly&collection=all").await["total"], 1);
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/codex/collections/{yid}"),
            None,
        )
        .await,
        204,
        "delete Youth again",
    );
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/codex/collection-trash/{yid}"),
            None,
        )
        .await,
        204,
        "purge Youth",
    );
    assert_eq!(search(addr, "q=firefly&collection=all").await["total"], 0);

    // A reload finds what changed on disk.
    let p = model::Presentation::new("Meadow Psalm");
    write_doc(
        &content_dir(&fx.dir),
        "portable",
        "presentation",
        &p.id.0.to_string(),
        &p,
    );
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
    assert_eq!(titles(&search(addr, "q=meadow").await), ["Meadow Psalm"]);

    // Bad queries: 400 with the field.
    for (query, field) in [
        ("", "q"),
        ("q=x&kinds=screen", "kinds"),
        ("q=x&kinds=nonsense", "kinds"),
        ("q=x&limit=0", "limit"),
        ("q=x&limit=many", "limit"),
        ("q=x&collection=elsewhere", "collection"),
    ] {
        let r = call(addr, "GET", &format!("/api/v1/codex/search?{query}"), None).await;
        assert_eq!(r.status, 400, "{query}");
        assert_eq!(r.json()["error"]["field"], field, "{query}");
    }
    let long = "a".repeat(501);
    let r = call(addr, "GET", &format!("/api/v1/codex/search?q={long}"), None).await;
    assert_eq!(r.status, 400);
}
