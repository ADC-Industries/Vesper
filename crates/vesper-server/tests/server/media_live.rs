//! Live media following show changes: a replaced video stays in the player registry until the
//! state without it is published, a collection switch or a data reload stops the media it
//! drops, and an audio playlist skips a track that cannot be opened. Synthetic media only
//! (system `ffmpeg` CLI).

use crate::audio::{add_asset, frequency, record, tone_file, video};
use crate::common::*;
use model::{
    MediaKind, MediaLayer, MediaTrigger, Playback, PlaylistItem, PlaylistItemKind, PlaylistNode,
    PlaylistNodeKind, PlaylistPurpose,
};
use serde_json::json;
use std::time::{Duration, Instant};
use vesper_server::RunningServer;

fn audios(server: &RunningServer) -> usize {
    server.shared.audios.lock().unwrap().len()
}

/// Waits (up to 5 s) for `pred`.
async fn until(what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pred() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn silent(x: &[f32]) -> bool {
    x[x.len() / 2..].iter().all(|v| v.abs() < 1e-3)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replaced_video_stays_registered_until_the_new_state_is_published() {
    let fx = fixture("media-retire");
    let first = video(&fx, "first", 6.0, &[1.0], Playback::default());
    let second = video(&fx, "second", 6.0, &[1.0], Playback::default());
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;
    a.ok(json!({"type": "triggerMedia", "asset": first.id.0.to_string()}))
        .await;
    let old = server.shared.state().media.and_then(|m| m.play).unwrap();
    assert!(server.shared.video(old).is_some());

    // While the second video opens, the render thread still draws the published state: its
    // play must stay in the registry (it used to be dropped first: a cut to black).
    let body = json!({"type": "triggerMedia", "asset": second.id.0.to_string()});
    let trigger =
        tokio::spawn(async move { http(addr, "POST", "/api/v1/commands", &[], Some(body)).await });
    let mut samples = 0;
    while !trigger.is_finished() {
        let s1 = server.shared.state().media.and_then(|m| m.play);
        let registered = s1.is_none_or(|p| server.shared.video(p).is_some());
        let s2 = server.shared.state().media.and_then(|m| m.play);
        if s1 == s2 {
            assert!(
                registered,
                "the published state's video {s1:?} left the registry"
            );
        }
        samples += 1;
        std::thread::sleep(Duration::from_micros(200));
    }
    let r = trigger.await.unwrap();
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert!(samples > 0);
    let new = server.shared.state().media.and_then(|m| m.play).unwrap();
    assert_ne!(new, old);
    assert!(server.shared.video(new).is_some());
    assert!(
        server.shared.video(old).is_none(),
        "the replaced video goes once the new state is out"
    );
    assert_eq!(server.shared.videos.lock().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_collection_switch_stops_the_audio_it_drops() {
    let fx = fixture("media-switch");
    tone_file(&fx.dir.join("media").join("walkin.wav"), 440.0, 20.0);
    let tone = add_asset(&fx, "walkin", "wav", MediaKind::Audio);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;
    a.ok(json!({"type": "triggerMedia", "asset": tone.id.0.to_string()}))
        .await;
    assert_eq!(a.state["layers"]["audio"]["name"], "walkin");
    assert_eq!(audios(&server), 1);

    let r = http(
        addr,
        "POST",
        "/api/v1/codex/collections",
        &[],
        Some(json!({"name": "Chapel"})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    let chapel = r.json()["collection"]["id"].as_str().unwrap().to_string();
    let r = http(
        addr,
        "PUT",
        "/api/v1/codex/assignment",
        &[],
        Some(json!({"collection": chapel, "confirm": true})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    a.wait_state("audio layer cleared", |_, s| s["layers"]["audio"].is_null())
        .await;
    until("the audio player is dropped", || audios(&server) == 0).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        silent(&record(&server, Duration::from_millis(300))),
        "the switched-away audio still plays"
    );
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reload_stops_the_audio_whose_asset_is_gone() {
    let fx = fixture("media-reload");
    tone_file(&fx.dir.join("media").join("walkin.wav"), 440.0, 20.0);
    let tone = add_asset(&fx, "walkin", "wav", MediaKind::Audio);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;
    a.ok(json!({"type": "triggerMedia", "asset": tone.id.0.to_string()}))
        .await;
    assert_eq!(audios(&server), 1);
    let doc = content_dir(&fx.dir).join(format!("portable/mediaAsset/{}.json", tone.id.0));
    std::fs::remove_file(&doc).unwrap();
    a.ok(json!({"type": "reloadData"})).await;
    assert!(a.state["layers"]["audio"].is_null(), "{}", a.state);
    until("the audio player is dropped", || audios(&server) == 0).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        silent(&record(&server, Duration::from_millis(300))),
        "the reloaded-away audio still plays"
    );
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_audio_playlist_skips_a_track_that_cannot_be_opened() {
    let fx = fixture("media-skip");
    tone_file(&fx.dir.join("media").join("low.wav"), 440.0, 0.6);
    tone_file(&fx.dir.join("media").join("high.wav"), 880.0, 3.0);
    let low = add_asset(&fx, "low", "wav", MediaKind::Audio);
    // Its file is missing.
    let gone = add_asset(&fx, "gone", "wav", MediaKind::Audio);
    let high = add_asset(&fx, "high", "wav", MediaKind::Audio);
    let items: Vec<PlaylistItem> = [&low, &gone, &high]
        .iter()
        .map(|a| {
            PlaylistItem::new(PlaylistItemKind::Media(MediaTrigger::new(
                a.id,
                MediaLayer::Audio,
            )))
        })
        .collect();
    let playlist = PlaylistNode {
        name: "Walk-in".into(),
        kind: PlaylistNodeKind::Playlist {
            purpose: PlaylistPurpose::Audio,
            items: items.clone(),
        },
        ..PlaylistNode::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "playlistNode",
        &playlist.id.0.to_string(),
        &playlist,
    );
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    let item = |i: usize| items[i].id.0.to_string();
    a.ok(json!({"type": "triggerPlaylistItem", "playlist": playlist.id.0.to_string(), "item": item(0)}))
        .await;
    assert_eq!(a.state["layers"]["audio"]["name"], "low");
    a.wait_state("the playlist skips the missing track", |_, s| {
        s["audio"]["playlist"]["item"] == item(2)
    })
    .await;
    assert_eq!(a.state["layers"]["audio"]["name"], "high");
    assert_eq!(a.state["layers"]["audio"]["playback"]["state"], "playing");
    let x = record(&server, Duration::from_millis(400));
    let f = frequency(&x[x.len() / 2..], 2);
    assert!((f - 880.0).abs() < 2.0, "{f} Hz");
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn embedded_audio_follows_a_change_of_the_screen_frame_rate() {
    // R4-5: the audio delay (videoDelay frames) used the frame period from when the output
    // was built; the output stays when only the screen's rate changes.
    let fx = fixture("media-rate");
    let id = model::OutputId::new();
    let binding = json!({
        "schemaVersion": 2,
        "id": id,
        "name": "Program",
        "screen": fx.audience.id,
        "transport": {"type": "idn", "sourceName": format!("rate-test-{}", std::process::id())},
        "embedAudio": true,
        "videoDelay": 10
    });
    write_doc(
        &fx.dir,
        "machine",
        "outputBinding",
        &id.0.to_string(),
        &binding,
    );
    let server = start_server(&fx, true, false).await;
    let rt = server.shared.screen(fx.audience.id).unwrap();
    let out = rt.outputs()[0].clone();
    assert!(out.active, "{:?}", out.detail);
    assert!(out.audio.is_some(), "embeds audio");
    let at = |fps: f64| Duration::from_secs_f64(1.0 / fps);
    assert!(
        out.frame_period()
            .abs_diff(at(fx.audience.frame_rate.as_f64()))
            < Duration::from_micros(1)
    );

    let mut screen = serde_json::to_value(&fx.audience).unwrap();
    screen["frameRate"] = json!({"num": 60, "den": 1});
    let r = http(
        server.addr,
        "PUT",
        &format!("/api/v1/screens/{}", fx.audience.id.0),
        &[],
        Some(screen),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let rt = server.shared.screen(fx.audience.id).unwrap();
    let now = rt.outputs()[0].clone();
    assert!(std::sync::Arc::ptr_eq(&now, &out), "the output is kept");
    assert!(now.frame_period().abs_diff(at(60.0)) < Duration::from_micros(1));
    let _ = std::fs::remove_dir_all(&fx.dir);
}
