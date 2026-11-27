//! The site's identity in a Codex export (M13): on the site's server a manual export holds the
//! Site Manager's folder (IDN's own site backup inside it) only when asked (`includeSite`;
//! owner, 2026-11-23), a scheduled one always; importing it onto another server with
//! `site: true` stages it, and the Site Manager puts it in place at the next start: the server
//! is then in the exported site. Synthetic fixtures, Site Managers on loopback ports.

use super::export::{export, export_with, upload_import};
use crate::common::*;
use crate::site::start_with_codex;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use vesper_server::codex::CodexOptions;
use vesper_server::codex::export::ExportOptions;
use vesper_server::site::SiteConfig;

const PASS: &str = "site export secret";

fn ok(r: &Resp, status: u16, what: &str) -> Value {
    assert_eq!(
        r.status,
        status,
        "{what}: {}",
        String::from_utf8_lossy(&r.body)
    );
    r.json()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_site_part_is_staged_and_applied_at_the_next_start() {
    let fa = fixture("cx-export-site-a");
    let a = start_with_codex(&fa, true, SiteConfig::loopback(), Default::default()).await;
    let site_a = get_json(a.addr, "/api/v1/site").await["siteId"].clone();
    assert!(site_a.is_string());
    // Not asked: no site in a manual export, even on the site's server.
    let plain = export(a.addr, PASS).await;
    let (_, files) = idn_open_protocol::backup::open(&plain, PASS).unwrap();
    let manifest: Value =
        serde_json::from_slice(&files.iter().find(|(n, _)| n == "export.json").unwrap().1).unwrap();
    assert_eq!(manifest["site"], false, "{manifest}");
    assert!(!files.iter().any(|(n, _)| n.starts_with("site/")));
    // Ticked: the site's identity in it.
    let file = export_with(a.addr, json!({"passphrase": PASS, "includeSite": true})).await;
    let (_, files) = idn_open_protocol::backup::open(&file, PASS).unwrap();
    let manifest: Value =
        serde_json::from_slice(&files.iter().find(|(n, _)| n == "export.json").unwrap().1).unwrap();
    assert_eq!(manifest["site"], true);
    let inner = &files
        .iter()
        .find(|(n, _)| n == "site/site-manager.idnbackup")
        .expect("the site's folder")
        .1;
    // IDN's own site backup, with the same passphrase, without what IDN leaves out.
    let (m, site_files) = idn_open_protocol::backup::open(inner, PASS).unwrap();
    assert_eq!(m.kind, "site-manager");
    assert!(!site_files.is_empty());
    assert!(
        !site_files
            .iter()
            .any(|(n, _)| n.starts_with("restore-pending/") || n.starts_with("reports/")),
        "{:?}",
        m.files
    );
    a.shutdown().await.unwrap();

    let fb = fixture("cx-export-site-b");
    let b = start_with_codex(&fb, true, SiteConfig::loopback(), Default::default()).await;
    let site_b = get_json(b.addr, "/api/v1/site").await["siteId"].clone();
    assert_ne!(site_a, site_b);
    let id = upload_import(b.addr, &file).await;
    let p = ok(
        &http(
            b.addr,
            "POST",
            &format!("/api/v1/codex/export/imports/{id}/preview"),
            &[],
            Some(json!({"passphrase": PASS})),
        )
        .await,
        200,
        "preview",
    );
    assert_eq!(p["site"], true, "{p}");
    let res = ok(
        &http(
            b.addr,
            "POST",
            &format!("/api/v1/codex/export/imports/{id}/apply"),
            &[],
            Some(json!({"passphrase": PASS, "site": true})),
        )
        .await,
        200,
        "apply",
    );
    assert_eq!(res["siteStaged"], true, "{res}");
    assert_eq!(res["restartNeeded"], true, "{res}");
    assert_eq!(res["added"].as_array().unwrap().len(), 1, "{res}");
    let site_dir = fb.dir.join("machine/site-manager");
    assert!(site_dir.join("restore-pending").is_dir());
    // Until the next start, B's site is its own.
    assert_eq!(get_json(b.addr, "/api/v1/site").await["siteId"], site_b);
    b.shutdown().await.unwrap();

    let b = start_with_codex(&fb, true, SiteConfig::loopback(), Default::default()).await;
    let st = get_json(b.addr, "/api/v1/site").await;
    assert_eq!(st["running"], true, "{st}");
    assert_eq!(
        st["siteId"], site_a,
        "the exported site after the restart: {st}"
    );
    assert!(!site_dir.join("restore-pending").exists());
    // The Codex beside it is a member of the restored site, as after any start (IDN Beta5:
    // `join_own_site` sets the old site's membership aside and joins anew).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let ca = b.shared.codex_member.enrollment().and_then(|e| e.ca_id());
        if ca.as_deref() == site_a.as_str() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the Codex's membership after the restore is of site {ca:?}, not the restored {site_a}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    b.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fa.dir).ok();
    std::fs::remove_dir_all(&fb.dir).ok();
}

/// A scheduled export on the site's server always holds the site's identity.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_scheduled_export_on_the_sites_server_holds_the_site() {
    let fx = fixture("cx-export-site-sched");
    let s = start_with_codex(
        &fx,
        true,
        SiteConfig::loopback(),
        CodexOptions {
            export: ExportOptions {
                tick: Duration::from_millis(200),
                every: Some(Duration::from_secs(1)),
                ..ExportOptions::default()
            },
            ..CodexOptions::default()
        },
    )
    .await;
    let folder = fx.dir.join("nas").join("codex");
    ok(
        &http(
            s.addr,
            "PUT",
            "/api/v1/codex/export/settings",
            &[],
            Some(json!({"folder": folder.display().to_string()})),
        )
        .await,
        200,
        "settings",
    );
    ok(
        &http(
            s.addr,
            "PUT",
            "/api/v1/codex/export/passphrase",
            &[],
            Some(json!({"passphrase": PASS})),
        )
        .await,
        200,
        "store",
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let name = loop {
        let list = get_json(s.addr, "/api/v1/codex/export/scheduled").await;
        if let Some(f) = list.as_array().unwrap().first() {
            break f["file"].as_str().unwrap().to_string();
        }
        assert!(Instant::now() < deadline, "no scheduled export");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let file = std::fs::read(folder.join(&name)).unwrap();
    let (_, files) = idn_open_protocol::backup::open(&file, PASS).unwrap();
    let manifest: Value =
        serde_json::from_slice(&files.iter().find(|(n, _)| n == "export.json").unwrap().1).unwrap();
    assert_eq!(manifest["reason"], "scheduled");
    assert_eq!(manifest["site"], true, "{manifest}");
    assert!(
        files
            .iter()
            .any(|(n, _)| n == "site/site-manager.idnbackup")
    );
    s.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}
