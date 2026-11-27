//! The site's update channel (G10): signed package uploads (good, tampered, wrong key, twice),
//! listing and downloading them, a Vesper Launcher reporting and picking up the actions queued
//! for it, and the packages surviving a restart. Throwaway signing keys only.
//!
//! ```text
//! sg render -c "cargo test -p vesper-server --test server updates::"
//! ```

use crate::common::*;
use base64::Engine;
use serde_json::{Value, json};
use vesper_launcher::channel::Channel;
use vesper_launcher::package::PackageMeta;
use vesper_launcher::sign::{Hasher, TestSigner};
use vesper_server::{RunningServer, ServerConfig, start};

const PLATFORM: &str = "linux-x86_64";

async fn start_with_key(fx: &Fixture, key: &str) -> RunningServer {
    start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        mdns: false,
        site: vesper_server::site::SiteConfig::off(),
        update_keys: vec![key.to_string()],
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts")
}

fn signed(signer: &TestSigner, bytes: &[u8], version: &str) -> String {
    let meta = PackageMeta {
        product: "vesper".into(),
        version: version.into(),
        platform: PLATFORM.into(),
    };
    signer.sign(&Hasher::bytes(bytes), &meta.comment())
}

fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

async fn upload(server: &RunningServer, bytes: &[u8], sig: &str, extra: &[(&str, &str)]) -> Resp {
    let h = b64(sig);
    let mut headers = vec![("X-Vesper-Signature", h.as_str())];
    headers.extend_from_slice(extra);
    http_bytes(
        server.addr,
        "POST",
        "/api/v1/updates/packages?file=vesper_0.2.0-1_amd64.deb",
        &headers,
        bytes,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn signed_packages_are_uploaded_listed_and_served() {
    let fx = fixture("updates-channel");
    let signer = TestSigner::generate();
    let server = start_with_key(&fx, &signer.public().to_text()).await;
    let caps = get_json(server.addr, "/api/v1/server").await;
    assert!(
        caps["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("updates"))
    );
    let bytes = b"a synthetic package, version 0.2.0".repeat(1000);
    let sig = signed(&signer, &bytes, "0.2.0");

    // Refused: no signature, tampered, another key; and from a page elsewhere (no key).
    let r = http_bytes(server.addr, "POST", "/api/v1/updates/packages", &[], &bytes).await;
    assert_eq!(r.status, 400, "{}", r.json());
    let mut tampered = bytes.clone();
    tampered[3] ^= 1;
    let r = upload(&server, &tampered, &sig, &[]).await;
    assert_eq!(r.status, 400);
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("doesn't match"),
        "{}",
        r.json()
    );
    let r = upload(
        &server,
        &bytes,
        &signed(&TestSigner::generate(), &bytes, "0.2.0"),
        &[],
    )
    .await;
    assert_eq!(r.status, 400);
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("isn't one of"),
        "{}",
        r.json()
    );
    let r = upload(
        &server,
        &bytes,
        &sig,
        &[("Origin", "http://updates.example.com")],
    )
    .await;
    assert_eq!(r.status, 401);

    // Accepted, once.
    let r = upload(&server, &bytes, &sig, &[]).await;
    assert_eq!(r.status, 201, "{}", r.json());
    let p = r.json();
    assert_eq!(
        (
            p["product"].as_str(),
            p["version"].as_str(),
            p["platform"].as_str()
        ),
        (Some("vesper"), Some("0.2.0"), Some(PLATFORM))
    );
    assert_eq!(p["size"], json!(bytes.len()));
    assert_eq!(p["keyId"], json!(signer.public().key_id_hex()));
    assert_eq!(p["fileName"], json!("vesper_0.2.0-1_amd64.deb"));
    let id = p["id"].as_str().unwrap().to_string();
    let r = upload(&server, &bytes, &sig, &[]).await;
    assert_eq!(r.status, 409);

    let list = get_json(server.addr, "/api/v1/updates/packages").await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let r = http(
        server.addr,
        "GET",
        &format!("/api/v1/updates/packages/{id}/file"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, bytes);
    let r = http(
        server.addr,
        "GET",
        &format!("/api/v1/updates/packages/{id}/signature"),
        &[],
        None,
    )
    .await;
    assert_eq!(String::from_utf8_lossy(&r.body), sig);

    // Kept over a restart; then deleted.
    server.shutdown().await.unwrap();
    let server = start_with_key(&fx, &signer.public().to_text()).await;
    let list = get_json(server.addr, "/api/v1/updates/packages").await;
    assert_eq!(list[0]["id"], json!(id));
    let r = http(
        server.addr,
        "DELETE",
        &format!("/api/v1/updates/packages/{id}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204);
    let list = get_json(server.addr, "/api/v1/updates/packages").await;
    assert_eq!(list, json!([]));
    server.shutdown().await.unwrap();
}

/// One report from the Launcher to the channel; the actions it got back.
async fn report(
    c: &Channel,
    l: &std::sync::Arc<vesper_launcher::Launcher>,
) -> Vec<protocol::LauncherAction> {
    let (c, l) = (c.clone(), l.clone());
    tokio::task::spawn_blocking(move || c.report(&l.report()))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launcher_reports_and_picks_up_its_actions() {
    let fx = fixture("updates-launchers");
    let signer = TestSigner::generate();
    let server = start_with_key(&fx, &signer.public().to_text()).await;
    let bytes = b"a synthetic package, version 0.2.0".to_vec();
    let sig = signed(&signer, &bytes, "0.2.0");
    assert_eq!(upload(&server, &bytes, &sig, &[]).await.status, 201);

    // A Launcher on "another machine", with this server as its update channel.
    let state = fx.dir.join("launcher-state");
    let cfg: vesper_launcher::Config = serde_json::from_value(json!({
        "stateDir": state,
        "keyDir": null,
        "keys": [signer.public().to_text()],
        "roles": ["display"],
        "machineName": "Chapel Box",
        "platform": PLATFORM,
        "channel": {"url": format!("http://{}", server.addr)},
        "products": [{
            "product": "vesper",
            "name": "Vesper",
            "install": {"kind": "dir"},
            "healthUrl": "http://127.0.0.1:9/api/v1/server",
        }],
    }))
    .unwrap();
    let launcher = vesper_launcher::Launcher::open(cfg).unwrap();
    let channel = Channel::new(launcher.config().channel.as_ref().unwrap());
    let machine = launcher.machine_id().to_string();
    assert!(report(&channel, &launcher).await.is_empty());

    let list = get_json(server.addr, "/api/v1/updates/launchers").await;
    let e = &list[0];
    assert_eq!(e["report"]["machineName"], json!("Chapel Box"));
    assert_eq!(e["online"], json!(true));
    assert_eq!(
        e["updates"],
        json!([{"product": "vesper", "version": "0.2.0", "package": e["updates"][0]["package"]}])
    );

    // Oriel queues an update; what can't be done is refused.
    let addr = server.addr;
    let path = format!("/api/v1/updates/launchers/{machine}/actions");
    let queue = |body: Value| http(addr, "POST", &path, &[], Some(body));
    let r = queue(json!({"type": "update", "product": "vesper", "version": "9.9.9"})).await;
    assert_eq!(r.status, 409, "{}", r.json());
    let r = queue(json!({"type": "rollback", "product": "vesper"})).await;
    assert_eq!(r.status, 409, "{}", r.json());
    let r = queue(json!({"type": "update", "product": "oriel", "version": "0.2.0"})).await;
    assert_eq!(r.status, 404);
    let r = http(
        server.addr,
        "POST",
        "/api/v1/updates/launchers/nobody/actions",
        &[],
        Some(json!({"type": "rollback", "product": "vesper"})),
    )
    .await;
    assert_eq!(r.status, 404);
    let r = queue(json!({"type": "update", "product": "vesper", "version": "0.2.0"})).await;
    assert_eq!(r.status, 202, "{}", r.json());
    assert_eq!(r.json()["pending"][0]["type"], json!("update"));

    // The next report gets it, once.
    let actions = report(&channel, &launcher).await;
    assert_eq!(
        actions,
        [protocol::LauncherAction::Update {
            product: "vesper".into(),
            version: "0.2.0".into()
        }]
    );
    assert!(report(&channel, &launcher).await.is_empty());

    // The Launcher downloads the package and verifies it into its store.
    let pkgs = {
        let c = channel.clone();
        tokio::task::spawn_blocking(move || c.packages())
            .await
            .unwrap()
            .unwrap()
    };
    let (c, l, p) = (channel.clone(), launcher.clone(), pkgs[0].clone());
    tokio::task::spawn_blocking(move || c.fetch(&l, &p))
        .await
        .unwrap()
        .unwrap();
    let stored = launcher.stored_package("vesper", "0.2.0").expect("stored");
    assert_eq!(std::fs::read(stored).unwrap(), bytes);

    // A report whose machine ID isn't the path's is refused.
    let mut r = launcher.report();
    r.machine_id = "someone-else".into();
    let resp = http(
        server.addr,
        "PUT",
        &format!("/api/v1/updates/launchers/{machine}"),
        &[],
        Some(serde_json::to_value(&r).unwrap()),
    )
    .await;
    assert_eq!(resp.status, 400);
    server.shutdown().await.unwrap();
}

/// The desktop app's pages are another origin than the server's: every custom header Oriel
/// sends must pass the CORS preflight, or the webview refuses the request ("Could not reach
/// the server"). The package upload sends its signature in `X-Vesper-Signature`.
#[tokio::test(flavor = "multi_thread")]
async fn the_cors_preflight_allows_every_header_oriel_sends() {
    let fx = fixture("updates-cors");
    let signer = TestSigner::generate();
    let server = start_with_key(&fx, &signer.public().to_text()).await;
    // (apps/control/src/lib/server: rest.ts, codex.ts, codexBackups.ts, updates.ts)
    let sent =
        "authorization, content-type, if-match, content-range, x-client-id, x-vesper-signature";
    let r = http(
        server.addr,
        "OPTIONS",
        "/api/v1/updates/packages",
        &[
            ("Origin", "tauri://localhost"),
            ("Access-Control-Request-Method", "POST"),
            ("Access-Control-Request-Headers", sent),
        ],
        None,
    )
    .await;
    assert!(r.status < 300, "preflight answered: {}", r.status);
    let allowed = r
        .header("access-control-allow-headers")
        .unwrap()
        .to_lowercase();
    for h in sent.split(", ") {
        assert!(allowed.contains(h), "{h} not in {allowed}");
    }
    server.shutdown().await.unwrap();
}

/// A Launcher's report takes the actions queued for its machine and sets what Site ▸ Updates
/// offers it: only the server machine or a key that may administer the site may send one
/// (until the first-run join gives each Launcher its own credential), not any paired client.
/// What the server keeps for Launchers is bounded.
#[tokio::test(flavor = "multi_thread")]
async fn only_an_admin_or_the_server_machine_reports_as_a_launcher() {
    let fx = fixture("updates-report-access");
    let server = start_with_site(&fx, true, vesper_server::site::SiteConfig::loopback()).await;
    let addr = server.addr;
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": server.pairing_code(), "clientName": "Stage tablet"})),
    )
    .await;
    assert_eq!(r.status, 200);
    let key = r.json()["token"].as_str().unwrap().to_string();
    let bearer = format!("Bearer {key}");
    let auth = [("Authorization", bearer.as_str())];
    let machine = "6d6f636b-0000-4000-8000-000000000001";
    let body = |products: usize| {
        json!({
            "machineId": machine, "machineName": "Chapel Box", "launcherVersion": "0.1.0",
            "platform": PLATFORM, "roles": ["display"],
            "products": (0..products).map(|i| json!({
                "product": format!("p{i}"), "name": "P", "version": "0.1.0", "previous": "0.0.9",
                "state": "running",
            })).collect::<Vec<_>>(),
            "health": {"cpus": 4},
        })
    };
    let path = format!("/api/v1/updates/launchers/{machine}");

    // The server machine reports; an admin queues a rollback for the machine.
    let r = http(addr, "PUT", &path, &[], Some(body(1))).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    // (Open access: every paired client is an admin, so the key may report too.)
    let r = http(addr, "PUT", &path, &auth, Some(body(1))).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));

    // Access `pin`: a locked client is no admin. Its report would take the queued rollback.
    let r = http(
        addr,
        "PUT",
        "/api/v1/site/settings",
        &[],
        Some(json!({"enabled": true, "joinMode": "code_or_approval", "access": "pin", "pin": "2468"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let r = http(
        addr,
        "POST",
        &format!("{path}/actions"),
        &[],
        Some(json!({"type": "rollback", "product": "p0"})),
    )
    .await;
    assert_eq!(r.status, 202, "{}", String::from_utf8_lossy(&r.body));
    let r = http(addr, "PUT", &path, &auth, Some(body(1))).await;
    assert_eq!(r.status, 403, "{}", String::from_utf8_lossy(&r.body));
    let other = body(1)
        .to_string()
        .replace(machine, "6d6f636b-0000-4000-8000-000000000002");
    let r = http_bytes(
        addr,
        "PUT",
        "/api/v1/updates/launchers/6d6f636b-0000-4000-8000-000000000002",
        &[auth[0], ("Content-Type", "application/json")],
        other.as_bytes(),
    )
    .await;
    assert_eq!(r.status, 403, "a new machine neither");
    let list = get_json(addr, "/api/v1/updates/launchers").await;
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
    assert_eq!(
        list[0]["pending"].as_array().unwrap().len(),
        1,
        "still queued: {list}"
    );
    let r = http(addr, "PUT", &path, &[], Some(body(1))).await;
    assert_eq!(
        r.json()["actions"].as_array().unwrap().len(),
        1,
        "the machine's Launcher gets it"
    );

    // Bounded: products per report, the report's size.
    let r = http(addr, "PUT", &path, &[], Some(body(33))).await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    let mut big = body(1);
    big["health"]["gpus"] = json!(vec!["x".repeat(1000); 100]);
    let r = http(addr, "PUT", &path, &[], Some(big)).await;
    assert_eq!(r.status, 413, "{}", String::from_utf8_lossy(&r.body));
    server.shutdown().await.unwrap();
}

/// A Launcher whose state is in `fx`'s `name` folder, reporting to nothing yet.
fn launcher(
    fx: &Fixture,
    name: &str,
    machine_name: &str,
) -> std::sync::Arc<vesper_launcher::Launcher> {
    let cfg: vesper_launcher::Config = serde_json::from_value(json!({
        "stateDir": fx.dir.join(name),
        "keyDir": null,
        "machineName": machine_name,
        "platform": PLATFORM,
        "products": [{
            "product": "vesper",
            "name": "Vesper",
            "install": {"kind": "dir"},
            "healthUrl": "http://127.0.0.1:9/api/v1/server",
        }],
    }))
    .unwrap();
    vesper_launcher::Launcher::open(cfg).unwrap()
}

/// One report through the Launcher's own channel (the site it joined): the status and the
/// actions, or the server's refusal.
async fn report_joined(
    l: &std::sync::Arc<vesper_launcher::Launcher>,
) -> anyhow::Result<Vec<protocol::LauncherAction>> {
    let l = l.clone();
    tokio::task::spawn_blocking(move || l.channel().expect("joined").report(&l.report()))
        .await
        .unwrap()
}

/// Waits until `l`'s join is answered.
async fn joined(l: &std::sync::Arc<vesper_launcher::Launcher>) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let s = serde_json::to_value(l.setup_view()).unwrap();
        if s["join"]["state"] != json!("waiting") {
            return s["join"].clone();
        }
        assert!(std::time::Instant::now() < deadline, "still waiting: {s}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// The join round (per-machine Launcher credentials): a Launcher joins the site's server with
/// its code, or waits for an admin's approval, and gets its machine's own Launcher key. With
/// it, it reports as its own machine and picks up only its own actions; it can't report as
/// another machine, take another machine's actions, or use any other route. Revoked, it stops;
/// joining again replaces it.
#[tokio::test(flavor = "multi_thread")]
async fn a_launcher_joins_the_site_and_reports_only_as_its_own_machine() {
    let fx = fixture("updates-launcher-join");
    let server = start_with_site(&fx, true, vesper_server::site::SiteConfig::loopback()).await;
    let addr = server.addr;
    let url = format!("http://{addr}");
    // Access `pin`: no paired client is an admin.
    let r = http(
        addr,
        "PUT",
        "/api/v1/site/settings",
        &[],
        Some(json!({"enabled": true, "joinMode": "code_or_approval", "access": "pin", "pin": "2468"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));

    // Chapel Box joins with the server's code: its key at once.
    let a = launcher(&fx, "launcher-a", "Chapel Box");
    assert!(a.setup_view().first_run);
    let s = a.join(&url, Some(server.pairing_code())).await.unwrap();
    assert_eq!(serde_json::to_value(&s).unwrap()["state"], json!("joined"));
    let key_a = a.site().unwrap();
    assert_eq!(key_a.server, url);
    let keys = get_json(addr, "/api/v1/site/api-keys").await;
    let k = keys
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["id"] == json!(key_a.key_id))
        .expect("listed");
    assert_eq!(k["name"], json!("Vesper Launcher – Chapel Box"));
    assert_eq!(k["engine"], json!("app:vesper-launcher"));
    assert!(report_joined(&a).await.unwrap().is_empty());

    // Lobby Box asks without a code: it waits for an admin's approval, listed as a Launcher.
    let b = launcher(&fx, "launcher-b", "Lobby Box");
    let s = b.join(&url, None).await.unwrap();
    assert_eq!(serde_json::to_value(&s).unwrap()["state"], json!("waiting"));
    let pairings = get_json(addr, "/api/v1/site/pairings").await;
    let p = &pairings[0];
    assert_eq!(p["client"], json!("Vesper Launcher – Lobby Box"));
    assert_eq!(p["launcher"], json!(b.machine_id()));
    let r = http(
        addr,
        "POST",
        &format!(
            "/api/v1/site/pairings/{}/approve",
            p["id"].as_str().unwrap()
        ),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(joined(&b).await["state"], json!("joined"));
    let key_b = b.site().unwrap();
    assert!(report_joined(&b).await.unwrap().is_empty());

    // Lobby Box has a version to go back to; an admin queues the rollback for it. Chapel
    // Box's report doesn't take it, and Chapel Box's key can't report as Lobby Box (nor as a
    // new machine); Lobby Box's own report does.
    let bearer_b = format!("Bearer {}", key_b.token);
    let mut rb = b.report();
    rb.products[0].version = Some("0.2.0".into());
    rb.products[0].previous = Some("0.1.0".into());
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/updates/launchers/{}", b.machine_id()),
        &[("Authorization", bearer_b.as_str())],
        Some(serde_json::to_value(&rb).unwrap()),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/updates/launchers/{}/actions", b.machine_id()),
        &[],
        Some(json!({"type": "rollback", "product": "vesper"})),
    )
    .await;
    assert_eq!(r.status, 202, "{}", r.json());
    assert!(report_joined(&a).await.unwrap().is_empty());
    let bearer_a = format!("Bearer {}", key_a.token);
    let auth_a = [("Authorization", bearer_a.as_str())];
    let mut as_b = b.report();
    as_b.machine_name = "Chapel Box pretending".into();
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/updates/launchers/{}", b.machine_id()),
        &auth_a,
        Some(serde_json::to_value(&as_b).unwrap()),
    )
    .await;
    assert_eq!(r.status, 403, "{}", String::from_utf8_lossy(&r.body));
    let list = get_json(addr, "/api/v1/updates/launchers").await;
    let lobby = list
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["report"]["machineId"] == json!(b.machine_id()))
        .unwrap();
    assert_eq!(
        lobby["report"]["machineName"],
        json!("Lobby Box"),
        "unchanged"
    );
    assert_eq!(
        lobby["pending"].as_array().unwrap().len(),
        1,
        "still queued"
    );
    let r = http(
        addr,
        "PUT",
        "/api/v1/updates/launchers/6d6f636b-0000-4000-8000-00000000000a",
        &auth_a,
        Some(
            serde_json::to_value(&{
                let mut n = a.report();
                n.machine_id = "6d6f636b-0000-4000-8000-00000000000a".into();
                n
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(r.status, 403, "a new machine neither");
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/updates/launchers/{}", b.machine_id()),
        &[("Authorization", bearer_b.as_str())],
        Some(serde_json::to_value(&rb).unwrap()),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.json()["actions"],
        json!([{"type": "rollback", "product": "vesper"}]),
        "Lobby Box's own Launcher takes it"
    );

    // A Launcher key reaches the package list, and nothing else: not the Launchers' list,
    // not the show, not queueing actions.
    let r = http(addr, "GET", "/api/v1/updates/packages", &auth_a, None).await;
    assert_eq!(r.status, 200);
    for (method, path) in [
        ("GET", "/api/v1/updates/launchers".to_string()),
        ("GET", "/api/v1/show".to_string()),
        ("GET", "/api/v1/site/api-keys".to_string()),
        (
            "POST",
            format!("/api/v1/updates/launchers/{}/actions", a.machine_id()),
        ),
    ] {
        let body = (method == "POST").then(|| json!({"type": "rollback", "product": "vesper"}));
        let r = http(addr, method, &path, &auth_a, body).await;
        assert_eq!(
            r.status,
            401,
            "{method} {path}: {}",
            String::from_utf8_lossy(&r.body)
        );
    }
    // A client's key (an Oriel tablet, locked under `pin`) can't report as a Launcher.
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": server.pairing_code(), "clientName": "Stage tablet"})),
    )
    .await;
    let tablet = format!("Bearer {}", r.json()["token"].as_str().unwrap());
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/updates/launchers/{}", b.machine_id()),
        &[("Authorization", tablet.as_str())],
        Some(serde_json::to_value(b.report()).unwrap()),
    )
    .await;
    assert_eq!(r.status, 403, "{}", String::from_utf8_lossy(&r.body));
    // A bad machine ID is refused before anything is made.
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"clientName": "x", "launcher": "a/b"})),
    )
    .await;
    assert_eq!(r.status, 400);

    // Revoked in Site ▸ API keys: Chapel Box's reports stop.
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/site/api-keys/{}", key_a.key_id),
        &[],
        None,
    )
    .await;
    assert!(r.status < 300, "{}", String::from_utf8_lossy(&r.body));
    // (This test's requests come from the server machine, which a key that is no longer valid
    // falls back to; from another machine the report is `401`.)
    assert_eq!(server.shared.site.verify_launcher(&key_a.token), None);

    // Lobby Box joins again (the wizard run again): a new key, the one before revoked.
    b.leave().unwrap();
    assert!(b.site().is_none());
    b.join(&url, Some(server.pairing_code())).await.unwrap();
    let key_b2 = b.site().unwrap();
    assert_ne!(key_b2.key_id, key_b.key_id);
    assert!(report_joined(&b).await.unwrap().is_empty());
    assert_eq!(
        server.shared.site.verify_launcher(&key_b.token),
        None,
        "the key from before is revoked"
    );
    assert_eq!(
        server
            .shared
            .site
            .verify_launcher(&key_b2.token)
            .map(|k| k.machine_id),
        Some(b.machine_id().to_string())
    );
    server.shutdown().await.unwrap();
}

/// Round 3: a display box's Launcher gets an IDN join code from the site's server with its own
/// Launcher key (for `idn-node join CODE`), so nobody types a second code. Only for its own
/// machine; nobody else's key gets one here. Asking again takes the machine's waiting code back.
#[tokio::test(flavor = "multi_thread")]
async fn a_launcher_gets_an_idn_join_code_for_its_own_machine_only() {
    let fx = fixture("updates-launcher-idn-join");
    let server = start_with_site(&fx, true, vesper_server::site::SiteConfig::loopback()).await;
    let addr = server.addr;
    let url = format!("http://{addr}");
    let r = http(
        addr,
        "PUT",
        "/api/v1/site/settings",
        &[],
        Some(json!({"enabled": true, "joinMode": "code_or_approval", "access": "pin", "pin": "2468"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let a = launcher(&fx, "launcher-a", "Lobby Box");
    a.join(&url, Some(server.pairing_code())).await.unwrap();
    let b = launcher(&fx, "launcher-b", "Chapel Box");
    b.join(&url, Some(server.pairing_code())).await.unwrap();
    let waiting = |v: &Value| v.as_array().unwrap().len();
    let before = waiting(&get_json(addr, "/api/v1/site/join-codes").await);

    // Lobby Box's Launcher asks for its machine: a six-digit code, waiting at the site.
    let (a2, machine) = (a.clone(), a.machine_id().to_string());
    let code = tokio::task::spawn_blocking(move || a2.channel().unwrap().idn_join_code(&machine))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(code.code.len(), 6, "{code:?}");
    assert!(code.code.chars().all(|c| c.is_ascii_digit()));
    assert!(code.expires_in_seconds > 500, "{code:?}");
    assert_eq!(
        waiting(&get_json(addr, "/api/v1/site/join-codes").await),
        before + 1
    );
    // Asking again replaces it: still one waiting for the machine.
    let (a2, machine) = (a.clone(), a.machine_id().to_string());
    let again = tokio::task::spawn_blocking(move || a2.channel().unwrap().idn_join_code(&machine))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(again.code, code.code);
    assert_eq!(
        waiting(&get_json(addr, "/api/v1/site/join-codes").await),
        before + 1
    );

    // Chapel Box's key can't ask for Lobby Box; no key (the server machine) or a client's key
    // gets none either.
    let bearer_b = format!("Bearer {}", b.site().unwrap().token);
    let path = format!("/api/v1/updates/launchers/{}/idn-join", a.machine_id());
    let r = http(
        addr,
        "POST",
        &path,
        &[("Authorization", bearer_b.as_str())],
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, 403, "{}", String::from_utf8_lossy(&r.body));
    let r = http(addr, "POST", &path, &[], Some(json!({}))).await;
    assert_eq!(r.status, 403, "{}", String::from_utf8_lossy(&r.body));
    server.shutdown().await.unwrap();
}
