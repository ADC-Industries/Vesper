//! Vesper's releases on GitHub in the site's update channel (`docs/PACKAGING.md`, "Releases"),
//! against a fake GitHub (synthetic releases signed by a throwaway key, passed as an extra
//! update key): new releases for the site's products and platforms come in by themselves, the
//! channel and the off switch, a refused signature, the rate limit, the catalogue and "Add
//! app" queued for a machine.
//!
//! ```text
//! sg render -c "cargo test -p vesper-server --test server releases::"
//! ```

use crate::common::*;
use serde_json::{Value, json};
use vesper_releases::fake::FakeGitHub;
use vesper_server::{RunningServer, ServerConfig, start};

const LINUX: &str = "linux-x86_64";
const WINDOWS: &str = "windows-x86_64";

async fn start_with(fx: &Fixture, fake: &FakeGitHub) -> RunningServer {
    let server = start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        mdns: false,
        site: vesper_server::site::SiteConfig::off(),
        update_keys: vec![fake.public_key()],
        github_api: Some(fake.api()),
        github_tick: std::time::Duration::from_millis(50),
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts");
    // The check at start (when on) ends first.
    idle(&server).await;
    server
}

/// Waits until no check is running or asked for.
async fn idle(server: &RunningServer) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let g = github(server).await;
        if g["checking"] == json!(false)
            && (g["lastCheck"].is_string() || g["settings"]["enabled"] == json!(false))
        {
            return g;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the check didn't end: {g}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// A machine's Launcher reports (as the server machine: trusted here).
async fn report(
    server: &RunningServer,
    id: &str,
    name: &str,
    platform: &str,
    roles: &[&str],
    products: Value,
) {
    let body = json!({
        "machineId": id, "machineName": name, "launcherVersion": "0.1.0", "platform": platform,
        "roles": roles, "products": products, "health": {"cpus": 4, "gpus": []},
    });
    let r = http(
        server.addr,
        "PUT",
        &format!("/api/v1/updates/launchers/{id}"),
        &[],
        Some(body),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.json());
}

async fn github(server: &RunningServer) -> Value {
    get_json(server.addr, "/api/v1/updates/github").await
}

async fn set(server: &RunningServer, settings: Value) -> Resp {
    http(
        server.addr,
        "PUT",
        "/api/v1/updates/github",
        &[],
        Some(settings),
    )
    .await
}

/// Checks now and waits for the check to end; its status.
async fn check(server: &RunningServer) -> Value {
    let r = http(
        server.addr,
        "POST",
        "/api/v1/updates/github/check",
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, 202, "{}", r.json());
    assert_eq!(r.json()["checking"], json!(true));
    idle(server).await
}

async fn packages(server: &RunningServer) -> Vec<(String, String, String, String)> {
    get_json(server.addr, "/api/v1/updates/packages")
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["product"].as_str().unwrap().to_string(),
                p["version"].as_str().unwrap().to_string(),
                p["platform"].as_str().unwrap().to_string(),
                p["source"].as_str().unwrap_or("upload").to_string(),
            )
        })
        .collect()
}

fn site() -> FakeGitHub {
    let fake = FakeGitHub::start();
    fake.release(
        "server-v0.2.0",
        "0.2.0",
        false,
        &[
            (
                "vesper",
                LINUX,
                "vesper_0.2.0-2_amd64.deb",
                b"synthetic server 0.2.0 linux",
            ),
            (
                "vesper",
                WINDOWS,
                "vesper-server-0.2.0-setup.exe",
                b"synthetic server 0.2.0 win",
            ),
        ],
    );
    fake.release(
        "server-v0.3.0-beta1",
        "0.3.0-beta1",
        true,
        &[(
            "vesper",
            LINUX,
            "vesper_0.3.0-beta1-2_amd64.deb",
            b"synthetic server beta",
        )],
    );
    fake.release(
        "oriel-v0.2.0",
        "0.2.0",
        false,
        &[(
            "vesper-oriel",
            LINUX,
            "vesper-oriel_0.2.0_amd64.deb",
            b"synthetic oriel",
        )],
    );
    fake
}

#[tokio::test(flavor = "multi_thread")]
async fn new_github_releases_come_into_the_sites_channel() {
    let fx = fixture("releases-channel");
    let fake = site();
    let server = start_with(&fx, &fake).await;
    let settings = json!({"enabled": true, "repository": "example/vesper", "channel": "stable", "intervalHours": 6});
    assert_eq!(
        set(
            &server,
            json!({"enabled": true, "repository": "nope", "channel": "stable", "intervalHours": 6})
        )
        .await
        .status,
        400
    );
    assert_eq!(set(&server, json!({"enabled": true, "repository": "example/vesper", "channel": "stable", "intervalHours": 0})).await.status, 400);
    assert_eq!(set(&server, settings.clone()).await.status, 200);
    // A Windows machine runs the server too: its platform is looked for as well.
    report(&server, "m-lobby", "Lobby PC", WINDOWS, &["presentation"], json!([
        {"product": "vesper", "name": "Vesper", "version": "0.1.0", "state": "running", "restarts": 0}
    ])).await;
    let g = check(&server).await;
    assert_eq!(g["error"], Value::Null, "{g}");
    assert!(g["lastSuccess"].is_string(), "{g}");
    let mut have = packages(&server).await;
    have.sort();
    let this = vesper_launcher::package::this_platform();
    let want_linux = this == LINUX;
    assert!(
        have.contains(&(
            "vesper".into(),
            "0.2.0".into(),
            WINDOWS.into(),
            "github".into()
        )),
        "{have:?}"
    );
    if want_linux {
        assert!(
            have.contains(&(
                "vesper".into(),
                "0.2.0".into(),
                LINUX.into(),
                "github".into()
            )),
            "{have:?}"
        );
    }
    // Only what the site has: no Oriel, and no beta on the stable channel.
    assert!(
        !have
            .iter()
            .any(|p| p.0 == "vesper-oriel" || p.1.contains("beta")),
        "{have:?}"
    );
    assert_eq!(g["added"].as_array().unwrap().len(), have.len(), "{g}");
    // The machine's Launcher is told about it like an upload.
    let l = get_json(server.addr, "/api/v1/updates/launchers").await;
    assert_eq!(l[0]["updates"][0]["version"], json!("0.2.0"), "{l}");

    // The next check sends the list's ETag: nothing changed, nothing downloaded again.
    let downloads = fake
        .requests()
        .iter()
        .filter(|r| r.path.ends_with(".deb") || r.path.ends_with(".exe"))
        .count();
    check(&server).await;
    let listings = fake.listings();
    assert_eq!(listings.last().unwrap().status, 304, "{listings:?}");
    assert_eq!(
        fake.requests()
            .iter()
            .filter(|r| r.path.ends_with(".deb") || r.path.ends_with(".exe"))
            .count(),
        downloads
    );

    // The beta channel takes the pre-release (where the site's platform has it).
    let mut beta = settings.clone();
    beta["channel"] = json!("beta");
    assert_eq!(set(&server, beta).await.status, 200);
    check(&server).await;
    if want_linux {
        assert!(packages(&server).await.iter().any(|p| p.1 == "0.3.0-beta1"));
    }

    // A release whose package is signed by another key is refused and named.
    let stranger = vesper_launcher::sign::TestSigner::generate();
    let bytes = b"synthetic server 0.4.0";
    let comment = format!("vesper-package product=vesper version=0.4.0 platform={WINDOWS}");
    fake.raw_release(
        "server-v0.4.0",
        false,
        false,
        vec![
            ("v4.exe".into(), bytes.to_vec()),
            ("v4.exe.minisig".into(), stranger.sign(&vesper_launcher::sign::Hasher::bytes(bytes), &comment).into_bytes()),
            ("manifest.json".into(), serde_json::to_vec(&json!({"assets": [{"product": "vesper", "platform": WINDOWS, "file": "v4.exe"}]})).unwrap()),
        ],
    );
    let g = check(&server).await;
    assert!(
        g["error"].as_str().unwrap_or_default().contains("refused"),
        "{g}"
    );
    assert!(!packages(&server).await.iter().any(|p| p.1 == "0.4.0"));

    // GitHub's rate limit: said, and not asked again until it ends.
    let reset = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    fake.rate_limit(Some(reset));
    let g = check(&server).await;
    assert!(
        g["error"]
            .as_str()
            .unwrap_or_default()
            .contains("rate limit"),
        "{g}"
    );
    let asked = fake.listings().len();
    fake.rate_limit(None);
    check(&server).await;
    assert_eq!(
        fake.listings().len(),
        asked,
        "nothing is sent while the limit lasts"
    );

    // The off switch.
    let mut off = settings.clone();
    off["enabled"] = json!(false);
    assert_eq!(set(&server, off).await.status, 200);
    let r = http(
        server.addr,
        "POST",
        "/api/v1/updates/github/check",
        &[],
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, 409);
    assert_eq!(github(&server).await["settings"]["enabled"], json!(false));
    // Kept across a restart.
    server.shutdown().await.unwrap();
    let server = start_with(&fx, &fake).await;
    assert_eq!(github(&server).await["settings"]["enabled"], json!(false));
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn add_app_is_queued_for_a_machine_that_can_run_it() {
    let fx = fixture("releases-add-app");
    let fake = site();
    let server = start_with(&fx, &fake).await;
    assert_eq!(
        set(&server, json!({"enabled": true, "repository": "example/vesper", "channel": "stable", "intervalHours": 6})).await.status,
        200
    );
    check(&server).await;
    report(&server, "m-chapel", "Chapel Box", LINUX, &["display"], json!([
        {"product": "vesper-idn-node", "name": "IDN Node (bundled)", "version": "0.3.8~beta18", "state": "running", "restarts": 0}
    ])).await;

    let all = get_json(server.addr, "/api/v1/updates/catalogue").await;
    let ids: Vec<&str> = all["products"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["product"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"vesper") && ids.contains(&"vesper-oriel"),
        "{all}"
    );
    // A display box: not the Vesper server.
    let cat = get_json(server.addr, "/api/v1/updates/catalogue?machine=m-chapel").await;
    let ids: Vec<&str> = cat["products"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["product"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["vesper-oriel"], "{cat}");
    assert_eq!(cat["products"][0]["stable"], json!("0.2.0"));
    assert_eq!(cat["products"][0]["sources"], json!(["github"]));

    let path = "/api/v1/updates/launchers/m-chapel/actions";
    let queue = |body: Value| http(server.addr, "POST", path, &[], Some(body));
    let r = queue(json!({"type": "install", "product": "vesper", "version": "0.2.0"})).await;
    assert_eq!(r.status, 409, "{}", r.json());
    let r = queue(json!({"type": "install", "product": "vesper-oriel", "version": "9.9.9"})).await;
    assert_eq!(r.status, 409, "{}", r.json());
    let r =
        queue(json!({"type": "install", "product": "vesper-idn-node", "version": "0.3.8~beta18"}))
            .await;
    assert_eq!(r.status, 409, "{}", r.json());
    let r = queue(json!({"type": "install", "product": "vesper-oriel", "version": "0.2.0"})).await;
    assert_eq!(r.status, 202, "{}", r.json());
    assert_eq!(
        r.json()["pending"],
        json!([{"type": "install", "product": "vesper-oriel", "version": "0.2.0"}])
    );
    // Only GitHub had it: the server brings it into the channel for the machine.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !packages(&server)
        .await
        .iter()
        .any(|p| p.0 == "vesper-oriel" && p.2 == LINUX)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "Oriel didn't come into the channel"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // The machine's Launcher gets the install with its next report.
    let body = json!({
        "machineId": "m-chapel", "machineName": "Chapel Box", "launcherVersion": "0.1.0", "platform": LINUX,
        "roles": ["display"], "products": [], "health": {"cpus": 4, "gpus": []},
    });
    let r = http(
        server.addr,
        "PUT",
        "/api/v1/updates/launchers/m-chapel",
        &[],
        Some(body),
    )
    .await;
    assert_eq!(
        r.json()["actions"],
        json!([{"type": "install", "product": "vesper-oriel", "version": "0.2.0"}])
    );
    server.shutdown().await.unwrap();
}
