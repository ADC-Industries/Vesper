//! Signage on the site's server (`docs/SIGNAGE-PLAN.md`): a channel gets its own screen and IDN
//! output named after it, renders its playlist's items (an announcement, an image) and switches
//! on its schedule, skips an item it can't show, shows and clears an override (audited), plays a
//! feed slide from a CSV file on the server, and comes back after a restart.

use crate::common::*;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vesper_server::site::SiteConfig;
use vesper_server::{RunningServer, ServerConfig, start};

/// An announcement's card colour (`signage::compose`'s `CARD`, sRGB).
const CARD: [u8; 3] = [26, 41, 71];
/// The urgent override's red.
const URGENT: [u8; 3] = [179, 18, 18];

async fn call(addr: SocketAddr, method: &str, path: &str, body: Option<Value>) -> Resp {
    http(addr, method, path, &[], body).await
}

fn ok(r: &Resp, status: u16, what: &str) -> Value {
    assert_eq!(
        r.status,
        status,
        "{what}: {}",
        String::from_utf8_lossy(&r.body)
    );
    if r.body.is_empty() {
        Value::Null
    } else {
        r.json()
    }
}

async fn until_signage(addr: SocketAddr, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let s = get_json(addr, "/api/v1/signage").await;
        if pred(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "{what}: {s:#}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn boot(fx: &Fixture) -> RunningServer {
    let server = start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: true,
        mdns: false,
        site: SiteConfig::loopback(),
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts");
    until_signage(server.addr, "managed", |s| s["managed"] == true).await;
    server
}

/// The index of the channel's screen among the running screens.
async fn screen_index(server: &RunningServer, screen: &str) -> usize {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(i) = server
            .shared
            .screens()
            .iter()
            .position(|r| r.id.0.to_string() == screen)
        {
            return i;
        }
        assert!(Instant::now() < deadline, "the channel's screen never ran");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Most of the frame (sampled) is `c`.
fn mostly(img: &image::RgbaImage, c: [u8; 3]) -> bool {
    let (w, h) = img.dimensions();
    let mut hit = 0;
    let mut all = 0;
    for y in (0..h).step_by(37) {
        for x in (0..w).step_by(41) {
            all += 1;
            if near(img.get_pixel(x, y), c, 8) {
                hit += 1;
            }
        }
    }
    hit * 2 > all
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_channel_renders_its_playlist_switches_on_schedule_and_takes_an_override() {
    let fx = fixture("signage");
    let server = boot(&fx).await;
    let addr = server.addr;
    let info = get_json(addr, "/api/v1/server").await;
    assert!(
        info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("signage"))
    );

    // Two playlists: an announcement, and an image after an item that can't be shown.
    let welcome = ok(
        &call(
            addr,
            "POST",
            "/api/v1/signage/playlists",
            Some(json!({"name": "Welcome", "items": [
                {"content": {"kind": "text", "title": "Welcome", "body": "Coffee in the hall"},
                 "duration": 30, "transition": {"kind": {"type": "cut"}}}
            ]})),
        )
        .await,
        201,
        "playlist",
    );
    let missing = uuid::Uuid::new_v4().to_string();
    let pictures = ok(
        &call(
            addr,
            "POST",
            "/api/v1/signage/playlists",
            Some(json!({"name": "Pictures", "items": [
                {"content": {"kind": "image", "asset": missing}, "duration": 2},
                {"content": {"kind": "image", "asset": fx.asset.id.0.to_string()},
                 "duration": 30, "transition": {"kind": {"type": "cut"}}}
            ]})),
        )
        .await,
        201,
        "playlist",
    );
    assert_eq!(pictures["items"].as_array().unwrap().len(), 2);
    assert!(!pictures["items"][0]["id"].as_str().unwrap().is_empty());

    // A channel: its screen and IDN output named after it.
    let ch = ok(
        &call(
            addr,
            "POST",
            "/api/v1/signage/channels",
            Some(json!({"name": "Lobby", "defaultPlaylist": welcome["id"]})),
        )
        .await,
        201,
        "channel",
    );
    let id = ch["id"].as_str().unwrap().to_string();
    let screen = ch["screen"].as_str().unwrap().to_string();
    assert!(
        !screen.is_empty(),
        "{ch} {}",
        get_json(addr, "/api/v1/signage").await
    );
    assert!(ch["source"].as_str().unwrap().ends_with("(Lobby)"), "{ch}");
    let screens = get_json(addr, "/api/v1/screens").await;
    let s = screens
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == screen.as_str())
        .unwrap_or_else(|| panic!("{screens:#}"));
    assert_eq!(s["name"], "Signage: Lobby");
    assert_eq!(s["kind"], json!({"type": "signage", "channel": id}));
    let outputs = get_json(addr, "/api/v1/outputs").await;
    assert!(
        outputs
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["screen"] == screen.as_str()
                && o["transport"] == json!({"type": "idn", "sourceName": "Lobby"})),
        "{outputs:#}"
    );
    let dup = call(
        addr,
        "POST",
        "/api/v1/signage/channels",
        Some(json!({"name": "lobby"})),
    )
    .await;
    assert_eq!(dup.status, 409);

    // The default playlist's announcement shows.
    let i = screen_index(&server, &screen).await;
    wait_frame(&server, i, "the announcement", |f| mostly(f, CARD)).await;
    let st = until_signage(addr, "now playing", |s| {
        s["channels"][0]["now"]["item"] == "Welcome"
    })
    .await;
    assert_eq!(st["channels"][0]["now"]["health"], "ok");
    assert!(st["channels"][0]["now"].get("slot").is_none());

    // A slot all day, every day: its playlist plays; the missing image is skipped.
    let ch = ok(
        &call(
            addr,
            "PUT",
            &format!("/api/v1/signage/channels/{id}"),
            Some(json!({"slots": [
                {"playlist": pictures["id"], "start": 0, "end": 1440, "priority": 1}
            ]})),
        )
        .await,
        200,
        "slots",
    );
    let slot = ch["slots"][0]["id"].as_str().unwrap().to_string();
    wait_frame(&server, i, "the scheduled image", |f| mostly(f, BLUE)).await;
    let st = until_signage(addr, "the slot plays", |s| {
        s["channels"][0]["now"]["slot"] == slot.as_str()
    })
    .await;
    let now = &st["channels"][0]["now"];
    assert_eq!(now["playlist"], pictures["id"]);
    assert_eq!(now["health"], "problem", "{now}");
    assert!(
        now["problem"]
            .as_str()
            .unwrap()
            .contains("not in the library"),
        "{now}"
    );
    // A playlist a channel plays can't be deleted.
    let r = call(
        addr,
        "DELETE",
        &format!(
            "/api/v1/signage/playlists/{}",
            pictures["id"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(r.status, 409);

    // The override: over everything at once, then cleared; audited.
    let o = ok(
        &call(
            addr,
            "POST",
            "/api/v1/signage/override",
            Some(json!({"text": "Please leave by the north doors", "style": "urgent"})),
        )
        .await,
        200,
        "override",
    );
    assert!(o["since"].as_u64().is_some());
    wait_frame(&server, i, "the override", |f| mostly(f, URGENT)).await;
    until_signage(addr, "overridden", |s| {
        s["channels"][0]["now"]["overridden"] == true && s["override"]["text"].is_string()
    })
    .await;
    ok(
        &call(addr, "DELETE", "/api/v1/signage/override", None).await,
        204,
        "clear",
    );
    wait_frame(&server, i, "the image again", |f| mostly(f, BLUE)).await;
    let audit = get_json(addr, "/api/v1/site/audit?n=100").await;
    let actions: Vec<&str> = audit
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();
    assert!(actions.contains(&"signage.override"), "{actions:?}");
    assert!(actions.contains(&"signage.override.clear"), "{actions:?}");
    let bad = call(
        addr,
        "POST",
        "/api/v1/signage/override",
        Some(json!({"text": "x", "channels": ["nope"]})),
    )
    .await;
    assert_eq!(bad.status, 404);

    // An override that clears itself.
    ok(
        &call(
            addr,
            "POST",
            "/api/v1/signage/override",
            Some(json!({"text": "Short", "channels": [id], "seconds": 1})),
        )
        .await,
        200,
        "timed override",
    );
    until_signage(addr, "the timed override ends", |s| {
        s.get("override").is_none()
    })
    .await;

    // Deleting the channel removes its screen and output.
    ok(
        &call(
            addr,
            "DELETE",
            &format!("/api/v1/signage/channels/{id}"),
            None,
        )
        .await,
        204,
        "delete",
    );
    let screens = get_json(addr, "/api/v1/screens").await;
    assert!(
        !screens
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == screen.as_str()),
        "{screens:#}"
    );
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_feed_slide_plays_and_a_restart_restores_the_channel() {
    let fx = fixture("signage-feed");
    let server = boot(&fx).await;
    let addr = server.addr;
    // Feed files live in the data directory's signage feeds folder (H02-5).
    let csv = fx.dir.join("signage-feeds").join("events.csv");
    std::fs::write(
        &csv,
        "When,Event\nSunday 9:00,Breakfast\nFriday 18:00,Youth night\n",
    )
    .unwrap();

    let preview = ok(
        &call(
            addr,
            "POST",
            "/api/v1/signage/feeds/preview",
            Some(json!({"name": "Events", "url": "events.csv"})),
        )
        .await,
        200,
        "preview",
    );
    assert_eq!(preview["columns"], json!(["When", "Event"]));
    assert_eq!(preview["rows"][1]["Event"], "Youth night");
    let bad = call(
        addr,
        "POST",
        "/api/v1/signage/feeds/preview",
        Some(json!({"name": "Gone", "url": "gone.csv"})),
    )
    .await;
    assert_eq!(bad.status, 400);
    // No file outside the folder (the server's own settings, say), no address of the server.
    std::fs::create_dir_all(fx.dir.join("machine")).unwrap();
    std::fs::write(fx.dir.join("machine").join("private.csv"), "Secret\nx\n").unwrap();
    for url in [
        fx.dir
            .join("machine")
            .join("private.csv")
            .display()
            .to_string(),
        "../machine/private.csv".to_string(),
        format!("http://{addr}/api/v1/signage"),
    ] {
        let r = call(
            addr,
            "POST",
            "/api/v1/signage/feeds/preview",
            Some(json!({"name": "No", "url": url})),
        )
        .await;
        assert_eq!(r.status, 400, "{url}");
        let r = call(
            addr,
            "POST",
            "/api/v1/signage/feeds",
            Some(json!({"name": "No", "url": url})),
        )
        .await;
        assert_eq!(r.status, 400, "{url}");
    }

    let feed = ok(
        &call(
            addr,
            "POST",
            "/api/v1/signage/feeds",
            Some(json!({"name": "Events", "url": csv.display().to_string(),
                        "mapping": {"title": "Event", "when": "When"}})),
        )
        .await,
        201,
        "feed",
    );
    let fid = feed["id"].as_str().unwrap().to_string();
    let p = ok(
        &call(
            addr,
            "POST",
            "/api/v1/signage/playlists",
            Some(json!({"name": "Events", "items": [
                {"content": {"kind": "feed", "feed": fid, "rows": 3}, "duration": 60}
            ]})),
        )
        .await,
        201,
        "playlist",
    );
    let ch = ok(
        &call(
            addr,
            "POST",
            "/api/v1/signage/channels",
            Some(json!({"name": "Hallway", "defaultPlaylist": p["id"],
                        "layout": "mainSidebarTicker", "ticker": "Welcome home"})),
        )
        .await,
        201,
        "channel",
    );
    let screen = ch["screen"].as_str().unwrap().to_string();
    let st = until_signage(addr, "the feed is read", |s| {
        s["feeds"][0]["rows"]
            .as_array()
            .is_some_and(|r| r.len() == 2)
    })
    .await;
    assert_eq!(st["feeds"][0]["columns"], json!(["When", "Event"]));
    until_signage(addr, "the feed slide plays", |s| {
        s["channels"][0]["now"]["item"] == "Feed: Events"
    })
    .await;
    let i = screen_index(&server, &screen).await;
    // The main zone shows the feed's card, the sidebar its panel.
    let frame = wait_frame(&server, i, "the feed slide", |f| {
        near(f.get_pixel(30, 500), CARD, 8) && near(f.get_pixel(1800, 900), [18, 20, 28], 8)
    })
    .await;
    // Kept for a look: the zones and the feed slide as the renderer drew them.
    frame
        .save(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("signage-feed-frame.png"))
        .ok();
    let r = call(
        addr,
        "DELETE",
        &format!("/api/v1/signage/feeds/{fid}"),
        None,
    )
    .await;
    assert_eq!(r.status, 409, "a feed in use stays");
    // Kept on disk, rows too.
    assert!(fx.dir.join("machine/signage.json").is_file());
    assert!(fx.dir.join("machine/signage-cache.json").is_file());
    server.shutdown().await.unwrap();

    // After a restart: the same channel, screen and output, playing again.
    let server = boot(&fx).await;
    let addr = server.addr;
    let st = until_signage(addr, "playing after the restart", |s| {
        s["channels"][0]["now"]["item"] == "Feed: Events"
    })
    .await;
    assert_eq!(st["channels"][0]["screen"], screen.as_str());
    let screens = get_json(addr, "/api/v1/screens").await;
    assert_eq!(
        screens
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["kind"]["type"] == "signage")
            .count(),
        1,
        "{screens:#}"
    );
    let i = screen_index(&server, &screen).await;
    wait_frame(&server, i, "the feed slide again", |f| {
        near(f.get_pixel(30, 500), CARD, 8)
    })
    .await;
    server.shutdown().await.unwrap();
}
