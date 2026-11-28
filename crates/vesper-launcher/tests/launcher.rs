//! The Launcher end to end on this machine, with a fake product: an update that comes up
//! healthy, one that doesn't (rolled back automatically), a rollback by hand, refused packages
//! and the local API.
//!
//! The fake product is this test binary itself: a package is a shell script that runs it with
//! `fake_product --exact`, which then answers `GET /api/v1/server` with its version.

#![cfg(unix)]

use protocol::{LauncherReport, ProductState, UpdateResult};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use vesper_launcher::package::{PackageMeta, this_platform};
use vesper_launcher::sign::{Hasher, TestSigner};
use vesper_launcher::{Config, Launcher};

/// The fake product: serves `{"version": …}` on its port when started by a package's script.
#[test]
fn fake_product() {
    let (Ok(port), Ok(version)) = (
        std::env::var("VESPER_FAKE_PRODUCT_PORT"),
        std::env::var("VESPER_FAKE_PRODUCT_VERSION"),
    ) else {
        return; // An ordinary test run.
    };
    let l = std::net::TcpListener::bind(format!("127.0.0.1:{port}")).expect("the port is free");
    for s in l.incoming() {
        let Ok(mut s) = s else { continue };
        let mut buf = [0u8; 2048];
        let _ = s.read(&mut buf);
        let body = format!("{{\"version\":\"{version}\"}}");
        let _ = write!(
            s,
            "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    signer: TestSigner,
    port: u16,
    launcher: Arc<Launcher>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let signer = TestSigner::generate();
    let port = free_port();
    let cfg: Config = serde_json::from_value(serde_json::json!({
        "stateDir": root.join("state"),
        "listen": "127.0.0.1:0",
        "keyDir": null,
        "keys": [signer.public().to_text()],
        "roles": ["presentation"],
        "machineName": "test-box",
        "healthTimeoutSecs": 4,
        "products": [{
            "product": "fake",
            "name": "Fake product",
            "install": {"kind": "dir"},
            "healthUrl": format!("http://127.0.0.1:{port}/api/v1/server"),
        }],
    }))
    .unwrap();
    let launcher = Launcher::open(cfg).unwrap();
    Fixture {
        _dir: dir,
        root,
        signer,
        port,
        launcher,
    }
}

impl Fixture {
    /// A package of version `version`: a script that runs the fake product (or `body`).
    fn package(&self, version: &str, healthy: bool) -> (PathBuf, String) {
        let dir = self.root.join("build").join(version);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake-product");
        let exe = std::env::current_exe().unwrap();
        let script = if healthy {
            format!(
                "#!/bin/sh\nVESPER_FAKE_PRODUCT_PORT={} VESPER_FAKE_PRODUCT_VERSION={version} exec '{}' fake_product --exact --nocapture --test-threads=1\n",
                self.port,
                exe.display()
            )
        } else {
            // A version that crashes at start.
            "#!/bin/sh\nexit 3\n".to_string()
        };
        std::fs::write(&path, script).unwrap();
        let meta = PackageMeta {
            product: "fake".into(),
            version: version.into(),
            platform: this_platform(),
        };
        let sig = self
            .signer
            .sign(&Hasher::file(&path).unwrap(), &meta.comment());
        (path, sig)
    }

    fn add(&self, version: &str, healthy: bool) {
        let (path, sig) = self.package(version, healthy);
        self.launcher
            .add_package(&path, "fake-product", &sig)
            .unwrap();
    }

    fn report(&self) -> LauncherReport {
        self.launcher.report()
    }
}

fn reported_version(port: u16) -> Option<String> {
    match vesper_launcher::health::probe(
        &format!("http://127.0.0.1:{port}/api/v1/server"),
        std::time::Duration::from_secs(2),
    ) {
        vesper_launcher::health::Probe::Up(v) => v,
        vesper_launcher::health::Probe::Down(_) => None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_that_fails_its_health_check_is_rolled_back() {
    let fx = fixture();
    // First install.
    fx.add("1.0.0", true);
    let o = fx.launcher.update("fake", "1.0.0").await.unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{o:?}");
    assert_eq!(reported_version(fx.port).as_deref(), Some("1.0.0"));

    // 2.0.0 crashes at start: back to 1.0.0 by itself.
    fx.add("2.0.0", false);
    let o = fx.launcher.update("fake", "2.0.0").await.unwrap();
    assert_eq!(o.result, UpdateResult::RolledBack, "{o:?}");
    assert_eq!(o.from.as_deref(), Some("1.0.0"));
    assert!(o.message.contains("wasn't healthy"), "{}", o.message);
    assert_eq!(reported_version(fx.port).as_deref(), Some("1.0.0"));
    let r = fx.report();
    let p = &r.products[0];
    assert_eq!(p.version.as_deref(), Some("1.0.0"));
    assert_eq!(p.state, ProductState::Running);
    assert!(p.restarts >= 1, "the crashing version was restarted: {p:?}");
    assert_eq!(
        p.last_update.as_ref().unwrap().result,
        UpdateResult::RolledBack
    );

    // 3.0.0 is healthy; then rolled back by hand to 1.0.0.
    fx.add("3.0.0", true);
    let o = fx.launcher.update("fake", "3.0.0").await.unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{o:?}");
    assert_eq!(reported_version(fx.port).as_deref(), Some("3.0.0"));
    let i = fx.launcher.installed("fake");
    assert_eq!(
        (i.version.as_deref(), i.previous.as_deref()),
        (Some("3.0.0"), Some("1.0.0"))
    );
    // Only the current and previous packages are kept.
    let kept: Vec<String> = std::fs::read_dir(fx.root.join("state/packages/fake"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(kept.len(), 2, "{kept:?}");
    let o = fx.launcher.rollback("fake").await.unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{o:?}");
    assert_eq!(reported_version(fx.port).as_deref(), Some("1.0.0"));
    let i = fx.launcher.installed("fake");
    assert_eq!(
        (i.version.as_deref(), i.previous.as_deref()),
        (Some("1.0.0"), Some("3.0.0"))
    );

    // The manifest survives a restart of the Launcher.
    fx.launcher.shutdown().await;
    let again = Launcher::open(fx.launcher.config().clone()).unwrap();
    assert_eq!(again.installed("fake").version.as_deref(), Some("1.0.0"));
    assert_eq!(again.machine_id(), fx.launcher.machine_id());
}

/// Marks an update from `from` to `to` as under way in the manifest, as a Launcher stopped
/// (a restart, a power cut) after the installer ran but before the health check ended
/// leaves it.
fn interrupt(fx: &Fixture, from: &str, to: &str) {
    let path = fx.root.join("state/manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let p = &mut m["products"]["fake"];
    assert_eq!(p["version"], from);
    p["inProgress"] = serde_json::json!({"from": from, "to": to});
    std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_update_is_finished_when_the_launcher_starts() {
    let fx = fixture();
    fx.add("1.0.0", true);
    fx.launcher.update("fake", "1.0.0").await.unwrap();
    fx.add("2.0.0", true);
    fx.launcher.shutdown().await;

    // Interrupted on the way to 2.0.0 (healthy): kept, with 1.0.0 to go back to.
    interrupt(&fx, "1.0.0", "2.0.0");
    let l = Launcher::open(fx.launcher.config().clone()).unwrap();
    l.start_products().await;
    assert_eq!(reported_version(fx.port).as_deref(), Some("2.0.0"));
    let i = l.installed("fake");
    assert_eq!(
        (i.version.as_deref(), i.previous.as_deref()),
        (Some("2.0.0"), Some("1.0.0"))
    );
    assert!(i.in_progress.is_none());
    assert_eq!(i.last_update.unwrap().result, UpdateResult::Installed);
    assert_eq!(l.report().products[0].state, ProductState::Running);
    l.shutdown().await;

    // Interrupted on the way to 3.0.0 (crashes): 2.0.0 is put back.
    fx.add("3.0.0", false);
    interrupt(&fx, "2.0.0", "3.0.0");
    let l = Launcher::open(fx.launcher.config().clone()).unwrap();
    l.start_products().await;
    assert_eq!(reported_version(fx.port).as_deref(), Some("2.0.0"));
    let i = l.installed("fake");
    assert_eq!(
        (i.version.as_deref(), i.previous.as_deref()),
        (Some("2.0.0"), Some("1.0.0")),
        "a hand rollback goes back to 1.0.0"
    );
    assert!(i.in_progress.is_none());
    assert_eq!(i.last_update.unwrap().result, UpdateResult::RolledBack);
    l.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_install_that_fails_has_nothing_to_go_back_to() {
    let fx = fixture();
    fx.add("1.0.0", false);
    let o = fx.launcher.update("fake", "1.0.0").await.unwrap();
    assert_eq!(o.result, UpdateResult::Failed, "{o:?}");
    assert!(o.message.contains("no earlier version"), "{}", o.message);
    assert_eq!(fx.report().products[0].state, ProductState::Failed);
    fx.launcher.shutdown().await;
}

#[test]
fn bad_packages_are_refused() {
    let fx = fixture();
    // Tampered after signing.
    let (path, sig) = fx.package("1.0.0", true);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"# changed\n")
        .unwrap();
    let e = fx
        .launcher
        .add_package(&path, "fake-product", &sig)
        .unwrap_err();
    assert!(e.to_string().contains("doesn't match"), "{e}");
    // Signed with a key this machine doesn't trust.
    let (path, _) = fx.package("1.0.1", true);
    let meta = PackageMeta {
        product: "fake".into(),
        version: "1.0.1".into(),
        platform: this_platform(),
    };
    let stranger = TestSigner::generate().sign(&Hasher::file(&path).unwrap(), &meta.comment());
    let e = fx
        .launcher
        .add_package(&path, "fake-product", &stranger)
        .unwrap_err();
    assert!(e.to_string().contains("isn't one of"), "{e}");
    // For another platform, or another product.
    for (product, platform, expect) in [
        ("fake", "plan9-mips", "is for plan9-mips"),
        ("other", this_platform().as_str(), "doesn't manage"),
    ] {
        let meta = PackageMeta {
            product: product.into(),
            version: "1.0.2".into(),
            platform: platform.into(),
        };
        let sig = fx
            .signer
            .sign(&Hasher::file(&path).unwrap(), &meta.comment());
        let e = fx
            .launcher
            .add_package(&path, "fake-product", &sig)
            .unwrap_err();
        assert!(e.to_string().contains(expect), "{e}");
    }
    // Nothing was stored, so nothing can be installed.
    assert!(fx.launcher.stored_package("fake", "1.0.0").is_none());
    assert!(!Path::new(&fx.root.join("state/packages/fake/1.0.1")).exists());
}

async fn get(addr: std::net::SocketAddr, path: &str, host: &str) -> (u16, String) {
    send(
        addr,
        &format!("GET {path} HTTP/1.0\r\nHost: {host}\r\n\r\n"),
    )
    .await
}

/// Sends `raw` (a whole HTTP/1.0 request): its status and body.
async fn send(addr: std::net::SocketAddr, raw: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(raw.as_bytes()).await.unwrap();
    let mut b = Vec::new();
    s.read_to_end(&mut b).await.unwrap();
    let t = String::from_utf8_lossy(&b).into_owned();
    let status = t.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = t
        .split_once("\r\n\r\n")
        .map(|x| x.1.to_string())
        .unwrap_or_default();
    (status, body)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_local_api_reports_and_answers_only_on_this_machine() {
    let fx = fixture();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vesper_launcher::api::router(fx.launcher.clone())
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let (s, body) = get(
        addr,
        "/launcher/v1/status",
        &format!("127.0.0.1:{}", addr.port()),
    )
    .await;
    assert_eq!(s, 200, "{body}");
    let r: LauncherReport = serde_json::from_str(&body).unwrap();
    assert_eq!(r.machine_name, "test-box");
    assert_eq!(r.roles, ["presentation"]);
    assert_eq!(r.products[0].product, "fake");
    assert_eq!(r.products[0].state, ProductState::Stopped);
    assert!(r.health.cpus >= 1);

    // A page that reached it under another name (DNS rebinding) is refused.
    let (s, _) = get(addr, "/launcher/v1/status", "updates.example.com").await;
    assert_eq!(s, 403);
    let (s, _) = get(addr, "/launcher/v1/products/nothing/logs", "localhost").await;
    assert_eq!(s, 404);

    // A web page open in a browser on this machine can't act through it: its `fetch()`
    // names it (`Origin`), its `<img>` and form posts are marked (`Sec-Fetch-Site`,
    // `Referer`), and a no-cors post can only send `text/plain`.
    let host = format!("127.0.0.1:{}", addr.port());
    let post = |headers: &str, body: &str| {
        format!(
            "POST /launcher/v1/actions HTTP/1.0\r\nHost: {host}\r\n{headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
    };
    let rollback = r#"{"type":"rollback","product":"nothing"}"#;
    for headers in [
        "Origin: http://attacker.example\r\nContent-Type: text/plain\r\n",
        "Origin: http://localhost:8080\r\nContent-Type: application/json\r\n",
        "Sec-Fetch-Site: cross-site\r\nContent-Type: application/json\r\n",
        "Referer: http://attacker.example/\r\nContent-Type: application/json\r\n",
    ] {
        let (s, body) = send(addr, &post(headers, rollback)).await;
        assert_eq!(s, 403, "{headers}: {body}");
    }
    let (s, body) = send(addr, &post("Content-Type: text/plain\r\n", rollback)).await;
    assert_eq!(s, 415, "a body that isn't JSON: {body}");
    let (s, _) = send(
        addr,
        &format!(
            "GET /launcher/v1/status HTTP/1.0\r\nHost: {host}\r\nSec-Fetch-Site: cross-site\r\n\r\n"
        ),
    )
    .await;
    assert_eq!(s, 403);
    // A script on the machine (or a URL typed into a browser) is answered.
    let (s, _) = send(
        addr,
        &format!(
            "GET /launcher/v1/status HTTP/1.0\r\nHost: {host}\r\nSec-Fetch-Site: none\r\n\r\n"
        ),
    )
    .await;
    assert_eq!(s, 200);
    let (s, body) = send(
        addr,
        &post(
            "Content-Type: application/json; charset=utf-8\r\n",
            rollback,
        ),
    )
    .await;
    assert_eq!(s, 404, "it reaches the Launcher (no such product): {body}");
}

/// The Launcher's own window (`GET /`, the tray app shows it) is a page of the local API: its
/// requests are same-origin and answered; a page of any other origin still isn't. The setup
/// (roles, the wizard's end) is kept; products start and stop by hand.
#[tokio::test(flavor = "multi_thread")]
async fn the_launchers_own_page_works_and_products_start_and_stop() {
    let fx = fixture();
    fx.add("1.0.0", true);
    let o = fx.launcher.update("fake", "1.0.0").await.unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{}", o.message);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vesper_launcher::api::router(fx.launcher.clone())
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let host = format!("127.0.0.1:{}", addr.port());
    let own = format!("http://{host}");

    // The page: typed into a browser (or opened by the tray), never framed.
    let (s, body) = send(
        addr,
        &format!("GET / HTTP/1.0\r\nHost: {host}\r\nSec-Fetch-Site: none\r\n\r\n"),
    )
    .await;
    assert_eq!(s, 200);
    assert!(body.contains("<title>Vesper Launcher</title>"));
    let req = |method: &str, path: &str, headers: &str, body: &str| {
        format!(
            "{method} {path} HTTP/1.0\r\nHost: {host}\r\n{headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
    };
    // Its own requests: same-origin `fetch()`es (`Origin` on changes, `Referer`).
    let page = format!(
        "Origin: {own}\r\nReferer: {own}/\r\nSec-Fetch-Site: same-origin\r\nContent-Type: application/json\r\n"
    );
    let (s, body) = send(
        addr,
        &req(
            "GET",
            "/launcher/v1/overview",
            &format!("Referer: {own}/\r\nSec-Fetch-Site: same-origin\r\n"),
            "",
        ),
    )
    .await;
    assert_eq!(s, 200, "{body}");
    let ov: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(ov["setup"]["firstRun"], true);
    assert_eq!(ov["setup"]["join"]["state"], "notJoined");
    assert_eq!(ov["status"]["products"][0]["state"], "running");
    assert_eq!(ov["light"], "ok");
    let (s, body) = send(
        addr,
        &req(
            "PUT",
            "/launcher/v1/setup",
            &page,
            r#"{"roles":["display"],"done":true}"#,
        ),
    )
    .await;
    assert_eq!(s, 200, "{body}");
    // Not a role; a page of another origin (another port on this machine, or a look-alike
    // prefix); a change sent as a form.
    let (s, _) = send(
        addr,
        &req("PUT", "/launcher/v1/setup", &page, r#"{"roles":["dj"]}"#),
    )
    .await;
    assert_eq!(s, 409);
    let (s, _) = send(
        addr,
        &req(
            "PUT",
            "/launcher/v1/setup",
            &page,
            r#"{"roles":["display","presentation"]}"#,
        ),
    )
    .await;
    assert_eq!(s, 409, "a display box runs no Vesper server");
    for headers in [
        "Origin: http://127.0.0.1:1\r\nContent-Type: application/json\r\n".to_string(),
        format!("Origin: {own}0\r\nContent-Type: application/json\r\n"),
        format!("Referer: {own}0/\r\nContent-Type: application/json\r\n"),
        "Sec-Fetch-Site: same-site\r\nContent-Type: application/json\r\n".to_string(),
    ] {
        let (s, body) = send(
            addr,
            &req("PUT", "/launcher/v1/setup", &headers, r#"{"roles":[]}"#),
        )
        .await;
        assert_eq!(s, 403, "{headers}: {body}");
    }
    let (s, _) = send(
        addr,
        &req(
            "DELETE",
            "/launcher/v1/setup/join",
            &format!("Origin: {own}\r\nContent-Type: text/plain\r\n"),
            "",
        ),
    )
    .await;
    assert_eq!(s, 415);

    // Kept over a restart of the Launcher.
    let again = Launcher::open(fx.launcher.config().clone()).unwrap();
    let v = again.setup_view();
    assert!(!v.first_run);
    assert_eq!(v.roles, ["display"]);
    assert_eq!(again.report().roles, ["display"]);

    // Stop by hand: down, shown `stopped` (not failed); Launch: up again.
    let (s, body) = send(
        addr,
        &req("POST", "/launcher/v1/products/fake/stop", &page, "{}"),
    )
    .await;
    assert_eq!(s, 204, "{body}");
    assert_eq!(reported_version(fx.port), None);
    fx.launcher.check().await;
    assert_eq!(fx.report().products[0].state, ProductState::Stopped);
    let (s, body) = send(
        addr,
        &req("POST", "/launcher/v1/products/fake/start", &page, "{}"),
    )
    .await;
    assert_eq!(s, 204, "{body}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while reported_version(fx.port).is_none() {
        assert!(std::time::Instant::now() < deadline, "it didn't come back");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    fx.launcher.check().await;
    assert_eq!(fx.report().products[0].state, ProductState::Running);

    // Joining a server that isn't there says so, for people.
    let (s, body) = send(
        addr,
        &req(
            "POST",
            "/launcher/v1/setup/join",
            &page,
            r#"{"server":"127.0.0.1:9"}"#,
        ),
    )
    .await;
    assert_eq!(s, 409, "{body}");
    assert!(body.contains("no Vesper server answers"), "{body}");
    fx.launcher.shutdown().await;
}

/// A `deb` product with a fake apt and dpkg: `apt-get` logs its call and "installs" the package
/// (a text file naming its version, and `broken` for one that never becomes healthy),
/// `dpkg-query` says what is installed, and a health server answers with the installed version.
struct DebFixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    signer: TestSigner,
    cfg: Config,
}

fn deb_fixture(installed: &str) -> DebFixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (bin, db) = (root.join("bin"), root.join("dpkg"));
    for d in [&bin, &db, &root.join("apt"), &root.join("build")] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(db.join("installed"), installed).unwrap();
    std::fs::write(db.join("lock-frontend"), "").unwrap();
    std::fs::write(db.join("lock"), "").unwrap();
    let db_s = db.display();
    let scripts = [
        (
            "dpkg-query",
            format!(
                "#!/bin/sh\n[ -s '{db_s}/installed' ] || exit 1\nprintf 'installed\\t%s' \"$(cat '{db_s}/installed')\"\n"
            ),
        ),
        (
            "apt-get",
            format!(
                "#!/bin/sh\necho \"$DEBIAN_FRONTEND $*\" >> '{db_s}/calls'\nfor a; do pkg=$a; done\nsed -n 's/^version=//p' \"$pkg\" | tr -d '\\n' > '{db_s}/installed'\nif grep -q broken \"$pkg\"; then touch '{db_s}/down'; else rm -f '{db_s}/down'; fi\n"
            ),
        ),
    ];
    for (name, text) in scripts {
        let p = bin.join(name);
        std::fs::write(&p, text).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // The product's health check: the installed version without its packaging revision.
    let health = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = health.local_addr().unwrap().port();
    let db2 = db.clone();
    std::thread::spawn(move || {
        for s in health.incoming() {
            let Ok(mut s) = s else { continue };
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let v = std::fs::read_to_string(db2.join("installed")).unwrap_or_default();
            let v = v.split('-').next().unwrap_or_default().to_string();
            let (status, body) = if db2.join("down").exists() || v.is_empty() {
                ("500 Down", String::new())
            } else {
                ("200 OK", format!("{{\"version\":\"{v}\"}}"))
            };
            let _ = write!(
                s,
                "HTTP/1.0 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let signer = TestSigner::generate();
    let cfg: Config = serde_json::from_value(serde_json::json!({
        "stateDir": root.join("state"),
        "listen": "127.0.0.1:0",
        "keyDir": null,
        "keys": [signer.public().to_text()],
        "healthTimeoutSecs": 2,
        "products": [{
            "product": "vesper",
            "name": "Vesper",
            "install": {"kind": "deb", "units": []},
            "healthUrl": format!("http://127.0.0.1:{port}/api/v1/server"),
        }],
        "dpkg": {
            "apt": bin.join("apt-get"),
            "query": bin.join("dpkg-query"),
            "adminDir": db,
            "lockTimeoutSecs": 20,
            "aptArchives": root.join("apt"),
        },
    }))
    .unwrap();
    DebFixture {
        _dir: dir,
        root,
        signer,
        cfg,
    }
}

impl DebFixture {
    /// A `.deb` of `version` (`healthy` or not), in `dir`.
    fn deb(&self, dir: &Path, version: &str, healthy: bool) -> PathBuf {
        let path = dir.join(format!("vesper_{version}_amd64.deb"));
        let text = format!(
            "version={version}\n{}",
            if healthy { "" } else { "broken\n" }
        );
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Stores a signed `.deb` of `version` in `l`.
    fn add(&self, l: &Launcher, version: &str, healthy: bool) {
        let path = self.deb(&self.root.join("build"), version, healthy);
        let meta = PackageMeta {
            product: "vesper".into(),
            version: version.into(),
            platform: this_platform(),
        };
        let sig = self
            .signer
            .sign(&Hasher::file(&path).unwrap(), &meta.comment());
        l.add_package(&path, &format!("vesper_{version}_amd64.deb"), &sig)
            .unwrap();
    }

    fn installed(&self) -> String {
        std::fs::read_to_string(self.root.join("dpkg/installed")).unwrap()
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.root.join("dpkg/calls")).unwrap_or_default()
    }
}

fn channel_package(version: &str) -> protocol::UpdatePackage {
    serde_json::from_value(serde_json::json!({
        "id": format!("p-{version}"), "product": "vesper", "version": version,
        "platform": this_platform(), "fileName": format!("vesper_{version}_amd64.deb"),
        "size": 1, "sha256": "", "keyId": "", "trustedComment": "", "uploadedAt": "2026-11-25T00:00:00Z",
    }))
    .unwrap()
}

/// A server installed with apt: the Launcher adopts dpkg's version, offers and runs only
/// newer ones (never the same or an older one), and installs without dpkg's questions; Launch
/// stays possible from its page (a system-run product).
#[tokio::test(flavor = "multi_thread")]
async fn an_apt_installed_server_is_adopted_and_only_updated_to_newer_versions() {
    let fx = deb_fixture("0.3.0-1");
    let l = Launcher::open(fx.cfg.clone()).unwrap();
    let r = l.report();
    assert_eq!(r.products[0].version.as_deref(), Some("0.3.0-1"));
    let pkgs: Vec<_> = ["0.2.9-1", "0.3.0-1"]
        .iter()
        .map(|v| channel_package(v))
        .collect();
    assert!(l.newer(&pkgs).is_empty(), "the same or older: not offered");
    let mut with_newer = pkgs.clone();
    with_newer.push(channel_package("0.3.1-1"));
    assert_eq!(l.newer(&with_newer)["vesper"], "0.3.1-1");

    for (v, why) in [("0.3.0-1", "already installed"), ("0.2.9-1", "is older")] {
        fx.add(&l, v, true);
        let e = l.update("vesper", v).await.unwrap_err();
        assert!(e.to_string().contains(why), "{v}: {e}");
    }
    assert_eq!(fx.calls(), "", "apt never ran");

    // The page: Launch works for it (`deb`); an update is offered until it is installed.
    l.reported(None, Some(l.newer(&with_newer)));
    let ov = serde_json::to_value(l.overview()).unwrap();
    assert_eq!(ov["kinds"]["vesper"], "deb");
    assert_eq!(ov["available"]["vesper"], "0.3.1-1");
    fx.add(&l, "0.3.1-1", true);
    let o = l.update("vesper", "0.3.1-1").await.unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{}", o.message);
    assert_eq!(fx.installed(), "0.3.1-1");
    let i = l.installed("vesper");
    assert_eq!(
        (i.version.as_deref(), i.previous.as_deref()),
        (Some("0.3.1-1"), Some("0.3.0-1"))
    );
    let calls = fx.calls();
    assert!(
        calls.starts_with(
            "noninteractive install -y -q --allow-downgrades --no-remove \
             -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold /"
        ),
        "with apt, an edited configuration file kept, never asked about, nothing removed: {calls}"
    );
    let ov = serde_json::to_value(l.overview()).unwrap();
    assert!(
        ov["available"].get("vesper").is_none(),
        "the version just installed isn't offered again: {ov}"
    );

    // Upgraded with apt by hand: adopted at the next health check.
    std::fs::write(fx.root.join("dpkg/installed"), "0.3.2-1").unwrap();
    l.check().await;
    let i = l.installed("vesper");
    assert_eq!(
        (i.version.as_deref(), i.previous.as_deref()),
        (Some("0.3.2-1"), Some("0.3.1-1"))
    );
    assert_eq!(l.report().products[0].state, ProductState::Running);

    // A package signed `0.3.3` that dpkg calls `0.3.3-1` (as the plan's signing example): the
    // same build, not an outside install, so the previous version is kept.
    let path = fx.root.join("build/vesper_0.3.3_amd64.deb");
    std::fs::write(&path, "version=0.3.3-1\n").unwrap();
    let meta = PackageMeta {
        product: "vesper".into(),
        version: "0.3.3".into(),
        platform: this_platform(),
    };
    let sig = fx
        .signer
        .sign(&Hasher::file(&path).unwrap(), &meta.comment());
    l.add_package(&path, "vesper_0.3.3_amd64.deb", &sig)
        .unwrap();
    let o = l.update("vesper", "0.3.3").await.unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{}", o.message);
    assert_eq!(fx.installed(), "0.3.3-1");
    l.check().await;
    let i = l.installed("vesper");
    assert_eq!(
        (i.version.as_deref(), i.previous.as_deref()),
        (Some("0.3.3"), Some("0.3.2-1"))
    );
}

/// H04-2 (bug run 6): a server package that needs a newer Launcher package (its `Depends`)
/// is installed together with the newest stored Launcher package meeting the bound, in one
/// dpkg run; without one, the update is refused before anything is installed, saying what it
/// needs.
#[tokio::test(flavor = "multi_thread")]
async fn a_package_needing_a_newer_launcher_is_installed_with_it() {
    let fx = deb_fixture("0.3.0-1");
    let (bin, db) = (fx.root.join("bin"), fx.root.join("dpkg"));
    let db_s = db.display();
    std::fs::write(db.join("installed-vesper-launcher"), "0.1.0-2").unwrap();
    // Per package: `name=` in the fake .deb (default vesper), `installed[-name]` in the db.
    script(
        &bin.join("dpkg-query"),
        &format!(
            "#!/bin/sh\nfor a; do n=$a; done\nf='{db_s}/installed'; [ \"$n\" = vesper ] || f='{db_s}/installed-'$n\n[ -s \"$f\" ] || exit 1\nprintf 'installed\\t%s' \"$(cat \"$f\")\"\n"
        ),
    );
    script(
        &bin.join("apt-get"),
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{db_s}/calls'\nfor a; do case $a in *.deb) n=$(sed -n 's/^name=//p' \"$a\"); v=$(sed -n 's/^version=//p' \"$a\" | tr -d '\\n'); if [ -z \"$n\" ]; then printf %s \"$v\" > '{db_s}/installed'; else printf %s \"$v\" > '{db_s}/installed-'$n; fi;; esac; done\n"
        ),
    );
    script(
        &bin.join("dpkg-deb"),
        "#!/bin/sh\nsed -n 's/^depends=//p' \"$2\"\n",
    );
    let mut cfg = fx.cfg.clone();
    cfg.dpkg.deb = bin.join("dpkg-deb");
    let mut me = cfg.products[0].clone();
    me.product = "vesper-launcher".into();
    me.name = "Vesper Launcher".into();
    me.is_self = true;
    me.health_url = "http://127.0.0.1:9/launcher/v1/version".into();
    cfg.products.push(me);
    let l = Launcher::open(cfg).unwrap();
    let add = |product: &str, version: &str, text: &str| {
        let path = fx
            .root
            .join("build")
            .join(format!("{product}_{version}_amd64.deb"));
        std::fs::write(&path, text).unwrap();
        let meta = PackageMeta {
            product: product.into(),
            version: version.into(),
            platform: this_platform(),
        };
        let sig = fx
            .signer
            .sign(&Hasher::file(&path).unwrap(), &meta.comment());
        l.add_package(&path, &format!("{product}_{version}_amd64.deb"), &sig)
            .unwrap();
    };
    add(
        "vesper",
        "0.4.0-1",
        "version=0.4.0-1\ndepends=libc6 (>= 2.43), vesper-launcher (>= 0.2.0), systemd\n",
    );
    add(
        "vesper-launcher",
        "0.1.5-1",
        "version=0.1.5-1\nname=vesper-launcher\n",
    );
    add(
        "vesper-launcher",
        "0.2.0-1",
        "version=0.2.0-1\nname=vesper-launcher\n",
    );
    add(
        "vesper",
        "0.5.0-1",
        "version=0.5.0-1\ndepends=vesper-launcher (>= 0.9.0)\n",
    );

    // Nothing stored meets the bound: refused, nothing installed.
    let e = l.update("vesper", "0.5.0-1").await.unwrap_err();
    assert!(e.to_string().contains("vesper-launcher (>= 0.9.0)"), "{e}");
    assert_eq!(fx.calls(), "");

    let o = l.update("vesper", "0.4.0-1").await.unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{}", o.message);
    let calls = fx.calls();
    assert_eq!(calls.lines().count(), 1, "one apt run: {calls}");
    assert!(
        calls.contains("vesper-launcher_0.2.0-1_amd64.deb")
            && calls.trim_end().ends_with("vesper_0.4.0-1_amd64.deb"),
        "{calls}"
    );
    assert_eq!(fx.installed(), "0.4.0-1");
    assert_eq!(
        std::fs::read_to_string(db.join("installed-vesper-launcher")).unwrap(),
        "0.2.0-1"
    );
}

/// A Windows installer product (`nsis`) installed by hand over the Launcher's version: the
/// version it reports is adopted (not only when the manifest has none), so an older channel
/// package isn't offered or installed as an update.
#[tokio::test(flavor = "multi_thread")]
async fn a_newer_installer_product_installed_by_hand_is_adopted() {
    let fx = deb_fixture("0.3.0-1");
    let mut cfg = fx.cfg.clone();
    cfg.products[0].install = vesper_launcher::config::InstallKind::Nsis {
        args: Vec::new(),
        task: None,
    };
    let l = Launcher::open(cfg).unwrap();
    l.check().await;
    assert_eq!(l.installed("vesper").version.as_deref(), Some("0.3.0"));
    // The same version with a packaging revision isn't a new install.
    std::fs::write(fx.root.join("dpkg/installed"), "0.3.0-2").unwrap();
    l.check().await;
    let i = l.installed("vesper");
    assert_eq!((i.version.as_deref(), i.previous), (Some("0.3.0"), None));
    // 0.4.0 installed by hand: adopted; 0.3.5 from the channel is older, not offered.
    std::fs::write(fx.root.join("dpkg/installed"), "0.4.0-1").unwrap();
    l.check().await;
    let i = l.installed("vesper");
    assert_eq!(
        (i.version.as_deref(), i.previous.as_deref()),
        (Some("0.4.0"), Some("0.3.0"))
    );
    assert!(l.newer(&[channel_package("0.3.5")]).is_empty());
}

/// The first update of an apt-installed server that isn't healthy goes back to the installed
/// version with apt's cached package of it; without one, the outcome says why it can't.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_first_update_goes_back_with_apts_package_or_says_why_not() {
    let fx = deb_fixture("0.3.0-1");
    let l = Launcher::open(fx.cfg.clone()).unwrap();
    fx.deb(&fx.root.join("apt"), "0.3.0-1", true);
    fx.add(&l, "0.4.0-1", false);
    let o = l.update("vesper", "0.4.0-1").await.unwrap();
    assert_eq!(o.result, UpdateResult::RolledBack, "{}", o.message);
    assert_eq!(fx.installed(), "0.3.0-1");
    assert_eq!(l.installed("vesper").version.as_deref(), Some("0.3.0-1"));
    assert_eq!(l.report().products[0].state, ProductState::Running);

    std::fs::remove_file(fx.root.join("apt/vesper_0.3.0-1_amd64.deb")).unwrap();
    let o = l.update("vesper", "0.4.0-1").await.unwrap();
    assert_eq!(o.result, UpdateResult::Failed, "{}", o.message);
    assert!(
        o.message.contains("installed outside the Launcher") && o.message.contains("apt's cache"),
        "{}",
        o.message
    );
}

/// apt can't install a package (a system package it needs isn't available): the update fails
/// with apt's reason, trimmed to its errors and the unmet dependency (not its progress lines),
/// and the installed version is put back.
#[tokio::test(flavor = "multi_thread")]
async fn an_apt_failure_says_why_and_the_installed_version_is_put_back() {
    let fx = deb_fixture("0.3.0-1");
    let db = fx.root.join("dpkg");
    let db_s = db.display();
    // A package with `unmet` in it: apt explains on stdout and stderr and installs nothing.
    let mut noise = String::new();
    for i in 0..40 {
        noise.push_str(&format!("echo 'Reading package lists... step {i}'\n"));
    }
    script(
        &fx.root.join("bin/apt-get"),
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{db_s}/calls'\nfor a; do pkg=$a; done\n\
             if grep -q unmet \"$pkg\"; then\n{noise}\
             echo 'The following packages have unmet dependencies:'\n\
             echo ' vesper : Depends: libexample-synthetic9 but it is not going to be installed'\n\
             echo 'E: Unable to correct problems, you have held broken packages.' >&2\n\
             exit 100\nfi\n\
             sed -n 's/^version=//p' \"$pkg\" | tr -d '\\n' > '{db_s}/installed'\nrm -f '{db_s}/down'\n"
        ),
    );
    let l = Launcher::open(fx.cfg.clone()).unwrap();
    fx.deb(&fx.root.join("apt"), "0.3.0-1", true);
    let path = fx.root.join("build/vesper_0.4.0-1_amd64.deb");
    std::fs::write(&path, "version=0.4.0-1\nunmet\n").unwrap();
    let meta = PackageMeta {
        product: "vesper".into(),
        version: "0.4.0-1".into(),
        platform: this_platform(),
    };
    let sig = fx
        .signer
        .sign(&Hasher::file(&path).unwrap(), &meta.comment());
    l.add_package(&path, "vesper_0.4.0-1_amd64.deb", &sig)
        .unwrap();
    let o = l.update("vesper", "0.4.0-1").await.unwrap();
    assert_eq!(o.result, UpdateResult::RolledBack, "{}", o.message);
    assert!(
        o.message.contains("apt couldn't install it")
            && o.message.contains(
                "vesper : Depends: libexample-synthetic9 but it is not going to be installed"
            )
            && o.message.contains("E: Unable to correct problems"),
        "{}",
        o.message
    );
    assert!(
        !o.message.contains("Reading package lists"),
        "{}",
        o.message
    );
    assert!(o.message.len() < 1200, "trimmed: {}", o.message.len());
    assert_eq!(fx.installed(), "0.3.0-1");
    let calls = fx.calls();
    assert_eq!(
        calls.lines().count(),
        2,
        "the update, then the way back: {calls}"
    );
    assert!(
        calls
            .lines()
            .last()
            .unwrap()
            .ends_with("apt/vesper_0.3.0-1_amd64.deb"),
        "{calls}"
    );
}

/// A package apt can't find in its out-of-date package lists: the lists are refreshed once
/// (`apt-get update`) and the install is tried again.
#[tokio::test(flavor = "multi_thread")]
async fn apt_refreshes_its_package_lists_once_when_a_dependency_cant_be_fetched() {
    let fx = deb_fixture("0.3.0-1");
    let db = fx.root.join("dpkg");
    let db_s = db.display();
    script(
        &fx.root.join("bin/apt-get"),
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{db_s}/calls'\n\
             if [ \"$1\" = update ]; then touch '{db_s}/lists'; exit 0; fi\n\
             for a; do pkg=$a; done\n\
             if [ ! -e '{db_s}/lists' ]; then\n\
             echo 'E: Unable to fetch some archives, maybe run apt-get update or try with --fix-missing?' >&2\n\
             exit 100\nfi\n\
             sed -n 's/^version=//p' \"$pkg\" | tr -d '\\n' > '{db_s}/installed'\nrm -f '{db_s}/down'\n"
        ),
    );
    let l = Launcher::open(fx.cfg.clone()).unwrap();
    fx.add(&l, "0.4.0-1", true);
    let o = l.update("vesper", "0.4.0-1").await.unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{}", o.message);
    assert_eq!(fx.installed(), "0.4.0-1");
    let calls: Vec<String> = fx.calls().lines().map(str::to_string).collect();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(calls[0].starts_with("install ") && calls[2].starts_with("install "));
    assert_eq!(calls[1], "update -q");
}

/// Holds dpkg's lock (a write record lock on `lock-frontend`, as dpkg takes it) until the file
/// is dropped. An open-file-description lock: a process's classic record lock would be
/// released when the Launcher, in this same process, closes the file after asking.
fn hold_dpkg_lock(admin_dir: &Path) -> std::fs::File {
    use std::os::fd::AsRawFd;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(admin_dir.join("lock-frontend"))
        .unwrap();
    // SAFETY: a plain C struct; all zeroes is valid.
    let mut l: libc::flock = unsafe { std::mem::zeroed() };
    l.l_type = libc::F_WRLCK as libc::c_short;
    l.l_whence = libc::SEEK_SET as libc::c_short;
    // SAFETY: `f` is open for writing and `l` a valid `flock`.
    assert_eq!(
        unsafe { libc::fcntl(f.as_raw_fd(), libc::F_OFD_SETLK, &l) },
        0
    );
    f
}

/// The Launcher restarted while the dpkg of its update kept running (its unit's
/// `KillMode=process`): at its start it waits for that dpkg, then only checks the version it
/// installed (no second install, no rollback over it).
#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_deb_update_waits_for_its_dpkg_and_keeps_what_it_installed() {
    let fx = deb_fixture("0.3.0-1");
    let l = Launcher::open(fx.cfg.clone()).unwrap();
    fx.add(&l, "0.3.1-1", true);
    drop(l);
    let path = fx.root.join("state/manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    m["products"]["vesper"]["inProgress"] = serde_json::json!({"from": "0.3.0-1", "to": "0.3.1-1"});
    std::fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();

    // The orphaned dpkg: holds the lock for a while, then has installed 0.3.1-1.
    assert!(!vesper_launcher::install::dpkg_locked(
        &fx.root.join("dpkg")
    ));
    let lock = hold_dpkg_lock(&fx.root.join("dpkg"));
    assert!(vesper_launcher::install::dpkg_locked(&fx.root.join("dpkg")));
    let installed = fx.root.join("dpkg/installed");
    let orphan = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        std::fs::write(installed, "0.3.1-1").unwrap();
        drop(lock);
    });
    let started = std::time::Instant::now();
    let l = Launcher::open(fx.cfg.clone()).unwrap();
    assert_eq!(
        l.installed("vesper").version.as_deref(),
        Some("0.3.0-1"),
        "not adopted while the update is under way"
    );
    l.start_products().await;
    orphan.join().unwrap();
    assert!(started.elapsed() >= std::time::Duration::from_millis(1400));
    assert_eq!(fx.calls(), "", "no second install");
    let i = l.installed("vesper");
    assert_eq!(
        (i.version.as_deref(), i.previous.as_deref()),
        (Some("0.3.1-1"), Some("0.3.0-1"))
    );
    assert!(i.in_progress.is_none());
    assert_eq!(i.last_update.unwrap().result, UpdateResult::Installed);
}

/// An update asked for on the Launcher's page carries on when the page goes away (a reload, a
/// closed window): its outcome is recorded, the rollback runs.
#[tokio::test(flavor = "multi_thread")]
async fn an_update_from_the_page_finishes_when_the_page_goes_away() {
    let fx = fixture();
    fx.add("1.0.0", true);
    fx.launcher.update("fake", "1.0.0").await.unwrap();
    fx.add("2.0.0", false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vesper_launcher::api::router(fx.launcher.clone())
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let body = r#"{"type":"update","product":"fake","version":"2.0.0"}"#;
    let raw = format!(
        "POST /launcher/v1/actions HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        addr.port(),
        body.len()
    );
    {
        use tokio::io::AsyncWriteExt;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(raw.as_bytes()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        // The page is reloaded: the connection closes.
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let i = fx.launcher.installed("fake");
        if i.in_progress.is_none() && i.last_update.as_ref().is_some_and(|o| o.to == "2.0.0") {
            assert_eq!(
                i.last_update.unwrap().result,
                UpdateResult::RolledBack,
                "the update ran to its end"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the update was dropped with the request: {i:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(reported_version(fx.port).as_deref(), Some("1.0.0"));
    fx.launcher.shutdown().await;
}

/// Writes an executable shell script.
fn script(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake site's server for a Launcher: its name, pairing with code `123456` (the machine's
/// Launcher key `tok-1`), reports, the package list, and an IDN join code (`482913`) for the
/// machine whose key asks. The paths it was asked, in order.
fn fake_site() -> (std::net::SocketAddr, Arc<std::sync::Mutex<Vec<String>>>) {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let asked2 = asked.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // Headers, then the body by Content-Length.
            let (head, mut body) = loop {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break (String::from_utf8_lossy(&buf).into_owned(), Vec::new());
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break (
                        String::from_utf8_lossy(&buf[..i]).into_owned(),
                        buf[i + 4..].to_vec(),
                    );
                }
            };
            let len: usize = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse().ok())?
                })
                .unwrap_or(0);
            while body.len() < len {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
            let mut words = head.split_whitespace();
            let (method, path) = (
                words.next().unwrap_or("").to_string(),
                words.next().unwrap_or("").to_string(),
            );
            let authed = head.contains("Authorization: Bearer tok-1");
            asked2.lock().unwrap().push(format!("{method} {path}"));
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            let (status, answer) = match (method.as_str(), path.as_str()) {
                ("GET", "/api/v1/server") => (
                    "200 OK",
                    r#"{"name":"Main Server","version":"0.1.0"}"#.to_string(),
                ),
                ("POST", "/api/v1/pair") if body["code"] == "123456" => (
                    "200 OK",
                    r#"{"token":"tok-1","clientId":"key-1"}"#.to_string(),
                ),
                ("POST", "/api/v1/pair") => ("401 Unauthorized", "{}".to_string()),
                ("POST", p) if p.ends_with("/idn-join") && authed => (
                    "200 OK",
                    r#"{"code":"482913","expiresInSeconds":600}"#.to_string(),
                ),
                ("PUT", _) if authed => ("200 OK", r#"{"actions":[]}"#.to_string()),
                ("GET", "/api/v1/updates/packages") if authed => ("200 OK", "[]".to_string()),
                _ => ("403 Forbidden", "{}".to_string()),
            };
            let _ = write!(
                s,
                "HTTP/1.0 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{answer}",
                answer.len()
            );
        }
    });
    (addr, asked)
}

/// Waits until the run of the roles towards `target` is done.
async fn roles_done(l: &Launcher, target: &[&str]) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let v = serde_json::to_value(l.setup_view()).unwrap();
        let run = &v["roleStatus"]["run"];
        if run["target"] == serde_json::json!(target) && run["state"] == "done" {
            return v;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the roles weren't made true: {v}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Round 3: a display box set up from the command line (`vesper-launcher join CODE --role
/// display`), with fakes for the site's server, the IDN Node and systemd. The Launcher joins
/// with the server's code, gets an IDN join code for its machine from the site's server and
/// joins its IDN Node with it (nobody types a second code), switches the headless display
/// service on and keeps the Vesper server off. Switched to a presentation server, it undoes
/// that and starts the server.
#[tokio::test(flavor = "multi_thread")]
async fn a_display_box_joins_from_the_command_line_and_its_roles_are_acted_on() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let calls = root.join("calls");
    script(
        &bin.join("idn-node"),
        &format!("#!/bin/sh\necho \"idn-node $*\" >> '{}'\n", calls.display()),
    );
    script(
        &bin.join("systemctl"),
        &format!(
            "#!/bin/sh\n[ \"$1\" = is-enabled ] && exit 1\necho \"systemctl $*\" >> '{}'\n",
            calls.display()
        ),
    );
    let (site, asked) = fake_site();
    let cfg: Config = serde_json::from_value(serde_json::json!({
        "stateDir": root.join("state"),
        "listen": "127.0.0.1:0",
        "keyDir": null,
        "machineName": "Lobby Box",
        "checkIntervalSecs": 1,
        // Not joined yet: it would check GitHub itself (never from a test).
        "github": {"enabled": false},
        "products": [{
            "product": "vesper",
            "name": "Vesper",
            "install": {"kind": "deb", "units": ["vesper.service", "vesper-display.service"]},
            "healthUrl": "http://127.0.0.1:9/api/v1/server",
        }],
        "dpkg": {"query": root.join("no-dpkg-query")},
        "system": {"systemctl": bin.join("systemctl"), "idnDir": bin, "idnNodeUnit": root.join("idn-node.service")},
    }))
    .unwrap();
    let l = Launcher::open(cfg).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = listener.local_addr().unwrap();
    let app = vesper_launcher::api::router(l.clone())
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let l2 = l.clone();
    let reporter = tokio::spawn(async move { vesper_launcher::channel::run(l2).await });

    // A wrong code is refused with the server's reason.
    let a = vesper_launcher::cli::JoinArgs {
        api: api.to_string(),
        server: Some(site.to_string()),
        code: Some("000000".into()),
        role: Some("display".into()),
        wait: true,
        wait_for: std::time::Duration::from_secs(20),
    };
    let a2 = a.clone();
    let e = tokio::task::spawn_blocking(move || vesper_launcher::cli::join(&a2, &mut Vec::new()))
        .await
        .unwrap()
        .unwrap_err();
    assert!(e.to_string().contains("code isn't right"), "{e}");
    // The right code: joined, and the wizard counts as done.
    let a = vesper_launcher::cli::JoinArgs {
        code: Some("123 456".into()),
        ..a
    };
    let text = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        vesper_launcher::cli::join(&a, &mut out).map(|()| String::from_utf8(out).unwrap())
    })
    .await
    .unwrap()
    .unwrap();
    assert!(text.contains("Joined the site at Main Server"), "{text}");
    assert!(!l.setup_view().first_run);
    assert_eq!(l.roles(), ["display"]);

    let v = roles_done(&l, &["display"]).await;
    assert_eq!(v["roleStatus"]["applied"], serde_json::json!(["display"]));
    let got = std::fs::read_to_string(&calls).unwrap();
    assert_eq!(
        got.lines().collect::<Vec<_>>(),
        [
            "systemctl disable --now vesper.service vesper-display.service",
            "idn-node install",
            "idn-node set server=127.0.0.1",
            "idn-node join 482913",
            "idn-node set apps-display=on",
            "idn-node set apps-display-boot=on",
        ],
        "{got}"
    );
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .any(|p| *p == format!("POST /api/v1/updates/launchers/{}/idn-join", l.machine_id())),
        "the join code came from the site's server for this machine"
    );
    // Applied once: a later check changes nothing.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(std::fs::read_to_string(&calls).unwrap(), got);

    // Vesper updates this IDN Node, not the Site Manager: the marker beside it, put back when
    // it goes (IDN Beta14, lane I).
    let marker = bin.join("idn-updates-managed-by");
    assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "Vesper");
    std::fs::remove_file(&marker).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.is_file() {
        assert!(
            std::time::Instant::now() < deadline,
            "the marker wasn't put back"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        l.idn_node_view()
            .unwrap()
            .0
            .contains("updates managed by Vesper")
    );
    // Another IDN install replaced the Node's service: the bundled one is put back, with a
    // warning.
    let unit = root.join("idn-node.service");
    std::fs::write(&unit, "[Service]\nExecStart=/opt/other-idn/idn-node run\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !std::fs::read_to_string(&calls)
        .unwrap()
        .ends_with("idn-node install\n")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the Node's service wasn't put back"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // (The fake `idn-node install` leaves the unit as it was: still another program at the
    // next check, flagged.)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let (view, bad) = l.idn_node_view().unwrap();
        if bad && view.contains("/opt/other-idn/idn-node instead of") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{view}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // Ours: left alone, and no longer flagged (H04-4 / R3-4), the last warning kept as what
    // happened.
    std::fs::write(
        &unit,
        format!(
            "[Service]\nExecStart={} run\n",
            bin.join("idn-node").display()
        ),
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let (view, bad) = l.idn_node_view().unwrap();
        if !bad {
            assert!(view.contains("updates managed by Vesper"), "{view}");
            assert!(view.contains("instead of"), "{view}");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{view}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Switched to a presentation server: the display service off, the Node's service removed
    // (the Launcher installed it), the Vesper server on.
    std::fs::write(&calls, "").unwrap();
    let body = r#"{"roles":["presentation"]}"#;
    let (s, body) = send(
        api,
        &format!(
            "PUT /launcher/v1/setup HTTP/1.0\r\nHost: {api}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    )
    .await;
    assert_eq!(s, 200, "{body}");
    roles_done(&l, &["presentation"]).await;
    assert_eq!(
        std::fs::read_to_string(&calls)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "idn-node set apps-display=off",
            "idn-node set apps-display-boot=off",
            "idn-node uninstall",
            "systemctl enable --now vesper.service",
        ]
    );
    // What was made true survives a restart of the Launcher.
    reporter.abort();
    let again = Launcher::open(l.config().clone()).unwrap();
    assert_eq!(again.roles_view().applied, ["presentation"]);
}

/// H04-3 / R3-4 (bug run 7): the diagnostics "IDN Node" row says a marker that can't be
/// written from the very first check (flagged), and disappears when the bundled Node is gone.
#[tokio::test(flavor = "multi_thread")]
async fn the_idn_node_row_follows_what_the_last_check_found() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let idn = root.join("idn");
    std::fs::create_dir_all(&idn).unwrap();
    let cfg: Config = serde_json::from_value(serde_json::json!({
        "stateDir": root.join("state"),
        "listen": "127.0.0.1:0",
        "keyDir": null,
        "products": [],
        "dpkg": {"query": root.join("no-dpkg-query")},
        "system": {"systemctl": root.join("no-systemctl"), "idnDir": idn, "idnNodeUnit": root.join("idn-node.service")},
    }))
    .unwrap();
    let l = Launcher::open(cfg).unwrap();
    l.idn_node_guard().await;
    assert_eq!(l.idn_node_view(), None, "no bundled Node: no row");
    let node = idn.join("idn-node");
    std::fs::write(&node, "").unwrap();
    // A marker that can't be read (a folder in its place) from the first check on.
    let marker = idn.join("idn-updates-managed-by");
    std::fs::create_dir(&marker).unwrap();
    l.idn_node_guard().await;
    let (view, bad) = l.idn_node_view().expect("a row");
    assert!(bad && view.contains("can't be written"), "{view}");
    // Put right: kept as the last warning, no longer flagged.
    std::fs::remove_dir(&marker).unwrap();
    l.idn_node_guard().await;
    let (view, bad) = l.idn_node_view().unwrap();
    assert!(!bad && view.contains("updates managed by Vesper"), "{view}");
    assert!(view.contains("was missing: put back"), "{view}");
    // The package removed: no row, no stale warning.
    std::fs::remove_file(&node).unwrap();
    l.idn_node_guard().await;
    assert_eq!(l.idn_node_view(), None);
}

/// H04-2 / R3-2 (bug run 7): a package of programs (the bundled IDN Node, `vesper-idn-node`:
/// no health URL) is a product like the others for updates from the channel: installed with
/// dpkg and healthy once dpkg has the version (else put back), up while installed, and never
/// launched or stopped.
#[tokio::test(flavor = "multi_thread")]
async fn a_package_of_programs_is_updated_like_a_product() {
    let fx = deb_fixture("0.3.6~beta17-1");
    let mut cfg = fx.cfg.clone();
    cfg.products[0].product = "vesper-idn-node".into();
    cfg.products[0].name = "IDN Node (bundled)".into();
    cfg.products[0].health_url = String::new();
    let l = Launcher::open(cfg).unwrap();
    l.check().await;
    let p = &l.report().products[0];
    assert_eq!(
        (p.version.as_deref(), p.state),
        (Some("0.3.6~beta17-1"), ProductState::Running)
    );
    assert_eq!(l.overview().kinds["vesper-idn-node"], "package");
    assert!(l.start_product("vesper-idn-node").await.is_err());
    assert!(l.stop_product("vesper-idn-node").await.is_err());
    let add = |version: &str, text: &str| {
        let path = fx
            .root
            .join("build")
            .join(format!("vesper-idn-node_{version}_amd64.deb"));
        std::fs::write(&path, text).unwrap();
        let meta = PackageMeta {
            product: "vesper-idn-node".into(),
            version: version.into(),
            platform: this_platform(),
        };
        let sig = fx
            .signer
            .sign(&Hasher::file(&path).unwrap(), &meta.comment());
        l.add_package(&path, &format!("vesper-idn-node_{version}_amd64.deb"), &sig)
            .unwrap();
    };
    add("0.3.6~beta18-1", "version=0.3.6~beta18-1\n");
    let o = l.update("vesper-idn-node", "0.3.6~beta18-1").await.unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{}", o.message);
    assert_eq!(fx.installed(), "0.3.6~beta18-1");
    l.check().await;
    assert_eq!(l.report().products[0].state, ProductState::Running);
    // A package dpkg doesn't end up having (its version not installed): put back.
    add("0.3.6~beta19-1", "version=0.3.6~beta19-0\n");
    let o = l.update("vesper-idn-node", "0.3.6~beta19-1").await.unwrap();
    assert_ne!(o.result, UpdateResult::Installed, "{}", o.message);
    assert!(
        o.message.contains("dpkg has 0.3.6~beta19-0"),
        "{}",
        o.message
    );
    // A healthUrl is needed by anything but a deb package of programs.
    let f = fx.root.join("bad.json");
    std::fs::write(
        &f,
        r#"{"products": [{"product": "x", "name": "X", "self": true, "install": {"kind": "deb"}, "healthUrl": ""}]}"#,
    )
    .unwrap();
    assert!(Config::load(&f).is_err());
    let f = fx.root.join("ok.json");
    std::fs::write(&f, r#"{"products": []}"#).unwrap();
    std::fs::create_dir_all(fx.root.join("launcher.d")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging/linux/launcher.d/vesper-idn-node.json"),
        fx.root.join("launcher.d/vesper-idn-node.json"),
    )
    .unwrap();
    let c = Config::load(&f).unwrap();
    assert_eq!(c.products[0].product, "vesper-idn-node");
    assert!(c.products[0].health_url.is_empty());
}

/// A display box whose machine hasn't joined the site yet: the IDN Node's join waits (the
/// step fails with what to do) and is retried as soon as the machine joins.
#[tokio::test(flavor = "multi_thread")]
async fn a_display_box_role_waits_for_the_site_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let cfg: Config = serde_json::from_value(serde_json::json!({
        "stateDir": root.join("state"),
        "listen": "127.0.0.1:0",
        "keyDir": null,
        "products": [],
        "dpkg": {"query": root.join("no-dpkg-query")},
        "system": {"systemctl": root.join("no-systemctl"), "idnDir": root.join("no-idn")},
    }))
    .unwrap();
    let l = Launcher::open(cfg).unwrap();
    l.set_setup(Some(vec!["display".into()]), None).unwrap();
    l.act_on_roles().await;
    let v = serde_json::to_value(l.setup_view()).unwrap();
    let run = &v["roleStatus"]["run"];
    assert_eq!(run["state"], "failed", "{v}");
    let failed = run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["state"] == "failed")
        .unwrap();
    assert_eq!(failed["id"], "nodeInstall");
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .contains("install the vesper-idn-node package"),
        "{failed}"
    );
    // No Vesper server here: its step is skipped, not failed.
    assert_eq!(run["steps"][0]["id"], "serverOff");
    assert_eq!(run["steps"][0]["state"], "skipped");
    assert_eq!(v["roleStatus"]["applied"], serde_json::json!([]));
}

/// A run that stops partway (a display box picked as a presentation server, with no Vesper
/// server on it: the display service is off and the Node's service removed before the server's
/// step fails) doesn't leave the box dark: picked as a display box again, the Launcher sets it
/// up again; with no role, the Node's service it installed goes.
#[tokio::test(flavor = "multi_thread")]
async fn a_role_run_that_stops_partway_is_undone_when_the_role_changes_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let calls = root.join("calls");
    script(
        &bin.join("idn-node"),
        &format!("#!/bin/sh\necho \"idn-node $*\" >> '{}'\n", calls.display()),
    );
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join("roles.json"),
        r#"{"applied":["display"],"nodeInstalled":true}"#,
    )
    .unwrap();
    let cfg: Config = serde_json::from_value(serde_json::json!({
        "stateDir": state,
        "listen": "127.0.0.1:0",
        "keyDir": null,
        "products": [],
        "dpkg": {"query": root.join("no-dpkg-query")},
        "system": {"systemctl": root.join("no-systemctl"), "idnDir": bin, "idnNodeUnit": root.join("idn-node.service")},
    }))
    .unwrap();
    let l = Launcher::open(cfg).unwrap();
    let calls_now = || {
        let c = std::fs::read_to_string(&calls).unwrap_or_default();
        std::fs::write(&calls, "").unwrap();
        c.lines().map(str::to_string).collect::<Vec<_>>()
    };
    let step = |v: &serde_json::Value, id: &str| {
        v["roleStatus"]["run"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .cloned()
            .unwrap_or_default()
    };

    // Picked as a presentation server by mistake: undone, then the server's step fails.
    l.set_setup(Some(vec!["presentation".into()]), None)
        .unwrap();
    l.act_on_roles().await;
    let v = serde_json::to_value(l.setup_view()).unwrap();
    assert_eq!(v["roleStatus"]["run"]["state"], "failed", "{v}");
    assert_eq!(step(&v, "serverOn")["state"], "failed", "{v}");
    assert_eq!(v["roleStatus"]["applied"], serde_json::json!([]), "{v}");
    assert_eq!(
        calls_now(),
        [
            "idn-node set apps-display=off",
            "idn-node set apps-display-boot=off",
            "idn-node uninstall",
        ]
    );

    // A display box again: set up again (its join waits for the site here).
    l.set_setup(Some(vec!["display".into()]), None).unwrap();
    l.act_on_roles().await;
    let v = serde_json::to_value(l.setup_view()).unwrap();
    assert_eq!(
        v["roleStatus"]["run"]["target"],
        serde_json::json!(["display"]),
        "{v}"
    );
    assert_eq!(step(&v, "nodeInstall")["state"], "done", "{v}");
    assert_eq!(step(&v, "nodeJoin")["state"], "failed", "{v}");
    assert_eq!(calls_now(), ["idn-node install"]);

    // No role at all: the Node's service the Launcher installed goes, and the stopped run is
    // over (nothing is retried).
    l.set_setup(Some(vec![]), None).unwrap();
    l.act_on_roles().await;
    let v = serde_json::to_value(l.setup_view()).unwrap();
    assert_eq!(v["roleStatus"]["run"]["state"], "done", "{v}");
    assert_eq!(
        calls_now(),
        [
            "idn-node set apps-display=off",
            "idn-node set apps-display-boot=off",
            "idn-node uninstall",
        ]
    );
    l.act_on_roles().await;
    assert!(calls_now().is_empty());
}

/// The Launcher's own update (round 3), with a fake dpkg and systemd: the helper installs the
/// new package, restarts the Launcher and keeps the version when its health check answers;
/// a version that doesn't come up is replaced by the kept one. The Launcher that starts next
/// records what the helper found.
#[tokio::test(flavor = "multi_thread")]
async fn the_launchers_self_update_helper_keeps_a_healthy_version_and_goes_back_otherwise() {
    let fx = deb_fixture("1.0.0-1");
    let calls = fx.root.join("dpkg/systemctl-calls");
    script(
        &fx.root.join("bin/systemctl"),
        &format!("#!/bin/sh\necho \"$*\" >> '{}'\n", calls.display()),
    );
    let health = fx.cfg.products[0].health_url.clone();
    let config = fx.root.join("launcher.json");
    std::fs::write(
        &config,
        serde_json::to_vec_pretty(&serde_json::json!({
            "stateDir": fx.root.join("state"),
            "listen": "127.0.0.1:0",
            "keyDir": null,
            "healthTimeoutSecs": 2,
            "products": [{
                "product": "vesper-launcher",
                "name": "Vesper Launcher",
                "self": true,
                "install": {"kind": "deb", "units": ["vesper-launcher.service"]},
                "healthUrl": health,
            }],
            "dpkg": {
                "apt": fx.root.join("bin/apt-get"),
                "query": fx.root.join("bin/dpkg-query"),
                "adminDir": fx.root.join("dpkg"),
                "lockTimeoutSecs": 20,
                "aptArchives": fx.root.join("apt"),
            },
            "system": {"systemctl": fx.root.join("bin/systemctl")},
        }))
        .unwrap(),
    )
    .unwrap();
    let build = fx.root.join("build");
    let req = |from: &str, to: &str, healthy: bool| vesper_launcher::selfupdate::Request {
        config: config.clone(),
        product: "vesper-launcher".into(),
        from: Some(from.into()),
        to: to.into(),
        package: fx.deb(&build, to, healthy),
        back: Some(fx.deb(&build, from, true)),
    };
    let state = fx.root.join("state");
    std::fs::create_dir_all(vesper_launcher::selfupdate::dir(&state)).unwrap();
    let run = |r: vesper_launcher::selfupdate::Request| {
        let path = vesper_launcher::selfupdate::dir(&state).join("request.json");
        std::fs::write(&path, serde_json::to_vec(&r).unwrap()).unwrap();
        move || vesper_launcher::selfupdate::run_helper(&path).unwrap()
    };

    // 2.0.0-1 comes up: kept.
    let o = tokio::task::spawn_blocking(run(req("1.0.0-1", "2.0.0-1", true)))
        .await
        .unwrap();
    assert_eq!((o.result, o.done), (UpdateResult::Installed, true), "{o:?}");
    assert_eq!(fx.installed(), "2.0.0-1");
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap(),
        "restart vesper-launcher.service\n"
    );

    // 3.0.0-1 doesn't: 2.0.0-1 is installed again and restarted.
    let o = tokio::task::spawn_blocking(run(req("2.0.0-1", "3.0.0-1", false)))
        .await
        .unwrap();
    assert_eq!(o.result, UpdateResult::RolledBack, "{o:?}");
    assert!(o.message.contains("2.0.0-1 is back"), "{}", o.message);
    assert_eq!(fx.installed(), "2.0.0-1");
    assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 3);
    assert_eq!(
        vesper_launcher::selfupdate::read_outcome(&state).unwrap(),
        o
    );

    // The Launcher that starts finds its update under way and records the helper's outcome.
    let manifest = serde_json::json!({"products": {"vesper-launcher": {
        "version": "2.0.0-1",
        "inProgress": {"from": "2.0.0-1", "to": "3.0.0-1"},
    }}});
    std::fs::write(state.join("manifest.json"), manifest.to_string()).unwrap();
    let l = Launcher::open(Config::load(&config).unwrap()).unwrap();
    l.start_products().await;
    let i = l.installed("vesper-launcher");
    assert!(i.in_progress.is_none());
    assert_eq!(i.version.as_deref(), Some("2.0.0-1"));
    let u = i.last_update.unwrap();
    assert_eq!(u.result, UpdateResult::RolledBack);
    assert!(u.message.contains("is back"), "{}", u.message);
    // The Launcher isn't started or stopped from its own window.
    assert!(l.stop_product("vesper-launcher").await.is_err());
    assert_eq!(l.overview().kinds["vesper-launcher"], "self");
}

/// Rolling the Launcher itself back goes through the self-update helper like an update (the
/// running Launcher never installs its own package): here, with no configuration file for the
/// helper, it fails saying so and dpkg never runs.
#[tokio::test(flavor = "multi_thread")]
async fn the_launchers_own_rollback_goes_through_the_self_update_helper() {
    let fx = deb_fixture("2.0.0-1");
    std::fs::write(
        fx.root.join("apt/vesper-launcher_1.0.0-1_amd64.deb"),
        "version=1.0.0-1\n",
    )
    .unwrap();
    let mut cfg = fx.cfg.clone();
    cfg.products[0].product = "vesper-launcher".into();
    cfg.products[0].is_self = true;
    std::fs::create_dir_all(&cfg.state_dir).unwrap();
    let manifest = serde_json::json!({"products": {"vesper-launcher": {
        "version": "2.0.0-1", "previous": "1.0.0-1",
    }}});
    std::fs::write(cfg.state_dir.join("manifest.json"), manifest.to_string()).unwrap();
    let l = Launcher::open(cfg).unwrap();
    let o = l.rollback("vesper-launcher").await.unwrap();
    assert_eq!(o.result, UpdateResult::Failed, "{o:?}");
    assert!(o.message.contains("configuration file"), "{}", o.message);
    assert_eq!(fx.calls(), "", "dpkg never ran inside the Launcher");
    let i = l.installed("vesper-launcher");
    assert!(i.in_progress.is_none());
    assert_eq!(i.version.as_deref(), Some("2.0.0-1"));
}

/// Products come from `launcher.d/*.json` too (the server's package adds itself there); one in
/// the main file wins over a file of the same ID. Without roles chosen or configured, a machine
/// with the server is reported as a presentation server; without it, it has no role. A file
/// that appears later is picked up by the running Launcher.
#[test]
fn products_come_from_the_configuration_and_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("launcher.json");
    let product = |id: &str, name: &str| {
        serde_json::json!({"product": id, "name": name, "install": {"kind": "dir"},
            "healthUrl": "http://127.0.0.1:9/"})
    };
    std::fs::write(
        &main,
        serde_json::json!({"stateDir": dir.path().join("state"), "keyDir": null,
            "products": [product("vesper-launcher", "Vesper Launcher")]})
        .to_string(),
    )
    .unwrap();
    let alone = Launcher::open(Config::load(&main).unwrap()).unwrap();
    assert!(alone.roles().is_empty());
    std::fs::create_dir_all(dir.path().join("launcher.d")).unwrap();
    std::fs::write(
        dir.path().join("launcher.d/vesper.json"),
        product("vesper", "Vesper").to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("launcher.d/again.json"),
        product("vesper-launcher", "Another").to_string(),
    )
    .unwrap();
    let c = Config::load(&main).unwrap();
    let names: Vec<&str> = c.products.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Vesper Launcher", "Vesper"]);
    assert_eq!(c.path.as_deref(), Some(main.as_path()));
    let l = Launcher::open(c).unwrap();
    assert_eq!(l.roles(), ["presentation"]);
    // A product file that appeared while the Launcher ran (a package installed since, or
    // configured after the Launcher's own): managed from the next check, once.
    alone.pick_up_product_files();
    alone.pick_up_product_files();
    let ids: Vec<String> = alone.products().into_iter().map(|p| p.product).collect();
    assert_eq!(ids, ["vesper-launcher", "vesper"]);
    assert_eq!(alone.report().products.len(), 2);
    assert_eq!(alone.roles(), ["presentation"]);
}

// ---- Releases from GitHub: the direct mode and "Add app" (a fake GitHub, a throwaway key) ----

fn github_config(
    fake: &vesper_releases::fake::FakeGitHub,
) -> vesper_launcher::github::GithubConfig {
    serde_json::from_value(serde_json::json!({
        "api": fake.api(), "repository": "example/vesper", "channel": "stable",
    }))
    .unwrap()
}

/// The fake product as the Vesper server (`vesper`, folder kind), version `version`.
fn server_package(port: u16, version: &str) -> Vec<u8> {
    let exe = std::env::current_exe().unwrap();
    format!(
        "#!/bin/sh\nVESPER_FAKE_PRODUCT_PORT={port} VESPER_FAKE_PRODUCT_VERSION={version} exec '{}' fake_product --exact --nocapture --test-threads=1\n",
        exe.display()
    )
    .into_bytes()
}

/// A Launcher with no church server checks GitHub itself: it offers the newest stable release
/// of its products, downloads and verifies it, and installs it with the usual health check;
/// a package signed by another key is refused.
#[tokio::test(flavor = "multi_thread")]
async fn without_a_church_server_the_launcher_updates_from_github() {
    let fake = vesper_releases::fake::FakeGitHub::start();
    let platform = this_platform();
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let v1 = server_package(port, "1.1.0");
    fake.release(
        "server-v1.1.0",
        "1.1.0",
        false,
        &[("vesper", platform.as_str(), "vesper-1.1.0", &v1)],
    );
    let beta = server_package(port, "1.2.0-beta1");
    fake.release(
        "server-v1.2.0-beta1",
        "1.2.0-beta1",
        true,
        &[("vesper", platform.as_str(), "vesper-1.2.0-beta1", &beta)],
    );
    let mut cfg: Config = serde_json::from_value(serde_json::json!({
        "stateDir": dir.path().join("state"),
        "listen": "127.0.0.1:0",
        "keyDir": null,
        "keys": [fake.public_key()],
        "healthTimeoutSecs": 4,
        "products": [{
            "product": "vesper", "name": "Vesper", "install": {"kind": "dir"},
            "healthUrl": format!("http://127.0.0.1:{port}/api/v1/server"),
        }],
    }))
    .unwrap();
    cfg.github = github_config(&fake);
    let l = Launcher::open(cfg).unwrap();
    // 1.0.0 installed by hand first.
    let v0 = server_package(port, "1.0.0");
    let p0 = dir.path().join("vesper-1.0.0");
    std::fs::write(&p0, &v0).unwrap();
    l.add_package(
        &p0,
        "vesper-1.0.0",
        &fake.sign(&v0, "vesper", "1.0.0", &platform),
    )
    .unwrap();
    assert_eq!(
        l.update("vesper", "1.0.0").await.unwrap().result,
        UpdateResult::Installed
    );

    assert!(l.github_direct(), "no channel: GitHub directly");
    l.check_github().await;
    let ov = serde_json::to_value(l.overview()).unwrap();
    assert_eq!(ov["github"]["direct"], serde_json::json!(true));
    assert!(ov["github"]["status"]["lastSuccess"].is_string(), "{ov}");
    // The stable channel: not the beta.
    assert_eq!(
        ov["available"]["vesper"],
        serde_json::json!("1.1.0"),
        "{ov}"
    );
    // The next check waits for the interval: nothing is asked again.
    let asked = fake.listings().len();
    l.check_github().await;
    assert_eq!(fake.listings().len(), asked);

    let a = protocol::LauncherAction::Update {
        product: "vesper".into(),
        version: "1.1.0".into(),
    };
    let o = vesper_launcher::channel::run_action(&l, &a).await.unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{o:?}");
    assert_eq!(reported_version(port).as_deref(), Some("1.1.0"));

    // Signed by someone else: refused before anything changes.
    let stranger = TestSigner::generate();
    let bad = server_package(port, "1.3.0");
    let comment = PackageMeta {
        product: "vesper".into(),
        version: "1.3.0".into(),
        platform: platform.clone(),
    }
    .comment();
    fake.raw_release(
        "server-v1.3.0",
        false,
        false,
        vec![
            ("vesper-1.3.0".into(), bad.clone()),
            (
                "vesper-1.3.0.minisig".into(),
                stranger.sign(&Hasher::bytes(&bad), &comment).into_bytes(),
            ),
            (
                "manifest.json".into(),
                serde_json::to_vec(&serde_json::json!({"assets": [
                    {"product": "vesper", "platform": platform, "file": "vesper-1.3.0"}]}))
                .unwrap(),
            ),
        ],
    );
    let a = protocol::LauncherAction::Update {
        product: "vesper".into(),
        version: "1.3.0".into(),
    };
    let e = vesper_launcher::channel::run_action(&l, &a)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("isn't one of"), "{e}");
    assert_eq!(l.installed("vesper").version.as_deref(), Some("1.1.0"));
    l.shutdown().await;
}

/// "Add app" in the Launcher's own window: the catalogue offers only what this machine can run
/// and doesn't have; the install downloads from GitHub (no church server), verifies and
/// installs it, and the Launcher manages the product from then on, also after a restart.
#[tokio::test(flavor = "multi_thread")]
async fn add_app_installs_a_product_this_machine_can_run() {
    let fake = vesper_releases::fake::FakeGitHub::start();
    let linux = "linux-x86_64";
    fake.release(
        "oriel-v0.2.0",
        "0.2.0",
        false,
        &[
            (
                "vesper-oriel",
                linux,
                "vesper-oriel_0.2.0_amd64.deb",
                b"version=0.2.0\n",
            ),
            (
                "vesper-oriel",
                "windows-x86_64",
                "vesper-oriel_0.2.0_x64-setup.exe",
                b"exe",
            ),
        ],
    );
    fake.release(
        "server-v0.2.0",
        "0.2.0",
        false,
        &[(
            "vesper",
            linux,
            "vesper_0.2.0-2_amd64.deb",
            b"version=0.2.0\n",
        )],
    );
    let fx = deb_fixture("");
    let mut cfg = fx.cfg.clone();
    cfg.products.clear();
    cfg.platform = Some(linux.into());
    cfg.roles = vec!["display".into()];
    cfg.keys.push(fake.public_key());
    cfg.github = github_config(&fake);
    let l = Launcher::open(cfg.clone()).unwrap();
    let cat = tokio::task::spawn_blocking({
        let l = l.clone();
        move || l.catalogue()
    })
    .await
    .unwrap()
    .unwrap();
    let ids: Vec<&str> = cat.iter().map(|c| c.product.as_str()).collect();
    // A display box: no Vesper server.
    assert_eq!(ids, ["vesper-oriel"]);
    assert_eq!(cat[0].stable.as_deref(), Some("0.2.0"));

    let add = |product: &str| protocol::LauncherAction::Install {
        product: product.into(),
        version: "0.2.0".into(),
    };
    let o = vesper_launcher::channel::run_action(&l, &add("vesper-oriel"))
        .await
        .unwrap();
    assert_eq!(o.result, UpdateResult::Installed, "{o:?}");
    assert_eq!(
        l.installed("vesper-oriel").version.as_deref(),
        Some("0.2.0")
    );
    assert!(
        l.report()
            .products
            .iter()
            .any(|p| p.product == "vesper-oriel")
    );
    assert!(
        fx.calls().contains("vesper-oriel_0.2.0_amd64.deb"),
        "{}",
        fx.calls()
    );
    // Not twice; not what the machine can't run.
    let e = vesper_launcher::channel::run_action(&l, &add("vesper-oriel"))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("already installed"), "{e}");
    let e = vesper_launcher::channel::run_action(&l, &add("vesper"))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("display box"), "{e}");
    let e = vesper_launcher::channel::run_action(&l, &add("vesper-launcher"))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("can't be added"), "{e}");
    // The refused adds are reported with their reasons (newest first), so Oriel can say why;
    // not the one of a product managed here.
    let failed = l.report().failed_installs;
    let got: Vec<(&str, &str)> = failed
        .iter()
        .map(|f| (f.product.as_str(), f.name.as_str()))
        .collect();
    assert_eq!(
        got,
        [("vesper-launcher", "Vesper Launcher"), ("vesper", "Vesper")]
    );
    assert_eq!(failed[1].outcome.result, UpdateResult::Failed);
    assert_eq!(failed[1].outcome.to, "0.2.0");
    assert!(
        failed[1].outcome.message.contains("display box"),
        "{failed:?}"
    );
    // Kept across a restart.
    l.shutdown().await;
    let again = Launcher::open(cfg).unwrap();
    assert!(again.products().iter().any(|p| p.product == "vesper-oriel"));
}
