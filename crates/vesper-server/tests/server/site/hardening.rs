//! Holes closed in the site's access checks (each test proves one):
//!
//! - a burst of PIN unlocks sent at once tries no more PINs than [`PIN_TRIES`] before the
//!   lockout;
//! - a client can't mint itself a token for Vesper's own audience (`app:vesper`), which would
//!   be taken as a paired client's key and outlive the revocation of its real one;
//! - odd spellings of a path don't get past the token check;
//! - a web page from elsewhere, open in a browser on the server machine, isn't trusted as
//!   the server machine;
//! - a user name with spaces around it (which the Site Manager trims) is checked as that
//!   user: the last admin stays an admin;
//! - a pairing code sent by two clients at once pairs one of them;
//! - an approved pairing whose key its client never fetches doesn't leave that key valid;
//! - one address can't fill the waiting room of pairings, and a waiting pairing whose client
//!   went away ends;
//! - a Site Manager that didn't start (its port in use at boot) is tried again: clients aren't
//!   locked out until a restart;
//! - a Site Manager whose API port is taken runs without its API, and its status says why
//!   (until a start with the API).
//! - a server started from a damaged `site.json` says so in its status (`settingsDamaged`)
//!   until an admin sets the access again.

use super::*;
use vesper_server::site::PIN_TRIES;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_of_pin_unlocks_tries_no_more_than_the_allowed_pins() {
    let fx = fixture("site-pin-burst");
    let server = start_with(
        &fx,
        false,
        SiteConfig {
            pin_lockout: Duration::from_secs(600),
            ..SiteConfig::loopback()
        },
    )
    .await;
    let addr = server.addr;
    let a = pair(&server, "Booth").await;
    ok(
        &set_access(addr, Some(&a), "pin", Some("97531")).await,
        200,
        "pin",
    );

    // 40 wrong PINs at once, as a guesser would send them.
    let tries: Vec<_> = (0..40)
        .map(|i| {
            let a = a.clone();
            tokio::spawn(async move {
                call(
                    addr,
                    "POST",
                    "/api/v1/site/unlock",
                    Some(&a),
                    Some(json!({"pin": format!("{:04}", 1000 + i)})),
                )
                .await
                .status
            })
        })
        .collect();
    let mut checked = 0;
    for t in tries {
        let status = t.await.unwrap();
        assert!(status == 401 || status == 429, "{status}");
        if status == 401 {
            checked += 1;
        }
    }
    assert!(
        checked <= PIN_TRIES as usize,
        "{checked} PINs were checked before the lockout (at most {PIN_TRIES})"
    );
    // Locked out: even the right PIN waits.
    assert_eq!(
        call(
            addr,
            "POST",
            "/api/v1/site/unlock",
            Some(&a),
            Some(json!({"pin": "97531"}))
        )
        .await
        .status,
        429
    );
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_cannot_mint_a_vesper_key_that_outlives_its_revocation() {
    let fx = fixture("site-mint");
    let server = start_site(&fx, false).await;
    let addr = server.addr;
    let a = pair(&server, "Booth").await;
    let b = pair(&server, "Tablet").await;
    let a_id = idn_open_protocol::trust::token::peek(&a)
        .unwrap()
        .sub
        .trim_start_matches("key:")
        .to_string();

    // "open": the client is the site's admin, and may have engine tokens, but not one for
    // Vesper's own API (that is what a paired client's key is for).
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/site/tokens",
            Some(&a),
            Some(json!({"engine": "desk", "scope": "control"})),
        )
        .await,
        201,
        "an engine token",
    );
    let r = call(
        addr,
        "POST",
        "/api/v1/site/tokens",
        Some(&a),
        Some(json!({"engine": "app:vesper", "scope": "control"})),
    )
    .await;
    let minted = (r.status == 201).then(|| r.json()["token"].as_str().unwrap().to_string());

    // Its key revoked: the client is out, whatever it minted before.
    assert_eq!(
        call(
            addr,
            "DELETE",
            &format!("/api/v1/site/api-keys/{a_id}"),
            Some(&b),
            None
        )
        .await
        .status,
        204
    );
    assert_eq!(
        call(addr, "GET", "/api/v1/state", Some(&a), None)
            .await
            .status,
        401
    );
    if let Some(t) = &minted {
        assert_eq!(
            call(addr, "GET", "/api/v1/state", Some(t), None)
                .await
                .status,
            401,
            "a token the revoked client minted for app:vesper still gets in"
        );
    }
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn odd_path_spellings_need_a_key_too() {
    let fx = fixture("site-paths");
    let server = start_site(&fx, false).await;
    let addr = server.addr;
    for path in [
        "//api/v1/state",
        "/api//v1/state",
        "/api/v1//state",
        "/api/v1/state/",
        "/%61pi/v1/state",
        "/api/v1/%73tate",
        "/api/v1/pair/../state",
        "/api/v1/pair/%2e%2e/state",
        "/API/v1/state",
    ] {
        let r = http(addr, "GET", path, &[], None).await;
        let body = String::from_utf8_lossy(&r.body);
        assert!(
            !body.contains("\"revision\""),
            "{path}: the state without a key ({})",
            r.status
        );
    }
    for path in ["//api/v1/site/settings", "/api/v1/pair/../site/settings"] {
        let r = http(
            addr,
            "PUT",
            path,
            &[],
            Some(json!({"enabled": true, "joinMode": "code_or_approval", "access": "open"})),
        )
        .await;
        assert_ne!(r.status, 200, "{path}");
    }
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_web_page_from_elsewhere_is_not_the_server_machine() {
    let fx = fixture("site-origin");
    // Localhost trusted: the server machine's own programs need no key.
    let server = start_site(&fx, true).await;
    let addr = server.addr;
    let code = |origin: Option<&'static str>| async move {
        let h: Vec<(&str, &str)> = origin.map(|o| vec![("Origin", o)]).unwrap_or_default();
        http(addr, "GET", "/api/v1/pairing/code", &h, None).await
    };
    // The server's own programs: no browser (no Origin), the desktop app's pages, a page
    // the server machine serves itself.
    for origin in [
        None,
        Some("tauri://localhost"),
        Some("http://tauri.localhost"),
        Some("https://tauri.localhost"),
        Some("http://localhost:5173"),
        Some("http://127.0.0.1:8080"),
        Some("http://[::1]:8080"),
    ] {
        let r = code(origin).await;
        assert_eq!(r.status, 200, "{origin:?}");
    }
    // A web page from anywhere else, opened in a browser on the server machine, isn't the
    // server machine (it would read the pairing code, or drive the show).
    for origin in [
        "http://attacker.example",
        "https://attacker.example:8443",
        "http://localhost.attacker.example",
        "http://127.0.0.1.attacker.example",
        "null",
    ] {
        let r = code(Some(origin)).await;
        assert_eq!(
            r.status,
            401,
            "{origin}: {}",
            String::from_utf8_lossy(&r.body)
        );
        let info = http(addr, "GET", "/api/v1/server", &[("Origin", origin)], None)
            .await
            .json();
        assert_eq!(info["auth"], "unpaired", "{origin}");
        assert!(info.get("pairingCode").is_none(), "{origin}: {info}");
    }
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = format!("ws://{addr}/api/v1/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Origin", "http://attacker.example".parse().unwrap());
    assert!(
        tokio_tungstenite::connect_async(req).await.is_err(),
        "a WebSocket from a page elsewhere"
    );
    // Browsers mark what a page sends (`Sec-Fetch-Site`, `Referer`), even where it carries
    // no `Origin`. What the desktop app's page sends on Linux (WebKitGTK, `tauri://localhost`):
    // `<img>` loads with no `Origin` or `Referer`, `Sec-Fetch-Site: cross-site` (reads, they
    // keep the machine's trust); `fetch()` with its `Origin`. Oriel served here: same-origin.
    let img = [
        ("Sec-Fetch-Site", "cross-site"),
        ("Sec-Fetch-Mode", "no-cors"),
        ("Sec-Fetch-Dest", "image"),
    ];
    assert_eq!(
        http(addr, "GET", "/api/v1/state", &img, None).await.status,
        200
    );
    let clear = json!({"type": "clearAll"});
    let (own, own_page) = (format!("http://{addr}"), format!("http://{addr}/show"));
    for h in [
        &[
            ("Origin", "tauri://localhost"),
            ("Sec-Fetch-Site", "cross-site"),
        ][..],
        &[
            ("Origin", "http://tauri.localhost"),
            ("Referer", "http://tauri.localhost/"),
            ("Sec-Fetch-Site", "cross-site"),
        ],
        &[
            ("Origin", own.as_str()),
            ("Referer", own_page.as_str()),
            ("Sec-Fetch-Site", "same-origin"),
        ],
        &[("Sec-Fetch-Site", "none")],
    ] {
        let r = http(addr, "POST", "/api/v1/commands", h, Some(clear.clone())).await;
        assert!(
            r.status < 300,
            "{h:?}: {} {}",
            r.status,
            String::from_utf8_lossy(&r.body)
        );
    }
    // A page elsewhere: a change without its `Origin` (no browser does that, but no
    // `Origin` must not mean "the machine" for a page), or one that names its page.
    for h in [
        &[
            ("Sec-Fetch-Site", "cross-site"),
            ("Sec-Fetch-Mode", "no-cors"),
        ][..],
        &[("Sec-Fetch-Site", "same-site")],
        &[("Referer", "http://attacker.example/page")],
        &[("Referer", "http://localhost.attacker.example/")],
    ] {
        let r = http(addr, "POST", "/api/v1/commands", h, Some(clear.clone())).await;
        assert_eq!(r.status, 401, "{h:?}");
    }
    let r = http(
        addr,
        "GET",
        "/api/v1/state",
        &[("Referer", "http://attacker.example/page")],
        None,
    )
    .await;
    assert_eq!(r.status, 401);
    // A key still works from such a page (a paired browser client).
    let key = pair(&server, "Browser").await;
    let bearer = format!("Bearer {key}");
    let r = http(
        addr,
        "GET",
        "/api/v1/state",
        &[
            ("Origin", "http://attacker.example"),
            ("Authorization", &bearer),
        ],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_user_named_with_spaces_around_is_that_user() {
    let fx = fixture("site-user-spaces");
    let server = start_site(&fx, false).await;
    let addr = server.addr;
    let a = pair(&server, "Booth").await;
    let a = Some(a.as_str());
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
    ok(&set_access(addr, a, "users", None).await, 200, "users");
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
        "sign in",
    );

    // The site's last admin user, spelled with a space after: still the last admin.
    let r = call(
        addr,
        "PUT",
        "/api/v1/site/users/root%20",
        a,
        Some(json!({"role": "viewer", "password": "new-root-password"})),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));
    let users = get_json_auth(addr, "/api/v1/site/users", a).await;
    assert_eq!(users[0]["name"], "root");
    assert_eq!(users[0]["role"], "admin", "{users}");
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pairing_code_sent_twice_at_once_pairs_one_client() {
    let fx = fixture("site-pair-twice");
    let server = start_site(&fx, false).await;
    let addr = server.addr;
    let code = server.pairing_code();
    let sends: Vec<_> = ["One", "Two"]
        .into_iter()
        .map(|name| {
            let code = code.clone();
            tokio::spawn(async move {
                http(
                    addr,
                    "POST",
                    "/api/v1/pair",
                    &[],
                    Some(json!({"code": code, "clientName": name})),
                )
                .await
            })
        })
        .collect();
    let mut keys = Vec::new();
    let mut statuses = Vec::new();
    for s in sends {
        let r = s.await.unwrap();
        statuses.push(r.status);
        if r.status == 200 {
            keys.push(r.json()["token"].as_str().unwrap().to_string());
        }
    }
    statuses.sort();
    assert_eq!(statuses, [200, 401], "the code pairs one client");
    let listed = get_json_auth(addr, "/api/v1/site/api-keys", Some(&keys[0])).await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "one key: {listed}");
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// The site's key named `name`, as `GET /api/v1/site/api-keys` lists it.
async fn listed_key(addr: SocketAddr, auth: &str, name: &str) -> Value {
    let keys = get_json_auth(addr, "/api/v1/site/api-keys", Some(auth)).await;
    keys.as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"] == name)
        .unwrap_or_else(|| panic!("no key {name}: {keys}"))
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_approved_pairing_whose_answer_is_never_fetched_revokes_its_key() {
    let fx = fixture("site-pair-unfetched");
    let life = Duration::from_secs(2);
    let server = start_with(
        &fx,
        false,
        SiteConfig {
            pairing_life: life,
            ..SiteConfig::loopback()
        },
    )
    .await;
    let addr = server.addr;
    let admin = pair(&server, "Booth").await;
    let ask = |name: &'static str| async move {
        let r = http(
            addr,
            "POST",
            "/api/v1/pair",
            &[],
            Some(json!({"clientName": name})),
        )
        .await;
        ok(&r, 202, "pair")["pairingId"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let approve = |admin: String| async move {
        let w = get_json_auth(addr, "/api/v1/site/pairings", Some(&admin)).await;
        let id = w[0]["id"].as_str().unwrap().to_string();
        ok(
            &call(
                addr,
                "POST",
                &format!("/api/v1/site/pairings/{id}/approve"),
                Some(&admin),
                None,
            )
            .await,
            204,
            "approve",
        );
    };

    // Approved and fetched: the key stays.
    let fetched = ask("Phone").await;
    approve(admin.clone()).await;
    let r = http(addr, "GET", &format!("/api/v1/pair/{fetched}"), &[], None).await;
    ok(&r, 200, "fetch the key");
    // Approved, but the client never fetches its key (gone, or its answer was lost).
    let unfetched = ask("Tablet").await;
    approve(admin.clone()).await;
    let key = listed_key(addr, &admin, "Oriel – Tablet").await;
    assert_eq!(key["revoked"], false, "{key}");
    // Another, whose key is revoked before its pairing ends (as the Site Manager does itself
    // since IDN Beta8 for a key approved as its client withdrew): the expiry takes it in its
    // stride.
    let early = ask("Laptop").await;
    approve(admin.clone()).await;
    let laptop = listed_key(addr, &admin, "Oriel – Laptop").await;
    let laptop_id = laptop["id"].as_str().unwrap().to_string();
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/site/api-keys/{laptop_id}"),
            Some(&admin),
            None,
        )
        .await,
        204,
        "revoked first",
    );

    // The pairing ends: the key that reached no one is revoked.
    tokio::time::sleep(life).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let key = listed_key(addr, &admin, "Oriel – Tablet").await;
        if key["revoked"] == true {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the unfetched key is still valid: {key}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let r = http(addr, "GET", &format!("/api/v1/pair/{unfetched}"), &[], None).await;
    assert_eq!(r.status, 404, "the pairing ended");
    let r = http(addr, "GET", &format!("/api/v1/pair/{early}"), &[], None).await;
    assert_eq!(
        r.status, 404,
        "the pairing whose key was revoked first ended too"
    );
    let laptop = listed_key(addr, &admin, "Oriel – Laptop").await;
    assert_eq!(laptop["revoked"], true, "{laptop}");
    let phone = listed_key(addr, &admin, "Oriel – Phone").await;
    assert_eq!(phone["revoked"], false, "the fetched key stays: {phone}");
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn waiting_pairings_are_limited_per_address_and_end_when_their_client_goes() {
    let fx = fixture("site-pair-room");
    let idle = Duration::from_millis(1500);
    let server = start_with(
        &fx,
        false,
        SiteConfig {
            pairing_idle: idle,
            ..SiteConfig::loopback()
        },
    )
    .await;
    let addr = server.addr;
    let admin = pair(&server, "Booth").await;
    let ask = |name: &'static str| async move {
        http(
            addr,
            "POST",
            "/api/v1/pair",
            &[],
            Some(json!({"clientName": name})),
        )
        .await
    };
    let mut ids = Vec::new();
    for name in ["One", "Two", "Three"] {
        ids.push(
            ok(&ask(name).await, 202, name)["pairingId"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    // A fourth from the same address waits for those.
    let r = ask("Four").await;
    assert_eq!(r.status, 429, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("from this address"),
        "{}",
        r.json()
    );
    // "One" keeps asking for its answer; the others went away.
    let until = Instant::now() + idle * 2;
    while Instant::now() < until {
        let r = http(addr, "GET", &format!("/api/v1/pair/{}", ids[0]), &[], None).await;
        assert_eq!(r.status, 202, "still waiting");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    for id in &ids[1..] {
        let r = http(addr, "GET", &format!("/api/v1/pair/{id}"), &[], None).await;
        assert_eq!(r.status, 404, "ended once its client stopped asking");
    }
    let w = get_json_auth(addr, "/api/v1/site/pairings", Some(&admin)).await;
    assert_eq!(w.as_array().unwrap().len(), 1, "{w}");
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_site_manager_whose_port_was_taken_starts_once_it_is_free() {
    let fx = fixture("site-start-retry");
    // Another program (another discovery service) holds the discovery port at boot.
    let blocker = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    let udp = std::net::UdpSocket::bind(("0.0.0.0", port)).unwrap();
    let server = start_with(
        &fx,
        true,
        SiteConfig {
            discovery_port: port,
            start_retry: Duration::from_millis(300),
            ..SiteConfig::loopback()
        },
    )
    .await;
    let addr = server.addr;
    let st = get_json(addr, "/api/v1/site").await;
    assert_eq!(st["running"], false, "{st}");
    assert!(st["error"].is_string(), "{st}");
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": server.pairing_code(), "clientName": "Booth"})),
    )
    .await;
    assert_eq!(r.status, 503, "no site to pair with yet");

    // The port is free again: the next try starts it, and clients pair.
    drop((blocker, udp));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let st = get_json(addr, "/api/v1/site").await;
        if st["running"] == true {
            assert!(st.get("error").is_none_or(Value::is_null), "{st}");
            break;
        }
        assert!(Instant::now() < deadline, "never started: {st}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let key = pair(&server, "Booth").await;
    assert_eq!(
        call(addr, "GET", "/api/v1/state", Some(&key), None)
            .await
            .status,
        200
    );
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_site_manager_whose_api_port_is_taken_runs_and_says_why() {
    let fx = fixture("site-api-port-taken");
    // Another program holds the API (admin panel) port at boot: the Site Manager runs
    // without its API, and the status says why.
    let blocker = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    let server = start_with(
        &fx,
        true,
        SiteConfig {
            api_port: port,
            ..SiteConfig::loopback()
        },
    )
    .await;
    let addr = server.addr;
    let st = get_json(addr, "/api/v1/site").await;
    assert_eq!(st["running"], true, "{st}");
    assert!(st.get("error").is_none_or(Value::is_null), "{st}");
    assert!(st.get("apiPort").is_none_or(Value::is_null), "{st}");
    let why = st["apiError"].as_str().unwrap_or_else(|| panic!("{st}"));
    assert!(why.contains(&port.to_string()), "names the port: {st}");
    // Discovery and the site's authority run; what needs the API (pairing a client, a
    // change run as a user) says so.
    assert!(st["discoveryPort"].as_u64().is_some_and(|p| p > 0), "{st}");
    let r = http(
        addr,
        "POST",
        "/api/v1/pair",
        &[],
        Some(json!({"code": server.pairing_code(), "clientName": "Booth"})),
    )
    .await;
    assert_ne!(r.status, 200, "pairing needs the API");

    server.shutdown().await.unwrap();

    // The port is free at the next start: the API runs, the reason goes, clients pair, and
    // the new site's join mode (which needs the API) is set now.
    drop(blocker);
    let server = start_with(
        &fx,
        true,
        SiteConfig {
            api_port: port,
            ..SiteConfig::loopback()
        },
    )
    .await;
    let addr = server.addr;
    let st = get_json(addr, "/api/v1/site").await;
    assert_eq!(st["running"], true, "{st}");
    assert_eq!(st["apiPort"], port, "{st}");
    assert!(st.get("apiError").is_none_or(Value::is_null), "{st}");
    assert_eq!(st["joinMode"], "code_or_approval", "{st}");
    let key = pair(&server, "Booth").await;
    assert_eq!(
        call(addr, "GET", "/api/v1/state", Some(&key), None)
            .await
            .status,
        200
    );
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_damaged_site_file_shows_in_the_status_until_the_access_is_set_again() {
    let fx = fixture("site-damaged-status");
    // As a power cut right after a save leaves it: zeros.
    std::fs::create_dir_all(fx.dir.join("machine")).unwrap();
    std::fs::write(fx.dir.join("machine").join("site.json"), [0u8; 64]).unwrap();
    let server = start_with(&fx, true, SiteConfig::loopback()).await;
    let addr = server.addr;
    let key = pair(&server, "Tablet").await;
    // Every client sees it, the paired one (which only reads now) and the server machine.
    let st = get_json_auth(addr, "/api/v1/site", Some(&key)).await;
    assert_eq!(st["settingsDamaged"], true, "{st}");
    assert_eq!(st["you"]["access"], "users", "{st}");
    assert_eq!(st["you"]["role"], "viewer", "{st}");
    let st = get_json_auth(addr, "/api/v1/site", None).await;
    assert_eq!(st["settingsDamaged"], true, "{st}");
    // The paired client can't clear it; the server machine sets the access again (the same
    // `users`), and it goes, for everyone and after a restart.
    assert_eq!(
        set_access(addr, Some(&key), "users", None).await.status,
        403
    );
    assert_eq!(
        get_json_auth(addr, "/api/v1/site", None).await["settingsDamaged"],
        true
    );
    ok(
        &set_access(addr, None, "users", None).await,
        200,
        "access set again",
    );
    for who in [None, Some(key.as_str())] {
        let st = get_json_auth(addr, "/api/v1/site", who).await;
        assert!(st.get("settingsDamaged").is_none(), "{st}");
    }
    // E05-2: lifting the lockdown (the access unchanged) is in the audit.
    let audit = get_json_auth(addr, "/api/v1/site/audit?n=100", None).await;
    assert!(
        audit_has(&audit, "settings", |e| e["ok"] == true
            && e["detail"]["wasDamaged"] == true),
        "{audit:#}"
    );
    server.shutdown().await.unwrap();
    let server = start_with(&fx, true, SiteConfig::loopback()).await;
    let st = get_json_auth(server.addr, "/api/v1/site", None).await;
    assert!(st.get("settingsDamaged").is_none(), "{st}");
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}
