//! The Codex's encrypted export and import (M13), synthetic fixtures only:
//!
//! - a manual export with its own passphrase: an IDN backup of kind `vesper-codex` holding the
//!   collections and settings backups, never media; passphrases too short or wrong refused;
//! - the stored passphrase of scheduled exports never in an answer, a settings snapshot or an
//!   export;
//! - the preview against this Codex (same, older, new), the replace path (the current copy in
//!   the collection trash as a collection of its own: the undo; the change log and events),
//!   the active collection not replaceable;
//! - a fresh server importing it: collections, search and settings backups back; media missing
//!   until the same bytes are in its library, then relinked by SHA-256;
//! - scheduled exports: none without a stored passphrase (the status says why), then one per
//!   tick knob into the folder, pruned to one a day;
//! - the `codexExport` capability, and `mdns` only when the server is announced.

use super::media::{noise_png, probed, sha_hex, upload};
use crate::common::*;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use vesper_server::codex::CodexOptions;
use vesper_server::codex::backups::BackupOptions;
use vesper_server::codex::export::ExportOptions;
use vesper_server::site::SiteConfig;
use vesper_server::{RunningServer, ServerConfig, start};

const STORED: &str = "stored nightly secret 1";
const MANUAL: &str = "manual export secret 2";

fn options(export: ExportOptions) -> CodexOptions {
    CodexOptions {
        backups: BackupOptions {
            poll: Duration::from_secs(3600),
            change_delay: Duration::from_secs(3600),
            ..BackupOptions::default()
        },
        export,
        ..CodexOptions::default()
    }
}

async fn start_with(fx: &Fixture, codex: CodexOptions) -> RunningServer {
    start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: true,
        mdns: false,
        site: SiteConfig::off(),
        codex,
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts")
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

async fn post(addr: SocketAddr, path: &str, body: Value) -> Resp {
    http(addr, "POST", path, &[], Some(body)).await
}

/// A manual export: the file.
pub(crate) async fn export(addr: SocketAddr, passphrase: &str) -> Vec<u8> {
    export_with(addr, json!({"passphrase": passphrase})).await
}

/// A manual export asked with `body` (`CreateExport`): the file.
pub(crate) async fn export_with(addr: SocketAddr, body: Value) -> Vec<u8> {
    let r = post(addr, "/api/v1/codex/export", body).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.header("content-type"), Some("application/octet-stream"));
    assert!(
        r.header("content-disposition")
            .unwrap()
            .contains(".vcodex\""),
        "{:?}",
        r.headers
    );
    assert_eq!(r.header("x-codex-media-included"), Some("false"));
    assert!(r.header("x-codex-export-note").unwrap().contains("media"));
    r.body
}

/// Uploads an export for import: its ID.
pub(crate) async fn upload_import(addr: SocketAddr, file: &[u8]) -> String {
    let r = http_bytes(addr, "POST", "/api/v1/codex/export/imports", &[], file).await;
    ok(&r, 201, "import upload")["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// The export opened with IDN's library: its files.
fn open(file: &[u8], passphrase: &str) -> Vec<(String, Vec<u8>)> {
    let (m, files) = idn_open_protocol::backup::open(file, passphrase).expect("opens");
    assert_eq!(m.kind, "vesper-codex");
    files
}

async fn wait_search(addr: SocketAddr, q: &str, collection: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let r = get_json(
            addr,
            &format!("/api/v1/codex/search?q={q}&collection={collection}"),
        )
        .await;
        if r["total"].as_u64().unwrap_or(0) > 0 {
            return r;
        }
        assert!(Instant::now() < deadline, "no hit for {q}: {r}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn counts(summary: &Value) -> u64 {
    summary["counts"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_u64().unwrap())
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn export_import_round_trip_replace_and_relink() {
    let fa = fixture("cx-export-a");
    let a = start_with(&fa, options(ExportOptions::default())).await;
    let addr = a.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;

    let info = get_json(addr, "/api/v1/server").await;
    let caps = info["capabilities"].as_array().unwrap();
    assert!(caps.contains(&json!("codexExport")), "{info}");
    assert!(!caps.contains(&json!("mdns")), "not announced: {info}");

    // A's content: the fixture's collection with a media file in the library, a second
    // collection, a settings snapshot.
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let png = noise_png(48, 32, 7);
    let up = upload(addr, &main, "Sunrise.png", &png).await;
    let asset = up["asset"]["id"].as_str().unwrap().to_string();
    // (Its probe is written into it: before the export, so main stays as exported.)
    probed(addr, &asset).await;
    let youth = ok(
        &post(addr, "/api/v1/codex/collections", json!({"name": "Youth"})).await,
        201,
        "youth",
    )["collection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &post(addr, "/api/v1/codex/backups/snapshots", json!({})).await,
        201,
        "snapshot",
    );

    // The stored passphrase: set, never answered back.
    let r = http(
        addr,
        "PUT",
        "/api/v1/codex/export/passphrase",
        &[],
        Some(json!({"passphrase": "short"})),
    )
    .await;
    assert_eq!(ok(&r, 400, "short stored")["error"]["field"], "passphrase");
    let r = http(
        addr,
        "PUT",
        "/api/v1/codex/export/passphrase",
        &[],
        Some(json!({"passphrase": STORED})),
    )
    .await;
    let st = ok(&r, 200, "stored");
    assert_eq!(st["passphraseSet"], true);
    assert!(!contains(&r.body, STORED));
    let r = http(addr, "GET", "/api/v1/codex/export/settings", &[], None).await;
    let st = ok(&r, 200, "settings");
    assert!(!contains(&r.body, STORED));
    assert_eq!(st["mediaIncluded"], false);
    assert!(
        st["note"]
            .as_str()
            .unwrap()
            .contains("media folder needs its own backup")
    );
    assert_eq!(st["settings"]["keepNightly"], 7);
    assert_eq!(st["settings"]["keepWeekly"], 4);
    assert_eq!(st["settings"]["time"], "02:00");
    assert!(
        st["folder"].as_str().unwrap().ends_with("codex/exports"),
        "{st}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(fa.dir.join("machine/codex-export-passphrase"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // Not in a settings snapshot either.
    let snap = ok(
        &post(addr, "/api/v1/codex/backups/snapshots", json!({})).await,
        200,
        "snapshot again (unchanged)",
    );
    assert_eq!(
        snap["created"], false,
        "the passphrase isn't a setting: {snap}"
    );
    let set = snap["snapshot"]["set"].as_str().unwrap().to_string();
    let sid = snap["snapshot"]["id"].as_str().unwrap().to_string();
    let r = http(
        addr,
        "GET",
        &format!("/api/v1/codex/backups/{set}/snapshots/{sid}"),
        &[],
        None,
    )
    .await;
    assert!(!contains(
        &ok(&r, 200, "contents").to_string().into_bytes(),
        STORED
    ));

    // A manual export: its own passphrase (at least 8 characters).
    let r = post(
        addr,
        "/api/v1/codex/export",
        json!({"passphrase": "seven77"}),
    )
    .await;
    assert_eq!(ok(&r, 400, "short")["error"]["field"], "passphrase");
    // The site's identity only on the site's server (this box doesn't run its Site Manager).
    let r = post(
        addr,
        "/api/v1/codex/export",
        json!({"passphrase": MANUAL, "includeSite": true}),
    )
    .await;
    assert!(
        ok(&r, 409, "includeSite without a site")["error"]["message"]
            .as_str()
            .unwrap()
            .contains("site's server"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let file = export(addr, MANUAL).await;
    assert!(!contains(&file, MANUAL) && !contains(&file, STORED));
    assert!(!contains(&file, "Test Song"), "encrypted");
    let files = open(&file, MANUAL);
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"export.json"), "{names:?}");
    assert!(names.contains(&"codex/codex.json"));
    assert!(
        names
            .iter()
            .any(|n| n.starts_with(&format!("codex/collections/{main}/portable/presentation/")))
    );
    assert!(names.contains(&format!("codex/collections/{youth}/collection.json").as_str()));
    assert!(names.iter().any(|n| n.starts_with("codex/backups/sets/")));
    assert!(names.iter().any(|n| n.starts_with("codex/backups/blobs/")));
    assert!(
        !names.iter().any(|n| n.starts_with("site/")),
        "no site here: {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with("codex/library") || n.starts_with("machine")),
        "{names:?}"
    );
    for (name, bytes) in &files {
        assert!(
            bytes.as_slice() != png.as_slice(),
            "media in the export: {name}"
        );
        assert!(
            !contains(bytes, STORED) && !contains(bytes, MANUAL),
            "{name}"
        );
    }
    let manifest: Value =
        serde_json::from_slice(&files.iter().find(|(n, _)| n == "export.json").unwrap().1).unwrap();
    assert_eq!(manifest["format"], "vesper-codex");
    assert_eq!(manifest["formatVersion"], 1);
    assert_eq!(manifest["mediaIncluded"], false);
    assert_eq!(manifest["site"], false);
    assert_eq!(manifest["collections"].as_array().unwrap().len(), 2);
    assert!(manifest["backupSets"].as_u64().unwrap() >= 1);

    // After the export, Youth gets a library (assigned for a moment).
    let assign = |c: &str| json!({"collection": c, "confirm": true});
    ok(
        &http(
            addr,
            "PUT",
            "/api/v1/codex/assignment",
            &[],
            Some(assign(&youth)),
        )
        .await,
        200,
        "assign youth",
    );
    ok(
        &post(addr, "/api/v1/libraries", json!({"name": "Youth Songs"})).await,
        201,
        "library",
    );
    ok(
        &http(
            addr,
            "PUT",
            "/api/v1/codex/assignment",
            &[],
            Some(assign(&main)),
        )
        .await,
        200,
        "assign main",
    );

    // The preview on A: main the same, Youth older than here, A's backup set present.
    let id = upload_import(addr, &file).await;
    let preview = format!("/api/v1/codex/export/imports/{id}/preview");
    let r = post(addr, &preview, json!({"passphrase": "wrong passphrase"})).await;
    let e = ok(&r, 400, "wrong passphrase");
    assert_eq!(e["error"]["field"], "passphrase", "{e}");
    let r = post(addr, &preview, json!({"passphrase": "short"})).await;
    assert_eq!(ok(&r, 400, "short")["error"]["field"], "passphrase");
    let p = ok(
        &post(addr, &preview, json!({"passphrase": MANUAL})).await,
        200,
        "preview",
    );
    assert_eq!(p["mediaIncluded"], false);
    assert!(
        p["note"]
            .as_str()
            .unwrap()
            .contains("media folder needs its own backup")
    );
    assert_eq!(p["site"], false);
    assert_eq!(
        p["manifest"]["codexId"],
        get_json(addr, "/api/v1/codex").await["id"]
    );
    let by_id = |id: &str| {
        p["collections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["collection"]["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(by_id(&main)["comparison"], "same", "{p}");
    assert_eq!(by_id(&main)["active"], true);
    assert_eq!(by_id(&youth)["comparison"], "older", "{p}");
    assert_eq!(by_id(&youth)["collection"]["documents"], 0);
    assert_eq!(by_id(&youth)["here"]["documents"], 1);
    assert!(
        p["backupSets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["present"] == true),
        "{p}"
    );

    // Replacing the collection A shows: refused.
    let apply = format!("/api/v1/codex/export/imports/{id}/apply");
    let r = post(
        addr,
        &apply,
        json!({"passphrase": MANUAL, "replace": [main]}),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    // Youth replaced: its current copy goes to the collection trash first.
    let res = ok(
        &post(
            addr,
            &apply,
            json!({"passphrase": MANUAL, "replace": [youth]}),
        )
        .await,
        200,
        "apply",
    );
    assert_eq!(res["replaced"], json!([youth]), "{res}");
    assert_eq!(res["added"], json!([]));
    assert_eq!(res["skipped"], json!([main]));
    assert_eq!(res["backupSets"], 0);
    assert_eq!(res["siteStaged"], false);
    assert_eq!(res["restartNeeded"], false);
    let y = get_json(addr, &format!("/api/v1/codex/collections/{youth}")).await;
    assert_eq!(counts(&y), 0, "{y}");
    let ev = loop {
        let m = ws.next_any().await;
        if m["type"] == "documentChanged"
            && m["collection"] == youth.as_str()
            && m["change"] == "removed"
        {
            break m;
        }
    };
    assert_eq!(ev["kind"], "library");
    let log = get_json(
        addr,
        &format!("/api/v1/codex/collections/{youth}/changes?since=0"),
    )
    .await;
    let last = log["changes"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        (last["kind"].clone(), last["change"].clone()),
        (json!("library"), json!("removed"))
    );
    assert_eq!(last["by"]["client"], "server machine");
    // The used upload is gone.
    assert_eq!(
        post(addr, &preview, json!({"passphrase": MANUAL}))
            .await
            .status,
        404
    );
    // The undo: the copy from before, restored from the collection trash as its own
    // collection.
    let trash = get_json(addr, "/api/v1/codex/collection-trash").await;
    let before = trash
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["collection"]["name"] == "Youth (before import)")
        .unwrap_or_else(|| panic!("{trash}"))
        .clone();
    let bid = before["collection"]["id"].as_str().unwrap().to_string();
    assert_ne!(bid, youth);
    let restored = ok(
        &post(
            addr,
            &format!("/api/v1/codex/collection-trash/{bid}/restore"),
            json!({}),
        )
        .await,
        200,
        "restore the copy",
    );
    assert_eq!(counts(&restored), 1, "{restored}");

    // A fresh server takes the export: the collections, their search and the settings
    // backups; media not, until the same bytes are in its library.
    let fb = fixture("cx-export-b");
    let b = start_with(&fb, options(ExportOptions::default())).await;
    let id = upload_import(b.addr, &file).await;
    let p = ok(
        &post(
            b.addr,
            &format!("/api/v1/codex/export/imports/{id}/preview"),
            json!({"passphrase": MANUAL}),
        )
        .await,
        200,
        "preview on B",
    );
    assert!(
        p["collections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["comparison"] == "new"),
        "{p}"
    );
    assert!(
        p["backupSets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["present"] == false),
        "{p}"
    );
    // No site in this file (A doesn't run its site's Site Manager).
    let r = post(
        b.addr,
        &format!("/api/v1/codex/export/imports/{id}/apply"),
        json!({"passphrase": MANUAL, "site": true}),
    )
    .await;
    assert_eq!(ok(&r, 400, "no site")["error"]["field"], "site");
    let res = ok(
        &post(
            b.addr,
            &format!("/api/v1/codex/export/imports/{id}/apply"),
            json!({"passphrase": MANUAL}),
        )
        .await,
        200,
        "apply on B",
    );
    let mut added: Vec<String> = res["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    added.sort();
    let mut want = vec![main.clone(), youth.clone()];
    want.sort();
    assert_eq!(added, want, "{res}");
    assert!(res["backupSets"].as_u64().unwrap() >= 1, "{res}");
    assert!(res["snapshots"].as_u64().unwrap() >= 1, "{res}");
    assert_eq!(res["mediaIncluded"], false);
    let m = get_json(b.addr, &format!("/api/v1/codex/collections/{main}")).await;
    assert_eq!(
        counts(&m),
        counts(&get_json(addr, &format!("/api/v1/codex/collections/{main}")).await)
    );
    let hit = wait_search(b.addr, "Second", &main).await;
    assert_eq!(hit["hits"][0]["collection"], main.as_str(), "{hit}");
    let sets = get_json(b.addr, "/api/v1/codex/backups").await;
    let a_id = info["id"].as_str().unwrap();
    assert!(
        sets.as_array()
            .unwrap()
            .iter()
            .any(|s| s["set"]["machine"] == a_id && s["snapshots"].as_u64().unwrap() >= 1),
        "{sets}"
    );
    // The medium: missing on B, then found by its SHA-256 once the same bytes are in B's
    // library (the media folder restored; here uploaded into another collection, since an
    // upload into this one would take the missing asset itself).
    let media = format!("/api/v1/codex/collections/{main}/media");
    let missing = get_json(b.addr, &format!("{media}?status=missing")).await;
    assert!(
        missing
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["asset"]["id"] == asset.as_str()),
        "{missing}"
    );
    let spare = ok(
        &post(
            b.addr,
            "/api/v1/codex/collections",
            json!({"name": "Spare"}),
        )
        .await,
        201,
        "create",
    )["collection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    upload(b.addr, &spare, "Sunrise copy.png", &png).await;
    let r = ok(
        &post(b.addr, &format!("{media}/search"), json!({})).await,
        200,
        "relink search",
    );
    let found = r["relinked"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["asset"] == asset.as_str())
        .unwrap_or_else(|| panic!("{r}"))
        .clone();
    assert_eq!(found["method"], "contentHash");
    let list = get_json(b.addr, &media).await;
    let a2 = list
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["asset"]["id"] == asset.as_str())
        .unwrap()
        .clone();
    assert_eq!(a2["availability"], "online", "{a2}");
    assert_eq!(a2["asset"]["location"]["type"], "library");
    assert_eq!(a2["asset"]["location"]["sha256"], sha_hex(&png));

    b.shutdown().await.unwrap();
    a.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fa.dir).ok();
    std::fs::remove_dir_all(&fb.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduled_exports_need_a_stored_passphrase_and_are_pruned() {
    let fx = fixture("cx-export-sched");
    let s = start_with(
        &fx,
        options(ExportOptions {
            tick: Duration::from_millis(200),
            every: Some(Duration::from_secs(1)),
            ..ExportOptions::default()
        }),
    )
    .await;
    let addr = s.addr;
    let folder = fx.dir.join("nas").join("codex");
    let path = "/api/v1/codex/export/settings";
    let put = |body: Value| http(addr, "PUT", path, &[], Some(body));
    assert_eq!(
        ok(&put(json!({"time": "25:00"})).await, 400, "time")["error"]["field"],
        "time"
    );
    assert_eq!(
        ok(&put(json!({"folder": "relative/dir"})).await, 400, "folder")["error"]["field"],
        "folder"
    );
    assert_eq!(
        ok(&put(json!({"keepNightly": 0})).await, 400, "nightly")["error"]["field"],
        "keepNightly"
    );
    let st = ok(
        &put(json!({"folder": folder.display().to_string(), "time": "03:30"})).await,
        200,
        "settings",
    );
    assert_eq!(st["settings"]["time"], "03:30");
    assert_eq!(st["folder"], folder.display().to_string());
    // No stored passphrase: nothing runs, and the status says why.
    assert_eq!(st["passphraseSet"], false);
    assert_eq!(st["active"], false);
    assert!(
        st["problem"].as_str().unwrap().contains("no passphrase")
            && !st["problem"].as_str().unwrap().contains("/api/"),
        "{st}"
    );
    assert!(st.get("nextAt").is_none(), "{st}");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!folder.exists() || std::fs::read_dir(&folder).unwrap().next().is_none());
    assert_eq!(
        get_json(addr, "/api/v1/codex/export/scheduled").await,
        json!([])
    );

    // Stored: an export per second (the test knob), one kept a day; clients are told
    // (`codexChanged` `exports`) when one ran.
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    ok(
        &http(
            addr,
            "PUT",
            "/api/v1/codex/export/passphrase",
            &[],
            Some(json!({"passphrase": STORED})),
        )
        .await,
        200,
        "store",
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let first = loop {
        let list = get_json(addr, "/api/v1/codex/export/scheduled").await;
        if let Some(f) = list.as_array().unwrap().first() {
            break f.clone();
        }
        assert!(Instant::now() < deadline, "no scheduled export");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let name = first["file"].as_str().unwrap().to_string();
    assert!(
        name.starts_with("vesper-codex-") && name.ends_with(".vcodex"),
        "{name}"
    );
    assert!(first["bytes"].as_u64().unwrap() > 0);
    let file = std::fs::read(folder.join(&name)).unwrap();
    assert!(!contains(&file, STORED));
    let files = open(&file, STORED);
    let manifest: Value =
        serde_json::from_slice(&files.iter().find(|(n, _)| n == "export.json").unwrap().1).unwrap();
    assert_eq!(manifest["reason"], "scheduled");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut told = 0;
    while told < 2 {
        assert!(
            Instant::now() < deadline,
            "no codexChanged exports for the passphrase and the export: {:?}",
            ws.events
        );
        let m = ws.next_any().await;
        if m["type"] == "codexChanged" && m["parts"] == json!(["exports"]) {
            told += 1;
        }
    }
    drop(ws);
    assert!(
        !get_json(addr, path).await["problem"]
            .as_str()
            .is_some_and(|p| p.contains("/api/")),
        "no routes in people's messages"
    );
    // A later one takes its place (same day: one kept); other files in the folder stay.
    std::fs::write(folder.join("notes.txt"), b"left alone").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let list = get_json(addr, "/api/v1/codex/export/scheduled").await;
        let list = list.as_array().unwrap();
        if list.len() == 1 && list[0]["file"] != name.as_str() {
            break;
        }
        assert!(list.len() <= 2, "pruned: {list:?}");
        assert!(Instant::now() < deadline, "no newer export: {list:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!folder.join(&name).exists());
    assert!(folder.join("notes.txt").is_file());
    let st = get_json(addr, path).await;
    assert_eq!(st["active"], true, "{st}");
    assert!(st.get("problem").is_none(), "{st}");
    assert!(
        st["last"]["file"].is_string() && st["nextAt"].is_string(),
        "{st}"
    );

    // Cleared: no more exports.
    let st = ok(
        &http(addr, "DELETE", "/api/v1/codex/export/passphrase", &[], None).await,
        200,
        "clear",
    );
    assert_eq!(
        (st["passphraseSet"].clone(), st["active"].clone()),
        (json!(false), json!(false))
    );
    assert!(!fx.dir.join("machine/codex-export-passphrase").exists());

    s.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_scheduled_export_waits_before_it_is_tried_again() {
    let fx = fixture("cx-export-retry");
    let s = start_with(
        &fx,
        options(ExportOptions {
            tick: Duration::from_millis(100),
            every: Some(Duration::from_secs(1)),
            retry_after: Duration::from_secs(3600),
            ..ExportOptions::default()
        }),
    )
    .await;
    let addr = s.addr;
    // A folder that can't be made (a file where its parent should be), as a NAS gone away.
    std::fs::write(fx.dir.join("unplugged"), b"not a folder").unwrap();
    let gone = fx.dir.join("unplugged").join("codex");
    let path = "/api/v1/codex/export/settings";
    let put = |body: Value| http(addr, "PUT", path, &[], Some(body));
    ok(
        &put(json!({"folder": gone.display().to_string()})).await,
        200,
        "settings",
    );
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    ok(
        &http(
            addr,
            "PUT",
            "/api/v1/codex/export/passphrase",
            &[],
            Some(json!({"passphrase": STORED})),
        )
        .await,
        200,
        "store",
    );
    // About 20 ticks: one try, which fails (the status says why), then none.
    let until = Instant::now() + Duration::from_secs(2);
    let mut told = 0;
    while let Ok(m) = tokio::time::timeout(
        until.saturating_duration_since(Instant::now()),
        ws.next_any(),
    )
    .await
    {
        if m["type"] == "codexChanged" && m["parts"] == json!(["exports"]) {
            told += 1;
        }
    }
    assert_eq!(told, 2, "the passphrase stored, then one failed try");
    let st = get_json(addr, path).await;
    let problem = st["problem"].as_str().unwrap_or_default().to_string();
    assert!(problem.contains("can't be written"), "{st}");
    let next = chrono::DateTime::parse_from_rfc3339(st["nextAt"].as_str().unwrap()).unwrap();
    assert!(
        next > chrono::Local::now() + chrono::Duration::minutes(50),
        "the next try waits: {st}"
    );

    // Another folder: tried at once.
    let folder = fx.dir.join("nas");
    ok(
        &put(json!({"folder": folder.display().to_string()})).await,
        200,
        "settings",
    );
    // The file shows in the folder a moment before the tick records its success: wait for
    // both (M-3).
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let written = !get_json(addr, "/api/v1/codex/export/scheduled")
            .await
            .as_array()
            .unwrap()
            .is_empty();
        let st = get_json(addr, path).await;
        if written && st.get("problem").is_none() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no export into the new folder, or the problem stayed: {st}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The desktop app (another origin) reads a manual export's file name and note.
    let r = http(addr, "GET", path, &[("Origin", "tauri://localhost")], None).await;
    let exposed = r
        .header("access-control-expose-headers")
        .unwrap_or_default()
        .to_lowercase();
    for h in [
        "content-disposition",
        "x-codex-export-note",
        "x-codex-media-included",
    ] {
        assert!(exposed.contains(h), "{h} not exposed: {exposed}");
    }
    s.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}
