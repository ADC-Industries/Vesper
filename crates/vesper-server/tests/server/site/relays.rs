//! Pairing relays (IDN v0.3.4-Beta11): the site's server marks its Vesper boxes itself.
//!
//! - A Vesper box joining (by code, or approved, even under another name) is marked a pairing
//!   relay at once; no other machine is (another program's app, an output box, the Codex's own
//!   membership). The mark is in the Site Manager's audit, done by the server machine.
//! - An admin can take a mark off or put it on (`POST /site/devices/{fp}/relay`); it stays as
//!   set across a restart. Built in, or any machine that isn't a Vesper box (M-6): `409`; the
//!   mark comes off any machine.
//! - An existing site after the upgrade: a Vesper box enrolled before (not marked, nothing
//!   decided) is marked at the site server's start.
//!
//! That relayed pairings work through a marked box: [`super::keys`] (its `two_boxes` waits for
//! B's mark).

use super::*;

/// A machine joining the site at `ds_port` as `name`, exactly so (a Vesper box joins as
/// `<name> (Vesper)`).
fn enroll(fx: &Fixture, ds_port: u16, name: &str, role: DeviceRole) -> Arc<Enrollment> {
    Enrollment::start(EnrollmentConfig {
        discovery_server: format!("127.0.0.1:{ds_port}"),
        state_dir: fx.dir.join("machines").join(name.replace(' ', "_")),
        name: name.into(),
        role,
        passphrase: None,
        admin_code: None,
    })
    .expect("enrollment starts")
}

/// Joins with a fresh join code (the server machine is the site's admin).
async fn join_by_code(addr: SocketAddr, m: &Arc<Enrollment>) {
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    m.join_with_code(code["code"].as_str().unwrap()).unwrap();
    until(m, "joins", |s| matches!(s, TrustState::Enrolled { .. })).await;
}

/// The device list's entry for `fingerprint`, once the site lists it.
async fn device(addr: SocketAddr, fingerprint: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let devs = get_json(addr, "/api/v1/site/devices").await;
        if let Some(d) = devs
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["fingerprint"] == fingerprint)
        {
            return d.clone();
        }
        assert!(Instant::now() < deadline, "{fingerprint} listed: {devs}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until the site lists `fingerprint` as a relay (`on`) or not.
async fn relay_is(addr: SocketAddr, fingerprint: &str, on: bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let d = device(addr, fingerprint).await;
        if (d["relay"] == true) == on {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: {d}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn set_relay(addr: SocketAddr, fingerprint: &str, on: bool) -> u16 {
    call(
        addr,
        "POST",
        &format!("/api/v1/site/devices/{fingerprint}/relay"),
        None,
        Some(json!({ "on": on })),
    )
    .await
    .status
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_joined_vesper_box_is_marked_a_relay_and_no_other_machine() {
    let fx = fixture("site-relay-mark");
    let server = start_site(&fx, true).await;
    let addr = server.addr;
    let ds = get_json(addr, "/api/v1/site").await["discoveryPort"]
        .as_u64()
        .unwrap() as u16;

    // A Vesper box joining by code: marked as it joins.
    let lobby = enroll(&fx, ds, "LOBBY (Vesper)", DeviceRole::App);
    join_by_code(addr, &lobby).await;
    relay_is(addr, &lobby.fingerprint(), true, "a Vesper box is marked").await;
    assert!(
        server
            .shared
            .site
            .handle()
            .unwrap()
            .authority()
            .is_relay(&lobby.fingerprint()),
        "marked in IDN"
    );

    // Not Vesper boxes: another program's app, an output box named like one.
    let desk = enroll(&fx, ds, "Lighting desk", DeviceRole::App);
    join_by_code(addr, &desk).await;
    let screen = enroll(&fx, ds, "SCREEN (Vesper)", DeviceRole::Output);
    join_by_code(addr, &screen).await;

    // A Vesper box approved under another name: known by the name it asked with.
    let booth = enroll(&fx, ds, "BOOTH-PC (Vesper)", DeviceRole::App);
    until(&booth, "waits for the window", |s| {
        matches!(s, TrustState::WaitingForPairing)
    })
    .await;
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/pairing",
            None,
            Some(json!({"seconds": 120})),
        )
        .await,
        204,
        "open window",
    );
    booth.retry_now();
    until(&booth, "asks", |s| matches!(s, TrustState::Pending { .. })).await;
    let reqs = get_json(addr, "/api/v1/site/join-requests").await;
    let id = reqs[0]["id"].as_u64().unwrap();
    ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/site/join-requests/{id}/approve"),
            None,
            Some(json!({"name": "Booth"})),
        )
        .await,
        204,
        "approve",
    );
    until(&booth, "joins", |s| {
        matches!(s, TrustState::Enrolled { .. })
    })
    .await;
    relay_is(addr, &booth.fingerprint(), true, "renamed, still marked").await;
    assert_eq!(device(addr, &booth.fingerprint()).await["name"], "Booth");

    for (m, which) in [(&desk, "another program's app"), (&screen, "an output box")] {
        let d = device(addr, &m.fingerprint()).await;
        assert!(d.get("relay").is_none(), "{which} isn't marked: {d}");
    }
    let devs = get_json(addr, "/api/v1/site/devices").await;
    for d in devs.as_array().unwrap() {
        if d["builtIn"] == true {
            assert!(d.get("relay").is_none(), "the Codex isn't marked: {d}");
            let fp = d["fingerprint"].as_str().unwrap();
            assert_eq!(set_relay(addr, fp, true).await, 409, "built in");
        }
    }

    // In the Site Manager's audit, by the server machine.
    let audit = get_json(addr, "/api/v1/site/audit?n=100").await;
    let marks = audit
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "relay" && e["client"] == "server machine" && e["ok"] == true)
        .count();
    assert_eq!(marks, 2, "LOBBY and Booth, once each: {audit:#}");

    // Only a Vesper box is marked by an admin (M-6): another program's app, an output box
    // named like a Vesper box, a box renamed as it was approved are refused, unmarked.
    for (m, which) in [
        (&desk, "another program's app"),
        (&screen, "an output box"),
        (&booth, "a renamed box"),
    ] {
        assert_eq!(
            set_relay(addr, &m.fingerprint(), true).await,
            409,
            "{which}"
        );
    }
    relay_is(addr, &desk.fingerprint(), false, "the desk stays unmarked").await;
    relay_is(
        addr,
        &screen.fingerprint(),
        false,
        "the output stays unmarked",
    )
    .await;

    // An admin takes LOBBY's and Booth's marks off, then puts LOBBY's back on, and off again:
    // kept so across a restart.
    assert_eq!(set_relay(addr, &lobby.fingerprint(), false).await, 204);
    assert_eq!(set_relay(addr, &booth.fingerprint(), false).await, 204);
    relay_is(addr, &lobby.fingerprint(), false, "taken off").await;
    relay_is(addr, &booth.fingerprint(), false, "taken off a renamed box").await;
    assert_eq!(set_relay(addr, &lobby.fingerprint(), true).await, 204);
    relay_is(addr, &lobby.fingerprint(), true, "put on").await;
    assert_eq!(set_relay(addr, &lobby.fingerprint(), false).await, 204);
    relay_is(addr, &lobby.fingerprint(), false, "taken off again").await;
    assert_eq!(
        set_relay(addr, &"ab".repeat(32), true).await,
        404,
        "no such machine"
    );
    server.shutdown().await.unwrap();
    let server = start_site(&fx, true).await;
    let addr = server.addr;
    relay_is(addr, &lobby.fingerprint(), false, "an admin's off stays").await;
    relay_is(
        addr,
        &booth.fingerprint(),
        false,
        "an admin's off stays (renamed)",
    )
    .await;
    relay_is(addr, &desk.fingerprint(), false, "never marked").await;
    assert_eq!(set_relay(addr, &lobby.fingerprint(), true).await, 204);
    relay_is(addr, &lobby.fingerprint(), true, "an admin's on").await;
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_existing_unmarked_vesper_box_is_marked_at_the_start() {
    let fx = fixture("site-relay-upgrade");
    let server = start_site(&fx, true).await;
    let addr = server.addr;
    let ds = get_json(addr, "/api/v1/site").await["discoveryPort"]
        .as_u64()
        .unwrap() as u16;
    let old = enroll(&fx, ds, "OLD-BOX (Vesper)", DeviceRole::App);
    join_by_code(addr, &old).await;
    let other = enroll(&fx, ds, "Camera PC", DeviceRole::App);
    join_by_code(addr, &other).await;
    relay_is(addr, &old.fingerprint(), true, "marked as it joined").await;

    // As a site from before the upgrade: the box isn't marked, and nothing was decided on it.
    server
        .shared
        .site
        .handle()
        .unwrap()
        .authority()
        .set_relay(&old.fingerprint(), false)
        .unwrap();
    server.shutdown().await.unwrap();
    let file = fx.dir.join("machine/site.json");
    let mut stored: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert!(
        stored["relayDecided"]
            .as_array()
            .is_some_and(|a| a.contains(&json!(old.fingerprint()))),
        "{stored}"
    );
    stored.as_object_mut().unwrap().remove("relayDecided");
    std::fs::write(&file, stored.to_string()).unwrap();

    // The start marks it (once), and no other machine.
    let server = start_site(&fx, true).await;
    let addr = server.addr;
    relay_is(addr, &old.fingerprint(), true, "marked at the start").await;
    relay_is(addr, &other.fingerprint(), false, "not a Vesper box").await;
    let audit = get_json(addr, "/api/v1/site/audit?n=100").await;
    assert!(
        audit_has(&audit, "relay", |e| e["client"] == "server machine"
            && e["ok"] == true),
        "{audit:#}"
    );
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}
