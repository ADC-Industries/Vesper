//! The Codex's own membership of its site (`machine/codex-member/`): joined at the Site
//! Manager's start without anyone's click, once (a restart keeps the same membership), and
//! listed as a machine of the site (`builtIn`: part of Vesper, it can't be removed), and serves
//! the Codex's media library to the site with it. It isn't one of the site's "other machines":
//! the box may still join another site (see [`super::keys`]).

use super::*;

/// The site's devices named `Codex`.
async fn codex_devices(addr: SocketAddr) -> Vec<Value> {
    get_json(addr, "/api/v1/site/devices")
        .await
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["name"] == "Codex")
        .cloned()
        .collect()
}

/// Waits until the Codex's device is online.
async fn online(addr: SocketAddr) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let d = codex_devices(addr).await;
        if d.len() == 1 && d[0]["health"] == "online" {
            return d[0].clone();
        }
        assert!(Instant::now() < deadline, "the Codex's device: {d:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_codex_joins_its_own_site_once_and_keeps_its_membership() {
    let fx = fixture("site-codex-member");
    let server = start_site(&fx, true).await;
    let addr = server.addr;

    // At start: a member, with its identity under machine/.
    let fp = server
        .shared
        .codex_member
        .fingerprint()
        .expect("the Codex is a member once the server has started");
    let e = server.shared.codex_member.enrollment().unwrap();
    assert!(e.creds().is_some(), "enrolled");
    assert_eq!(
        e.ca_id(),
        get_json(addr, "/api/v1/site").await["siteId"]
            .as_str()
            .map(str::to_string)
    );
    assert!(fx.dir.join("machine/codex-member").is_dir());
    // With it, the Codex's media library is served to the site.
    assert!(
        server.shared.codex_link.serving(),
        "the media library is served"
    );
    assert_eq!(
        get_json(addr, "/api/v1/codex").await["mediaLibrary"],
        json!({"available": true, "served": true})
    );
    let d = online(addr).await;
    assert_eq!(d["fingerprint"], fp, "{d}");
    // Part of Vesper: marked built in, and it can't be removed (it would only join again).
    assert_eq!(d["builtIn"], true, "{d}");
    // No IDN Node reports for it: no machine details (G9) and no Node start, rather than
    // empty ones.
    assert!(
        d.get("machine").is_none() && d.get("nodeStartedAt").is_none(),
        "{d}"
    );
    let r = call(
        addr,
        "DELETE",
        &format!("/api/v1/site/devices/{fp}"),
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        String::from_utf8_lossy(&r.body).contains("built into Vesper"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let d = online(addr).await;
    assert_eq!(d["builtIn"], true, "still there: {d}");
    // Approved here at once: nothing waits, and the approval window is closed again.
    assert_eq!(
        get_json(addr, "/api/v1/site/join-requests").await,
        json!([])
    );
    let st = get_json(addr, "/api/v1/site").await;
    assert_eq!(st["pairingOpen"], false, "{st}");

    // A restart keeps the same membership: no second device.
    drop(e);
    server.shutdown().await.unwrap();
    let server = start_site(&fx, true).await;
    let addr = server.addr;
    assert_eq!(
        server.shared.codex_member.fingerprint().as_deref(),
        Some(fp.as_str()),
        "the same membership after a restart"
    );
    let d = online(addr).await;
    assert_eq!(d["fingerprint"], fp, "{d}");
    assert_eq!(d["builtIn"], true, "{d}");
    assert_eq!(codex_devices(addr).await.len(), 1);

    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}
