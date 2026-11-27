//! Show boxes (M8–M9): a server that joins a site whose server hosts the Codex mirrors one of
//! its collections over IDN's `channel/1` and keeps the media it needs in IDN's library cache.
//! Two servers in this test process (A, the site's server with the Codex; B, a machine of A's
//! site), on loopback ports, with synthetic fixtures.
//!
//! - B finds the Codex, puts its own Codex to sleep (`dormant`, kept intact), copies A's
//!   collection and shows it (`client`); an edit on A reaches B (the Codex's push), and with
//!   pushes off by B's poll; B's writes go through to A; B's assignment switches to another
//!   collection of A's Codex; leaving the site wakes B's own Codex.
//! - A restart of B resumes from its saved position; a position below A's compacted change log
//!   resyncs by the index; A down: B keeps showing what it has and says the Codex is
//!   unreachable.
//! - Media: an item in a media bin is pinned, lands in B's cache and plays from B's disk; an
//!   unscheduled item is `copying` until triggered, then copied.

use crate::common::*;
use crate::site::keys::{TwoBoxes, two_boxes_with};
use crate::site::{call, get_json_auth, ok, start_with_codex};
use model::MediaBin;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vesper_server::RunningServer;
use vesper_server::codex::CodexOptions;
use vesper_server::site::SiteConfig;

/// Waits until `f` holds (checked every 100 ms), up to `secs`.
pub(super) async fn until<F, Fut>(what: &str, secs: u64, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !f().await {
        assert!(Instant::now() < deadline, "{what}: not in {secs} s");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The next message matching `pred` (seen before or arriving within 30 s).
pub(super) async fn event(c: &mut Client, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    if let Some(i) = c.events.iter().position(&pred) {
        return c.events.remove(i);
    }
    let wait = async {
        loop {
            let m = c.next_any().await;
            if pred(&m) {
                return m;
            }
            c.absorb(&m);
        }
    };
    tokio::time::timeout(Duration::from_secs(30), wait)
        .await
        .unwrap_or_else(|_| panic!("{what}: no such event in 30 s"))
}

pub(super) async fn codex_status(addr: SocketAddr, key: &str) -> Value {
    ok(
        &call(addr, "GET", "/api/v1/codex", Some(key), None).await,
        200,
        "GET /codex",
    )
}

/// B shows A's collection (its own Codex dormant).
pub(super) async fn wait_client(t: &TwoBoxes) -> Value {
    let (b, key) = (t.b.addr, t.key_a.clone());
    until("B becomes a show box of A's Codex", 60, || {
        let key = key.clone();
        async move { codex_status(b, &key).await["role"] == "client" }
    })
    .await;
    codex_status(b, &t.key_a).await
}

pub(super) fn has_presentation(server: &RunningServer, id: &str) -> bool {
    server
        .shared
        .data()
        .content
        .presentations
        .iter()
        .any(|p| p.id.0.to_string() == id)
}

/// Creates a presentation on `addr` (in the first library); its ID.
pub(super) async fn create_presentation(addr: SocketAddr, key: &str, name: &str) -> String {
    let libs = ok(
        &call(addr, "GET", "/api/v1/libraries", Some(key), None).await,
        200,
        "libraries",
    );
    let lib = libs[0]["id"].as_str().unwrap().to_string();
    let r = call(
        addr,
        "POST",
        "/api/v1/presentations",
        Some(key),
        Some(json!({"name": name, "library": lib})),
    )
    .await;
    ok(&r, 201, "create a presentation")["id"]
        .as_str()
        .unwrap()
        .to_string()
}

pub(super) fn sha_hex(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

pub(super) fn solid_png(c: [u8; 3]) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(64, 36, image::Rgb(c));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// Uploads `bytes` as `name` into `collection` on `addr` (into `bin` when given); the asset.
pub(super) async fn upload(
    addr: SocketAddr,
    key: &str,
    collection: &str,
    name: &str,
    bytes: &[u8],
    bin: Option<&str>,
) -> Value {
    let mut body = json!({"collection": collection, "name": name, "size": bytes.len(),
                          "sha256": sha_hex(bytes)});
    if let Some(b) = bin {
        body["bin"] = json!(b);
    }
    let r = call(addr, "POST", "/api/v1/codex/uploads", Some(key), Some(body)).await;
    let id = ok(&r, 201, "create an upload")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bearer = format!("Bearer {key}");
    let range = format!("bytes 0-{}/{}", bytes.len() - 1, bytes.len());
    let r = http_bytes(
        addr,
        "PUT",
        &format!("/api/v1/codex/uploads/{id}"),
        &[("Content-Range", &range), ("Authorization", &bearer)],
        bytes,
    )
    .await;
    ok(&r, 200, "the chunk");
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/uploads/{id}/finish"),
        Some(key),
        None,
    )
    .await;
    ok(&r, 201, "finish")["asset"].clone()
}

/// An asset's availability on `addr` (`GET /codex/collections/{id}/media`).
async fn availability(addr: SocketAddr, key: &str, collection: &str, asset: &str) -> String {
    let list = ok(
        &call(
            addr,
            "GET",
            &format!("/api/v1/codex/collections/{collection}/media"),
            Some(key),
            None,
        )
        .await,
        200,
        "media",
    );
    list.as_array()
        .unwrap()
        .iter()
        .find(|m| m["asset"]["id"] == asset)
        .map(|m| m["availability"].as_str().unwrap().to_string())
        .unwrap_or_else(|| "absent".into())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_show_box_mirrors_the_sites_codex_and_follows_its_edits() {
    let t = two_boxes_with(
        "cx-mirror",
        CodexOptions::default(),
        CodexOptions {
            mirror_poll: Duration::from_secs(2),
            ..CodexOptions::default()
        },
        |_| {},
    )
    .await;
    let (a, b, key) = (t.a.addr, t.b.addr, t.key_a.clone());
    let own =
        t.b.shared
            .codex()
            .config
            .collection
            .expect("B's own collection");
    let a_codex = codex_status(a, &key).await;
    assert_eq!(a_codex["role"], "host");

    // B finds A's Codex, copies its collection and shows it; its own Codex sleeps, intact.
    let st = wait_client(&t).await;
    assert_eq!(st["id"], a_codex["id"], "{st}");
    assert_eq!(st["collection"], a_codex["collection"], "{st}");
    assert_eq!(st["dormantCollection"], own.to_string(), "{st}");
    let sync = &st["sync"];
    assert_eq!(sync["reachable"], true, "{st}");
    let a_server = get_json_auth(a, "/api/v1/server", Some(&key)).await;
    assert_eq!(sync["serverId"], a_server["id"]);
    assert_eq!(sync["name"], a_server["name"], "the Codex box's name: {st}");
    assert_eq!(sync["behind"], 0, "{st}");
    assert!(has_presentation(&t.b, &t.fa.song.id.0.to_string()));
    assert!(!has_presentation(&t.b, &t.fb.song.id.0.to_string()));
    let col = a_codex["collection"].as_str().unwrap().to_string();
    assert!(
        t.fb.dir
            .join(format!("codex-cache/collections/{col}/state.json"))
            .is_file()
    );
    assert!(
        t.fb.dir
            .join(format!("codex/collections/{own}/collection.json"))
            .is_file(),
        "B's own Codex is kept"
    );
    let cfg: Value =
        serde_json::from_slice(&std::fs::read(t.fb.dir.join("machine/codex.json")).unwrap())
            .unwrap();
    assert_eq!(cfg["role"], "client", "{cfg}");
    assert_eq!(cfg["ownCollection"], own.to_string(), "{cfg}");
    assert_eq!(
        cfg["codex"]["fingerprint"],
        t.a.shared.codex_member.fingerprint().unwrap(),
        "{cfg}"
    );

    // An edit on A reaches B, announced as an edit of the collection.
    let mut ws = Client::connect(b, &format!("protocolVersion=1&token={key}")).await;
    let id = create_presentation(a, &key, "Mirrored").await;
    until("the new presentation reaches B", 15, || async {
        has_presentation(&t.b, &id)
    })
    .await;
    let ev = event(&mut ws, "documentChanged on B", |e| {
        e["type"] == "documentChanged" && e["id"] == id.as_str()
    })
    .await;
    assert_eq!(ev["change"], "added", "{ev}");
    assert_eq!(ev["collection"], col.as_str(), "{ev}");
    // Its file on B is the Codex's, byte for byte.
    let rel = format!("collections/{col}/portable/presentation/{id}.json");
    assert_eq!(
        std::fs::read(t.fb.dir.join("codex-cache").join(&rel)).unwrap(),
        std::fs::read(t.fa.dir.join("codex").join(&rel)).unwrap()
    );

    // A rename of the collection on A reaches B's status and collection list (A06-6).
    ok(
        &call(
            a,
            "PUT",
            &format!("/api/v1/codex/collections/{col}"),
            Some(&key),
            Some(json!({"name": "Renamed on A"})),
        )
        .await,
        200,
        "rename on A",
    );
    until("the rename reaches B's status", 15, || async {
        codex_status(b, &key).await["collectionName"] == "Renamed on A"
    })
    .await;
    let list = ok(
        &call(b, "GET", "/api/v1/codex/collections", Some(&key), None).await,
        200,
        "B's collections",
    );
    assert_eq!(list[0]["collection"]["name"], "Renamed on A", "{list}");
    event(&mut ws, "codexChanged on B for the rename", |e| {
        e["type"] == "codexChanged"
            && e["collection"] == col.as_str()
            && e["parts"]
                .as_array()
                .is_some_and(|p| p.contains(&json!("status")))
    })
    .await;

    // Writes on B go through to A (write-through, `codex::write_through`); collection
    // administration belongs to A.
    let here = create_presentation(b, &key, "Here").await;
    assert!(has_presentation(&t.a, &here) && has_presentation(&t.b, &here));
    let r = call(
        b,
        "POST",
        "/api/v1/codex/collections",
        Some(&key),
        Some(json!({"name": "Not here"})),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["error"]["code"], "codexElsewhere");

    // B shows another collection of A's Codex: copied, then shown.
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
    let st = ok(&r, 200, "B's assignment");
    assert_eq!(st["collection"], youth.as_str(), "{st}");
    assert!(!has_presentation(&t.b, &t.fa.song.id.0.to_string()));
    assert!(
        t.fb.dir
            .join(format!("codex-cache/collections/{youth}/state.json"))
            .is_file()
    );
    // A collection A's Codex doesn't have: 400.
    let r = call(
        b,
        "PUT",
        "/api/v1/codex/assignment",
        Some(&key),
        Some(json!({"collection": own})),
    )
    .await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));

    // Leaving the site wakes B's own Codex: its collection is shown again.
    ok(
        &call(b, "POST", "/api/v1/site/leave", Some(&key), None).await,
        204,
        "B leaves",
    );
    until("B's own Codex wakes", 30, || async {
        let c = t.b.shared.codex();
        c.config.role == model::CodexRole::Host && c.active == own
    })
    .await;
    assert!(has_presentation(&t.b, &t.fb.song.id.0.to_string()));
    let cfg: Value =
        serde_json::from_slice(&std::fs::read(t.fb.dir.join("machine/codex.json")).unwrap())
            .unwrap();
    assert_eq!(cfg["role"], "host", "{cfg}");
    assert!(cfg.get("ownCollection").is_none(), "{cfg}");
    t.end().await;
}

/// The Codex pushes each change to the show boxes following the collection: with B's poll
/// far away, every edit still reaches B within seconds.
///
/// (Failed with IDN v0.3.1-Beta3: the pushes reached the media library's own channel on B.
/// Since Beta4 B's mirror finds the Codex over its own channel: one endpoint.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_codex_pushes_changes_to_its_show_boxes() {
    let t = two_boxes_with(
        "cx-push",
        CodexOptions::default(),
        CodexOptions {
            mirror_poll: Duration::from_secs(300),
            ..CodexOptions::default()
        },
        |_| {},
    )
    .await;
    let key = t.key_a.clone();
    wait_client(&t).await;
    for n in 0..3 {
        let id = create_presentation(t.a.addr, &key, &format!("Pushed {n}")).await;
        until("a pushed edit reaches B", 10, || async {
            has_presentation(&t.b, &id)
        })
        .await;
    }
    assert!(t.b.shared.mirror.counters().pushes >= 3);
    t.end().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mirror_polls_resumes_after_restarts_resyncs_and_outlives_the_codex() {
    // No pushes from A; B asks every second.
    let b_opts = CodexOptions {
        mirror_poll: Duration::from_secs(1),
        ..CodexOptions::default()
    };
    let t = two_boxes_with(
        "cx-poll",
        CodexOptions {
            push_changes: false,
            ..CodexOptions::default()
        },
        b_opts.clone(),
        |_| {},
    )
    .await;
    let key = t.key_a.clone();
    wait_client(&t).await;
    let TwoBoxes { fa, a, fb, b, .. } = t;

    // Pushes lost (off): B's poll brings the edit.
    let id = create_presentation(a.addr, &key, "Polled").await;
    until("the edit reaches B by its poll", 15, || async {
        has_presentation(&b, &id)
    })
    .await;
    assert_eq!(b.shared.mirror.counters().pushes, 0);

    // A restart of B resumes from its saved position (the change log, no index).
    b.shutdown().await.unwrap();
    let id2 = create_presentation(a.addr, &key, "While B was off").await;
    let b = start_with_codex(&fb, false, SiteConfig::loopback(), b_opts.clone()).await;
    until("B catches up after its restart", 60, || async {
        has_presentation(&b, &id2)
    })
    .await;
    let c = b.shared.mirror.counters();
    assert_eq!(c.index_syncs, 0, "{c:?}");
    assert!(c.change_batches > 0, "{c:?}");

    // B's position below A's compacted change log: a resync by the index.
    b.shutdown().await.unwrap();
    let id3 = create_presentation(a.addr, &key, "After the compaction").await;
    a.shared
        .codex_writer
        .tick(
            &a.shared.codex_server(),
            std::time::SystemTime::now() + Duration::from_secs(100 * 86_400),
        )
        .await;
    let b = start_with_codex(&fb, false, SiteConfig::loopback(), b_opts).await;
    until("B resyncs", 60, || async { has_presentation(&b, &id3) }).await;
    let c = b.shared.mirror.counters();
    assert!(c.index_syncs >= 1, "{c:?}");
    assert!(has_presentation(&b, &id) && has_presentation(&b, &id2));

    // A down: B keeps showing what it has, says the Codex is unreachable, refuses writes.
    a.shutdown().await.unwrap();
    until("B sees the Codex is unreachable", 60, || async {
        b.shared
            .mirror
            .status(&b.shared)
            .is_some_and(|s| !s.reachable)
    })
    .await;
    assert!(has_presentation(&b, &id3));
    assert_eq!(b.shared.codex().config.role, model::CodexRole::Client);
    let st = codex_status(b.addr, &key).await;
    assert_eq!(st["sync"]["reachable"], false, "{st}");
    assert!(st["sync"]["problem"].as_str().is_some(), "{st}");
    let libs = ok(
        &call(b.addr, "GET", "/api/v1/libraries", Some(&key), None).await,
        200,
        "B's libraries",
    );
    let r = call(
        b.addr,
        "POST",
        "/api/v1/presentations",
        Some(&key),
        Some(json!({"name": "Offline", "library": libs[0]["id"]})),
    )
    .await;
    assert_eq!(r.status, 503);
    b.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fa.dir).ok();
    std::fs::remove_dir_all(&fb.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduled_media_lands_in_the_show_boxs_cache_and_triggered_media_is_copied() {
    let bin = MediaBin {
        name: "Backgrounds".into(),
        ..MediaBin::default()
    };
    let bin_id = bin.id.0.to_string();
    let t = two_boxes_with(
        "cx-media",
        CodexOptions::default(),
        CodexOptions {
            mirror_poll: Duration::from_secs(2),
            ..CodexOptions::default()
        },
        move |fx| write_doc(&fx.dir, "portable", "mediaBin", &bin.id.0.to_string(), &bin),
    )
    .await;
    let (a, b, key) = (t.a.addr, t.b.addr, t.key_a.clone());
    let col = codex_status(a, &key).await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    wait_client(&t).await;

    // In a bin (scheduled): pinned, copied to B's cache.
    let red = upload(
        a,
        &key,
        &col,
        "Red.png",
        &solid_png([255, 0, 0]),
        Some(&bin_id),
    )
    .await;
    let red_id = red["id"].as_str().unwrap().to_string();
    let red_item = red["location"]["id"].as_str().unwrap().to_string();
    assert_eq!(red["location"]["type"], "library", "{red}");
    until("the scheduled item lands in B's cache", 60, || async {
        availability(b, &key, &col, &red_id).await == "online"
    })
    .await;
    assert!(t.b.shared.mirror.pins().contains(&red_item));
    let asset: model::MediaAssetId = red_id.parse::<uuid::Uuid>().unwrap().into();
    let path = t.b.shared.data().content.asset_path(asset).unwrap();
    assert!(
        path.starts_with(t.fb.dir.join("codex-cache/media")),
        "{}",
        path.display()
    );
    let st = codex_status(b, &key).await;
    assert!(st["sync"]["media"]["cached"].as_u64().unwrap() >= 1, "{st}");
    assert!(st["sync"]["media"]["pinned"].as_u64().unwrap() >= 1, "{st}");

    // B plays it from its own disk.
    let mut ws = Client::connect(b, &format!("protocolVersion=1&token={key}")).await;
    ws.ok(json!({"type": "triggerMedia", "asset": red_id}))
        .await;
    wait_frame(&t.b, 0, "the red background on B", |img| {
        near(img.get_pixel(W / 2, H / 2), RED, 40)
    })
    .await;

    // Not scheduled: `copying` (copied when triggered), then here.
    let green = upload(a, &key, &col, "Green.png", &solid_png([0, 255, 0]), None).await;
    let green_id = green["id"].as_str().unwrap().to_string();
    until("the unscheduled item's asset reaches B", 30, || async {
        availability(b, &key, &col, &green_id).await != "absent"
    })
    .await;
    assert_eq!(availability(b, &key, &col, &green_id).await, "copying");
    ws.ok(json!({"type": "triggerMedia", "asset": green_id}))
        .await;
    let n = event(&mut ws, "the still-copying notice", |e| {
        e["type"] == "notice"
            && e["message"]
                .as_str()
                .unwrap_or("")
                .contains("still copying")
    })
    .await;
    assert!(n["message"].as_str().unwrap().contains("Green"), "{n}");
    until("the triggered item is copied", 60, || async {
        availability(b, &key, &col, &green_id).await == "online"
    })
    .await;
    t.end().await;
}
