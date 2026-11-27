//! CCLI usage reports: a song shown from a playlist is one use however many slides; the same
//! song outside the playlist is another; a presentation without song metadata isn't logged;
//! the CSV has CCLI's columns. Synthetic songs.

use crate::common::*;
use model::SongInfo;
use serde_json::json;

async fn command(addr: std::net::SocketAddr, c: serde_json::Value) {
    let r = http(addr, "POST", "/api/v1/commands", &[], Some(c.clone())).await;
    assert_eq!(r.status, 200, "{c}: {}", String::from_utf8_lossy(&r.body));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn songs_shown_live_are_counted_once_per_service() {
    let fx = fixture("ccli-report");
    let mut song = fx.song.clone();
    song.song_info = Some(SongInfo {
        title: Some("Morning Light".into()),
        ccli_number: Some("7654321".into()),
        authors: Some("A. Writer, B. Composer".into()),
        ..SongInfo::default()
    });
    write_doc(
        &fx.dir,
        "portable",
        "presentation",
        &song.id.0.to_string(),
        &song,
    );
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let (playlist, items) = (fx.playlist.id.0.to_string(), fx.items());
    let item = |i: usize| items[i].id.0.to_string();
    // From the Sunday playlist: the song, three slides on, the second song (no metadata).
    command(
        addr,
        json!({"type": "triggerPlaylistItem", "playlist": playlist, "item": item(1)}),
    )
    .await;
    for _ in 0..3 {
        command(addr, json!({"type": "next"})).await;
    }
    command(
        addr,
        json!({"type": "triggerPlaylistItem", "playlist": playlist, "item": item(3)}),
    )
    .await;
    // Back to the song in the same service: the same use. Then outside the playlist.
    command(
        addr,
        json!({"type": "triggerPlaylistItem", "playlist": playlist, "item": item(1)}),
    )
    .await;
    command(addr, json!({"type": "clearAll"})).await;
    command(
        addr,
        json!({"type": "triggerSlide", "presentation": song.id.0.to_string(), "index": 0}),
    )
    .await;

    let today = chrono::Local::now().date_naive().to_string();
    let r = get_json(
        addr,
        &format!("/api/v1/reports/ccli?from={today}&to={today}"),
    )
    .await;
    let rows = r["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{r}");
    assert_eq!(rows[0]["title"], "Morning Light");
    assert_eq!(rows[0]["ccliNumber"], "7654321");
    assert_eq!(rows[0]["count"], 2, "{r}");
    assert_eq!(rows[0]["projected"], 2);
    assert_eq!(rows[0]["streamed"], 0);
    let uses = r["uses"].as_array().unwrap();
    assert_eq!(uses[0]["service"], "Sunday", "{r}");
    assert!(uses[1]["service"].is_null(), "{r}");
    // The default range ends today.
    let d = get_json(addr, "/api/v1/reports/ccli").await;
    assert_eq!(d["to"], today.as_str());
    assert_eq!(d["rows"][0]["count"], 2);

    let csv = http(addr, "GET", "/api/v1/reports/ccli.csv", &[], None).await;
    assert_eq!(csv.status, 200);
    assert!(csv.header("content-type").unwrap().starts_with("text/csv"));
    assert!(
        csv.header("content-disposition")
            .unwrap()
            .contains("ccli-usage-")
    );
    let text = String::from_utf8(csv.body.clone()).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        "Song Title,CCLI Song #,Authors,Times Used,Projected,Streamed,Recorded,From,To"
    );
    assert!(
        lines
            .next()
            .unwrap()
            .starts_with("Morning Light,7654321,\"A. Writer, B. Composer\",2,2,0,0,"),
        "{text}"
    );
    let bad = http(
        addr,
        "GET",
        "/api/v1/reports/ccli?from=2026-12-01&to=2026-11-01",
        &[],
        None,
    )
    .await;
    assert_eq!(bad.status, 400);
    let bad = http(
        addr,
        "GET",
        "/api/v1/reports/ccli?from=yesterday",
        &[],
        None,
    )
    .await;
    assert_eq!(bad.status, 400);
    assert!(fx.dir.join("reports/ccli-usage.jsonl").is_file());
    let _ = std::fs::remove_dir_all(&fx.dir);
}
