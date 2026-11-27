//! Pairing with `--no-trust-localhost`, which makes this test's localhost requests look like a
//! remote client's. The server runs its own site (on loopback ports), whose Site Manager makes
//! each paired client's key.
//!
//! - no key → 401, wrong code → 401, right code → a named site API key (`idn1.…`) that works
//!   for REST and the WebSocket; revoking it in the site → 401, and its open WebSocket closes;
//!   pair attempts are rate limited;
//! - without a code, the pairing waits for a site admin's approval at the site's server too
//!   (approve, deny; at access `users` only a signed-in admin answers); a browser's key lives
//!   a year;
//! - without a site, pairing answers 503 (and the code isn't used up);
//! - a client paired before site keys swaps its credential for a key once, without pairing
//!   again; the old credential goes when the key is first used;
//! - a request addressed to another host name (DNS rebinding) is a remote client's.

use crate::common::*;
use serde_json::{Value, json};
use vesper_server::site::SiteConfig;

async fn with_key(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    key: &str,
    body: Option<Value>,
) -> Resp {
    let bearer = format!("Bearer {key}");
    http(addr, method, path, &[("Authorization", &bearer)], body).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_gives_a_site_key_and_revoking_it_stops_it() {
    let fx = fixture("pairing");
    let server = start_with_site(&fx, false, SiteConfig::loopback()).await;
    let addr = server.addr;

    // Public: server info (no pairing code for remote clients).
    let info = get_json(addr, "/api/v1/server").await;
    assert_eq!(info["auth"], "unpaired");
    assert!(info.get("pairingCode").is_none(), "{info}");

    // Everything else needs a key.
    for path in ["/api/v1/state", "/api/v1/libraries", "/api/v1/pairing/code"] {
        let r = http(addr, "GET", path, &[], None).await;
        assert_eq!(r.status, 401, "{path}");
        assert_eq!(r.json()["error"]["code"], "unauthorized");
    }
    for bad in ["nonsense", "idn1.abc.def"] {
        let r = with_key(addr, "GET", "/api/v1/state", bad, None).await;
        assert_eq!(r.status, 401, "{bad}");
    }
    let ws = tokio_tungstenite::connect_async(format!("ws://{addr}/api/v1/ws")).await;
    assert!(ws.is_err(), "WebSocket without a key is refused");

    // Wrong code.
    let code = server.pairing_code();
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": wrong, "clientName": "Tablet"})),
    )
    .await;
    assert_eq!(r.status, 401);
    assert_eq!(r.json()["error"]["code"], "unauthorized");

    // Right code: a named site key.
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": code, "clientName": "Tablet"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let key = r.json()["token"].as_str().unwrap().to_string();
    let key_id = r.json()["clientId"].as_str().unwrap().to_string();
    assert!(key.starts_with("idn1."), "a site-signed key: {key}");
    assert_ne!(server.pairing_code(), code, "the code rotates after use");
    assert!(
        !fx.dir.join("machine/clients.json").exists(),
        "keys aren't kept in the old credential store"
    );
    let claims = idn_open_protocol::trust::token::peek(&key).unwrap();
    assert!(claims.key, "{claims:?}");
    assert_eq!(claims.sub, format!("key:{key_id}"));
    assert_eq!(claims.name, "Oriel – Tablet");
    assert_eq!(claims.engine, "app:vesper");
    assert_eq!(claims.machine, "*", "every Vesper server of the site");
    assert_eq!(claims.scope.as_string(), "control");
    assert!(
        claims.exp - claims.iat >= 9 * 365 * 24 * 3600,
        "as long as IDN allows: {claims:?}"
    );

    // The key works for REST and the WebSocket.
    let r = with_key(addr, "GET", "/api/v1/state", &key, None).await;
    assert_eq!(r.status, 200);
    let info = with_key(addr, "GET", "/api/v1/server", &key, None)
        .await
        .json();
    assert_eq!(info["auth"], "paired");
    assert!(info.get("upgradeCredential").is_none(), "{info}");
    let mut c = Client::connect(addr, &format!("protocolVersion=1&token={key}")).await;
    c.ok(json!({"type": "triggerSlide", "presentation": fx.song.id.0.to_string(), "index": 0}))
        .await;
    // Paired clients are not the server machine: client management stays local-only.
    for path in ["/api/v1/clients", "/api/v1/pairing/code"] {
        let r = with_key(addr, "GET", path, &key, None).await;
        assert_eq!(r.status, 403, "{path}");
        assert_eq!(r.json()["error"]["code"], "forbidden");
    }

    // Revoked in the site (access `open`: a paired client is the site's admin).
    let r = with_key(addr, "DELETE", "/api/v1/site/api-keys/nope", &key, None).await;
    assert_eq!(r.status, 404, "{}", String::from_utf8_lossy(&r.body));
    let r = with_key(
        addr,
        "DELETE",
        &format!("/api/v1/site/api-keys/{key_id}"),
        &key,
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let r = with_key(addr, "GET", "/api/v1/state", &key, None).await;
    assert_eq!(r.status, 401, "a revoked key stops at once");
    assert_eq!(
        closed_by_server(&mut c.ws).await,
        Some(4003),
        "its open WebSocket closes"
    );
    let ws = tokio_tungstenite::connect_async(format!("ws://{addr}/api/v1/ws?token={key}")).await;
    assert!(ws.is_err(), "a revoked key cannot open a WebSocket");
    let audit = server.shared.site.audit_log(50);
    assert!(
        audit
            .iter()
            .any(|e| e.action == "api_key" && e.client == "Oriel – Tablet"),
        "the key's making is in the site's audit with the client: {audit:?}"
    );

    // Rate limit: 2 attempts so far this minute; 3 more are allowed, the 6th is refused.
    for i in 0..3 {
        let r = http(
            addr,
            "POST",
            "/api/v1/pair",
            &[],
            Some(json!({"code": "x", "clientName": "y"})),
        )
        .await;
        assert_eq!(r.status, 401, "attempt {}", i + 3);
    }
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": server.pairing_code(), "clientName": "y"})),
    )
    .await;
    assert_eq!(r.status, 429, "6th attempt in a minute");
    assert_eq!(r.json()["error"]["code"], "rateLimited");

    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// The old credential (from before site keys) used by the migration test.
const OLD: &str = "old-pairing-credential-0001";
/// Its SHA-256, as the old store kept it.
const OLD_SHA256: &str = "c30d6c8bf080725e09492dd0b9539ccfe17385900cfe077f897a8613e305a362";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_old_credential_is_swapped_for_a_site_key_once() {
    let fx = fixture("pairing-upgrade");
    let old_id = "6a1d1c8e-0000-4000-8000-000000000001";
    std::fs::create_dir_all(fx.dir.join("machine")).unwrap();
    std::fs::write(
        fx.dir.join("machine/clients.json"),
        serde_json::to_vec_pretty(&json!({"clients": [{
            "id": old_id, "name": "Booth laptop", "tokenSha256": OLD_SHA256,
            "createdAt": "2026-10-19T09:30:45Z",
        }]}))
        .unwrap(),
    )
    .unwrap();
    let server = start_with_site(&fx, false, SiteConfig::loopback()).await;
    let addr = server.addr;
    assert_eq!(
        server.shared.auth.legacy_clients()[0].name,
        "Booth laptop",
        "listed until swapped"
    );

    // The old credential still works, and the server asks for the swap.
    let info = with_key(addr, "GET", "/api/v1/server", OLD, None)
        .await
        .json();
    assert_eq!(info["auth"], "paired");
    assert_eq!(info["upgradeCredential"], true);
    assert_eq!(
        with_key(addr, "GET", "/api/v1/state", OLD, None)
            .await
            .status,
        200
    );
    // Only an old credential is swapped.
    let r = http(addr, "POST", "/api/v1/pair/upgrade", &[], None).await;
    assert_eq!(r.status, 401);

    // The swap: a named key. Asked twice (the first answer lost), the first key is revoked.
    let r = with_key(addr, "POST", "/api/v1/pair/upgrade", OLD, None).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let lost = r.json()["token"].as_str().unwrap().to_string();
    let r = with_key(addr, "POST", "/api/v1/pair/upgrade", OLD, None).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let key = r.json()["token"].as_str().unwrap().to_string();
    assert_ne!(key, lost);
    assert_eq!(
        idn_open_protocol::trust::token::peek(&key).unwrap().name,
        "Oriel – Booth laptop"
    );
    assert_eq!(
        with_key(addr, "GET", "/api/v1/state", &lost, None)
            .await
            .status,
        401,
        "the key handed over first, never used, is revoked"
    );
    // Until the new key is used, the old credential still works (the answer may be lost).
    assert_eq!(
        with_key(addr, "GET", "/api/v1/state", OLD, None)
            .await
            .status,
        200
    );

    // The key used: the old credential is dropped, and the old store with it.
    let info = with_key(addr, "GET", "/api/v1/server", &key, None)
        .await
        .json();
    assert_eq!(info["auth"], "paired");
    assert!(info.get("upgradeCredential").is_none(), "{info}");
    assert_eq!(
        with_key(addr, "GET", "/api/v1/state", OLD, None)
            .await
            .status,
        401,
        "the old credential is gone"
    );
    assert!(server.shared.auth.legacy_clients().is_empty());
    let r = with_key(addr, "POST", "/api/v1/pair/upgrade", &key, None).await;
    assert_eq!(r.status, 409, "a key isn't swapped");
    assert!(
        !fx.dir.join("machine/clients.json").exists(),
        "no old credential left: the store is no longer used"
    );
    server.shutdown().await.unwrap();

    // After a restart, the key still works; nothing asks to pair again.
    let server = start_with_site(&fx, false, SiteConfig::loopback()).await;
    let info = with_key(server.addr, "GET", "/api/v1/server", &key, None)
        .await
        .json();
    assert_eq!(info["auth"], "paired");
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_code_a_pairing_waits_for_an_admin() {
    let fx = fixture("pairing-approval");
    let server = start_with_site(&fx, false, SiteConfig::loopback()).await;
    let addr = server.addr;
    // A browser pairs with the code: its key lives a year.
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": server.pairing_code(), "clientName": "Booth", "browser": true})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let admin = r.json()["token"].as_str().unwrap().to_string();
    let c = idn_open_protocol::trust::token::peek(&admin).unwrap();
    assert_eq!(c.exp - c.iat, 365 * 86_400, "a browser's key: a year");

    // No code: it waits, listed for admins; approved, the client gets its key once.
    let code_before = server.pairing_code();
    let ask = |name: &'static str| {
        http(
            addr,
            "POST",
            "/api/v1/pair",
            &[],
            Some(json!({"clientName": name})),
        )
    };
    let r = ask("Tablet").await;
    assert_eq!(r.status, 202, "{}", String::from_utf8_lossy(&r.body));
    let secret = r.json()["pairingId"].as_str().unwrap().to_string();
    assert_eq!(server.pairing_code(), code_before, "no code used");
    let w = with_key(addr, "GET", "/api/v1/site/pairings", &admin, None)
        .await
        .json();
    assert_eq!(w.as_array().unwrap().len(), 1, "{w}");
    assert_eq!(w[0]["client"], "Oriel – Tablet");
    assert_eq!(w[0]["relayed"], false);
    assert_eq!(w[0]["address"], "127.0.0.1");
    assert!(
        !w.to_string().contains(&secret),
        "the secret id isn't listed"
    );
    let id = w[0]["id"].as_str().unwrap().to_string();
    let r = http(addr, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
    assert_eq!(r.status, 202);
    let r = with_key(
        addr,
        "POST",
        &format!("/api/v1/site/pairings/{id}/approve"),
        &admin,
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let r = http(addr, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let key = r.json()["token"].as_str().unwrap().to_string();
    let c = idn_open_protocol::trust::token::peek(&key).unwrap();
    assert_eq!(c.name, "Oriel – Tablet");
    assert!(c.exp - c.iat > 9 * 365 * 86_400, "the app's key: 10 years");
    assert_eq!(
        with_key(addr, "GET", "/api/v1/state", &key, None)
            .await
            .status,
        200
    );
    let r = http(addr, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
    assert_eq!(r.status, 404, "given once");
    let keys = with_key(addr, "GET", "/api/v1/site/api-keys", &admin, None)
        .await
        .json();
    assert_eq!(keys.as_array().unwrap().len(), 2, "{keys}");

    // Denied: the client is told why.
    let r = ask("Phone").await;
    let secret = r.json()["pairingId"].as_str().unwrap().to_string();
    let w = with_key(addr, "GET", "/api/v1/site/pairings", &admin, None)
        .await
        .json();
    let id = w[0]["id"].as_str().unwrap().to_string();
    let r = with_key(
        addr,
        "POST",
        &format!("/api/v1/site/pairings/{id}/deny"),
        &admin,
        None,
    )
    .await;
    assert_eq!(r.status, 204);
    let r = http(addr, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
    assert_eq!(r.status, 403);
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("refused"),
        "{}",
        r.json()
    );

    // At access `users`, only a signed-in admin answers.
    let r = with_key(
        addr,
        "PUT",
        "/api/v1/site/users/root",
        &admin,
        Some(json!({"role": "admin", "password": "root-password"})),
    )
    .await;
    assert_eq!(r.status, 200);
    let r = with_key(
        addr,
        "PUT",
        "/api/v1/site/settings",
        &admin,
        Some(json!({"enabled": true, "joinMode": "code_or_approval", "access": "users"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let r = ask("Laptop").await;
    assert_eq!(r.status, 202);
    let secret = r.json()["pairingId"].as_str().unwrap().to_string();
    let w = with_key(addr, "GET", "/api/v1/site/pairings", &admin, None)
        .await
        .json();
    let id = w[0]["id"].as_str().unwrap().to_string();
    let approve = format!("/api/v1/site/pairings/{id}/approve");
    let r = with_key(addr, "POST", &approve, &admin, None).await;
    assert_eq!(r.status, 403, "not signed in");
    let r = with_key(
        addr,
        "POST",
        "/api/v1/site/session",
        &admin,
        Some(json!({"user": "root", "password": "root-password"})),
    )
    .await;
    assert_eq!(r.status, 200);
    let r = with_key(addr, "POST", &approve, &admin, None).await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let r = http(addr, "GET", &format!("/api/v1/pair/{secret}"), &[], None).await;
    assert_eq!(r.status, 200);
    let audit = server.shared.site.audit_log(100);
    assert!(
        audit
            .iter()
            .any(|e| e.action == "pairing/approve" && e.user == "root" && e.ok),
        "{audit:?}"
    );
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoking_an_old_credential_revokes_the_key_handed_over_for_it() {
    let fx = fixture("pairing-upgrade-revoke");
    let old_id = "6a1d1c8e-0000-4000-8000-000000000002";
    std::fs::create_dir_all(fx.dir.join("machine")).unwrap();
    std::fs::write(
        fx.dir.join("machine/clients.json"),
        serde_json::to_vec_pretty(&json!({"clients": [{
            "id": old_id, "name": "Booth laptop", "tokenSha256": OLD_SHA256,
            "createdAt": "2026-10-19T09:30:45Z",
        }]}))
        .unwrap(),
    )
    .unwrap();
    // The server machine itself revokes (localhost trusted): `auth` tells a key that still
    // holds ("paired") from one that doesn't (the request is then the machine's, "local").
    let server = start_with_site(&fx, true, SiteConfig::loopback()).await;
    let addr = server.addr;

    // The client swaps its credential for a key, but hasn't used the key yet.
    let r = with_key(addr, "POST", "/api/v1/pair/upgrade", OLD, None).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let key = r.json()["token"].as_str().unwrap().to_string();
    assert_eq!(
        server.shared.auth.legacy_clients().len(),
        1,
        "listed until the key is used"
    );

    // The operator revokes the client as listed: the key handed over for it goes too.
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/clients/{old_id}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(
        with_key(addr, "GET", "/api/v1/server", OLD, None)
            .await
            .json()["auth"],
        "local",
        "the old credential is gone"
    );
    assert_eq!(
        with_key(addr, "GET", "/api/v1/server", &key, None)
            .await
            .json()["auth"],
        "local",
        "the key handed over for the revoked client still works"
    );
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// The server machine is trusted only when it is addressed as itself: a page served as
/// `http://oriel.test:5621/` (a name resolving to this machine, e.g. after DNS rebinding)
/// sends its own reads without `Origin`, and they are a remote client's (REST and the
/// WebSocket), while `localhost`, `127.0.0.1` and `[::1]` (any port) stay trusted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_to_another_host_name_is_not_the_server_machine() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let fx = fixture("pairing-host");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;

    for host in [
        addr.to_string(),
        format!("localhost:{}", addr.port()),
        format!("[::1]:{}", addr.port()),
    ] {
        let r = http(addr, "GET", "/api/v1/state", &[("Host", &host)], None).await;
        assert_eq!(r.status, 200, "{host}");
    }
    for host in ["oriel.test:5621", "oriel.test", "192.0.2.10:5555"] {
        let r = http(addr, "GET", "/api/v1/state", &[("Host", host)], None).await;
        assert_eq!(r.status, 401, "{host}");
        let r = http(addr, "GET", "/api/v1/server", &[("Host", host)], None).await;
        assert_eq!(r.status, 200);
        let info = r.json();
        assert_eq!(info["auth"], "unpaired", "{host}");
        assert!(info.get("pairingCode").is_none(), "{info}");
        let r = http(addr, "GET", "/api/v1/pairing/code", &[("Host", host)], None).await;
        assert_eq!(r.status, 401, "{host}");
    }

    // The WebSocket: addressed as `oriel.test`, refused; as the machine, it connects.
    let req = "ws://oriel.test:5621/api/v1/ws"
        .into_client_request()
        .unwrap();
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let ws = tokio_tungstenite::client_async(req, tcp).await;
    match ws {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => assert_eq!(r.status(), 401),
        other => panic!(
            "WebSocket to another host name refused: {:?}",
            other.map(|_| ())
        ),
    }
    let req = format!("ws://localhost:{}/api/v1/ws", addr.port())
        .into_client_request()
        .unwrap();
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    assert!(tokio_tungstenite::client_async(req, tcp).await.is_ok());

    server.shutdown().await.unwrap();
}
