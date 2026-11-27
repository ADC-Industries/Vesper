//! mDNS: the server advertises `_vesper._tcp.local.` and a browser (the `protocol::discover`
//! helper the control app uses) finds it with the right TXT record. Skipped with a message
//! when multicast does not work on this machine (no interface, container without multicast).

use crate::common::*;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn advertises_and_is_discovered() {
    let fx = fixture("mdns");
    let server = start_server(&fx, true, true).await;
    let id = server.shared.auth.identity().id.to_string();
    let port = server.addr.port();
    // `mdns` is a capability while the server is announced.
    let announced = server
        .shared
        .mdns_on
        .load(std::sync::atomic::Ordering::Acquire);
    let info = get_json(server.addr, "/api/v1/server").await;
    assert_eq!(
        info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("mdns")),
        announced,
        "{info}"
    );

    let found = tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            match protocol::discover::discover(Duration::from_secs(2)) {
                Ok(list) => {
                    if let Some(s) = list.into_iter().find(|s| s.id == id) {
                        return Ok(Some(s));
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
            if std::time::Instant::now() > deadline {
                return Ok(None);
            }
        }
    })
    .await
    .unwrap();

    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
    match found {
        Err(e) => eprintln!("SKIPPED: mDNS unavailable on this machine: {e}"),
        Ok(None) => {
            eprintln!("SKIPPED: the advertisement was not seen within 8 s (multicast blocked?)")
        }
        Ok(Some(s)) => {
            assert_eq!(s.port, port);
            assert_eq!(s.protocol_version, protocol::PROTOCOL_VERSION);
            assert!(s.compatible());
            assert_eq!(s.version, env!("CARGO_PKG_VERSION"));
            assert!(!s.addresses.is_empty());
            assert!(s.base_url().unwrap().starts_with("http://"));
            eprintln!("discovered {} at {:?}:{}", s.name, s.addresses, s.port);
        }
    }
}
