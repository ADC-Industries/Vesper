//! Presence and edit locks site-wide (`docs/PRESENCE-PLAN.md` stage 3): a show box reports its
//! clients to the Codex box and shows the site's merged list; the Codex holds every lock, so a
//! lock taken through box A refuses a write through box B (`423`), and take-over works across
//! boxes. Two servers in this process (A hosts the Codex, B is its show box), synthetic
//! fixtures.

use super::showbox::wait_client;
use crate::common::*;
use crate::site::keys::two_boxes_with;
use crate::site::{call, ok};
use futures_util::SinkExt;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;
use vesper_server::codex::CodexOptions;

async fn report(c: &mut Client, client_id: &str, computer: &str, open: Value) {
    let m = json!({"type": "presence", "clientId": client_id, "computer": computer, "open": open});
    c.ws.send(Message::Text(m.to_string().into()))
        .await
        .unwrap();
}

async fn presence(addr: SocketAddr, key: &str) -> Value {
    ok(
        &call(addr, "GET", "/api/v1/presence", Some(key), None).await,
        200,
        "GET /presence",
    )
}

/// Waits until `GET /presence` on `addr` passes `pred` (15 s).
async fn until_presence(
    addr: SocketAddr,
    key: &str,
    what: &str,
    pred: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let v = presence(addr, key).await;
        if pred(&v) {
            return v;
        }
        assert!(Instant::now() < deadline, "{what}: {v}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn client<'a>(v: &'a Value, id: &str) -> Option<&'a Value> {
    v["clients"]
        .as_array()?
        .iter()
        .find(|c| c["clientId"] == id)
}

async fn edit(
    addr: SocketAddr,
    key: &str,
    pid: &str,
    slide: &str,
    client: &str,
    text: &str,
) -> Resp {
    call(
        addr,
        "POST",
        &format!("/api/v1/presentations/{pid}/edits"),
        Some(key),
        Some(json!({"ops": [{"type": "setSlideText", "slide": slide, "text": text}], "clientId": client})),
    )
    .await
}

async fn take_over(addr: SocketAddr, key: &str, pid: &str, client: &str) -> Resp {
    let bearer = format!("Bearer {key}");
    let body = json!({"kind": "presentation", "id": pid, "takeOver": true}).to_string();
    http_bytes(
        addr,
        "POST",
        "/api/v1/presence/locks",
        &[
            ("Authorization", &bearer),
            ("X-Client-Id", client),
            ("Content-Type", "application/json"),
        ],
        body.as_bytes(),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn locks_are_site_wide_across_a_show_box_and_the_codex_box() {
    let t = two_boxes_with(
        "cx-presence",
        CodexOptions::default(),
        CodexOptions {
            mirror_poll: Duration::from_secs(2),
            ..CodexOptions::default()
        },
        |_| {},
    )
    .await;
    let (a, b, key) = (t.a.addr, t.b.addr, t.key_a.clone());
    wait_client(&t).await;
    let a_name = t.a.shared.auth.identity().name;
    let pid = t.fa.song.id.0.to_string();
    let p = ok(
        &call(
            a,
            "GET",
            &format!("/api/v1/presentations/{pid}"),
            Some(&key),
            None,
        )
        .await,
        200,
        "the song",
    );
    let slide = p["sections"][0]["slides"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let open = json!([{"kind": "presentation", "id": pid, "place": "edit"}]);

    // One client on each box, both with the song open in Edit.
    let mut on_a = Client::connect(a, &format!("protocolVersion=1&token={key}")).await;
    let mut on_b = Client::connect(b, &format!("protocolVersion=1&token={key}")).await;
    report(&mut on_a, "alpha", "Hall PC", open.clone()).await;
    report(&mut on_b, "bravo", "Booth laptop", open.clone()).await;

    // Each box lists both, the other box's client naming its box.
    let seen_a = until_presence(a, &key, "A lists B's client", |v| {
        client(v, "bravo").is_some()
    })
    .await;
    assert_eq!(
        client(&seen_a, "bravo").unwrap()["server"],
        "BOX-B",
        "{seen_a}"
    );
    assert!(
        client(&seen_a, "alpha").unwrap().get("server").is_none(),
        "{seen_a}"
    );
    let seen_b = until_presence(b, &key, "B lists A's client", |v| {
        client(v, "alpha").is_some() && client(v, "bravo").is_some()
    })
    .await;
    assert_eq!(
        client(&seen_b, "alpha").unwrap()["server"],
        a_name.as_str(),
        "{seen_b}"
    );
    assert!(
        client(&seen_b, "bravo").unwrap().get("server").is_none(),
        "{seen_b}"
    );

    // Alpha's first edit (on A) takes the lock: Bravo's write through B is refused.
    ok(
        &edit(a, &key, &pid, &slide, "alpha", "Alpha was here").await,
        200,
        "alpha's edit",
    );
    let r = edit(b, &key, &pid, &slide, "bravo", "Bravo too").await;
    assert_eq!(r.status, 423, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["error"]["code"], "locked");
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Hall PC"),
        "{}",
        r.json()
    );
    let seen_b = until_presence(b, &key, "B shows alpha's lock", |v| {
        v["locks"]
            .as_array()
            .is_some_and(|l| l.len() == 1 && l[0]["clientId"] == "alpha")
    })
    .await;
    assert_eq!(seen_b["locks"][0]["server"], a_name.as_str(), "{seen_b}");

    // Bravo takes over on B: the Codex moves the lock; alpha is refused on A now.
    let r = take_over(b, &key, &pid, "bravo").await;
    let l = ok(&r, 200, "take over through B");
    assert_eq!(l["clientId"], "bravo", "{l}");
    let r = edit(a, &key, &pid, &slide, "alpha", "Alpha again").await;
    assert_eq!(r.status, 423, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Booth laptop"),
        "{}",
        r.json()
    );
    ok(
        &edit(b, &key, &pid, &slide, "bravo", "Bravo's turn").await,
        200,
        "bravo's edit",
    );
    let seen_a = until_presence(a, &key, "A shows bravo's lock", |v| {
        v["locks"]
            .as_array()
            .is_some_and(|l| l.len() == 1 && l[0]["clientId"] == "bravo")
    })
    .await;
    assert_eq!(seen_a["locks"][0]["server"], "BOX-B", "{seen_a}");

    // And back, through A.
    let r = take_over(a, &key, &pid, "alpha").await;
    assert_eq!(ok(&r, 200, "take back on A")["clientId"], "alpha");
    let r = edit(b, &key, &pid, &slide, "bravo", "Bravo once more").await;
    assert_eq!(r.status, 423, "{}", String::from_utf8_lossy(&r.body));

    // E05-1: places across versions. Alpha browses the song on the Show page and has a
    // timer's settings open. B (this release) sees both places.
    let timer = uuid::Uuid::new_v4().to_string();
    report(
        &mut on_a,
        "alpha",
        "Hall PC",
        json!([{"kind": "presentation", "id": pid, "place": "show"},
               {"kind": "timer", "id": timer, "place": "timers"}]),
    )
    .await;
    let places = |v: &Value, id: &str| -> Vec<String> {
        client(v, id).map_or(Vec::new(), |c| {
            c["open"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d["place"].as_str().unwrap().to_string())
                .collect()
        })
    };
    until_presence(b, &key, "B sees alpha's show and timers places", |v| {
        places(v, "alpha") == ["show", "timers"]
    })
    .await;
    // An older show box (its report says nothing of the places it reads) gets alpha cut down
    // to what it reads: the timer as `library`, the Show page left out.
    let (ch, codex) = t.b.shared.mirror.channel().unwrap();
    let body = json!({"name": "OLD-BOX", "clients": [], "stamped": true}).to_string();
    let reply = tokio::task::spawn_blocking(move || {
        ch.request(
            &codex,
            vesper_server::codex::site_presence::PRESENCE,
            body.as_bytes(),
            Duration::from_secs(5),
        )
    })
    .await
    .unwrap()
    .unwrap();
    let idn_open_protocol::channel::Reply::Ok(bytes) = reply else {
        panic!("{reply:?}")
    };
    let answer: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(answer["allPlaces"], true, "{answer}");
    assert_eq!(
        places(&answer["presence"], "alpha"),
        ["library"],
        "{answer}"
    );

    // Bravo's socket closes: its box reports it, and it leaves A's list.
    on_b.ws.close(None).await.unwrap();
    until_presence(a, &key, "bravo gone from A", |v| {
        client(v, "bravo").is_none()
    })
    .await;
    drop(on_a);
    t.end().await;
}
