//! Settings backups (M11) and restoring them (M12): this box's snapshots on demand, daily and
//! after changes, retention, the preview and restore with its `beforeRestore` undo, Oriel's
//! preferences per client, who may do what, and the seams for show boxes (a snapshot offered
//! by another machine; a replacement box taking over the old box's server ID once the site
//! removed it). All on synthetic fixtures.

use crate::common::*;
use idn_open_protocol::trust::{DeviceRole, Enrollment, EnrollmentConfig, TrustState};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vesper_server::codex::CodexOptions;
use vesper_server::codex::backups::retention::Retention;
use vesper_server::codex::backups::{
    BackupOptions, OfferedItem, RestoreTarget, SnapshotOffer, vesper,
};
use vesper_server::site::SiteConfig;
use vesper_server::{RunningServer, ServerConfig, start};

fn backups(poll: Duration, change_delay: Duration, retention: Retention) -> CodexOptions {
    CodexOptions {
        backups: BackupOptions {
            poll,
            change_delay,
            retention,
            ..BackupOptions::default()
        },
        ..CodexOptions::default()
    }
}

/// No snapshots of its own while the test runs.
fn quiet() -> CodexOptions {
    backups(
        Duration::from_secs(3600),
        Duration::from_secs(3600),
        Retention::default(),
    )
}

async fn start_with(fx: &Fixture, codex: CodexOptions, site: SiteConfig) -> RunningServer {
    start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: true,
        mdns: false,
        site,
        codex,
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts")
}

async fn call(
    addr: SocketAddr,
    method: &str,
    path: &str,
    auth: Option<&str>,
    body: Option<Value>,
) -> Resp {
    let bearer = auth.map(|t| format!("Bearer {t}"));
    let headers: Vec<(&str, &str)> = bearer
        .as_deref()
        .map(|b| vec![("Authorization", b)])
        .unwrap_or_default();
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

/// Renames a screen through the API (a settings change).
async fn rename_screen(addr: SocketAddr, id: &str, name: &str) {
    let list = get_json(addr, "/api/v1/screens").await;
    let mut doc = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap()
        .clone();
    let tag = format!("\"{}\"", doc["revision"].as_str().unwrap());
    doc["name"] = json!(name);
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/screens/{id}"),
        &[("If-Match", tag.as_str())],
        Some(doc),
    )
    .await;
    ok(&r, 200, "rename screen");
}

async fn screen_name(addr: SocketAddr, id: &str) -> String {
    let list = get_json(addr, "/api/v1/screens").await;
    list.as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .map(|s| s["name"].as_str().unwrap().to_string())
        .unwrap()
}

async fn own_set(addr: SocketAddr) -> Value {
    let sets = get_json(addr, "/api/v1/codex/backups").await;
    sets.as_array()
        .unwrap()
        .iter()
        .find(|s| s["thisBox"] == true)
        .cloned()
        .unwrap_or(Value::Null)
}

async fn snapshots(addr: SocketAddr, set: &str) -> Vec<Value> {
    get_json(addr, &format!("/api/v1/codex/backups/{set}/snapshots"))
        .await
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_preview_restore_and_undo() {
    let fx = fixture("codex-backups");
    let server = start_with(&fx, quiet(), SiteConfig::off()).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let audience = fx.audience.id.0.to_string();
    let server_id = get_json(addr, "/api/v1/server").await["id"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(get_json(addr, "/api/v1/codex/backups").await, json!([]));

    // A snapshot now: the box's two screens, two looks, show settings and its name; nothing
    // else.
    let path = "/api/v1/codex/backups/snapshots";
    let first = ok(&call(addr, "POST", path, None, None).await, 201, "snapshot");
    assert_eq!(first["created"], true);
    assert_eq!(first["snapshot"]["reason"], "manual");
    assert_eq!(first["snapshot"]["items"], 6, "{first}");
    assert_eq!(first["snapshot"]["by"]["client"], "server machine");
    // Nothing changed: no new snapshot, the newest is answered.
    let again = ok(&call(addr, "POST", path, None, None).await, 200, "again");
    assert_eq!(again["created"], false);
    assert_eq!(again["snapshot"]["id"], first["snapshot"]["id"]);

    let own = own_set(addr).await;
    assert_eq!(own["set"]["product"], "vesper");
    assert_eq!(own["set"]["machine"], server_id.as_str());
    assert_eq!(own["snapshots"], 1);
    assert_eq!(own["latest"]["id"], first["snapshot"]["id"]);
    assert!(own["set"].get("fingerprint").is_none(), "{own}");
    let set = own["set"]["id"].as_str().unwrap().to_string();
    let snap1 = first["snapshot"]["id"].as_str().unwrap().to_string();

    // Its contents: the documents as they were, never identities, credentials or content.
    let c = get_json(
        addr,
        &format!("/api/v1/codex/backups/{set}/snapshots/{snap1}"),
    )
    .await;
    let keys: Vec<&str> = c["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["item"]["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys.len(), 6, "{keys:?}");
    assert!(keys.contains(&"machine/server.json"));
    assert_eq!(
        keys.iter()
            .filter(|k| k.starts_with("portable/screen/"))
            .count(),
        2
    );
    assert_eq!(
        keys.iter()
            .filter(|k| k.starts_with("portable/look/"))
            .count(),
        2
    );
    let screen_key = format!("portable/screen/{audience}.json");
    let item = c["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["item"]["key"] == screen_key.as_str())
        .unwrap();
    assert_eq!(item["content"]["name"], "Audience");
    assert_eq!(item["item"]["kind"], "screen");
    assert_eq!(item["item"]["name"], "Audience");
    assert!(item["item"]["schemaVersion"].as_u64().unwrap() >= 1);
    assert_eq!(item["item"]["sha256"].as_str().unwrap().len(), 64);
    let blob = fx
        .dir
        .join("codex/backups/blobs")
        .join(item["item"]["sha256"].as_str().unwrap());
    assert!(blob.is_file());

    // Announced: refetch the backups.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ws.events.iter().any(|m| {
        m["type"] == "codexChanged" && m["parts"].as_array().unwrap().contains(&json!("backups"))
    }) {
        assert!(
            Instant::now() < deadline,
            "no codexChanged backups: {:?}",
            ws.events
        );
        let m = ws.next_any().await;
        ws.absorb(&m);
    }

    // Settings change: the screen renamed, an output added.
    rename_screen(addr, &audience, "Front").await;
    let out = ok(
        &call(
            addr,
            "POST",
            "/api/v1/outputs",
            None,
            Some(json!({"screen": audience, "transport": {"type": "idn", "sourceName": "Backup Feed"}})),
        )
        .await,
        201,
        "output",
    );
    let out_key = format!("machine/outputBinding/{}.json", out["id"].as_str().unwrap());

    let p = get_json(
        addr,
        &format!("/api/v1/codex/backups/{set}/snapshots/{snap1}/preview"),
    )
    .await;
    assert_eq!(
        p["summary"], "1 screen changed; 1 output binding removed",
        "{p}"
    );
    assert_eq!(p["unchanged"], 5);
    assert_eq!(p["restart"], json!([]));
    assert!(
        p.get("currentAsOf").is_none(),
        "this box: compared with now"
    );
    let screen = p["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["key"] == screen_key.as_str())
        .unwrap();
    assert_eq!(screen["change"], "changed");
    assert_eq!(screen["name"], "Audience");
    assert_eq!(screen["id"], audience.as_str());

    // A chosen item, unknown keys refused.
    let restore = format!("/api/v1/codex/backups/{set}/snapshots/{snap1}/restore");
    let r = call(
        addr,
        "POST",
        &restore,
        None,
        Some(json!({"items": ["portable/look/nope.json"]})),
    )
    .await;
    assert_eq!(ok(&r, 400, "unknown item")["error"]["field"], "items");
    let r = ok(
        &call(
            addr,
            "POST",
            &restore,
            None,
            Some(json!({"items": [screen_key]})),
        )
        .await,
        200,
        "restore the screen",
    );
    assert_eq!(r["summary"], "1 screen changed");
    assert_eq!(r["restart"], json!([]));
    assert_eq!(r["beforeRestore"]["reason"], "beforeRestore");
    let undo = r["beforeRestore"]["id"].as_str().unwrap().to_string();
    // Applied at once, no restart: the live screens and the API follow.
    assert_eq!(screen_name(addr, &audience).await, "Audience");
    let outs = get_json(addr, "/api/v1/outputs").await;
    assert!(
        outs.as_array()
            .unwrap()
            .iter()
            .any(|o| o["id"] == out["id"])
    );

    // Everything: the output the snapshot doesn't have goes.
    let r = ok(
        &call(addr, "POST", &restore, None, None).await,
        200,
        "restore all",
    );
    assert_eq!(r["summary"], "1 output binding removed");
    assert!(!fx.dir.join(&out_key).exists());
    let outs = get_json(addr, "/api/v1/outputs").await;
    assert!(
        !outs
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["id"] == out["id"])
    );

    // Undo the first restore: its beforeRestore snapshot brings back the rename and the output.
    let r = ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/codex/backups/{set}/snapshots/{undo}/restore"),
            None,
            None,
        )
        .await,
        200,
        "undo",
    );
    assert_eq!(
        r["summary"], "1 screen changed; 1 output binding added",
        "{r}"
    );
    assert_eq!(screen_name(addr, &audience).await, "Front");
    assert!(fx.dir.join(&out_key).is_file());

    // Newest first, each with its reason.
    let list = snapshots(addr, &set).await;
    let reasons: Vec<&str> = list.iter().map(|s| s["reason"].as_str().unwrap()).collect();
    assert_eq!(
        reasons,
        vec!["beforeRestore", "beforeRestore", "beforeRestore", "manual"]
    );
    let seqs: Vec<u64> = list.iter().map(|s| s["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, vec![4, 3, 2, 1]);

    // The server's name is restored too (after a restart); its ID never.
    let server_json = fx.dir.join("machine/server.json");
    let mut identity: Value =
        serde_json::from_slice(&std::fs::read(&server_json).unwrap()).unwrap();
    let old_name = identity["name"].clone();
    identity["name"] = json!("Renamed Box");
    std::fs::write(&server_json, serde_json::to_vec(&identity).unwrap()).unwrap();
    let p = get_json(
        addr,
        &format!("/api/v1/codex/backups/{set}/snapshots/{snap1}/preview"),
    )
    .await;
    assert_eq!(p["restart"], json!(["server name"]), "{p}");
    let r = ok(
        &call(addr, "POST", &restore, None, None).await,
        200,
        "restore the name",
    );
    assert_eq!(r["restart"], json!(["server name"]));
    let after: Value = serde_json::from_slice(&std::fs::read(&server_json).unwrap()).unwrap();
    assert_eq!(after["name"], old_name);
    assert_eq!(after["id"], identity["id"], "the ID stays");

    // Unknown sets and snapshots.
    let r = call(
        addr,
        "GET",
        &format!("/api/v1/codex/backups/{}/snapshots", uuid::Uuid::new_v4()),
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 404);
    let r = call(
        addr,
        "GET",
        "/api/v1/codex/backups/nope/snapshots",
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 400);
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daily_and_change_snapshots_and_retention() {
    let fx = fixture("codex-backups-auto");
    let delay = Duration::from_millis(800);
    let server = start_with(
        &fx,
        backups(
            Duration::from_millis(100),
            delay,
            Retention {
                last: 3,
                daily: 0,
                weekly: 0,
            },
        ),
        SiteConfig::off(),
    )
    .await;
    let addr = server.addr;
    let audience = fx.audience.id.0.to_string();

    // The first look takes the daily snapshot (there is none yet).
    let deadline = Instant::now() + Duration::from_secs(20);
    let own = loop {
        let own = own_set(addr).await;
        if !own.is_null() && !own["latest"].is_null() {
            break own;
        }
        assert!(Instant::now() < deadline, "no scheduled snapshot");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(own["latest"]["reason"], "scheduled");
    let set = own["set"]["id"].as_str().unwrap().to_string();

    // Two changes in quick succession: one snapshot, a while after the last.
    rename_screen(addr, &audience, "One").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    rename_screen(addr, &audience, "Two").await;
    let changed_at = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(20);
    let list = loop {
        let list = snapshots(addr, &set).await;
        if list.len() == 2 {
            break list;
        }
        assert!(list.len() < 3, "{list:?}");
        assert!(Instant::now() < deadline, "no change snapshot");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        changed_at.elapsed() >= delay - Duration::from_millis(150),
        "taken too early"
    );
    assert_eq!(list[0]["reason"], "change");
    let c = get_json(
        addr,
        &format!(
            "/api/v1/codex/backups/{set}/snapshots/{}",
            list[0]["id"].as_str().unwrap()
        ),
    )
    .await;
    let names: Vec<&str> = c["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["content"]["name"].as_str())
        .collect();
    assert!(names.contains(&"Two"), "{names:?}");
    // Nothing more while nothing changes.
    tokio::time::sleep(delay * 2).await;
    assert_eq!(snapshots(addr, &set).await.len(), 2);

    // Retention: the last 3 stay; blobs only they name remain.
    for n in 0..4 {
        rename_screen(addr, &audience, &format!("Name {n}")).await;
        ok(
            &call(addr, "POST", "/api/v1/codex/backups/snapshots", None, None).await,
            201,
            "snapshot",
        );
    }
    let list = snapshots(addr, &set).await;
    assert_eq!(list.len(), 3, "{list:?}");
    let mut used = std::collections::HashSet::new();
    for s in &list {
        let c = get_json(
            addr,
            &format!(
                "/api/v1/codex/backups/{set}/snapshots/{}",
                s["id"].as_str().unwrap()
            ),
        )
        .await;
        for i in c["items"].as_array().unwrap() {
            used.insert(i["item"]["sha256"].as_str().unwrap().to_string());
        }
    }
    let blobs: std::collections::HashSet<String> =
        std::fs::read_dir(fx.dir.join("codex/backups/blobs"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with('.'))
            .collect();
    assert_eq!(blobs, used);
    server.shutdown().await.unwrap();
}

/// Pairs a client with the server's pairing code; its token.
async fn pair(server: &RunningServer, name: &str) -> String {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oriel_preferences_and_access() {
    let fx = fixture("codex-backups-oriel");
    let server = start_with(&fx, quiet(), SiteConfig::loopback()).await;
    let addr = server.addr;
    let a = pair(&server, "Booth laptop").await;
    let b = pair(&server, "Stage tablet").await;
    let prefs = "/api/v1/codex/backups/oriel";

    assert_eq!(
        call(addr, "GET", prefs, Some(&a), None).await.status,
        404,
        "nothing yet"
    );
    let v1 = json!({"schemaVersion": 3, "preferences": {"theme": "dark", "grid": 4}});
    let first = ok(
        &call(addr, "PUT", prefs, Some(&a), Some(v1.clone())).await,
        201,
        "upload",
    );
    assert_eq!(first["snapshot"]["reason"], "upload");
    let same = ok(
        &call(addr, "PUT", prefs, Some(&a), Some(v1.clone())).await,
        200,
        "again",
    );
    assert_eq!(same["created"], false);
    let got = ok(
        &call(addr, "GET", prefs, Some(&a), None).await,
        200,
        "fetch",
    );
    assert_eq!(got["schemaVersion"], 3);
    assert_eq!(got["preferences"], v1["preferences"]);
    // Each client has its own.
    assert_eq!(call(addr, "GET", prefs, Some(&b), None).await.status, 404);
    ok(
        &call(
            addr,
            "PUT",
            prefs,
            Some(&b),
            Some(json!({"schemaVersion": 3, "preferences": {"grid": 6}})),
        )
        .await,
        201,
        "b uploads",
    );
    let v2 = json!({"schemaVersion": 4, "preferences": {"theme": "light"}});
    ok(
        &call(addr, "PUT", prefs, Some(&a), Some(v2.clone())).await,
        201,
        "a again",
    );
    let got = ok(
        &call(addr, "GET", prefs, Some(&a), None).await,
        200,
        "newest",
    );
    assert_eq!(got["preferences"], v2["preferences"]);
    assert_eq!(got["schemaVersion"], 4);
    let older = ok(
        &call(
            addr,
            "GET",
            &format!(
                "{prefs}?snapshot={}",
                first["snapshot"]["id"].as_str().unwrap()
            ),
            Some(&a),
            None,
        )
        .await,
        200,
        "older",
    );
    assert_eq!(older["preferences"], v1["preferences"]);
    // Not an object, too big.
    let r = call(
        addr,
        "PUT",
        prefs,
        Some(&a),
        Some(json!({"schemaVersion": 1, "preferences": [1]})),
    )
    .await;
    assert_eq!(ok(&r, 400, "array")["error"]["field"], "preferences");
    let big = "x".repeat((1 << 20) + 1);
    let r = call(
        addr,
        "PUT",
        prefs,
        Some(&a),
        Some(json!({"schemaVersion": 1, "preferences": {"big": big}})),
    )
    .await;
    assert_eq!(ok(&r, 400, "too big")["error"]["field"], "preferences");

    // Lists: every paired client sees every set; `yours` marks its own.
    let sets = ok(
        &call(addr, "GET", "/api/v1/codex/backups", Some(&b), None).await,
        200,
        "list",
    );
    let sets = sets.as_array().unwrap();
    assert_eq!(sets.len(), 2);
    assert!(
        sets.iter()
            .all(|s| s["set"]["product"] == "oriel" && s["thisBox"] == false)
    );
    assert_eq!(sets.iter().filter(|s| s["yours"] == true).count(), 1);
    let mine = sets.iter().find(|s| s["yours"] == true).unwrap();
    assert!(
        mine["set"]["name"]
            .as_str()
            .unwrap()
            .contains("Stage tablet"),
        "{mine}"
    );
    let a_set = sets.iter().find(|s| s["yours"] == false).unwrap();
    let (set, snap) = (
        a_set["set"]["id"].as_str().unwrap().to_string(),
        a_set["latest"]["id"].as_str().unwrap().to_string(),
    );
    // Oriel's sets aren't restored by the server.
    let r = call(
        addr,
        "GET",
        &format!("/api/v1/codex/backups/{set}/snapshots/{snap}/preview"),
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 409);

    // Access `pin`: a locked client reads the lists (and another computer's Oriel
    // preferences), not a box's contents, and can't restore or take snapshots; unlocked it
    // can.
    let r = call(
        addr,
        "PUT",
        "/api/v1/site/settings",
        None,
        Some(json!({"enabled": true, "joinMode": "code_or_approval", "access": "pin", "pin": "2468"})),
    )
    .await;
    ok(&r, 200, "pin");
    let snaps = format!("/api/v1/codex/backups/{set}/snapshots");
    ok(
        &call(addr, "GET", &snaps, Some(&b), None).await,
        200,
        "list snapshots",
    );
    for (method, path) in [
        ("GET", format!("{snaps}/{snap}")),
        ("GET", format!("{snaps}/{snap}/preview")),
        ("POST", format!("{snaps}/{snap}/restore")),
        ("POST", "/api/v1/codex/backups/snapshots".to_string()),
    ] {
        let r = call(addr, method, &path, Some(&b), None).await;
        assert_eq!(
            r.status,
            403,
            "{method} {path}: {}",
            String::from_utf8_lossy(&r.body)
        );
    }
    // Its own preferences stay its own business.
    ok(
        &call(addr, "GET", prefs, Some(&b), None).await,
        200,
        "own prefs",
    );
    // Taking over another computer's preferences needs no admin; a set that isn't Oriel's
    // (or doesn't exist) is 404.
    let taken = ok(
        &call(addr, "GET", &format!("{prefs}?set={set}"), Some(&b), None).await,
        200,
        "take over a's preferences",
    );
    assert_eq!(taken["preferences"], v2["preferences"]);
    assert_eq!(taken["snapshot"]["id"], snap.as_str());
    let older_of_a = ok(
        &call(
            addr,
            "GET",
            &format!(
                "{prefs}?set={set}&snapshot={}",
                first["snapshot"]["id"].as_str().unwrap()
            ),
            Some(&b),
            None,
        )
        .await,
        200,
        "an older one of a's",
    );
    assert_eq!(older_of_a["preferences"], v1["preferences"]);
    let r = call(
        addr,
        "GET",
        &format!("{prefs}?set={}", uuid::Uuid::new_v4()),
        Some(&b),
        None,
    )
    .await;
    assert_eq!(r.status, 404);
    // Reads need pairing only, locked or not, and aren't audited.
    for path in [
        "/api/v1/codex/export/settings",
        "/api/v1/codex/export/scheduled",
        "/api/v1/codex/backups/restores",
    ] {
        ok(&call(addr, "GET", path, Some(&b), None).await, 200, path);
    }
    let audit = get_json(addr, "/api/v1/site/audit?n=200").await;
    assert!(
        !audit.as_array().unwrap().iter().any(|e| {
            let a = e["action"].as_str().unwrap_or("");
            [
                "codex.readExportSettings",
                "codex.readExports",
                "codex.readRestores",
            ]
            .contains(&a)
        }),
        "no audit rows for reads: {audit}"
    );
    ok(
        &call(addr, "GET", &format!("{snaps}/{snap}"), None, None).await,
        200,
        "the server machine reads contents",
    );
    let c = ok(
        &call(addr, "GET", &format!("{snaps}/{snap}"), None, None).await,
        200,
        "contents",
    );
    assert_eq!(c["items"][0]["item"]["key"], "preferences");
    assert_eq!(c["items"][0]["item"]["kind"], "orielPreferences");
    assert_eq!(c["items"][0]["content"], v2["preferences"]);
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/unlock",
            Some(&b),
            Some(json!({"pin": "2468"})),
        )
        .await,
        200,
        "unlock",
    );
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/codex/backups/snapshots",
            Some(&b),
            None,
        )
        .await,
        201,
        "unlocked: a snapshot of the box",
    );
    let audit = get_json(addr, "/api/v1/site/audit?n=200").await;
    let backup_now: Vec<&Value> = audit
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "codex.backupNow")
        .collect();
    assert!(backup_now.iter().any(|e| e["error"].is_string()), "{audit}");
    assert!(backup_now.iter().any(|e| e["error"].is_null()), "{audit}");
    server.shutdown().await.unwrap();
}

/// Waits (off the async threads) until the machine's trust state matches.
async fn until(m: &Arc<Enrollment>, what: &str, pred: fn(&TrustState) -> bool) {
    let m = m.clone();
    let what = what.to_string();
    tokio::task::spawn_blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let s = m.state();
            if pred(&s) {
                return;
            }
            assert!(Instant::now() < deadline, "{what}: stuck in {s:?}");
            let _ = m.recv(Duration::from_millis(100));
        }
    })
    .await
    .unwrap()
}

fn item(key: &str, kind: &str, name: &str, doc: Value) -> OfferedItem {
    OfferedItem::new(
        key.into(),
        kind.into(),
        None,
        Some(name.into()),
        2,
        serde_json::to_vec_pretty(&doc).unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn show_box_offers_and_replacement_takeover() {
    let fx = fixture("codex-backups-replace");
    let server = start_with(&fx, quiet(), SiteConfig::loopback()).await;
    let addr = server.addr;
    let sh = server.shared.clone();

    // BOX-1 joins the site by code.
    let ds = get_json(addr, "/api/v1/site").await["discoveryPort"]
        .as_u64()
        .unwrap() as u16;
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    )["code"]
        .as_str()
        .unwrap()
        .to_string();
    let one = Enrollment::start(EnrollmentConfig {
        discovery_server: format!("127.0.0.1:{ds}"),
        state_dir: fx.dir.join("machines").join("BOX-1"),
        name: "BOX-1 (Test)".into(),
        role: DeviceRole::App,
        passphrase: None,
        admin_code: None,
    })
    .unwrap();
    one.join_with_code(&code).unwrap();
    until(&one, "BOX-1 joins", |s| {
        matches!(s, TrustState::Enrolled { .. })
    })
    .await;

    // BOX-1 offers its settings (what the box-to-Codex link will carry).
    let old_id = uuid::Uuid::new_v4();
    let screen_id = uuid::Uuid::new_v4();
    let screen_key = format!("portable/screen/{screen_id}.json");
    let offer = SnapshotOffer {
        product: "vesper".into(),
        machine: old_id.to_string(),
        name: "BOX-1".into(),
        fingerprint: Some(one.fingerprint()),
        reason: model::SnapshotReason::Change,
        by: model::ChangeBy {
            client: "server machine".into(),
            client_id: None,
            machine: old_id.to_string(),
        },
        version: "test".into(),
        items: vec![
            item(
                &screen_key,
                "screen",
                "Old Stage",
                json!({"schemaVersion": 2, "id": screen_id, "name": "Old Stage"}),
            ),
            item(
                "machine/server.json",
                "server",
                "BOX-1",
                json!({"id": old_id, "name": "BOX-1"}),
            ),
        ],
    };
    let taken = sh
        .backups
        .accept(&sh.codex_server(), offer.clone())
        .await
        .unwrap();
    assert!(taken.created);
    let again = sh
        .backups
        .accept(&sh.codex_server(), offer.clone())
        .await
        .unwrap();
    assert!(!again.created);
    // A damaged offer is refused.
    let mut bad = offer.clone();
    bad.items[0].bytes.push(b' ');
    assert_eq!(
        sh.backups
            .accept(&sh.codex_server(), bad)
            .await
            .unwrap_err()
            .field
            .as_deref(),
        Some("items")
    );

    let sets = get_json(addr, "/api/v1/codex/backups").await;
    let box1 = sets
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["set"]["machine"] == old_id.to_string().as_str())
        .unwrap()
        .clone();
    assert_eq!(box1["thisBox"], false);
    assert_eq!(box1["set"]["fingerprint"], one.fingerprint().as_str());
    let set: model::BackupSetId = box1["set"]["id"].as_str().unwrap().parse().unwrap();
    let snap: model::SnapshotId = box1["latest"]["id"].as_str().unwrap().parse().unwrap();

    // Its preview compares with its own newest snapshot here; restoring it is queued for
    // BOX-1, which takes it when it asks the Codex (this test's BOX-1 never does).
    let p = get_json(
        addr,
        &format!("/api/v1/codex/backups/{set}/snapshots/{snap}/preview"),
    )
    .await;
    assert_eq!(p["summary"], "nothing would change", "{p}");
    assert_eq!(p["currentAsOf"], box1["latest"]["takenAt"]);
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/backups/{set}/snapshots/{snap}/restore"),
        None,
        None,
    )
    .await;
    let q = ok(&r, 202, "a restore of BOX-1's settings");
    assert_eq!(q["state"], "queued", "{q}");
    assert_eq!(q["target"], old_id.to_string().as_str(), "{q}");
    // Onto a box that never offered its settings: nowhere to send it.
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/backups/{set}/snapshots/{snap}/restore"),
        None,
        Some(json!({"target": uuid::Uuid::new_v4().to_string()})),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));

    // A replacement box (this one) restoring BOX-1's settings.
    let own_id = sh.auth.identity().id;
    let current: Vec<model::SnapshotItem> = vesper::collect(&fx.dir)
        .unwrap()
        .into_iter()
        .map(|i| i.item)
        .collect();
    let target = || RestoreTarget {
        server_id: own_id.to_string(),
        current: current.clone(),
    };
    let adopt = protocol::RestoreRequest {
        adopt_server_id: true,
        ..Default::default()
    };
    // Taking over its server ID waits until the site removed BOX-1.
    let e = sh
        .backups
        .prepare_restore(&sh.codex_server(), set, snap, &adopt, target())
        .await
        .unwrap_err();
    assert_eq!(e.code, protocol::ErrorCode::Conflict);
    assert!(
        e.message.contains("still a machine of the site"),
        "{}",
        e.message
    );
    // Without taking it over: fine.
    let plain = sh
        .backups
        .prepare_restore(
            &sh.codex_server(),
            set,
            snap,
            &protocol::RestoreRequest::default(),
            target(),
        )
        .await
        .unwrap();
    assert!(plain.package.adopt_server_id.is_none());
    let summary = vesper_server::codex::backups::preview::summary(&plain.changes);
    assert_eq!(
        summary,
        "1 server name changed; 1 screen added; 2 looks, 2 screens, 1 show settings removed",
        "{:?}",
        plain.changes
    );
    // Oriel's "Onto: this box": the preview compares with this box (not with BOX-1's own
    // newest snapshot, which would say nothing changes)...
    let p = get_json(
        addr,
        &format!("/api/v1/codex/backups/{set}/snapshots/{snap}/preview?target={own_id}"),
    )
    .await;
    assert_eq!(p["summary"], summary.as_str(), "{p}");
    assert!(p.get("currentAsOf").is_none_or(Value::is_null), "{p}");
    // ...onto a box that never offered its settings: nothing to compare with.
    let r = call(
        addr,
        "GET",
        &format!(
            "/api/v1/codex/backups/{set}/snapshots/{snap}/preview?target={}",
            uuid::Uuid::new_v4()
        ),
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    // ...and the restore onto this box runs at once, its undo in this box's own set.
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/backups/{set}/snapshots/{snap}/restore"),
        None,
        Some(json!({"target": own_id.to_string(), "items": [screen_key]})),
    )
    .await;
    let done = ok(&r, 200, "BOX-1's screen restored onto this box");
    assert_eq!(done["summary"], "1 screen added", "{done}");
    assert_ne!(
        done["beforeRestore"]["set"],
        set.to_string().as_str(),
        "{done}"
    );
    assert!(fx.dir.join(&screen_key).is_file());

    let fp = one.fingerprint();
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/site/devices/{fp}"),
            None,
            None,
        )
        .await,
        204,
        "remove BOX-1",
    );
    until(&one, "BOX-1 revoked", |s| matches!(s, TrustState::Revoked)).await;
    let prepared = sh
        .backups
        .prepare_restore(&sh.codex_server(), set, snap, &adopt, target())
        .await
        .unwrap();
    assert_eq!(prepared.package.adopt_server_id, Some(old_id));

    // The replacement box applies it (here into a scratch data folder): BOX-1's settings and
    // server ID, its own device identity untouched.
    let scratch = fx.dir.join("replacement");
    std::fs::create_dir_all(scratch.join("machine/site-member")).unwrap();
    std::fs::write(scratch.join("machine/site-member/identity.key"), b"own").unwrap();
    let new_id = uuid::Uuid::new_v4();
    std::fs::write(
        scratch.join("machine/server.json"),
        serde_json::to_vec(&json!({"id": new_id, "name": "NEW-BOX"})).unwrap(),
    )
    .unwrap();
    let applied = vesper::apply(&scratch, &prepared.package).unwrap();
    assert_eq!(
        applied.restart,
        vec!["server name".to_string(), "server ID".to_string()]
    );
    let identity: Value =
        serde_json::from_slice(&std::fs::read(scratch.join("machine/server.json")).unwrap())
            .unwrap();
    assert_eq!(identity, json!({"id": old_id, "name": "BOX-1"}));
    assert!(scratch.join(&screen_key).is_file());
    assert_eq!(
        std::fs::read(scratch.join("machine/site-member/identity.key")).unwrap(),
        b"own"
    );

    // A box that wasn't a machine of the site: its ID can't be taken over.
    let lone = uuid::Uuid::new_v4();
    let offer = SnapshotOffer {
        machine: lone.to_string(),
        name: "BOX-2".into(),
        fingerprint: None,
        items: vec![item(
            "machine/server.json",
            "server",
            "BOX-2",
            json!({"id": lone, "name": "BOX-2"}),
        )],
        ..offer
    };
    let t = sh.backups.accept(&sh.codex_server(), offer).await.unwrap();
    let e = sh
        .backups
        .prepare_restore(
            &sh.codex_server(),
            t.snapshot.set,
            t.snapshot.id,
            &adopt,
            target(),
        )
        .await
        .unwrap_err();
    assert!(
        e.message.contains("was not a machine of the site"),
        "{}",
        e.message
    );
    server.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_daily_snapshot_that_failed_is_tried_again() {
    use std::os::unix::fs::PermissionsExt as _;
    let fx = fixture("codex-backups-retry");
    let server = start_with(&fx, quiet(), SiteConfig::off()).await;
    let addr = server.addr;
    let sh = server.shared.clone();
    // The backups' folder can't be written (a full or failing disk): the first daily
    // snapshot fails.
    let backups = fx.dir.join("codex/backups");
    for d in ["sets", "snapshots", "blobs"] {
        std::fs::create_dir_all(backups.join(d)).unwrap();
    }
    let mode = |m: u32| {
        for d in ["sets", "snapshots", "blobs", ""] {
            std::fs::set_permissions(backups.join(d), std::fs::Permissions::from_mode(m)).unwrap();
        }
    };
    mode(0o555);
    if std::fs::write(backups.join("blobs/probe"), b"").is_ok() {
        // (Run as root: permissions don't stop it; nothing to show here.)
        mode(0o755);
        server.shutdown().await.unwrap();
        return;
    }
    let now = std::time::SystemTime::now();
    sh.backups.tick(&sh.codex_server(), now).await;
    mode(0o755);
    assert!(own_set(addr).await.is_null(), "nothing taken");
    // The next look (a minute on) takes it, not a day later.
    sh.backups
        .tick(&sh.codex_server(), now + Duration::from_secs(60))
        .await;
    let own = own_set(addr).await;
    assert_eq!(own["latest"]["reason"], "scheduled", "{own}");
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}
