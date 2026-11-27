//! Presence and edit locks (G8): who has what open, locks taken on the first edit, `423
//! locked` for others, take-over, and expiry when a client closes the document, its socket, or
//! stops answering. Synthetic data only.

use crate::common::*;
use futures_util::SinkExt;
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

async fn report(c: &mut Client, client_id: &str, computer: &str, open: Value) {
    let m = json!({"type": "presence", "clientId": client_id, "computer": computer, "open": open});
    c.ws.send(Message::Text(m.to_string().into()))
        .await
        .unwrap();
}

/// Reads until a `presenceChanged` whose list passes `pred`.
async fn wait_presence(c: &mut Client, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let r = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let m = c.next_any().await;
            if m["type"] == "presenceChanged" && pred(&m["presence"]) {
                return m["presence"].clone();
            }
        }
    })
    .await;
    r.unwrap_or_else(|_| panic!("no presenceChanged with {what}"))
}

async fn wait_list(addr: std::net::SocketAddr, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    // A deadline, not a count of tries: under load each GET can take a while (M-3).
    let end = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let v = get_json(addr, "/api/v1/presence").await;
        if pred(&v) {
            return v;
        }
        assert!(
            std::time::Instant::now() < end,
            "GET /presence never showed {what}: {v}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn edit_body(client: Option<&str>, slide: &str, text: &str) -> Value {
    let mut b = json!({"ops": [{"type": "setSlideText", "slide": slide, "text": text}]});
    if let Some(c) = client {
        b["clientId"] = json!(c);
    }
    b
}

fn first_slide(p: &Value) -> String {
    p["sections"][0]["slides"][0]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn presence_is_listed_and_goes_with_the_socket() {
    let fx = fixture("presence-list");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let pid = fx.song.id.0.to_string();

    let mut a = Client::connect(addr, "protocolVersion=1").await;
    let mut b = Client::connect(addr, "protocolVersion=1").await;
    // Clients that haven't reported aren't listed.
    let v = get_json(addr, "/api/v1/presence").await;
    assert_eq!(v["clients"].as_array().unwrap().len(), 0, "{v}");

    report(
        &mut a,
        "oriel-a",
        "Booth laptop",
        json!([{"kind": "presentation", "id": pid, "place": "edit"}]),
    )
    .await;
    let seen = wait_presence(&mut b, "Booth laptop", |p| {
        p["clients"].as_array().unwrap().len() == 1
    })
    .await;
    let c = &seen["clients"][0];
    assert_eq!(c["computer"], "Booth laptop", "{seen}");
    assert_eq!(c["clientId"], "oriel-a");
    assert_eq!(c["name"], "server machine");
    assert_eq!(c["open"][0]["place"], "edit");
    assert_eq!(c["open"][0]["id"], pid);

    report(
        &mut b,
        "oriel-b",
        "Lobby laptop",
        json!([{"kind": "presentation", "id": pid, "place": "library"}]),
    )
    .await;
    wait_list(addr, "two clients", |v| {
        v["clients"].as_array().unwrap().len() == 2
    })
    .await;

    // Too many open documents: an error, the list unchanged.
    let many: Vec<Value> = (0..40)
        .map(|i| json!({"kind": "presentation", "id": format!("x{i}"), "place": "library"}))
        .collect();
    report(&mut b, "oriel-b", "Lobby laptop", Value::Array(many)).await;
    loop {
        let m = b.next_any().await;
        if m["type"] == "error" {
            assert_eq!(m["error"]["code"], "badRequest", "{m}");
            break;
        }
    }

    // A closes: B sees it go.
    a.ws.close(None).await.unwrap();
    let after = wait_presence(&mut b, "A gone", |p| {
        p["clients"].as_array().unwrap().len() == 1
    })
    .await;
    assert_eq!(after["clients"][0]["clientId"], "oriel-b", "{after}");
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_edit_locks_and_take_over_moves_the_lock() {
    let fx = fixture("presence-locks");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let pid = fx.song.id.0.to_string();
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let slide = first_slide(&p);
    let path = format!("/api/v1/presentations/{pid}/edits");
    let open = json!([{"kind": "presentation", "id": pid, "place": "edit"}]);

    let mut a = Client::connect(addr, "protocolVersion=1").await;
    let mut b = Client::connect(addr, "protocolVersion=1").await;
    report(&mut a, "oriel-a", "Booth laptop", open.clone()).await;
    report(&mut b, "oriel-b", "Lobby laptop", open.clone()).await;
    wait_list(addr, "two clients", |v| {
        v["clients"].as_array().unwrap().len() == 2
    })
    .await;

    // Opening alone takes no lock; A's first edit does.
    assert_eq!(get_json(addr, "/api/v1/presence").await["locks"], json!([]));
    let r = http(
        addr,
        "POST",
        &path,
        &[],
        Some(edit_body(Some("oriel-a"), &slide, "A one")),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let v = get_json(addr, "/api/v1/presence").await;
    assert_eq!(v["locks"][0]["clientId"], "oriel-a", "{v}");
    assert_eq!(v["locks"][0]["computer"], "Booth laptop");

    // B is refused (body clientId, or the X-Client-Id header), and so is a write without one.
    let r = http(
        addr,
        "POST",
        &path,
        &[],
        Some(edit_body(Some("oriel-b"), &slide, "B one")),
    )
    .await;
    assert_eq!(r.status, 423, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["error"]["code"], "locked");
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Booth laptop")
    );
    let r = http(
        addr,
        "POST",
        &path,
        &[("X-Client-Id", "oriel-b")],
        Some(edit_body(None, &slide, "B two")),
    )
    .await;
    assert_eq!(r.status, 423);
    let r = http(
        addr,
        "POST",
        &path,
        &[],
        Some(edit_body(None, &slide, "anon")),
    )
    .await;
    assert_eq!(r.status, 423);
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/presentations/{pid}"),
        &[("X-Client-Id", "oriel-b")],
        None,
    )
    .await;
    assert_eq!(r.status, 423, "deletes are locked too");
    // Reading and duplicating aren't.
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{pid}/duplicate"),
        &[("X-Client-Id", "oriel-b")],
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    // The refused writes changed nothing.
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert!(p.to_string().contains("A one") && !p.to_string().contains("B one"));

    // Taking it without takeOver: 423. With it: B holds it, A is told and refused.
    let lock = json!({"kind": "presentation", "id": pid});
    let r = http(
        addr,
        "POST",
        "/api/v1/presence/locks",
        &[("X-Client-Id", "oriel-b")],
        Some(lock.clone()),
    )
    .await;
    assert_eq!(r.status, 423);
    let r = http(
        addr,
        "POST",
        "/api/v1/presence/locks",
        &[],
        Some(lock.clone()),
    )
    .await;
    assert_eq!(r.status, 400, "the header is needed");
    let mut over = lock.clone();
    over["takeOver"] = json!(true);
    let r = http(
        addr,
        "POST",
        "/api/v1/presence/locks",
        &[("X-Client-Id", "oriel-b")],
        Some(over),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["clientId"], "oriel-b");
    wait_presence(&mut a, "B's lock", |p| {
        p["locks"][0]["clientId"] == "oriel-b"
    })
    .await;
    let r = http(
        addr,
        "POST",
        &path,
        &[],
        Some(edit_body(Some("oriel-a"), &slide, "A two")),
    )
    .await;
    assert_eq!(r.status, 423);
    let r = http(
        addr,
        "POST",
        &path,
        &[],
        Some(edit_body(Some("oriel-b"), &slide, "B three")),
    )
    .await;
    assert_eq!(r.status, 200);

    // A client that isn't connected can't hold a lock.
    let mut over = lock.clone();
    over["takeOver"] = json!(true);
    let r = http(
        addr,
        "POST",
        "/api/v1/presence/locks",
        &[("X-Client-Id", "nobody")],
        Some(over),
    )
    .await;
    assert_eq!(r.status, 409, "{}", String::from_utf8_lossy(&r.body));

    // B closes the document: the lock goes, and A's next edit takes it.
    report(&mut b, "oriel-b", "Lobby laptop", json!([])).await;
    wait_list(addr, "no lock", |v| v["locks"] == json!([])).await;
    let r = http(
        addr,
        "POST",
        &path,
        &[],
        Some(edit_body(Some("oriel-a"), &slide, "A three")),
    )
    .await;
    assert_eq!(r.status, 200);
    // A releases it.
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/presence/locks/presentation/{pid}"),
        &[("X-Client-Id", "oriel-a")],
        None,
    )
    .await;
    assert_eq!(r.status, 204);
    assert_eq!(get_json(addr, "/api/v1/presence").await["locks"], json!([]));
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_silent_holder_loses_its_lock_with_the_heartbeat() {
    let fx = fixture("presence-expiry");
    // Pings every 1.5 s: the holder is dropped after 4.5 s of silence, long after its edit
    // even on a loaded machine (M-3: 400 ms let it go before the lock was checked).
    let server = start_server_with(&fx, |c| c.ws_ping = Duration::from_millis(1500)).await;
    let addr = server.addr;
    let pid = fx.song.id.0.to_string();
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let slide = first_slide(&p);

    // Reports, edits (taking the lock), then never reads again: no pongs.
    let mut a = Client::connect(addr, "protocolVersion=1").await;
    report(
        &mut a,
        "oriel-a",
        "Booth laptop",
        json!([{"kind": "presentation", "id": pid, "place": "reflow"}]),
    )
    .await;
    wait_list(addr, "A", |v| v["clients"].as_array().unwrap().len() == 1).await;
    let path = format!("/api/v1/presentations/{pid}/edits");
    let r = http(
        addr,
        "POST",
        &path,
        &[],
        Some(edit_body(Some("oriel-a"), &slide, "A")),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        get_json(addr, "/api/v1/presence").await["locks"][0]["clientId"],
        "oriel-a"
    );

    // Dropped after three silent ping intervals: its presence and lock go.
    wait_list(addr, "A dropped", |v| {
        v["clients"] == json!([]) && v["locks"] == json!([])
    })
    .await;
    let r = http(
        addr,
        "POST",
        &path,
        &[("X-Client-Id", "oriel-b")],
        Some(edit_body(None, &slide, "B")),
    )
    .await;
    assert_eq!(r.status, 200);
    drop(a);
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// The places `show` and `timers` (capability `presenceBrowsing`): browsing the Show page and a
/// collection on the Library page is listed and takes no lock; a timer's settings and a prop
/// lock like the other documents.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn browsing_is_listed_and_timers_and_props_lock() {
    let fx = fixture("presence-browsing");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let pid = fx.song.id.0.to_string();
    let caps = get_json(addr, "/api/v1/server").await["capabilities"].clone();
    assert!(
        caps.as_array()
            .unwrap()
            .contains(&json!("presenceBrowsing"))
    );

    let r = http(
        addr,
        "POST",
        "/api/v1/timers",
        &[],
        Some(json!({"name": "Countdown", "kind": {"type": "countdown", "seconds": 300}})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let timer = r.json();
    let tid = timer["id"].as_str().unwrap().to_string();
    let r = http(
        addr,
        "POST",
        "/api/v1/props",
        &[],
        Some(json!({"name": "Logo bug"})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let prop = r.json();
    let prop_id = prop["id"].as_str().unwrap().to_string();
    let collection = "6f1c2c1e-0000-4000-8000-0000000000c0";

    let mut a = Client::connect(addr, "protocolVersion=1").await;
    let mut b = Client::connect(addr, "protocolVersion=1").await;
    report(
        &mut a,
        "oriel-a",
        "Booth laptop",
        json!([
            {"kind": "timer", "id": tid, "place": "timers"},
            {"kind": "prop", "id": prop_id, "place": "edit"},
            {"kind": "collection", "id": collection, "place": "library"},
        ]),
    )
    .await;
    report(
        &mut b,
        "oriel-b",
        "Lobby laptop",
        json!([
            {"kind": "playlistNode", "id": "6f1c2c1e-0000-4000-8000-0000000000d0", "place": "show"},
            {"kind": "presentation", "id": pid, "place": "show"},
            {"kind": "timer", "id": tid, "place": "show"},
            {"kind": "collection", "id": collection, "place": "library"},
            {"kind": "presentation", "id": pid, "place": "somewhereNew"},
        ]),
    )
    .await;
    let v = wait_list(addr, "two clients", |v| {
        v["clients"].as_array().unwrap().len() == 2
    })
    .await;
    let places: Vec<&str> = v["clients"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["open"].as_array().unwrap())
        .map(|o| o["place"].as_str().unwrap())
        .collect();
    assert!(
        places.contains(&"show") && places.contains(&"timers") && places.contains(&"other"),
        "{v}"
    );

    // The Show page's write takes no lock (B only browses there).
    let put = |who: &'static str, mut doc: Value, path: String| async move {
        let rev = doc["revision"].as_str().unwrap().to_string();
        doc.as_object_mut().unwrap().remove("revision");
        http(
            addr,
            "PUT",
            &path,
            &[("X-Client-Id", who), ("If-Match", &format!("\"{rev}\""))],
            Some(doc),
        )
        .await
    };
    let mut t = timer.clone();
    t["name"] = json!("Countdown (B)");
    let r = put("oriel-b", t, format!("/api/v1/timers/{tid}")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(get_json(addr, "/api/v1/presence").await["locks"], json!([]));
    let timer = r.json();

    // A edits the timer on the Timers page: it takes the lock, B is refused.
    let mut t = timer.clone();
    t["name"] = json!("Countdown (A)");
    let r = put("oriel-a", t, format!("/api/v1/timers/{tid}")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let timer = r.json();
    let v = get_json(addr, "/api/v1/presence").await;
    assert_eq!(v["locks"][0]["kind"], "timer", "{v}");
    assert_eq!(v["locks"][0]["clientId"], "oriel-a");
    let mut t = timer.clone();
    t["name"] = json!("Countdown (B again)");
    let r = put("oriel-b", t, format!("/api/v1/timers/{tid}")).await;
    assert_eq!(r.status, 423, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("this timer")
    );

    // A prop open in Edit: A's change takes its lock, B's delete is refused.
    let mut p = prop.clone();
    p["name"] = json!("Logo bug (A)");
    let r = put("oriel-a", p, format!("/api/v1/props/{prop_id}")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/props/{prop_id}"),
        &[("X-Client-Id", "oriel-b")],
        None,
    )
    .await;
    assert_eq!(r.status, 423, "{}", String::from_utf8_lossy(&r.body));

    // A leaves the Timers page and the prop: both locks go.
    report(
        &mut a,
        "oriel-a",
        "Booth laptop",
        json!([{"kind": "collection", "id": collection, "place": "library"}]),
    )
    .await;
    wait_list(addr, "the locks gone", |v| v["locks"] == json!([])).await;
    drop((a, b));
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}
