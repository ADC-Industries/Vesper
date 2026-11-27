//! Paired clients' site keys on several Vesper servers of one site (two servers in this test
//! process, on loopback ports), and a server joining another server's site and leaving it.
//!
//! - A key made at the site's server works on every server of the site, checked locally on
//!   each; revoked in the site, it stops on all of them (and their open WebSockets close).
//! - One Site Manager per site: a server that joins another's site stops its own, can't turn
//!   it on again while it is in, and runs its own site again when it leaves. A server whose
//!   own site still has other machines can't join another.
//! - At access `users` a key still runs the show; signing in is only for site admin actions.
//! - Pairing at a server of the site that isn't its server (a relay) is relayed to the site's
//!   server. The relay shows a code the site's Site Manager made for it: typed right, the
//!   pairing is approved at once (one request); the code works once and the relay shows the
//!   next. Without it (or with a wrong one) the pairing waits for an admin's approval there
//!   (a wrong or used code is said at once: `codeWrong`, the client may type it again):
//!   approved, the key comes back through it and works on both (and after the relay is gone);
//!   denied, the client is told why; the relay disconnecting ends the request; the client
//!   may take a waiting pairing back (`DELETE /api/v1/pair/{pairingId}`). The code's
//!   expiry is checked with a code life shortened to seconds (IDN's `set_pairing_code_life`).

use super::*;
use idn_open_protocol::trust::token;

/// The site's server A and a second server B ("BOX-B"), B a machine of A's site; a key paired
/// at A, and B's key from its own site before it joined.
pub(crate) struct TwoBoxes {
    pub(crate) fa: Fixture,
    pub(crate) a: RunningServer,
    pub(crate) fb: Fixture,
    pub(crate) b: RunningServer,
    pub(crate) key_a: String,
    pub(crate) key_b: String,
    pub(crate) ds: u16,
    pub(crate) site_id: Value,
}

/// A key's id (`key:<id>`).
fn key_id(key: &str) -> String {
    token::peek(key)
        .unwrap()
        .sub
        .strip_prefix("key:")
        .unwrap()
        .to_string()
}

/// Waits until `pred` holds for server B (checked on the blocking pool).
async fn wait_for(
    server: &RunningServer,
    what: &str,
    pred: fn(&vesper_server::site::Site) -> bool,
) {
    let site = server.shared.site.clone();
    let what = what.to_string();
    tokio::task::spawn_blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !pred(&site) {
            assert!(
                Instant::now() < deadline,
                "{what}: membership {:?}",
                site.membership()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    })
    .await
    .unwrap();
}

async fn two_boxes(tag: &str) -> TwoBoxes {
    two_boxes_with(tag, Default::default(), Default::default(), |_| {}).await
}

/// [`two_boxes`] with each box's Codex options, and A's fixture prepared by `prepare_a`
/// before A starts.
pub(crate) async fn two_boxes_with(
    tag: &str,
    codex_a: vesper_server::codex::CodexOptions,
    codex_b: vesper_server::codex::CodexOptions,
    prepare_a: impl FnOnce(&Fixture),
) -> TwoBoxes {
    two_boxes_prepared(tag, codex_a, codex_b, prepare_a, |_| {}).await
}

/// [`two_boxes_with`] with B's fixture prepared by `prepare_b` before B starts too.
pub(crate) async fn two_boxes_prepared(
    tag: &str,
    codex_a: vesper_server::codex::CodexOptions,
    codex_b: vesper_server::codex::CodexOptions,
    prepare_a: impl FnOnce(&Fixture),
    prepare_b: impl FnOnce(&Fixture),
) -> TwoBoxes {
    two_boxes_site(
        tag,
        codex_a,
        codex_b,
        prepare_a,
        prepare_b,
        SiteConfig::loopback(),
    )
    .await
}

/// [`two_boxes_prepared`] with B's site settings.
async fn two_boxes_site(
    tag: &str,
    codex_a: vesper_server::codex::CodexOptions,
    codex_b: vesper_server::codex::CodexOptions,
    prepare_a: impl FnOnce(&Fixture),
    prepare_b: impl FnOnce(&Fixture),
    site_b: SiteConfig,
) -> TwoBoxes {
    let fa = fixture(&format!("{tag}-a"));
    prepare_a(&fa);
    let fb = fixture(&format!("{tag}-b"));
    prepare_b(&fb);
    std::fs::create_dir_all(fb.dir.join("machine")).unwrap();
    std::fs::write(
        fb.dir.join("machine/server.json"),
        json!({"id": "6a1d1c8e-0000-4000-8000-0000000000b0", "name": "BOX-B"}).to_string(),
    )
    .unwrap();
    // Remote-style clients on both (no trust for localhost): keys only.
    let a = start_with_codex(&fa, false, SiteConfig::loopback(), codex_a).await;
    let b = start_with_codex(&fb, false, site_b, codex_b).await;
    let key_a = pair(&a, "Booth").await;
    let key_b = pair(&b, "Tablet").await;
    let st = get_json_auth(a.addr, "/api/v1/site", Some(&key_a)).await;
    let ds = st["discoveryPort"].as_u64().unwrap() as u16;
    let code = ok(
        &call(a.addr, "POST", "/api/v1/site/join-code", Some(&key_a), None).await,
        201,
        "join code",
    );
    // B joins A's site (B's own site has no other machines).
    let m = ok(
        &call(
            b.addr,
            "POST",
            "/api/v1/site/join",
            Some(&key_b),
            Some(json!({"server": format!("127.0.0.1:{ds}"), "code": code["code"]})),
        )
        .await,
        200,
        "join",
    );
    assert_eq!(m["server"], format!("127.0.0.1:{ds}"), "{m}");
    assert_eq!(m["fingerprint"].as_str().unwrap().len(), 64, "{m}");
    wait_for(
        &b,
        "B is in A's site, connected, and its own Site Manager stopped",
        |s| {
            let m = s.membership();
            m.state == protocol::MembershipState::Enrolled && m.connected && s.handle().is_none()
        },
    )
    .await;
    // A marks B, a Vesper box, a pairing relay as it joins (IDN Beta11: only a relay shows
    // the site's pairing codes).
    let fp_b = m["fingerprint"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let devs = get_json_auth(a.addr, "/api/v1/site/devices", Some(&key_a)).await;
        let d = devs
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["fingerprint"] == fp_b);
        if d.is_some_and(|d| d["relay"] == true) {
            break;
        }
        assert!(Instant::now() < deadline, "A marks B a relay: {devs}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    TwoBoxes {
        fa,
        a,
        fb,
        b,
        key_a,
        key_b,
        ds,
        site_id: st["siteId"].clone(),
    }
}

impl TwoBoxes {
    pub(crate) async fn end(self) {
        self.a.shutdown().await.unwrap();
        self.b.shutdown().await.unwrap();
        std::fs::remove_dir_all(&self.fa.dir).ok();
        std::fs::remove_dir_all(&self.fb.dir).ok();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_key_works_on_every_box_and_revoking_it_stops_it_everywhere() {
    let t = two_boxes("site-keys").await;
    let (a, b) = (t.a.addr, t.b.addr);
    let key = Some(t.key_a.as_str());

    // B, a machine of A's site: its own Site Manager is off; A's key works there.
    let st = get_json_auth(b, "/api/v1/site", key).await;
    assert_eq!(
        (st["enabled"].clone(), st["running"].clone()),
        (json!(false), json!(false)),
        "{st}"
    );
    assert_eq!(st["membership"]["state"], "enrolled", "{st}");
    assert_eq!(st["membership"]["siteId"], t.site_id, "{st}");
    assert_eq!(
        get_json_auth(b, "/api/v1/site/membership", key).await["server"],
        format!("127.0.0.1:{}", t.ds)
    );
    let devs = get_json_auth(a, "/api/v1/site/devices", key).await;
    assert!(
        devs.as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "BOX-B (Vesper)"),
        "A lists B: {devs}"
    );

    // The key on both servers: REST, and a WebSocket on B.
    for (addr, which) in [(a, "A"), (b, "B")] {
        let r = call(addr, "GET", "/api/v1/state", key, None).await;
        assert_eq!(r.status, 200, "on {which}");
        let info = get_json_auth(addr, "/api/v1/server", key).await;
        assert_eq!(info["auth"], "paired", "on {which}");
    }
    let mut ws_b = Client::connect(b, &format!("protocolVersion=1&token={}", t.key_a)).await;
    // (A command that doesn't depend on the content B shows: B turns into a show box of A's
    // Codex at any moment now.)
    ws_b.ok(json!({"type": "clearLayer", "layer": "slide"}))
        .await;
    // B's own site is gone, and its keys with it: its clients pair again.
    assert_eq!(
        call(b, "GET", "/api/v1/state", Some(&t.key_b), None)
            .await
            .status,
        401
    );

    // Revoked in the site (at A): refused at A at once, and at B as soon as the site tells it.
    let r = call(
        a,
        "DELETE",
        &format!("/api/v1/site/api-keys/{}", key_id(&t.key_a)),
        key,
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(call(a, "GET", "/api/v1/state", key, None).await.status, 401);
    let deadline = Instant::now() + Duration::from_secs(15);
    while call(b, "GET", "/api/v1/state", key, None).await.status != 401 {
        assert!(Instant::now() < deadline, "B still takes the revoked key");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        closed_by_server(&mut ws_b.ws).await,
        Some(4003),
        "its WebSocket on B closes"
    );
    t.end().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_joined_box_keeps_its_site_manager_off_until_it_leaves() {
    let mut t = two_boxes("site-member").await;
    let key = Some(t.key_a.as_str());
    let b = t.b.addr;

    // One Site Manager per site: B's can't be turned on while B is in A's site.
    let r = call(
        b,
        "PUT",
        "/api/v1/site/settings",
        key,
        Some(json!({"enabled": true, "joinMode": "code_or_approval", "access": "open"})),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("leave that site first"),
        "{}",
        r.json()
    );
    let r = call(
        b,
        "POST",
        "/api/v1/site/join",
        key,
        Some(json!({"server": "192.0.2.10"})),
    )
    .await;
    assert_eq!(r.status, 409, "already in a site");
    assert!(t.b.shared.site.handle().is_none());

    // A restart keeps B in A's site, its own Site Manager off, A's key taken.
    t.b.shutdown().await.unwrap();
    t.b = start_with(&t.fb, false, SiteConfig::loopback()).await;
    let b = t.b.addr;
    assert!(t.b.shared.site.handle().is_none(), "still off");
    let st = get_json_auth(b, "/api/v1/site", key).await;
    assert_eq!(st["membership"]["state"], "enrolled", "{st}");
    assert_eq!(st["enabled"], false);

    // Leaving: B tells A's site (which lists it no more), runs its own site again, and A's
    // keys stop there.
    wait_for(&t.b, "B connected to A's site again", |s| {
        s.membership().connected
    })
    .await;
    let fp = t.b.shared.site.membership().fingerprint.unwrap();
    let mut ws_a = Client::connect(t.a.addr, &format!("protocolVersion=1&token={}", t.key_a)).await;
    let r = call(b, "POST", "/api/v1/site/leave", key, None).await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let left = site_event(&mut ws_a, "device_left").await;
    assert_eq!(left["fingerprint"], fp, "{left}");
    assert_eq!(left["machine"], "BOX-B (Vesper)", "{left}");
    let devs = get_json_auth(t.a.addr, "/api/v1/site/devices", key).await;
    assert!(
        !devs
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["fingerprint"] == fp && d["health"] != "removed"),
        "A lists B no more: {devs}"
    );
    drop(ws_a);
    assert!(
        t.b.shared.site.handle().is_some(),
        "its own site runs again"
    );
    assert_eq!(
        t.b.shared.site.membership().state,
        protocol::MembershipState::None
    );
    assert!(t.b.shared.site.settings().enabled);
    assert_eq!(call(b, "GET", "/api/v1/state", key, None).await.status, 401);
    let key_b = pair(&t.b, "Tablet").await;
    let st = get_json_auth(b, "/api/v1/site", Some(&key_b)).await;
    assert_eq!(st["running"], true);
    assert_ne!(st["siteId"], t.site_id, "B's own site");
    t.end().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_box_removed_from_a_site_forgets_it_and_isnt_pulled_back_in() {
    let t = two_boxes("site-removed").await;
    let key = Some(t.key_a.as_str());
    let a = t.a.addr;
    let fp = t.b.shared.site.membership().fingerprint.unwrap();

    // A removes B: B runs its own site again, and forgets A's.
    ok(
        &call(
            a,
            "DELETE",
            &format!("/api/v1/site/devices/{fp}"),
            key,
            None,
        )
        .await,
        204,
        "remove B",
    );
    wait_for(&t.b, "B runs its own site again", |s| s.handle().is_some()).await;

    // A lets B ask again, with its approval window open: B doesn't ask (joining again is a
    // fresh join on B), so nothing pulls it back in. (IDN asks again every 15 s.)
    ok(
        &call(
            a,
            "POST",
            &format!("/api/v1/site/devices/{fp}/allow"),
            key,
            None,
        )
        .await,
        204,
        "allow B again",
    );
    ok(
        &call(
            a,
            "POST",
            "/api/v1/site/pairing",
            key,
            Some(json!({"seconds": 120})),
        )
        .await,
        204,
        "open the window",
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let asks = get_json_auth(a, "/api/v1/site/join-requests", key).await;
        if let Some(r) = asks.as_array().unwrap().first() {
            // What an admin would do with it: B would re-enter and stop its own site.
            let id = r["id"].as_u64().unwrap();
            ok(
                &call(
                    a,
                    "POST",
                    &format!("/api/v1/site/join-requests/{id}/approve"),
                    key,
                    None,
                )
                .await,
                204,
                "approve",
            );
            tokio::time::sleep(Duration::from_secs(3)).await;
            panic!(
                "B, removed, asked to join A's site again by itself ({r}); approved, its own Site Manager runs: {}",
                t.b.shared.site.handle().is_some()
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(t.b.shared.site.handle().is_some(), "B's own site runs");
    assert_eq!(
        t.b.shared.site.membership().state,
        protocol::MembershipState::None
    );
    assert!(!t.fb.dir.join("machine/site-member").exists());
    t.end().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_box_whose_site_has_other_machines_refuses_to_join() {
    let fx = fixture("site-orphans");
    let server = start_site(&fx, true).await;
    let addr = server.addr;
    let ds = get_json(addr, "/api/v1/site").await["discoveryPort"]
        .as_u64()
        .unwrap() as u16;
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    let one = machine(&fx, ds, "BOX-1");
    one.join_with_code(code["code"].as_str().unwrap()).unwrap();
    until(&one, "joins", |s| matches!(s, TrustState::Enrolled { .. })).await;

    // Bad requests first.
    let r = call(addr, "POST", "/api/v1/site/join", None, Some(json!({}))).await;
    assert_eq!(r.status, 400, "no server");
    let r = call(
        addr,
        "POST",
        "/api/v1/site/join",
        None,
        Some(json!({"server": "192.0.2.10", "code": "12"})),
    )
    .await;
    assert_eq!(r.status, 400, "a code is six digits");

    // BOX-1 would be left without a site.
    let r = call(
        addr,
        "POST",
        "/api/v1/site/join",
        None,
        Some(json!({"server": "192.0.2.10"})),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    let msg = r.json()["error"]["message"].as_str().unwrap().to_string();
    assert!(msg.contains("BOX-1 (Test)"), "{msg}");
    assert!(msg.contains("remove it"), "{msg}");
    assert!(
        !msg.contains("Codex"),
        "the box's own Codex isn't another machine: {msg}"
    );
    let st = get_json(addr, "/api/v1/site").await;
    assert_eq!(st["membership"]["state"], "none");
    assert_eq!(st["running"], true);
    let audit = get_json(addr, "/api/v1/site/audit").await;
    assert!(audit_has(&audit, "join", |e| e["ok"] == false));

    // Removed: the join may go ahead (to an address nobody answers at), and stop again.
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/site/devices/{}", one.fingerprint()),
            None,
            None,
        )
        .await,
        204,
        "remove BOX-1",
    );
    let m = ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/join",
            None,
            Some(json!({"server": "192.0.2.10"})),
        )
        .await,
        200,
        "join",
    );
    assert_ne!(m["state"], "enrolled", "{m}");
    assert_eq!(
        get_json(addr, "/api/v1/site").await["running"],
        true,
        "its own site runs until it is in the other"
    );
    ok(
        &call(addr, "POST", "/api/v1/site/leave", None, None).await,
        204,
        "leave",
    );
    assert_eq!(
        get_json(addr, "/api/v1/site/membership").await["state"],
        "none"
    );
    let r = call(addr, "POST", "/api/v1/site/leave", None, None).await;
    assert_eq!(r.status, 409, "not in another site");
    drop(one);
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn at_access_users_a_key_runs_the_show_and_signing_in_is_for_admin_only() {
    let fx = fixture("site-keys-users");
    let server = start_site(&fx, false).await;
    let addr = server.addr;
    let key = pair(&server, "Booth").await;
    let k = Some(key.as_str());
    ok(
        &call(
            addr,
            "PUT",
            "/api/v1/site/users/root",
            k,
            Some(json!({"role": "admin", "password": "root-password"})),
        )
        .await,
        200,
        "add an admin",
    );
    ok(&set_access(addr, k, "users", None).await, 200, "users");

    // Not signed in: the show runs on the key alone.
    assert_eq!(
        call(addr, "GET", "/api/v1/state", k, None).await.status,
        200
    );
    let trigger =
        json!({"type": "triggerSlide", "presentation": fx.song.id.0.to_string(), "index": 0});
    let r = call(addr, "POST", "/api/v1/commands", k, Some(trigger.clone())).await;
    assert_eq!(ok(&r, 200, "command")["ok"], true);
    let mut ws = Client::connect(addr, &format!("protocolVersion=1&token={key}")).await;
    ws.ok(trigger).await;
    let s = get_json_auth(addr, "/api/v1/site/session", k).await;
    assert_eq!(
        (s["access"].clone(), s["signedIn"].clone()),
        (json!("users"), json!(false))
    );
    assert!(s.get("user").is_none(), "{s}");

    // Site admin actions need a sign-in.
    let r = call(addr, "POST", "/api/v1/site/join-code", k, None).await;
    assert_eq!(r.status, 403);
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/session",
            k,
            Some(json!({"user": "root", "password": "root-password"})),
        )
        .await,
        200,
        "sign in",
    );
    ok(
        &call(addr, "POST", "/api/v1/site/join-code", k, None).await,
        201,
        "a join code, signed in",
    );
    // Signed out: still running the show.
    ok(
        &call(addr, "DELETE", "/api/v1/site/session", k, None).await,
        204,
        "sign out",
    );
    assert_eq!(
        call(addr, "GET", "/api/v1/state", k, None).await.status,
        200
    );
    ws.ok(json!({"type": "triggerSlide", "presentation": fx.song.id.0.to_string(), "index": 0}))
        .await;
    drop(ws);
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// `POST /api/v1/pair` without a code (or at a relay): `202`, the pairing's secret id.
async fn pair_waiting(addr: SocketAddr, name: &str, code: Option<&str>) -> String {
    let mut body = json!({"clientName": name});
    if let Some(c) = code {
        body["code"] = json!(c);
    }
    let r = http(addr, "POST", "/api/v1/pair", &[], Some(body)).await;
    let p = ok(&r, 202, "pairing waits");
    let left = p["expiresInSeconds"].as_u64().unwrap();
    assert!((590..=600).contains(&left), "{p}");
    let id = p["pairingId"].as_str().unwrap().to_string();
    assert_eq!(id.len(), 32, "a secret id");
    id
}

/// The waiting pairing says its code was wrong (the site's server said so; soon after the
/// pairing was asked, if not in its first answer).
async fn says_code_wrong(addr: SocketAddr, secret: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let r = http(addr, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
        let p = ok(&r, 202, "still waiting for an admin");
        if p["codeWrong"] == true {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "never said the code was wrong: {p}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Asks again for the pairing's answer until it isn't `202` any more.
async fn pairing_answer(addr: SocketAddr, secret: &str) -> Resp {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let r = http(addr, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
        if r.status != 202 {
            return r;
        }
        assert!(Instant::now() < deadline, "the pairing stays unanswered");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The pairing code relay `server` shows, once the site's server gave it one.
async fn relay_code(server: &RunningServer) -> protocol::PairingCode {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(c) = server.shared.site.relay_code() {
            assert_eq!(server.pairing_code(), c.code, "the code the relay shows");
            return c;
        }
        assert!(
            Instant::now() < deadline,
            "the relay has no code from its site"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `POST /api/v1/pair` with a code; the response.
async fn pair_with(addr: SocketAddr, name: &str, code: &str, browser: bool) -> Resp {
    http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": code, "clientName": name, "browser": browser})),
    )
    .await
}

/// The pairings waiting at A, once `n` are there.
async fn waiting_at(addr: SocketAddr, key: &str, n: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let w = get_json_auth(addr, "/api/v1/site/pairings", Some(key)).await;
        if w.as_array().unwrap().len() == n {
            return w;
        }
        assert!(Instant::now() < deadline, "{n} pairings waiting: {w}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relayed_pairing_approved_works_on_both_and_outlives_the_relay() {
    let t = two_boxes("site-relay").await;
    let (a, b) = (t.a.addr, t.b.addr);
    let admin = Some(t.key_a.as_str());

    // An Oriel pairs at B: relayed to A, waiting there.
    let secret = pair_waiting(b, "Stage laptop", None).await;
    let r = http(b, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
    assert_eq!(ok(&r, 202, "still waiting")["pairingId"], secret);
    assert_eq!(
        http(b, "GET", "/api/v1/pair/0123", &[], None).await.status,
        404,
        "an unknown pairing"
    );
    let w = waiting_at(a, &t.key_a, 1).await;
    let req = &w[0];
    assert_eq!(req["client"], "Oriel – Stage laptop", "{w}");
    assert_eq!(req["via"], "BOX-B (Vesper)", "{w}");
    assert_eq!(req["relayed"], true, "{w}");
    assert!(
        req["viaFingerprint"].is_string() && req["askedAt"].is_string(),
        "{w}"
    );
    let id = req["id"].as_str().unwrap().to_string();
    assert!(id.starts_with('k'), "{id}");
    assert_eq!(
        get_json_auth(b, "/api/v1/site/pairings", admin).await,
        json!([]),
        "answered at the site's server, not at the relay"
    );

    // Approved at A: the key comes back through B, once.
    let r = call(
        a,
        "POST",
        &format!("/api/v1/site/pairings/{id}/approve"),
        admin,
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let r = pairing_answer(b, &secret).await;
    let p = ok(&r, 200, "approved");
    let key = p["token"].as_str().unwrap().to_string();
    assert_eq!(token::peek(&key).unwrap().name, "Oriel – Stage laptop");
    assert_eq!(p["clientId"], key_id(&key));
    assert_eq!(
        http(b, "GET", &format!("/api/v1/pair/{secret}"), &[], None)
            .await
            .status,
        404,
        "given once"
    );
    assert_eq!(waiting_at(a, &t.key_a, 0).await, json!([]));
    for (addr, which) in [(a, "A"), (b, "B")] {
        let r = call(addr, "GET", "/api/v1/state", Some(&key), None).await;
        assert_eq!(r.status, 200, "the relayed key on {which}");
    }
    let keys = get_json_auth(a, "/api/v1/site/api-keys", admin).await;
    let k = keys
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["id"] == key_id(&key))
        .unwrap_or_else(|| panic!("listed: {keys}"));
    assert_eq!(k["name"], "Oriel – Stage laptop");
    assert_eq!(k["engine"], "app:vesper");
    assert_eq!(k["scope"], "control");
    assert_eq!(k["revoked"], false);
    assert!(
        k["by"].as_str().unwrap().contains("via BOX-B (Vesper)"),
        "{k}"
    );

    // The relay removed from the site: the key it relayed still works at A.
    let fp = t.b.shared.site.membership().fingerprint.unwrap();
    ok(
        &call(
            a,
            "DELETE",
            &format!("/api/v1/site/devices/{fp}"),
            admin,
            None,
        )
        .await,
        204,
        "remove B",
    );
    assert_eq!(
        call(a, "GET", "/api/v1/state", Some(&key), None)
            .await
            .status,
        200,
        "keys outlive the relay"
    );
    t.end().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relayed_pairing_denied_and_the_relay_disconnecting() {
    let mut t = two_boxes("site-relay-deny").await;
    let (a, b) = (t.a.addr, t.b.addr);
    let admin = Some(t.key_a.as_str());
    let mut ws_a = Client::connect(a, &format!("protocolVersion=1&token={}", t.key_a)).await;

    // A wrong code typed at the relay: B doesn't judge it, the site's server finds it wrong,
    // and the pairing waits for an admin's approval like one without a code; the client is
    // told the code was wrong (it may type it again).
    let code = relay_code(&t.b).await.code;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let secret = pair_waiting(b, "Tablet", Some(wrong)).await;
    says_code_wrong(b, &secret).await;
    let asked = site_event(&mut ws_a, "key_requested").await;
    assert_eq!(asked["machine"], "BOX-B (Vesper)", "{asked}");
    assert_eq!(asked["detail"]["codeOk"], false, "{asked}");
    let id = asked["detail"]["pairing"].as_str().unwrap().to_string();
    assert_eq!(waiting_at(a, &t.key_a, 1).await[0]["id"], id.as_str());
    let r = http(b, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
    assert_eq!(r.status, 202, "not paired by a wrong code");
    // Not a pairing id.
    let r = call(a, "POST", "/api/v1/site/pairings/x1/deny", admin, None).await;
    assert_eq!(r.status, 400);
    let r = call(
        a,
        "POST",
        &format!("/api/v1/site/pairings/{id}/deny"),
        admin,
        Some(json!({"reason": "not now"})),
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let r = pairing_answer(b, &secret).await;
    assert_eq!(r.status, 403, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["error"]["code"], "forbidden");
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not now"),
        "{}",
        r.json()
    );
    let r = call(
        a,
        "POST",
        &format!("/api/v1/site/pairings/{id}/approve"),
        admin,
        None,
    )
    .await;
    assert_eq!(r.status, 404, "answered already");

    // The relay goes away before an answer: the request ends at A.
    let _secret = pair_waiting(b, "Booth", None).await;
    let w = waiting_at(a, &t.key_a, 1).await;
    let id = w[0]["id"].as_str().unwrap().to_string();
    t.b.shutdown().await.unwrap();
    let ended = site_event(&mut ws_a, "key_request_cancelled").await;
    assert_eq!(ended["detail"]["pairing"], id, "{ended}");
    assert_eq!(waiting_at(a, &t.key_a, 0).await, json!([]));
    let r = call(
        a,
        "POST",
        &format!("/api/v1/site/pairings/{id}/approve"),
        admin,
        None,
    )
    .await;
    assert_eq!(r.status, 404, "{}", String::from_utf8_lossy(&r.body));
    drop(ws_a);
    // (B back, for the end.)
    t.b = start_with(&t.fb, false, SiteConfig::loopback()).await;
    t.end().await;
}

/// The client takes a waiting pairing back (`DELETE /api/v1/pair/{pairingId}`, no token: the id
/// is the secret), as Oriel does when the operator goes back or types the code again: one
/// waiting at the site's server ends there (`key_request_cancelled`); a relayed one (a wrong
/// code at the relay) is taken back at the site's server too. Unknown or answered: `404`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_waiting_pairing_is_taken_back_by_its_client() {
    let t = two_boxes("site-pair-withdraw").await;
    let (a, b) = (t.a.addr, t.b.addr);
    let admin = Some(t.key_a.as_str());
    let mut ws_a = Client::connect(a, &format!("protocolVersion=1&token={}", t.key_a)).await;
    let delete = |addr: SocketAddr, secret: String| async move {
        http(addr, "DELETE", &format!("/api/v1/pair/{secret}"), &[], None)
            .await
            .status
    };

    // Waiting at the site's server.
    let secret = pair_waiting(a, "Laptop", None).await;
    let w = waiting_at(a, &t.key_a, 1).await;
    let id = w[0]["id"].as_str().unwrap().to_string();
    assert_eq!(delete(a, secret.clone()).await, 204);
    let ended = site_event(&mut ws_a, "key_request_cancelled").await;
    assert_eq!(ended["detail"]["pairing"], id.as_str(), "{ended}");
    assert_eq!(waiting_at(a, &t.key_a, 0).await, json!([]));
    let r = http(a, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
    assert_eq!(r.status, 404, "ended");
    assert_eq!(delete(a, secret).await, 404, "once");

    // Relayed, with a wrong code: it waits at the site's server; taken back, it is gone there.
    let code = relay_code(&t.b).await.code;
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let secret = pair_waiting(b, "Tablet", Some(wrong)).await;
    says_code_wrong(b, &secret).await;
    assert_eq!(
        waiting_at(a, &t.key_a, 1).await[0]["client"],
        "Oriel – Tablet"
    );
    assert_eq!(delete(b, secret.clone()).await, 204);
    assert_eq!(
        waiting_at(a, &t.key_a, 0).await,
        json!([]),
        "gone at the site's server"
    );
    let r = http(b, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
    assert_eq!(r.status, 404);
    assert_eq!(delete(b, secret).await, 404);

    // Answered already: the answer stays to be fetched.
    let secret = pair_waiting(a, "Booth", None).await;
    let w = waiting_at(a, &t.key_a, 1).await;
    let id = w[0]["id"].as_str().unwrap().to_string();
    let r = call(
        a,
        "POST",
        &format!("/api/v1/site/pairings/{id}/approve"),
        admin,
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(delete(a, secret.clone()).await, 404, "answered");
    let r = pairing_answer(a, &secret).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(delete(a, "nothing-like-this".into()).await, 404);
    drop(ws_a);
    t.end().await;
}

/// A relayed pairing whose client stopped asking ends at the relay and is withdrawn at the
/// site's server too (D05-4): not left listed there for an admin to approve for no one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relayed_pairing_whose_client_went_is_withdrawn_at_the_sites_server() {
    let t = two_boxes_site(
        "site-relay-idle",
        Default::default(),
        Default::default(),
        |_| {},
        |_| {},
        SiteConfig {
            pairing_idle: Duration::from_millis(1500),
            ..SiteConfig::loopback()
        },
    )
    .await;
    let b = t.b.addr;
    let secret = pair_waiting(b, "Laptop", None).await;
    assert_eq!(
        waiting_at(t.a.addr, &t.key_a, 1).await[0]["client"],
        "Oriel – Laptop"
    );
    // The client stops asking: gone at the relay, then at the site's server.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let r = http(b, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
    assert_eq!(r.status, 404, "ended at the relay");
    assert_eq!(waiting_at(t.a.addr, &t.key_a, 0).await, json!([]));
    t.end().await;
}

/// The right code typed at a relay pairs at once: the relay shows the code the site's Site
/// Manager made for it, passes the typed code on, and the site's server approves the pairing
/// without anyone's click (a relayed key: 10 years, even for a browser).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_right_code_at_a_relay_pairs_at_once() {
    let t = two_boxes("site-relay-code").await;
    let (a, b) = (t.a.addr, t.b.addr);
    let admin = Some(t.key_a.as_str());
    let mut ws_a = Client::connect(a, &format!("protocolVersion=1&token={}", t.key_a)).await;
    let code = relay_code(&t.b).await;
    assert!(
        code.code.len() == 6 && code.code.bytes().all(|c| c.is_ascii_digit()),
        "{code:?}"
    );
    assert!(
        (290..=300).contains(&code.expires_in_seconds),
        "the site's codes last 5 minutes: {code:?}"
    );
    let r = pair_with(b, "Stage laptop", &code.code, true).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let key = r.json()["token"].as_str().unwrap().to_string();
    assert_eq!(r.json()["clientId"], key_id(&key));
    for (addr, which) in [(a, "A"), (b, "B")] {
        let r = call(addr, "GET", "/api/v1/state", Some(&key), None).await;
        assert_eq!(r.status, 200, "on {which}");
    }
    let asked = site_event(&mut ws_a, "key_requested").await;
    assert_eq!(asked["detail"]["codeOk"], true, "{asked}");
    assert_eq!(waiting_at(a, &t.key_a, 0).await, json!([]), "nothing waits");
    let keys = get_json_auth(a, "/api/v1/site/api-keys", admin).await;
    let k = keys
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["id"] == key_id(&key))
        .unwrap_or_else(|| panic!("listed: {keys}"));
    assert_eq!(k["name"], "Oriel – Stage laptop", "{k}");
    assert_eq!(k["scope"], "control", "{k}");
    assert!(
        k["by"].as_str().unwrap().contains("via BOX-B (Vesper)"),
        "{k}"
    );
    // 10 years although the client said it is a browser: a relayed request can't say so.
    let made = token::peek(&key).unwrap();
    let years = (made.exp - made.iat) as f64 / (365.25 * 86_400.0);
    assert!(years > 9.0, "a relayed key lives 10 years: {years}");
    // The Site Manager audited the approval, with the machine it came through.
    let audit = get_json_auth(a, "/api/v1/site/audit", admin).await;
    assert!(
        audit_has(&audit, "key_request/approve", |e| e["ok"] == true),
        "{audit}"
    );
    drop(ws_a);
    t.end().await;
}

/// A relay's code works once: the relay shows the site's next one at once, and the used one
/// typed again only waits for an admin's approval.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relay_code_is_single_use() {
    let t = two_boxes("site-relay-once").await;
    let (a, b) = (t.a.addr, t.b.addr);
    let used = relay_code(&t.b).await.code;
    let r = pair_with(b, "Booth", &used, false).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let next = relay_code(&t.b).await.code;
    assert_ne!(next, used, "the relay shows the site's next code");
    let secret = pair_waiting(b, "Booth again", Some(&used)).await;
    says_code_wrong(b, &secret).await;
    let w = waiting_at(a, &t.key_a, 1).await;
    assert_eq!(w[0]["client"], "Oriel – Booth again", "{w}");
    assert_eq!(
        http(b, "GET", &format!("/api/v1/pair/{secret}"), &[], None)
            .await
            .status,
        202,
        "a used code doesn't pair"
    );
    // The next code works.
    let r = pair_with(b, "Booth third", &next, false).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    t.end().await;
}

/// A relay's code ends (after 5 minutes, the site's rule; here shortened to 3 seconds): the
/// relay shows the site's next one, and the old one doesn't pair any more.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relay_code_is_refreshed_once_it_ends() {
    let t = two_boxes("site-relay-expiry").await;
    let (a, b) = (t.a.addr, t.b.addr);
    t.a.shared
        .site
        .handle()
        .unwrap()
        .authority()
        .set_pairing_code_life(Duration::from_secs(3));
    // The code the relay shows now was made with the long life: used up, the next is short.
    let first = relay_code(&t.b).await;
    let r = pair_with(b, "Early", &first.code, false).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let deadline = Instant::now() + Duration::from_secs(10);
    let old = loop {
        let c = relay_code(&t.b).await;
        if c.code != first.code {
            break c;
        }
        assert!(Instant::now() < deadline, "no next code");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(old.expires_in_seconds <= 5, "a short code: {old:?}");
    // Ended: the relay asks the site again (at least 5 s after it was given) and shows the next.
    let deadline = Instant::now() + Duration::from_secs(20);
    let new = loop {
        let c = relay_code(&t.b).await;
        if c.code != old.code {
            break c;
        }
        assert!(
            Instant::now() < deadline,
            "the relay kept showing the old code"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_ne!(new.code, old.code, "a new code once the old one ended");
    let secret = pair_waiting(b, "Late", Some(&old.code)).await;
    says_code_wrong(b, &secret).await;
    waiting_at(a, &t.key_a, 1).await;
    let r = pair_with(b, "On time", &new.code, false).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    t.end().await;
}

/// Keys Vesper recorded as made for Oriel pairings that are never used are revoked a day after
/// they were made (here: 2 seconds), audited; a key in use, and keys an admin made by hand in the
/// Site Manager (even one named `Oriel – …`: only recorded keys are swept), stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_keys_never_used_are_revoked_after_a_day() {
    let fx = fixture("site-unused-keys");
    let server = start_with(
        &fx,
        false,
        SiteConfig {
            unused_key_life: Duration::from_secs(2),
            unused_key_sweep: Duration::from_millis(200),
            ..SiteConfig::loopback()
        },
    )
    .await;
    let addr = server.addr;
    let used = pair(&server, "Booth").await;
    let k = Some(used.as_str());
    assert_eq!(
        call(addr, "GET", "/api/v1/state", k, None).await.status,
        200
    );
    let unused = key_id(&pair(&server, "Forgotten tablet").await);
    // Keys an admin made by hand in the Site Manager (never used either), one of them named
    // as Vesper names its pairing keys.
    let by_hand = |name: &str| {
        server
            .shared
            .site
            .handle()
            .unwrap()
            .run_from(
                "admin",
                &idn_site_manager::host::Caller {
                    address: String::new(),
                    client: "Site Manager".into(),
                },
                &idn_site_manager::typed::SiteCommand::ApiKey {
                    name: name.into(),
                    aud: idn_site_manager::typed::TokenAud {
                        engine: "app:vesper".into(),
                        machine: None,
                    },
                    scope: "control".into(),
                    days: None,
                },
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let hand = by_hand("Lighting desk");
    let named = by_hand("Oriel – Hand made");
    let find = |keys: &Value, id: &str| {
        keys.as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} listed: {keys}"))
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let keys = loop {
        let keys = get_json_auth(addr, "/api/v1/site/api-keys", k).await;
        if find(&keys, &unused)["revoked"] == true {
            break keys;
        }
        assert!(Instant::now() < deadline, "the unused key stays: {keys}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(find(&keys, &key_id(&used))["revoked"], false, "in use");
    assert_eq!(find(&keys, &hand)["revoked"], false, "an admin's own key");
    assert_eq!(
        find(&keys, &named)["revoked"],
        false,
        "an admin's own key named `Oriel – …`, not recorded"
    );
    assert_eq!(
        call(addr, "GET", "/api/v1/state", k, None).await.status,
        200
    );
    // The sweep's audit entry may land a moment after the revocation shows (under load):
    // poll for it as for the revocation.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let audit = get_json_auth(addr, "/api/v1/site/audit", k).await;
        if audit_has(&audit, "api_key/revoke_unused", |e| {
            e["ok"] == true
                && e["detail"]["id"] == unused.as_str()
                && e["client"] == "server machine"
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "no audit entry: {audit}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // Later sweeps leave the others alone.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let keys = get_json_auth(addr, "/api/v1/site/api-keys", k).await;
    assert_eq!(find(&keys, &hand)["revoked"], false);
    assert_eq!(find(&keys, &named)["revoked"], false);
    assert_eq!(find(&keys, &key_id(&used))["revoked"], false);
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}
