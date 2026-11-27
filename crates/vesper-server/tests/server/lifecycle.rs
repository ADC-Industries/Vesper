//! A server that is dropped instead of shut down (what a panicking test does to the servers it
//! started) still stops: its Site Manager, its render thread and screen clocks, and the shared
//! state they hold, without blocking the async threads it is dropped on.

use crate::common::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Waits up to 15 s (polling) for `done`.
async fn eventually(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_server_stops_everything() {
    let fx = fixture("drop");
    let server = start_with_site(&fx, true, vesper_server::site::SiteConfig::loopback()).await;
    let ds = server
        .shared
        .site
        .handle()
        .expect("the site's Site Manager runs")
        .discovery_port();
    assert!(std::net::TcpStream::connect(("127.0.0.1", ds)).is_ok());
    let mut c = Client::connect(server.addr, "").await;
    c.ok(serde_json::json!({"type": "triggerSlide",
        "presentation": fx.song.id.0.to_string(), "index": 0}))
        .await;
    let shared = Arc::downgrade(&server.shared);
    let site = server.shared.site.clone();

    // Dropping must not block this (async) thread on joins.
    let t = Instant::now();
    drop(server);
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "dropping took {:?}",
        t.elapsed()
    );

    eventually("the Site Manager stops", || site.handle().is_none()).await;
    eventually("its discovery port closes", || {
        std::net::TcpStream::connect(("127.0.0.1", ds)).is_err()
    })
    .await;
    drop(site);
    drop(c);
    eventually(
        "the render thread, clocks and tasks let go of the shared state",
        || shared.upgrade().is_none(),
    )
    .await;
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// A test body that panics while its server runs (a WebSocket open, a slide live) ends at once
/// with the panic, as `#[tokio::test]` runs it: the runtime is torn down while the server is
/// still up. Its site stops afterwards.
#[test]
fn a_panicking_test_with_a_running_server_ends() {
    let fx = fixture("panic");
    let (song, dir) = (fx.song.id.0.to_string(), fx.dir.clone());
    let (site_tx, site_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let server =
                    start_with_site(&fx, true, vesper_server::site::SiteConfig::loopback()).await;
                site_tx.send(server.shared.site.clone()).unwrap();
                let mut c = Client::connect(server.addr, "").await;
                c.ok(serde_json::json!({"type": "triggerSlide", "presentation": song, "index": 0}))
                    .await;
                panic!("deliberate panic of a failing test (expected in this test's output)");
            })
        }));
        let _ = done_tx.send(r.is_err());
    });
    let site = site_rx
        .recv_timeout(Duration::from_secs(60))
        .expect("the server starts");
    let panicked = done_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the panicking test ends instead of hanging");
    assert!(panicked);
    let deadline = Instant::now() + Duration::from_secs(15);
    while site.handle().is_some() {
        assert!(Instant::now() < deadline, "the Site Manager stops");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::fs::remove_dir_all(&dir).ok();
}
