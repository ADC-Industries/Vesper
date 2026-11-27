//! The site: IDN's Site Manager running as a module of the server, on loopback ports.
//! Machines are IDN `Enrollment`s in temporary folders (the same library every IDN program
//! joins a site with), talking to the server's discovery server on 127.0.0.1.
//!
//! - joining in `code_or_approval` (a typed code joins at once; without one, the approval
//!   window and approve, under a name given there, / deny), Remove and allow again, the typed events on the WebSocket
//!   and the audit with who and where;
//! - `code_and_approval` (a right code still waits for approval), wrong codes counting down
//!   to the burn, a code taken back, the settings' `If-Match`;
//! - the access levels: `open` (every paired client is the site's admin), `pin` (unlock with
//!   the admin PIN; idle re-lock, lock on demand, lockout after wrong PINs), `users` (users,
//!   roles, sign-in per paired client, engine tokens per scope and role), switching between
//!   them and their audit (the existing pairing is used to get the clients);
//! - turning the module off and on, and a restart keeping the site.
//!
//! The code's 10-minute expiry is checked by an ignored test (`--ignored`): it waits it out.
//!
//! Paired clients' site keys across several Vesper servers of one site, joining another
//! server's site and leaving it: [`keys`]. The site's server marking its Vesper boxes pairing
//! relays: [`relays`]. Mosaic's display boxes and channels against a fake display service:
//! [`mosaic`].

mod codex_member;
mod hardening;
pub(crate) mod keys;
mod mosaic;
mod relays;

use crate::common::*;
use idn_open_protocol::trust::{DeviceRole, Enrollment, EnrollmentConfig, TrustState};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vesper_server::site::SiteConfig;
use vesper_server::{RunningServer, ServerConfig, start};

async fn start_site(fx: &Fixture, trust_localhost: bool) -> RunningServer {
    start_with(fx, trust_localhost, SiteConfig::loopback()).await
}

async fn start_with(fx: &Fixture, trust_localhost: bool, site: SiteConfig) -> RunningServer {
    start_with_codex(fx, trust_localhost, site, Default::default()).await
}

/// [`start_with`] with the Codex's options.
pub(crate) async fn start_with_codex(
    fx: &Fixture,
    trust_localhost: bool,
    site: SiteConfig,
    codex: vesper_server::codex::CodexOptions,
) -> RunningServer {
    start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost,
        mdns: false,
        site,
        codex,
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts")
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

/// `PUT /api/v1/site/settings` with the current join mode.
async fn set_access(addr: SocketAddr, auth: Option<&str>, access: &str, pin: Option<&str>) -> Resp {
    let mut body = json!({"enabled": true, "joinMode": "code_or_approval", "access": access});
    if let Some(p) = pin {
        body["pin"] = json!(p);
    }
    call(addr, "PUT", "/api/v1/site/settings", auth, Some(body)).await
}

/// A machine asking to join the site at `ds_port`.
fn machine(fx: &Fixture, ds_port: u16, name: &str) -> Arc<Enrollment> {
    Enrollment::start(EnrollmentConfig {
        discovery_server: format!("127.0.0.1:{ds_port}"),
        state_dir: fx.dir.join("machines").join(name),
        name: format!("{name} (Test)"),
        role: DeviceRole::App,
        passphrase: None,
        admin_code: None,
    })
    .expect("enrollment starts")
}

/// Waits (off the async threads) until the machine's trust state matches.
async fn until(m: &Arc<Enrollment>, what: &str, pred: fn(&TrustState) -> bool) -> TrustState {
    let m = m.clone();
    let what = what.to_string();
    tokio::task::spawn_blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let s = m.state();
            if pred(&s) {
                return s;
            }
            assert!(Instant::now() < deadline, "{what}: stuck in {s:?}");
            let _ = m.recv(Duration::from_millis(100));
        }
    })
    .await
    .unwrap()
}

pub(crate) async fn call(
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

pub(crate) fn ok(r: &Resp, status: u16, what: &str) -> Value {
    assert_eq!(
        r.status,
        status,
        "{what}: {}",
        String::from_utf8_lossy(&r.body)
    );
    r.json()
}

/// The next `siteEvent` named `name` (others are kept in `client.events`).
async fn site_event(c: &mut Client, name: &str) -> Value {
    if let Some(i) = c
        .events
        .iter()
        .position(|m| m["type"] == "siteEvent" && m["event"]["name"] == name)
    {
        return c.events.remove(i)["event"].clone();
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(
            Instant::now() < deadline,
            "no {name} event; seen: {:?}",
            c.events
                .iter()
                .filter(|m| m["type"] == "siteEvent")
                .map(|m| m["event"]["name"].clone())
                .collect::<Vec<_>>()
        );
        let m = c.next_any().await;
        if m["type"] == "siteEvent" && m["event"]["name"] == name {
            return m["event"].clone();
        }
        if m.get("binary").is_none() {
            c.absorb(&m);
        }
    }
}

fn audit_has(audit: &Value, action: &str, pred: impl Fn(&Value) -> bool) -> bool {
    audit
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["action"] == action && pred(e))
}

fn has_site_ca(dir: &Path) -> bool {
    dir.join("machine/site-manager/ca.key").is_file()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_or_approval_joins_removes_and_audits() {
    let fx = fixture("site-join");
    let server = start_site(&fx, true).await;
    let addr = server.addr;

    let st = get_json(addr, "/api/v1/site").await;
    assert_eq!(st["enabled"], true);
    assert_eq!(st["running"], true, "{st}");
    assert_eq!(st["joinMode"], "code_or_approval", "the default");
    assert_eq!(st["pairingOpen"], false);
    assert_eq!(
        st["you"]["user"], "admin",
        "the server machine acts as admin"
    );
    assert_eq!(st["you"]["role"], "admin");
    assert_eq!(st["siteId"].as_str().unwrap().len(), 16);
    let ds = st["discoveryPort"].as_u64().unwrap() as u16;
    assert!(st["apiPort"].as_u64().is_some());
    assert!(
        has_site_ca(&fx.dir),
        "the site CA lives in the server's machine folder"
    );
    let info = get_json(addr, "/api/v1/server").await;
    assert!(
        info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("site"))
    );
    // Only the box's own Codex is a member yet (see `codex_member`).
    let devs = get_json(addr, "/api/v1/site/devices").await;
    assert_eq!(devs.as_array().unwrap().len(), 1, "{devs}");
    assert_eq!(devs[0]["name"], "Codex", "{devs}");
    assert_eq!(
        call(addr, "GET", "/api/v1/site/join-code", None, None)
            .await
            .status,
        404,
        "no site code yet"
    );

    let mut ws = Client::connect(addr, "protocolVersion=1").await;

    // A site code: six digits shown "xxx xxx", 10 minutes, 5 tries.
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "new code",
    );
    let shown = code["code"].as_str().unwrap().to_string();
    assert!(
        shown.len() == 7
            && shown.as_bytes()[3] == b' '
            && shown.replace(' ', "").chars().all(|c| c.is_ascii_digit()),
        "{shown}"
    );
    let left = code["expiresInSeconds"].as_u64().unwrap();
    assert!((590..=600).contains(&left), "{left}");
    assert_eq!(code["triesLeft"], 5);
    assert_eq!(code["issuer"], "admin");
    assert_eq!(get_json(addr, "/api/v1/site/join-code").await, code);
    let issued = site_event(&mut ws, "join_code_issued").await;
    assert_eq!(issued["codeId"], code["id"]);

    // A machine that types it joins at once.
    let one = machine(&fx, ds, "BOX-1");
    one.join_with_code(&shown).unwrap();
    until(&one, "BOX-1 joins by code", |s| {
        matches!(s, TrustState::Enrolled { .. })
    })
    .await;
    let used = site_event(&mut ws, "join_code_used").await;
    assert_eq!(used["codeId"], code["id"]);
    let joined = site_event(&mut ws, "device_joined").await;
    assert_eq!(joined["fingerprint"], one.fingerprint());
    assert_eq!(joined["machine"], "BOX-1 (Test)");
    // (BOX-1's: the Codex's own may come too.)
    while site_event(&mut ws, "device_online").await["fingerprint"] != one.fingerprint() {}
    let devs = get_json(addr, "/api/v1/site/devices").await;
    assert_eq!(devs[0]["fingerprint"], one.fingerprint());
    assert_eq!(devs[0]["health"], "online", "{devs}");
    assert_eq!(devs[0]["node"], false);
    // The Site Manager's health: nothing needs attention, no Node to report a build.
    assert_eq!(devs[0]["problems"], json!([]), "{devs}");
    assert_eq!(devs[0]["restarting"], json!([]));
    assert!(devs[0].get("version").is_none() && devs[0].get("updating").is_none());
    assert!(devs[0]["lastSeen"].is_string());
    assert_eq!(
        call(addr, "GET", "/api/v1/site/join-code", None, None)
            .await
            .status,
        404,
        "single use"
    );

    // Without a code it needs the approval window, then an operator.
    let two = machine(&fx, ds, "BOX-2");
    until(&two, "BOX-2 waits for the window", |s| {
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
    two.retry_now();
    let TrustState::Pending { code: compare } = until(&two, "BOX-2 asks", |s| {
        matches!(s, TrustState::Pending { .. })
    })
    .await
    else {
        unreachable!()
    };
    let asked = site_event(&mut ws, "join_requested").await;
    assert_eq!(asked["fingerprint"], two.fingerprint());
    assert_eq!(asked["address"], "127.0.0.1");
    let reqs = get_json(addr, "/api/v1/site/join-requests").await;
    assert_eq!(reqs.as_array().unwrap().len(), 1, "{reqs}");
    assert_eq!(
        reqs[0]["code"],
        idn_open_protocol::trust::show_code(&compare),
        "both screens show the same code"
    );
    assert_eq!(reqs[0]["byCode"], false);
    assert_eq!(get_json(addr, "/api/v1/site").await["pairingOpen"], true);
    let id = reqs[0]["id"].as_u64().unwrap();
    assert_eq!(asked["requestId"], id);
    assert_eq!(
        call(
            addr,
            "POST",
            "/api/v1/site/join-requests/999/approve",
            None,
            None
        )
        .await
        .status,
        404
    );
    // Approved under another name than the one it asked with: its certificate carries it,
    // so the site's device list and the machine itself show it.
    ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/site/join-requests/{id}/approve"),
            None,
            Some(json!({"name": "Front of House"})),
        )
        .await,
        204,
        "approve",
    );
    until(&two, "BOX-2 approved", |s| {
        matches!(s, TrustState::Enrolled { .. })
    })
    .await;
    let joined = site_event(&mut ws, "device_joined").await;
    assert_eq!(joined["fingerprint"], two.fingerprint());
    assert_eq!(two.name_in_site().as_deref(), Some("Front of House"));
    let d = get_json(addr, &format!("/api/v1/site/devices/{}", two.fingerprint())).await;
    assert_eq!(d["name"], "Front of House", "the approved name: {d}");
    let devs = get_json(addr, "/api/v1/site/devices").await;
    assert!(
        devs.as_array()
            .unwrap()
            .iter()
            .any(|x| x["fingerprint"] == two.fingerprint() && x["name"] == "Front of House"),
        "{devs}"
    );
    // A machine without an IDN Node reports no services.
    assert_eq!(d["services"], json!([]), "{d}");

    // A third is denied.
    let three = machine(&fx, ds, "BOX-3");
    until(&three, "BOX-3 asks", |s| {
        matches!(s, TrustState::Pending { .. })
    })
    .await;
    let asked = site_event(&mut ws, "join_requested").await;
    let id = asked["requestId"].as_u64().unwrap();
    ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/site/join-requests/{id}/deny"),
            None,
            None,
        )
        .await,
        204,
        "deny",
    );
    until(&three, "BOX-3 denied", |s| matches!(s, TrustState::Denied)).await;
    let denied = site_event(&mut ws, "join_denied").await;
    assert_eq!(denied["fingerprint"], three.fingerprint());
    assert_eq!(denied["level"], "warning");
    assert!(
        !ws.events
            .iter()
            .any(|m| m["event"]["name"] == "join_cancelled"),
        "a denied request isn't 'cancelled'"
    );
    ok(
        &call(addr, "DELETE", "/api/v1/site/pairing", None, None).await,
        204,
        "close window",
    );

    // Remove BOX-1: it is cut off and listed as removed; allowing it lets it ask again.
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
        "remove",
    );
    until(&one, "BOX-1 revoked", |s| matches!(s, TrustState::Revoked)).await;
    let revoked = site_event(&mut ws, "device_revoked").await;
    assert_eq!(revoked["fingerprint"], fp);
    let d = get_json(addr, &format!("/api/v1/site/devices/{fp}")).await;
    assert_eq!(d["health"], "removed");
    assert!(d["lastSeen"].is_string(), "{d}");
    assert_eq!(
        call(
            addr,
            "DELETE",
            &format!("/api/v1/site/devices/{fp}"),
            None,
            None
        )
        .await
        .status,
        409,
        "already removed"
    );
    ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/site/devices/{fp}/allow"),
            None,
            None,
        )
        .await,
        204,
        "allow again",
    );
    // Identify needs an IDN Node on the machine: refused with the reason.
    let fp2 = two.fingerprint();
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/site/devices/{fp2}/identify"),
        None,
        None,
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(
        call(addr, "GET", "/api/v1/site/devices/abc", None, None)
            .await
            .status,
        404
    );

    // The audit: every change with who, from where, and the machines' own entries.
    let audit = get_json(addr, "/api/v1/site/audit?n=100").await;
    for action in [
        "join_code",
        "pairing",
        "approve",
        "deny",
        "close",
        "revoke",
        "clear",
    ] {
        assert!(
            audit_has(&audit, action, |e| e["user"] == "admin"
                && e["peer"] == "127.0.0.1"
                && e["client"] == "server machine"
                && e["source"] == "siteManager"
                && e["ok"] == true),
            "{action} in {audit:#}"
        );
    }
    assert!(audit_has(&audit, "identify", |e| e["ok"] == false));
    assert!(
        audit_has(&audit, "join_code/use", |e| e["source"] == "siteManager"
            && e["user"] == ""),
        "the machine's code use, from the Site Manager's audit: {audit:#}"
    );
    let ats: Vec<&str> = audit
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["at"].as_str().unwrap())
        .collect();
    assert!(ats.windows(2).all(|w| w[0] >= w[1]), "newest first");
    let recent = get_json(addr, "/api/v1/site/events?n=5").await;
    assert_eq!(recent.as_array().unwrap().len(), 5);
    assert!(
        ws.events.iter().any(|m| m["type"] == "siteChanged"
            && m["parts"].as_array().unwrap().contains(&json!("devices"))),
        "siteChanged for the device list"
    );
    drop((one, two, three));
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_and_approval_wrong_codes_and_lockout() {
    let fx = fixture("site-codes");
    let server = start_site(&fx, true).await;
    let addr = server.addr;
    let ds = get_json(addr, "/api/v1/site").await["discoveryPort"]
        .as_u64()
        .unwrap() as u16;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;

    // The settings: If-Match guards them.
    let r = call(addr, "GET", "/api/v1/site/settings", None, None).await;
    let settings = ok(&r, 200, "settings");
    let etag = r.header("etag").unwrap().to_string();
    assert_eq!(
        settings["revision"].as_str().unwrap(),
        etag.trim_matches('"')
    );
    let stale = http(
        addr,
        "PUT",
        "/api/v1/site/settings",
        &[("If-Match", "\"0000\"")],
        Some(json!({"enabled": true, "joinMode": "code_and_approval", "access": "open"})),
    )
    .await;
    assert_eq!(stale.status, 409);
    assert_eq!(stale.json()["current"]["joinMode"], "code_or_approval");
    let bad = http(
        addr,
        "PUT",
        "/api/v1/site/settings",
        &[("If-Match", etag.as_str())],
        Some(json!({"enabled": true, "joinMode": "whenever", "access": "open"})),
    )
    .await;
    assert_eq!(bad.status, 400);
    let r = http(
        addr,
        "PUT",
        "/api/v1/site/settings",
        &[("If-Match", etag.as_str())],
        Some(json!({"enabled": true, "joinMode": "code_and_approval", "access": "open"})),
    )
    .await;
    assert_eq!(ok(&r, 200, "join mode")["joinMode"], "code_and_approval");
    assert_ne!(r.header("etag").unwrap(), etag);
    assert_eq!(
        get_json(addr, "/api/v1/site").await["joinMode"],
        "code_and_approval"
    );

    // A right code still waits for approval.
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    let one = machine(&fx, ds, "BOX-1");
    one.join_with_code(code["code"].as_str().unwrap()).unwrap();
    until(&one, "BOX-1 code accepted", |s| {
        matches!(s, TrustState::CodeAccepted)
    })
    .await;
    let asked = site_event(&mut ws, "join_requested").await;
    let reqs = get_json(addr, "/api/v1/site/join-requests").await;
    assert_eq!(reqs[0]["byCode"], true, "{reqs}");
    assert!(reqs[0].get("code").is_none(), "no comparison code by code");
    ok(
        &call(
            addr,
            "POST",
            &format!("/api/v1/site/join-requests/{}/approve", asked["requestId"]),
            None,
            Some(json!({"name": "Booth"})),
        )
        .await,
        204,
        "approve",
    );
    until(&one, "BOX-1 in", |s| {
        matches!(s, TrustState::Enrolled { .. })
    })
    .await;

    // Wrong codes count the site code's tries down; the fifth burns it.
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    let right = code["code"].as_str().unwrap().replace(' ', "");
    let wrong = if right == "000000" {
        "000001"
    } else {
        "000000"
    };
    let mut tries = Vec::new();
    for k in 0..5u32 {
        let m = machine(&fx, ds, &format!("GUESS-{k}"));
        m.join_with_code(wrong).unwrap();
        let s = until(&m, "a wrong code", |s| {
            matches!(s, TrustState::WrongCode { .. })
        })
        .await;
        assert_eq!(s, TrustState::WrongCode { left: 4 - k });
        let e = site_event(&mut ws, "join_code_wrong").await;
        assert_eq!(e["triesLeft"], 4 - k, "{e}");
        assert_eq!(e["address"], "127.0.0.1");
        if k < 4 {
            assert_eq!(
                get_json(addr, "/api/v1/site/join-code").await["triesLeft"],
                4 - k
            );
        }
        tries.push(m);
    }
    let burned = site_event(&mut ws, "join_code_burned").await;
    assert_eq!(burned["codeId"], code["id"]);
    assert_eq!(get_json(addr, "/api/v1/site/join-codes").await, json!([]));
    assert_eq!(
        call(addr, "GET", "/api/v1/site/join-code", None, None)
            .await
            .status,
        404
    );

    // A code taken back can't be used: the machine is told the site has no code waiting.
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    // A newer site code takes the older one back (one code shown at a time).
    let newer = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "newer code",
    );
    let cancelled = site_event(&mut ws, "join_code_cancelled").await;
    assert_eq!(cancelled["codeId"], code["id"]);
    let waiting = get_json(addr, "/api/v1/site/join-codes").await;
    assert_eq!(waiting.as_array().unwrap().len(), 1, "{waiting}");
    assert_eq!(waiting[0]["id"], newer["id"]);
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/site/join-codes/{}", newer["id"]),
            None,
            None,
        )
        .await,
        204,
        "take back",
    );
    site_event(&mut ws, "join_code_cancelled").await;
    let late = machine(&fx, ds, "LATE");
    late.join_with_code(code["code"].as_str().unwrap()).unwrap();
    until(&late, "a code no longer waiting", |s| {
        matches!(s, TrustState::CodeExpired)
    })
    .await;

    let audit = get_json(addr, "/api/v1/site/audit").await;
    assert!(audit_has(&audit, "option", |e| e["detail"]["value"]
        == "code_and_approval"
        && e["user"] == "admin"));
    assert!(audit_has(&audit, "join_code/wrong", |e| e["source"]
        == "siteManager"
        && e["peer"] == "127.0.0.1"));
    drop((one, tries, late));
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn users_roles_sessions_and_tokens() {
    let fx = fixture("site-users");
    // Remote-style clients: they pair with the existing pairing code first.
    let server = start_site(&fx, false).await;
    let addr = server.addr;
    let a = pair(&server, "Booth").await;
    let b = pair(&server, "Tablet").await;
    let (a, b) = (Some(a.as_str()), Some(b.as_str()));
    assert_eq!(
        call(addr, "GET", "/api/v1/site", None, None).await.status,
        401,
        "the site API needs pairing like the rest"
    );

    // Access "open" (the default): a paired client acts as admin and adds the users.
    let s = ok(
        &call(addr, "GET", "/api/v1/site/session", a, None).await,
        200,
        "session",
    );
    assert_eq!(
        s,
        json!({"access": "open", "user": "admin", "role": "admin", "signedIn": false, "unlocked": false})
    );
    for (name, role) in [("alice", "operator"), ("vic", "viewer")] {
        let r = call(
            addr,
            "PUT",
            &format!("/api/v1/site/users/{name}"),
            a,
            Some(json!({"role": role, "password": format!("{name}-password")})),
        )
        .await;
        let u = ok(&r, 200, "add user");
        assert_eq!(u["role"], role);
        assert_eq!(r.header("etag").unwrap().trim_matches('"'), u["revision"]);
    }
    // The Site Manager's rules: names of letters, digits and . _ - @; passwords of 8 or more.
    for (name, password, why) in [
        ("a%20b", "long-enough-1", "a space in the name"),
        ("a%2Fb", "long-enough-1", "a slash in the name"),
        ("shorty", "short1", "a 6-character password"),
    ] {
        let r = call(
            addr,
            "PUT",
            &format!("/api/v1/site/users/{name}"),
            a,
            Some(json!({"role": "viewer", "password": password})),
        )
        .await;
        assert_eq!(r.status, 400, "{why}: {}", String::from_utf8_lossy(&r.body));
        let msg = r.json()["error"]["message"].as_str().unwrap().to_string();
        eprintln!("{why}: {msg}");
        assert!(!msg.is_empty() && !msg.contains("Error {"), "{why}: {msg}");
    }
    let r = call(
        addr,
        "PUT",
        "/api/v1/site/password",
        a,
        Some(json!({"password": "short12"})),
    )
    .await;
    assert_eq!(r.status, 400, "a 7-character site password");
    eprintln!("site password: {}", r.json()["error"]["message"]);
    let r = set_access(addr, a, "users", None).await;
    assert_eq!(r.status, 409, "access \"users\" needs an admin user");
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/settings", a).await["access"],
        "open"
    );
    ok(
        &call(
            addr,
            "PUT",
            "/api/v1/site/users/root",
            a,
            Some(json!({"role": "admin", "password": "root-password"})),
        )
        .await,
        200,
        "add an admin",
    );
    assert_eq!(
        call(
            addr,
            "POST",
            "/api/v1/site/session",
            a,
            Some(json!({"user": "root", "password": "root-password"}))
        )
        .await
        .status,
        409,
        "signing in is for access \"users\""
    );
    let s = ok(&set_access(addr, a, "users", None).await, 200, "users");
    assert_eq!(s["access"], "users");
    assert_eq!(s["pinSet"], false);
    assert_eq!(
        call(
            addr,
            "PUT",
            "/api/v1/site/users/admin",
            a,
            Some(json!({"password": "whatever-1"}))
        )
        .await
        .status,
        400
    );
    assert_eq!(
        call(
            addr,
            "PUT",
            "/api/v1/site/users/nopw",
            a,
            Some(json!({"role": "viewer"}))
        )
        .await
        .status,
        400,
        "a new user needs a password"
    );
    let users = ok(
        &call(addr, "GET", "/api/v1/site/users", a, None).await,
        200,
        "users",
    );
    assert_eq!(users.as_array().unwrap().len(), 3, "{users}");

    // Access "users": without signing in, a client may only read.
    let s = ok(
        &call(addr, "GET", "/api/v1/site/session", b, None).await,
        200,
        "session",
    );
    assert_eq!(
        s,
        json!({"access": "users", "signedIn": false, "unlocked": false})
    );
    assert_eq!(set_access(addr, b, "open", None).await.status, 403);
    let r = call(addr, "POST", "/api/v1/site/join-code", b, None).await;
    assert_eq!(r.status, 403);
    assert_eq!(r.json()["error"]["code"], "forbidden");
    ok(
        &call(addr, "GET", "/api/v1/site/devices", b, None).await,
        200,
        "read",
    );
    assert_eq!(
        call(addr, "GET", "/api/v1/site/audit", b, None)
            .await
            .status,
        403,
        "the audit needs a site role"
    );

    // Sign in (wrong password first).
    let r = call(
        addr,
        "POST",
        "/api/v1/site/session",
        b,
        Some(json!({"user": "vic", "password": "nope-nope"})),
    )
    .await;
    assert_eq!(r.status, 401);
    let s = ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/session",
            b,
            Some(json!({"user": "vic", "password": "vic-password"})),
        )
        .await,
        200,
        "sign in",
    );
    assert_eq!(
        s,
        json!({"access": "users", "user": "vic", "role": "viewer", "signedIn": true, "unlocked": false})
    );
    assert_eq!(
        get_json_auth(addr, "/api/v1/site", b).await["you"]["user"],
        "vic"
    );

    // A viewer: read tokens only.
    let token = |scope: &str, ttl: Option<u64>| {
        let mut body = json!({"engine": "desk", "scope": scope});
        if let Some(t) = ttl {
            body["ttlSeconds"] = json!(t);
        }
        body
    };
    let t = ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/tokens",
            b,
            Some(token("read", None)),
        )
        .await,
        201,
        "viewer read token",
    );
    assert!(t["token"].as_str().unwrap().starts_with("idn1."));
    assert_eq!(t["user"], "vic");
    assert_eq!(t["scope"], "read");
    assert_eq!(t["machine"], "*");
    for scope in ["control", "bus:main", "iem:2"] {
        let r = call(
            addr,
            "POST",
            "/api/v1/site/tokens",
            b,
            Some(token(scope, None)),
        )
        .await;
        assert_eq!(r.status, 403, "viewer {scope}");
    }
    assert_eq!(
        call(addr, "POST", "/api/v1/site/join-code", b, None)
            .await
            .status,
        403,
        "a viewer may not issue join codes"
    );

    // An operator: control, a bus, an IEM mix; not join codes (the admins'), not past 12 h.
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/session",
            a,
            Some(json!({"user": "alice", "password": "alice-password"})),
        )
        .await,
        200,
        "alice signs in",
    );
    for scope in ["read", "control", "bus:main", "iem:3"] {
        let t = ok(
            &call(
                addr,
                "POST",
                "/api/v1/site/tokens",
                a,
                Some(token(scope, Some(3600))),
            )
            .await,
            201,
            scope,
        );
        assert_eq!(t["scope"], scope);
        assert!(t["expiresAt"].as_str().unwrap() > t["issuedAt"].as_str().unwrap());
    }
    for (body, status) in [
        (token("control", Some(13 * 3600)), 400),
        (json!({"engine": "vesper", "scope": "read"}), 400),
        (token("everything", None), 400),
    ] {
        let r = call(addr, "POST", "/api/v1/site/tokens", a, Some(body.clone())).await;
        assert_eq!(
            r.status,
            status,
            "{body}: {}",
            String::from_utf8_lossy(&r.body)
        );
    }
    let r = call(addr, "POST", "/api/v1/site/join-code", a, None).await;
    assert_eq!(r.status, 403, "IDN: join codes are the admins'");
    assert_eq!(
        call(addr, "DELETE", "/api/v1/site/tokens/vic", a, None)
            .await
            .status,
        403,
        "only admins withdraw another user's tokens"
    );
    ok(
        &call(addr, "DELETE", "/api/v1/site/tokens/vic", b, None).await,
        204,
        "vic withdraws their own",
    );

    // An admin changes a role with If-Match, removes a user (their sign-in goes with them).
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/session",
            a,
            Some(json!({"user": "root", "password": "root-password"})),
        )
        .await,
        200,
        "root signs in",
    );
    let vic = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["name"] == "vic")
        .unwrap()
        .clone();
    let bearer = format!("Bearer {}", a.unwrap());
    let r = http(
        addr,
        "PUT",
        "/api/v1/site/users/vic",
        &[("Authorization", &bearer), ("If-Match", "\"stale\"")],
        Some(json!({"role": "operator"})),
    )
    .await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["current"]["role"], "viewer");
    let rev = format!("\"{}\"", vic["revision"].as_str().unwrap());
    let r = http(
        addr,
        "PUT",
        "/api/v1/site/users/vic",
        &[("Authorization", &bearer), ("If-Match", &rev)],
        Some(json!({"role": "operator"})),
    )
    .await;
    assert_eq!(ok(&r, 200, "promote")["role"], "operator");
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", b).await["role"],
        "operator",
        "the role is read at each request"
    );
    // A new password (the old one leaked) ends the user's sign-ins on other clients; the
    // admin's own new password keeps the admin's own sign-in.
    ok(
        &call(
            addr,
            "PUT",
            "/api/v1/site/users/vic",
            a,
            Some(json!({"password": "vic-password-2"})),
        )
        .await,
        200,
        "vic's new password",
    );
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", b).await,
        json!({"access": "users", "signedIn": false, "unlocked": false}),
        "signed out by the new password"
    );
    ok(
        &call(
            addr,
            "PUT",
            "/api/v1/site/users/root",
            a,
            Some(json!({"password": "root-password"})),
        )
        .await,
        200,
        "root's own new password",
    );
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", a).await["user"],
        "root",
        "still signed in"
    );
    ok(
        &call(addr, "DELETE", "/api/v1/site/users/vic", a, None).await,
        204,
        "remove vic",
    );
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", b).await,
        json!({"access": "users", "signedIn": false, "unlocked": false})
    );
    // The last admin user stays one while the access is "users".
    assert_eq!(
        call(
            addr,
            "PUT",
            "/api/v1/site/users/root",
            a,
            Some(json!({"role": "operator"}))
        )
        .await
        .status,
        409,
        "demote the last admin"
    );
    assert_eq!(
        call(addr, "DELETE", "/api/v1/site/users/root", a, None)
            .await
            .status,
        409,
        "remove the last admin"
    );
    assert_eq!(
        call(addr, "DELETE", "/api/v1/site/users/vic", a, None)
            .await
            .status,
        404
    );
    ok(
        &call(addr, "DELETE", "/api/v1/site/session", a, None).await,
        204,
        "sign out",
    );
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", a).await,
        json!({"access": "users", "signedIn": false, "unlocked": false}),
        "signing out doesn't make a client admin again"
    );

    // The audit: who, which client, from where; no secrets.
    let r = call(
        addr,
        "POST",
        "/api/v1/site/session",
        a,
        Some(json!({"user": "root", "password": "root-password"})),
    )
    .await;
    ok(&r, 200, "root again");
    let audit = ok(
        &call(addr, "GET", "/api/v1/site/audit?n=500", a, None).await,
        200,
        "audit",
    );
    // Tokens issued: the Site Manager's entry, with the client and its address.
    assert!(audit_has(&audit, "token (host)", |e| e["user"] == "alice"
        && e["client"] == "Oriel – Booth"
        && e["peer"] == "127.0.0.1"
        && e["source"] == "siteManager"
        && e["detail"]["scope"] == "bus:main"));
    assert!(audit_has(&audit, "token", |e| e["user"] == "vic"
        && e["ok"] == false
        && e["client"] == "Oriel – Tablet"));
    assert!(audit_has(&audit, "sign_in", |e| e["ok"] == false));
    assert!(audit_has(&audit, "user", |e| e["detail"]["name"] == "vic"
        && e["detail"]["action"] == "remove"
        && e["user"] == "root"));
    let text = audit.to_string();
    for secret in ["root-password", "alice-password", "vic-password", "idn1."] {
        assert!(!text.contains(secret), "{secret} in the audit");
    }
    assert!(audit_has(&audit, "settings", |e| e["detail"]["access"]
        == "users"
        && e["ok"] == true
        && e["client"] == "Oriel – Booth"));
    assert!(audit_has(&audit, "settings", |e| e["detail"]["access"]
        == "users"
        && e["ok"] == false));
    let stored = std::fs::read_to_string(fx.dir.join("machine/site.json")).unwrap();
    assert!(!stored.contains("password"));

    // Back to "open" (an admin's change): every paired client is admin again; the sign-ins end.
    ok(&set_access(addr, a, "open", None).await, 200, "open");
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", b).await,
        json!({"access": "open", "user": "admin", "role": "admin", "signedIn": false, "unlocked": false})
    );
    let t = ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/tokens",
            b,
            Some(json!({"engine": "switcher", "scope": "control"})),
        )
        .await,
        201,
        "open: a token of the client's own",
    );
    let own = t["user"].as_str().unwrap().to_string();
    assert!(own.starts_with("vesper:"), "{t}");
    assert_ne!(own, "vesper:", "{t}");
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn access_pin_unlock_lock_timeout_and_lockout() {
    let fx = fixture("site-pin");
    let idle = Duration::from_secs(3);
    let server = start_with(
        &fx,
        false,
        SiteConfig {
            unlock_idle: idle,
            pin_lockout: Duration::from_secs(1),
            ..SiteConfig::loopback()
        },
    )
    .await;
    let addr = server.addr;
    let a = pair(&server, "Booth").await;
    let b = pair(&server, "Tablet").await;
    let (a, b) = (Some(a.as_str()), Some(b.as_str()));
    let mut ws = Client::connect(addr, &format!("protocolVersion=1&token={}", a.unwrap())).await;

    // "open" (the default): a paired client does admin actions without a PIN.
    let st = get_json_auth(addr, "/api/v1/site", b).await;
    assert_eq!(st["you"]["access"], "open");
    ok(
        &call(addr, "POST", "/api/v1/site/join-code", b, None).await,
        201,
        "open: a join code",
    );
    assert_eq!(
        call(
            addr,
            "POST",
            "/api/v1/site/unlock",
            b,
            Some(json!({"pin": "1234"}))
        )
        .await
        .status,
        409,
        "no PIN with access \"open\""
    );

    // To "pin": a PIN is needed, 4 to 8 digits, and only with access "pin".
    for (access, pin, status) in [
        ("pin", None, 400),
        ("pin", Some("123"), 400),
        ("pin", Some("123456789"), 400),
        ("pin", Some("12a4"), 400),
        ("open", Some("1234"), 400),
    ] {
        assert_eq!(
            set_access(addr, a, access, pin).await.status,
            status,
            "{access} {pin:?}"
        );
    }
    let r = set_access(addr, a, "pin", Some("2468")).await;
    let s = ok(&r, 200, "pin");
    assert_eq!(s["access"], "pin");
    assert_eq!(s["pinSet"], true);
    assert!(s.get("pin").is_none(), "the PIN never goes out: {s}");
    let stored = std::fs::read_to_string(fx.dir.join("machine/site.json")).unwrap();
    assert!(
        !stored.contains("2468") && stored.contains("$argon2id$"),
        "stored hashed"
    );

    // Locked: the state can be read (the audit too); changes and tokens need the PIN.
    let s = get_json_auth(addr, "/api/v1/site/session", b).await;
    assert_eq!(
        s,
        json!({"access": "pin", "role": "viewer", "signedIn": false, "unlocked": false})
    );
    for path in [
        "/api/v1/site",
        "/api/v1/site/devices",
        "/api/v1/site/join-requests",
        "/api/v1/site/join-codes",
        "/api/v1/site/users",
        "/api/v1/site/settings",
        "/api/v1/site/audit",
        "/api/v1/site/events",
    ] {
        get_json_auth(addr, path, b).await;
    }
    for (method, path, body) in [
        ("POST", "/api/v1/site/join-code", None),
        ("POST", "/api/v1/site/pairing", None),
        (
            "PUT",
            "/api/v1/site/users/eve",
            Some(json!({"role": "admin", "password": "eve-password"})),
        ),
        (
            "POST",
            "/api/v1/site/tokens",
            Some(json!({"engine": "desk", "scope": "read"})),
        ),
        (
            "PUT",
            "/api/v1/site/settings",
            Some(json!({"enabled": true, "joinMode": "code_or_approval", "access": "open"})),
        ),
    ] {
        let r = call(addr, method, path, b, body).await;
        assert_eq!(r.status, 403, "{method} {path}");
        assert!(
            r.json()["error"]["message"]
                .as_str()
                .unwrap()
                .contains("unlock"),
            "{}",
            String::from_utf8_lossy(&r.body)
        );
    }
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/settings", b).await["access"],
        "pin"
    );

    // Unlock: a wrong PIN, then the right one; admin until locked on demand.
    let r = call(
        addr,
        "POST",
        "/api/v1/site/unlock",
        b,
        Some(json!({"pin": "1111"})),
    )
    .await;
    assert_eq!(r.status, 401);
    let s = ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/unlock",
            b,
            Some(json!({"pin": "2468"})),
        )
        .await,
        200,
        "unlock",
    );
    assert_eq!(s["user"], "admin");
    assert_eq!(s["unlocked"], true);
    assert_eq!(s["unlockSecondsLeft"], idle.as_secs());
    ok(
        &call(addr, "POST", "/api/v1/site/join-code", b, None).await,
        201,
        "unlocked: a join code",
    );
    let t = ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/tokens",
            b,
            Some(json!({"engine": "desk", "scope": "bus:main"})),
        )
        .await,
        201,
        "unlocked: a token of the client's own",
    );
    let b_tokens = t["user"].as_str().unwrap().to_string();
    assert!(b_tokens.starts_with("vesper:"), "{t}");
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", a).await["unlocked"],
        false,
        "an unlock is per client"
    );
    ok(
        &call(addr, "DELETE", "/api/v1/site/unlock", b, None).await,
        204,
        "lock",
    );
    assert_eq!(
        call(addr, "POST", "/api/v1/site/join-code", b, None)
            .await
            .status,
        403,
        "locked again"
    );
    // Each client's tokens are its own: another (locked) client may not withdraw them; the
    // client itself may, locked or not.
    let path = format!("/api/v1/site/tokens/{b_tokens}");
    assert_eq!(call(addr, "DELETE", &path, a, None).await.status, 403);
    ok(
        &call(addr, "DELETE", &path, b, None).await,
        204,
        "withdraw its own",
    );

    // Idle: each admin action starts the time again; without one it locks by itself.
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/unlock",
            b,
            Some(json!({"pin": "2468"})),
        )
        .await,
        200,
        "unlock",
    );
    tokio::time::sleep(idle / 2).await;
    ok(
        &call(addr, "DELETE", "/api/v1/site/pairing", b, None).await,
        204,
        "an admin action",
    );
    tokio::time::sleep(idle / 2 + Duration::from_millis(300)).await;
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", b).await["unlocked"],
        true,
        "still unlocked: the action restarted the idle time"
    );
    tokio::time::sleep(idle).await;
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", b).await,
        json!({"access": "pin", "role": "viewer", "signedIn": false, "unlocked": false}),
        "locked after idling"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ws.events.iter().any(|m| {
        m["type"] == "siteChanged" && m["parts"].as_array().unwrap().contains(&json!("session"))
    }) {
        assert!(Instant::now() < deadline, "siteChanged session");
        let m = ws.next_any().await;
        if m.get("binary").is_none() {
            ws.absorb(&m);
        }
    }

    // Wrong PINs: 5 in a row lock the client out (429), longer each time; the right PIN after
    // the lockout works and starts the count again.
    let unlock = |pin: &'static str| {
        call(
            addr,
            "POST",
            "/api/v1/site/unlock",
            b,
            Some(json!({"pin": pin})),
        )
    };
    for k in 0..5 {
        assert_eq!(unlock("0000").await.status, 401, "wrong {k}");
    }
    let r = unlock("2468").await;
    assert_eq!(r.status, 429, "locked out, even with the right PIN");
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("try again"),
    );
    ok(
        &call(addr, "GET", "/api/v1/site/session", a, None).await,
        200,
        "another client isn't locked out",
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    for k in 0..5 {
        assert_eq!(unlock("0000").await.status, 401, "wrong again {k}");
    }
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        unlock("2468").await.status,
        429,
        "the second lockout is longer"
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    ok(&unlock("2468").await, 200, "after the lockout");

    // A new PIN locks the other unlocked clients, not the one setting it.
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/unlock",
            a,
            Some(json!({"pin": "2468"})),
        )
        .await,
        200,
        "a unlocks",
    );
    ok(
        &set_access(addr, a, "pin", Some("13579")).await,
        200,
        "new PIN",
    );
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", a).await["unlocked"],
        true
    );
    assert_eq!(
        get_json_auth(addr, "/api/v1/site/session", b).await["unlocked"],
        false
    );
    assert_eq!(unlock("2468").await.status, 401, "the old PIN");
    ok(&unlock("13579").await, 200, "the new PIN");

    // Switching levels: "users" needs an admin user; back to "open" forgets the PIN.
    assert_eq!(set_access(addr, b, "users", None).await.status, 409);
    ok(
        &call(
            addr,
            "PUT",
            "/api/v1/site/users/root",
            b,
            Some(json!({"role": "admin", "password": "root-password"})),
        )
        .await,
        200,
        "an admin user",
    );
    ok(&set_access(addr, b, "users", None).await, 200, "users");
    let s = get_json_auth(addr, "/api/v1/site/session", b).await;
    assert_eq!(
        s,
        json!({"access": "users", "signedIn": false, "unlocked": false})
    );
    let settings = get_json_auth(addr, "/api/v1/site/settings", b).await;
    assert_eq!(settings["pinSet"], false, "leaving \"pin\" forgets the PIN");
    assert_eq!(set_access(addr, b, "open", None).await.status, 403);

    // The audit: unlocks (right, wrong, locked out), locks and the level changes; no PINs.
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/session",
            a,
            Some(json!({"user": "root", "password": "root-password"})),
        )
        .await,
        200,
        "root signs in",
    );
    let audit = get_json_auth(addr, "/api/v1/site/audit?n=500", a).await;
    assert!(audit_has(&audit, "unlock", |e| e["ok"] == true
        && e["client"] == "Oriel – Tablet"
        && e["peer"] == "127.0.0.1"));
    assert!(audit_has(&audit, "unlock", |e| e["ok"] == false
        && e["error"].as_str().unwrap().starts_with("wrong PIN")));
    assert!(audit_has(&audit, "unlock", |e| e["ok"] == false
        && e["error"]
            .as_str()
            .unwrap()
            .starts_with("too many wrong PINs")));
    assert!(audit_has(&audit, "lock", |e| e["client"] == "Oriel – Tablet"));
    for level in ["pin", "users"] {
        assert!(
            audit_has(&audit, "settings", |e| e["detail"]["access"] == level
                && e["ok"] == true),
            "{level}"
        );
    }
    assert!(audit_has(&audit, "settings", |e| e["detail"]["newPin"]
        == true
        && e["user"] == "admin"
        && e["client"] == "Oriel – Booth"));
    assert!(audit_has(&audit, "join_code", |e| e["user"] == "admin"
        && e["client"] == "Oriel – Tablet"
        && e["ok"] == true));
    let text = audit.to_string();
    for secret in ["\"2468\"", "\"13579\"", "root-password", "idn1."] {
        assert!(!text.contains(secret), "{secret} in the audit");
    }
    drop(ws);
    server.shutdown().await.unwrap();
}

pub(crate) async fn get_json_auth(addr: SocketAddr, path: &str, token: Option<&str>) -> Value {
    let r = call(addr, "GET", path, token, None).await;
    ok(&r, 200, path)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn own_site_stays_on_and_survives_a_restart() {
    let fx = fixture("site-life");
    let server = start_site(&fx, true).await;
    let addr = server.addr;
    let st = get_json(addr, "/api/v1/site").await;
    let site_id = st["siteId"].clone();
    let ds = st["discoveryPort"].as_u64().unwrap() as u16;
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    let one = machine(&fx, ds, "BOX-1");
    one.join_with_code(code["code"].as_str().unwrap()).unwrap();
    until(&one, "joins", |s| matches!(s, TrustState::Enrolled { .. })).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while get_json(addr, "/api/v1/site/devices").await[0]["health"] != "online" {
        assert!(Instant::now() < deadline, "BOX-1 online");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Every server is in a site: its own Site Manager can't be turned off while it isn't in
    // another server's site (joining one turns it off: see `keys`).
    let r = call(
        addr,
        "PUT",
        "/api/v1/site/settings",
        None,
        Some(json!({"enabled": false, "joinMode": "code_or_approval", "access": "open"})),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("joins another server's site"),
        "{}",
        r.json()
    );
    let st = get_json(addr, "/api/v1/site").await;
    assert_eq!(
        (st["enabled"].clone(), st["running"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(
        st["membership"],
        json!({"state": "none", "connected": false})
    );
    // The join mode changes as before.
    let r = call(
        addr,
        "PUT",
        "/api/v1/site/settings",
        None,
        Some(json!({"enabled": true, "joinMode": "approval", "access": "open"})),
    )
    .await;
    assert_eq!(ok(&r, 200, "join mode")["joinMode"], "approval");
    let devs = get_json(addr, "/api/v1/site/devices").await;
    assert_eq!(devs[0]["fingerprint"], one.fingerprint(), "{devs}");
    drop(one);
    server.shutdown().await.unwrap();

    // A restart keeps the site, its join mode and the designation.
    let server = start_site(&fx, true).await;
    let st = get_json(server.addr, "/api/v1/site").await;
    assert_eq!(st["running"], true);
    assert_eq!(st["siteId"], site_id);
    assert_eq!(st["joinMode"], "approval");
    let devs = get_json(server.addr, "/api/v1/site/devices").await;
    assert_eq!(devs[0]["health"], "offline", "{devs}");
    assert!(devs[0]["lastSeen"].is_string(), "{devs}");
    server.shutdown().await.unwrap();

    // Not designated: off on the command line.
    let server = start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        mdns: false,
        site: SiteConfig::off(),
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .unwrap();
    let st = get_json(server.addr, "/api/v1/site").await;
    assert_eq!(st["running"], false);
    assert!(
        st["error"].as_str().unwrap().contains("command line"),
        "{st}"
    );
    server.shutdown().await.unwrap();
}

/// The code's 10 minutes: run with `--ignored` (it waits them out).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "waits 10 minutes for a join code to expire"]
async fn join_code_expires_after_ten_minutes() {
    let fx = fixture("site-expiry");
    let server = start_site(&fx, true).await;
    let addr = server.addr;
    let ds = get_json(addr, "/api/v1/site").await["discoveryPort"]
        .as_u64()
        .unwrap() as u16;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let code = ok(
        &call(addr, "POST", "/api/v1/site/join-code", None, None).await,
        201,
        "code",
    );
    tokio::time::sleep(Duration::from_secs(605)).await;
    let mut expired = None;
    while expired.is_none() {
        let m = ws.next_any().await;
        if m["type"] == "siteEvent" && m["event"]["name"] == "join_code_expired" {
            expired = Some(m["event"].clone());
        }
    }
    assert_eq!(expired.unwrap()["codeId"], code["id"]);
    let late = machine(&fx, ds, "LATE");
    late.join_with_code(code["code"].as_str().unwrap()).unwrap();
    until(&late, "expired code", |s| {
        matches!(s, TrustState::CodeExpired)
    })
    .await;
    server.shutdown().await.unwrap();
}
