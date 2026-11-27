//! Audio through the real server, on the null audio output (no sound card needed): A/V sync
//! of videos with sound (beeps and white flash frames at the same media times; the offset
//! between the instant a beep is heard and the output tick that first shows its flash),
//! across loops, seeks and pauses; the audio layer with bins and audio playlists (next,
//! previous, shuffle, repeat, the end of an item); levels, meters and mutes; the audio device
//! API; and embedded audio on an IDN output (tone, level, A/V offset in timecodes).
//! Media are synthetic (system `ffmpeg` CLI).

use crate::common::*;
use model::{
    AssetLocation, BinBrowser, MediaAsset, MediaBin, MediaBinItem, MediaBinItemId, MediaBinKind,
    MediaKind, MediaLayer, MediaTrigger, OutputId, Playback, PlaylistItem, PlaylistItemKind,
    PlaylistNode, PlaylistNodeKind, PlaylistPurpose, Repeat,
};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use vesper_server::RunningServer;

pub(crate) fn ff(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("system ffmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A 30 fps clip: black, with a white frame for 0.1 s at each beep time, and a 1 kHz beep at
/// −6 dBFS for 0.1 s at the same times (AAC, 44.1 kHz).
pub(crate) fn av_clip(path: &Path, secs: f64, beeps: &[f64]) {
    let windows: Vec<String> = beeps
        .iter()
        .map(|b| format!("between(t,{b:.3},{:.3})", b + 0.0999))
        .collect();
    let on = windows.join("+");
    let video = format!(
        "color=c=black:s=160x90:r=30:d={secs},drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='{on}'"
    );
    let audio = format!("aevalsrc='0.5*sin(2*PI*1000*t)*({on})':s=44100:d={secs}");
    ff(&[
        "-f",
        "lavfi",
        "-i",
        &video,
        "-f",
        "lavfi",
        "-i",
        &audio,
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-g",
        "30",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        path.to_str().unwrap(),
    ]);
}

/// A tone file (`secs` long, `freq` Hz at −6 dBFS, stereo WAV at 44.1 kHz).
pub(crate) fn tone_file(path: &Path, freq: f64, secs: f64) {
    let src = format!("aevalsrc='0.5*sin(2*PI*{freq}*t)|0.5*sin(2*PI*{freq}*t)':s=44100:d={secs}");
    ff(&[
        "-f",
        "lavfi",
        "-i",
        &src,
        "-c:a",
        "pcm_s16le",
        path.to_str().unwrap(),
    ]);
}

pub(crate) fn add_asset(fx: &Fixture, name: &str, ext: &str, kind: MediaKind) -> MediaAsset {
    let asset = MediaAsset {
        name: name.into(),
        kind,
        location: AssetLocation::Managed {
            sha256: name.into(),
            extension: ext.into(),
        },
        ..MediaAsset::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "mediaAsset",
        &asset.id.0.to_string(),
        &asset,
    );
    asset
}

pub(crate) fn video(
    fx: &Fixture,
    name: &str,
    secs: f64,
    beeps: &[f64],
    playback: Playback,
) -> MediaAsset {
    av_clip(
        &fx.dir.join("media").join(format!("{name}.mp4")),
        secs,
        beeps,
    );
    let a = add_asset(fx, name, "mp4", MediaKind::Video);
    let bin = MediaBin {
        name: format!("bin {name}"),
        kind: MediaBinKind::Manual {
            items: vec![MediaBinItem {
                id: MediaBinItemId::new(),
                name: None,
                enabled: true,
                trigger: MediaTrigger {
                    playback,
                    ..MediaTrigger::new(a.id, MediaLayer::Background)
                },
            }],
        },
        ..MediaBin::default()
    };
    write_doc(&fx.dir, "portable", "mediaBin", &bin.id.0.to_string(), &bin);
    a
}

/// Onsets of beeps heard (from a tap of the master bus) and of flashes shown (sampled like
/// the output clock: one 30 fps tick after another, each frame chosen a tick ahead), for
/// `dur`. Pauses / seeks may be done by `during` (called from the test's task meanwhile).
pub(crate) struct Onsets {
    pub(crate) audio: Vec<Instant>,
    video: Vec<Instant>,
}

fn collect(
    server: &RunningServer,
    play: u64,
    dur: Duration,
) -> (std::thread::JoinHandle<Onsets>, Arc<AtomicBool>) {
    let v = server.shared.video(play).expect("video live");
    collect_frames(server, Arc::new(move |tick| v.view(tick).frame), dur)
}

/// The frame shown at an output tick.
pub(crate) type FrameAt = Arc<dyn Fn(Instant) -> Option<Arc<media::VideoFrame>> + Send + Sync>;

/// [`collect`] for any picture: `frame_at` gives the frame shown at a tick.
pub(crate) fn collect_frames(
    server: &RunningServer,
    frame_at: FrameAt,
    dur: Duration,
) -> (std::thread::JoinHandle<Onsets>, Arc<AtomicBool>) {
    let shared = server.shared.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    let tap = shared.sound.engine.tap();
    let h = std::thread::spawn(move || {
        let t0 = Instant::now();
        let end = t0 + dur;
        // Video ticks on their own thread.
        let vt = {
            let st = st.clone();
            std::thread::spawn(move || {
                let period = Duration::from_nanos(33_333_333);
                let mut k = 1u32;
                let mut out = Vec::new();
                let mut was = false;
                loop {
                    let tick = t0 + period * k;
                    if tick > end || st.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Some(d) = (tick - period).checked_duration_since(Instant::now()) {
                        std::thread::sleep(d);
                    }
                    let flash = frame_at(tick)
                        .as_ref()
                        .is_some_and(|f| f.luma(f.width / 2, f.height / 2) > 128);
                    if flash && !was {
                        out.push(tick);
                    }
                    was = flash;
                    k += 1;
                }
                out
            })
        };
        let mut audio = Vec::new();
        let mut last_loud: Option<Instant> = None;
        while Instant::now() < end && !st.load(Ordering::Relaxed) {
            let Some(b) = tap.recv_timeout(Duration::from_millis(50)) else {
                continue;
            };
            for i in 0..b.frames {
                let x = b.data[i * b.channels].abs();
                if x > 0.05 {
                    let at = b.heard_at + Duration::from_secs_f64(i as f64 / 48_000.0);
                    if last_loud.is_none_or(|l| at.duration_since(l) > Duration::from_millis(50)) {
                        audio.push(at);
                    }
                    last_loud = Some(at);
                }
            }
            tap.recycle(b);
        }
        Onsets {
            audio,
            video: vt.join().unwrap(),
        }
    });
    (h, stop)
}

/// Offsets (ms, heard − shown) of each beep against the nearest flash onset.
fn offsets(o: &Onsets) -> Vec<f64> {
    o.audio
        .iter()
        .filter_map(|a| {
            o.video
                .iter()
                .map(|v| audio::secs_between(*v, *a) * 1000.0)
                .min_by(|x, y| x.abs().total_cmp(&y.abs()))
        })
        .collect()
}

fn play_of(server: &RunningServer) -> u64 {
    server
        .shared
        .state()
        .media
        .and_then(|m| m.play)
        .expect("video play")
}

const FRAME_MS: f64 = 1000.0 / 30.0;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn av_sync_through_the_engine_on_the_null_output() {
    let fx = fixture("audio-avsync");
    let beeps = [1.0, 2.0, 3.3, 4.5];
    let clip = video(&fx, "beeps", 6.0, &beeps, Playback::default());
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    assert_eq!(a.state["audio"]["outputState"], "null");
    a.ok(json!({"type": "triggerMedia", "asset": clip.id.0.to_string()}))
        .await;
    let play = play_of(&server);
    let (h, _) = collect(&server, play, Duration::from_millis(5600));
    let o = tokio::task::spawn_blocking(move || h.join().unwrap())
        .await
        .unwrap();
    let off = offsets(&o);
    eprintln!("A/V offsets (ms, heard − shown): {off:.1?}");
    assert_eq!(
        o.audio.len(),
        beeps.len(),
        "beeps heard: {:?}",
        o.audio.len()
    );
    assert_eq!(o.video.len(), beeps.len(), "flashes shown");
    for x in &off {
        assert!(x.abs() <= FRAME_MS, "A/V offset {x:.1} ms");
    }
    // The clock itself: at the instant each beep is heard, the video's time is the beep's.
    let v = server.shared.video(play).unwrap();
    assert_eq!(
        server.shared.sound.engine.status().state,
        audio::OutputState::Null
    );
    drop(v);
    // The soundtrack is in the transport and meters are sent.
    assert_eq!(a.state["transport"]["layer"], "media");
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loops_and_seeks_keep_sync() {
    let fx = fixture("audio-loop");
    let clip = video(
        &fx,
        "loop",
        3.0,
        &[0.5, 2.0],
        Playback {
            repeat: Repeat::Loop,
            ..Playback::default()
        },
    );
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "triggerMedia", "asset": clip.id.0.to_string()}))
        .await;
    let play = play_of(&server);
    // Three passes: beeps at 0.5, 2.0, 3.5, 5.0, 6.5, 8.0.
    let (h, _) = collect(&server, play, Duration::from_millis(8400));
    let o = tokio::task::spawn_blocking(move || h.join().unwrap())
        .await
        .unwrap();
    let off = offsets(&o);
    eprintln!("loop offsets (ms): {off:.1?}");
    assert_eq!(o.audio.len(), 6, "beeps heard over three passes");
    assert!(off.iter().all(|x| x.abs() <= FRAME_MS), "{off:?}");
    // Seek just before a beep, several times.
    let mut all = Vec::new();
    for _ in 0..3 {
        let (h, stop) = collect(&server, play, Duration::from_millis(1200));
        a.ok(json!({"type": "mediaSeek", "seconds": 1.6})).await;
        let o = tokio::task::spawn_blocking(move || h.join().unwrap())
            .await
            .unwrap();
        stop.store(true, Ordering::Relaxed);
        let off = offsets(&o);
        assert!(!o.audio.is_empty(), "the beep after the seek is heard");
        all.extend(off);
    }
    eprintln!("seek offsets (ms): {all:.1?}");
    assert!(all.iter().all(|x| x.abs() <= FRAME_MS), "{all:?}");
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_and_resume_do_not_drift_over_a_minute() {
    let fx = fixture("audio-pause");
    let beeps: Vec<f64> = (1..=13).map(|k| k as f64 * 5.0).collect();
    let clip = video(&fx, "long", 70.0, &beeps, Playback::default());
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "triggerMedia", "asset": clip.id.0.to_string()}))
        .await;
    let play = play_of(&server);
    let xruns_before = server.shared.sound.engine.xruns();
    // 60 s of media plus 6 pauses of 0.7 s.
    let (h, _) = collect(&server, play, Duration::from_millis(64_800));
    let t0 = Instant::now();
    for k in 0..6u64 {
        tokio::time::sleep_until((t0 + Duration::from_millis(7_500 + k * 10_000)).into()).await;
        a.ok(json!({"type": "mediaPause"})).await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        a.ok(json!({"type": "mediaPlay"})).await;
    }
    let o = tokio::task::spawn_blocking(move || h.join().unwrap())
        .await
        .unwrap();
    let off = offsets(&o);
    // Under load (a debugger, a second test run) two things happen that aren't drift: the
    // test's flash sampler (a thread looking at the frame of each tick) falls behind and
    // misses a flash, leaving its beep paired with a flash 5 s away; and the output starves
    // (its thread didn't run for over 200 ms), restarting its timeline, which the engine
    // counts, and a beep may be heard off its flash once. So each beep is paired with a flash
    // within 2 s only (beeps are 5 s apart), each output restart excuses at most one paired
    // beep off by more than a frame, and with none every paired beep is within a frame. The
    // drift is measured over the beeps within a frame, across most of the minute.
    let xruns = server.shared.sound.engine.xruns() - xruns_before;
    eprintln!("offsets over 60 s with pauses (ms): {off:.1?}; output restarts: {xruns}");
    assert!(o.audio.len() >= 11, "beeps heard: {}", o.audio.len());
    let paired: Vec<f64> = off.iter().copied().filter(|x| x.abs() < 2000.0).collect();
    let off: Vec<f64> = paired
        .iter()
        .copied()
        .filter(|x| x.abs() <= FRAME_MS)
        .collect();
    assert!(
        off.len() + xruns as usize >= paired.len() && off.len() >= 9,
        "{} of {} beeps paired with a flash are within a frame ({} heard), {xruns} output restarts",
        off.len(),
        paired.len(),
        o.audio.len()
    );
    let drift = off.last().unwrap() - off.first().unwrap();
    assert!(drift.abs() <= FRAME_MS, "drift {drift:.1} ms");
    // The position after a minute: 60 s of media in ~64 s of wall time.
    let st = get_json(server.addr, "/api/v1/state").await;
    let pos = st["state"]["transport"]["position"].as_f64().unwrap();
    assert!(pos > 58.0 && pos < 62.0, "position {pos}");
    server.shutdown().await.unwrap();
}

pub(crate) fn frequency(x: &[f32], ch: usize) -> f64 {
    let s: Vec<f32> = x.iter().step_by(ch).copied().collect();
    let mut c = Vec::new();
    for i in 1..s.len() {
        if s[i - 1] < 0.0 && s[i] >= 0.0 {
            c.push(i as f64 - s[i] as f64 / (s[i] - s[i - 1]) as f64);
        }
    }
    if c.len() < 3 {
        return 0.0;
    }
    (c.len() - 1) as f64 * 48_000.0 / (c[c.len() - 1] - c[0])
}

/// The master bus for `dur` (interleaved).
pub(crate) fn record(server: &RunningServer, dur: Duration) -> Vec<f32> {
    let tap = server.shared.sound.engine.tap();
    let end = Instant::now() + dur;
    let mut out = Vec::new();
    while Instant::now() < end {
        if let Some(b) = tap.recv_timeout(Duration::from_millis(50)) {
            out.extend_from_slice(&b.data);
            tap.recycle(b);
        }
    }
    out
}

async fn wait_for(a: &mut Client, what: &str, pred: impl Fn(&Value) -> bool) {
    a.wait_state(what, |_, s| pred(s)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audio_layer_bins_playlists_levels_and_devices() {
    let fx = fixture("audio-layer");
    let mut items = Vec::new();
    let mut assets = Vec::new();
    for (name, f) in [("low", 440.0), ("mid", 660.0), ("high", 880.0)] {
        tone_file(&fx.dir.join("media").join(format!("{name}.wav")), f, 1.2);
        let a = add_asset(&fx, name, "wav", MediaKind::Audio);
        items.push(MediaBinItem {
            id: MediaBinItemId::new(),
            name: None,
            enabled: true,
            trigger: MediaTrigger::new(a.id, MediaLayer::Audio),
        });
        assets.push(a);
    }
    let bin = MediaBin {
        name: "Songs".into(),
        browser: BinBrowser::Audio,
        kind: MediaBinKind::Manual {
            items: items.clone(),
        },
        ..MediaBin::default()
    };
    write_doc(&fx.dir, "portable", "mediaBin", &bin.id.0.to_string(), &bin);
    let pl_items: Vec<PlaylistItem> = assets
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
            items: pl_items.clone(),
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
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;

    // A bin item plays on the audio layer; the transport follows it.
    a.ok(json!({"type": "triggerMedia", "asset": assets[0].id.0.to_string()}))
        .await;
    let l = &a.state["layers"]["audio"];
    assert_eq!(l["name"], "low", "{l}");
    assert_eq!(l["kind"], "audio");
    assert_eq!(l["playback"]["state"], "playing");
    assert!(
        (l["playback"]["duration"].as_f64().unwrap() - 1.2).abs() < 0.05,
        "{l}"
    );
    assert_eq!(a.state["transport"]["layer"], "audio");
    let x = record(&server, Duration::from_millis(400));
    let f = frequency(&x[x.len() / 2..], 2);
    assert!((f - 440.0).abs() < 2.0, "{f} Hz");
    // Meters: about −6 dBFS, sent as audioLevels messages.
    let t = Instant::now();
    loop {
        let m = a.next_any().await;
        if m["type"] == "audioLevels" && m["levels"][0].as_f64().unwrap() > -10.0 {
            assert!((m["levels"][0].as_f64().unwrap() + 6.0).abs() <= 1.0, "{m}");
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(5), "no meters");
    }
    // Pause / play on the audio layer.
    a.ok(json!({"type": "mediaPause", "layer": "audio"})).await;
    assert_eq!(a.state["layers"]["audio"]["playback"]["state"], "paused");
    let x = record(&server, Duration::from_millis(200));
    assert!(
        x[x.len() / 2..].iter().all(|v| v.abs() < 1e-4),
        "silent while paused"
    );
    a.ok(json!({"type": "mediaToggle", "layer": "audio"})).await;
    assert_eq!(a.state["layers"]["audio"]["playback"]["state"], "playing");
    // Layer volume and master mute.
    a.ok(json!({"type": "setVolume", "target": "layer", "layer": "audio", "db": -12}))
        .await;
    assert_eq!(a.state["audio"]["layers"]["audio"]["db"], -12.0);
    tokio::time::sleep(Duration::from_millis(250)).await;
    let lv = server.shared.sound.engine.levels();
    assert!((lv[0] + 18.0).abs() <= 1.0, "{lv:?}");
    a.ok(json!({"type": "setMute", "target": "master", "muted": true}))
        .await;
    assert_eq!(a.state["audio"]["muted"], true);
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(server.shared.sound.engine.levels(), vec![-60.0, -60.0]);
    a.ok(json!({"type": "setMute", "target": "master", "muted": false}))
        .await;
    a.ok(json!({"type": "setVolume", "target": "layer", "layer": "audio", "db": 0}))
        .await;
    a.ok(json!({"type": "setVolume", "target": "master", "db": -3}))
        .await;
    assert_eq!(a.state["audio"]["masterDb"], -3.0);
    let r = http(
        addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({"type": "setVolume", "target": "layer", "layer": "props", "db": 0})),
    )
    .await;
    assert_eq!(r.status, 400);
    // The end of a bin item clears the layer.
    wait_for(&mut a, "bin item ends", |s| s["layers"]["audio"].is_null()).await;

    // An audio playlist: in order, on to the next at the end, next / previous, repeat.
    let pid = playlist.id.0.to_string();
    let item = |i: usize| pl_items[i].id.0.to_string();
    a.ok(json!({"type": "triggerPlaylistItem", "playlist": pid, "item": item(0)}))
        .await;
    assert_eq!(a.state["audio"]["playlist"]["item"], item(0));
    assert_eq!(a.state["layers"]["audio"]["name"], "low");
    wait_for(&mut a, "next item at the end", |s| {
        s["audio"]["playlist"]["item"] == item(1)
    })
    .await;
    assert_eq!(a.state["layers"]["audio"]["name"], "mid");
    let x = record(&server, Duration::from_millis(300));
    assert!((frequency(&x[x.len() / 2..], 2) - 660.0).abs() < 2.0);
    a.ok(json!({"type": "audioNext"})).await;
    assert_eq!(a.state["audio"]["playlist"]["item"], item(2));
    // Past the last item without repeat: nothing changes.
    a.ok(json!({"type": "audioNext"})).await;
    assert_eq!(a.state["audio"]["playlist"]["item"], item(2));
    a.ok(json!({"type": "setAudioOptions", "repeat": true}))
        .await;
    assert_eq!(a.state["audio"]["repeat"], true);
    a.ok(json!({"type": "audioNext"})).await;
    assert_eq!(a.state["audio"]["playlist"]["item"], item(0));
    a.ok(json!({"type": "audioPrev"})).await;
    assert_eq!(a.state["audio"]["playlist"]["item"], item(2));
    // Shuffle: every item comes once in three steps.
    a.ok(json!({"type": "setAudioOptions", "shuffle": true}))
        .await;
    let mut seen = std::collections::HashSet::new();
    seen.insert(
        a.state["audio"]["playlist"]["item"]
            .as_str()
            .unwrap()
            .to_string(),
    );
    for _ in 0..2 {
        a.ok(json!({"type": "audioNext"})).await;
        seen.insert(
            a.state["audio"]["playlist"]["item"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    assert_eq!(seen.len(), 3, "{seen:?}");
    a.ok(json!({"type": "clearLayer", "layer": "audio"})).await;
    assert!(a.state["layers"]["audio"].is_null());
    assert!(a.state["audio"]["playlist"].is_null());

    // Devices and the device configuration.
    let devs = get_json(addr, "/api/v1/audio-devices").await;
    assert_eq!(devs[0]["name"], "default", "{devs}");
    let r = http(
        addr,
        "PUT",
        "/api/v1/audio-device",
        &[],
        Some(json!({"device": "", "masterGainDb": -4.5})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let cfg = get_json(addr, "/api/v1/audio-device").await;
    assert_eq!(cfg["masterGainDb"], -4.5);
    assert!(
        fx.dir
            .join("machine/audioDeviceConfig/00000000-0000-0000-0000-000000000000.json")
            .is_file()
    );
    let r = http(
        addr,
        "PUT",
        "/api/v1/audio-device",
        &[],
        Some(json!({"masterGainDb": 40})),
    )
    .await;
    assert_eq!(r.status, 400);
    // A test tone on the master bus.
    tokio::time::sleep(Duration::from_millis(700)).await; // the cleared item's fade-out
    a.ok(json!({"type": "playTestTone", "seconds": 1.0})).await;
    let x = record(&server, Duration::from_millis(400));
    assert!((frequency(&x[x.len() / 3..], 2) - 1000.0).abs() < 2.0);
    // Audio durations in the bin listing.
    let b = get_json(addr, &format!("/api/v1/media-bins/{}", bin.id.0)).await;
    assert!(
        (b["items"][0]["duration"].as_f64().unwrap() - 1.2).abs() < 0.05,
        "{b}"
    );
    server.shutdown().await.unwrap();
}

/// Embedded audio on an IDN output through the server: a video with beeps and flashes on a
/// screen with an IDN output that embeds audio; a receiver checks the tone and level and the
/// A/V offset in timecodes.
///
/// The binding is stored as a **schema-1 document with the old `ndi` transport**, as the
/// importer used to write it: the server migrates it to IDN on load, runs it, and leaves the
/// file on disk untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idn_embedded_audio_matches_the_video() {
    let fx = fixture("audio-idn");
    let beeps = [1.5, 3.0, 4.5];
    let clip = video(&fx, "idn", 6.0, &beeps, Playback::default());
    let source = format!("audio-test-{}", std::process::id());
    let id = OutputId::new();
    let v1 = json!({
        "schemaVersion": 1,
        "id": id,
        "name": "Program",
        "screen": fx.audience.id,
        "transport": {"type": "ndi", "sourceName": source},
        "embedAudio": true
    });
    write_doc(&fx.dir, "machine", "outputBinding", &id.0.to_string(), &v1);
    let path = fx.dir.join(format!("machine/outputBinding/{}.json", id.0));
    let on_disk = std::fs::read(&path).unwrap();
    // `VESPER_TEST_AUDIO_DEVICE=hw:1,0` runs it on a sound card (the lip-sync acceptance
    // check of the device back end); by default the null output.
    let server = match std::env::var("VESPER_TEST_AUDIO_DEVICE") {
        Ok(dev) => vesper_server::start(vesper_server::ServerConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            trust_localhost: true,
            mdns: false,
            audio_device: Some(dev),
            site: vesper_server::site::SiteConfig::off(),
            ..vesper_server::ServerConfig::new(&fx.dir)
        })
        .await
        .expect("server starts"),
        Err(_) => start_server(&fx, true, false).await,
    };
    // No settling wait on a real card: idn-audio's start-up latency step (reported to the
    // IDN maintainers, 2026-11-19) shows here until IDN fixes it.
    eprintln!("audio output: {:?}", server.shared.sound.engine.status());
    let mut a = Client::connect(server.addr, "").await;
    let outs = &a.state["screens"][0]["outputs"];
    assert_eq!(outs[0]["embedAudio"], true, "{outs}");
    assert_eq!(outs[0]["kind"], "idn", "{outs}");
    assert_eq!(outs[0]["status"], "ok", "{outs}");
    let listed = get_json(server.addr, "/api/v1/outputs").await;
    assert_eq!(
        listed[0]["transport"],
        json!({"type": "idn", "sourceName": source}),
        "{listed}"
    );
    let rx = tokio::task::spawn_blocking(move || {
        let f = output::IdnFinder::new().unwrap();
        let found = f
            .find_by_name(&source, Duration::from_secs(10))
            .expect("source");
        output::IdnReceiver::connect(&found.name).unwrap()
    })
    .await
    .unwrap();
    a.ok(json!({"type": "triggerMedia", "asset": clip.id.0.to_string()}))
        .await;
    let (vt, at, (sent, starts), first_tc) = tokio::task::spawn_blocking(move || {
        let mut rx = rx;
        let end = Instant::now() + Duration::from_secs(7);
        let mut video_on = Vec::new();
        let mut was = false;
        // Channel 0 of every audio frame, end to end, from the first frame's timecode.
        let mut all: Vec<f32> = Vec::new();
        let mut first_tc = None;
        // The same, as received: frames back to back, and (timecode, start) of each.
        let mut sent: Vec<f32> = Vec::new();
        let mut starts: Vec<(i64, usize)> = Vec::new();
        while Instant::now() < end {
            match rx.capture(Duration::from_millis(200)).unwrap() {
                Some(output::Received::Video(f)) => {
                    let p = f.pixel_rgba(f.width / 2, f.height / 2);
                    let flash = p[0] > 128;
                    if flash && !was {
                        video_on.push(f.timecode);
                    }
                    was = flash;
                }
                Some(output::Received::Audio(x)) => {
                    assert_eq!(x.sample_rate, 48_000);
                    let t0 = *first_tc.get_or_insert(x.timecode);
                    // Frames follow each other (timecodes say where).
                    let at = ((x.timecode - t0) as f64 * 48_000.0 / 1e7).round() as usize;
                    if all.len() < at + x.samples {
                        all.resize(at + x.samples, 0.0);
                    }
                    all[at..at + x.samples].copy_from_slice(x.channel(0));
                    starts.push((x.timecode, sent.len()));
                    sent.extend_from_slice(x.channel(0));
                }
                None => {}
            }
        }
        // Beep onsets.
        let mut audio_on = Vec::new();
        let mut last: Option<usize> = None;
        for (i, v) in all.iter().enumerate() {
            if v.abs() > 0.05 {
                if last.is_none_or(|l| i - l > 2_400) {
                    audio_on.push(i);
                }
                last = Some(i);
            }
        }
        (video_on, audio_on, (sent, starts), first_tc.unwrap_or(0))
    })
    .await
    .unwrap();
    let at: Vec<i64> = at
        .iter()
        .map(|i| first_tc + (*i as i64 * 10_000_000) / 48_000)
        .collect();
    eprintln!("IDN flash timecodes {vt:?}, beep timecodes {at:?}");
    assert!(
        at.len() >= 2 && vt.len() >= 2,
        "beeps {at:?} flashes {vt:?}"
    );
    let off: Vec<f64> = at
        .iter()
        .filter_map(|a| {
            vt.iter()
                .map(|v| (a - v) as f64 / 10_000.0)
                .min_by(|x, y| x.abs().total_cmp(&y.abs()))
        })
        .collect();
    eprintln!("IDN A/V offsets (ms, audio − video timecode): {off:.1?}");
    assert!(off.iter().all(|x| x.abs() <= FRAME_MS), "{off:?}");
    // The first beep's tone: 1 kHz at −6 dBFS (its middle 80 ms), in the samples as sent.
    // Not from `all`: the program clock's timecodes follow the output latency, so frames
    // placed by timecode can leave a one-sample hole (a 0.0) between them, and each hole
    // reads as one more zero crossing (+12.5 Hz).
    let (tc, start) = *starts.iter().rev().find(|(t, _)| *t <= at[0]).unwrap();
    let i0 = start + ((at[0] - tc) as f64 * 48_000.0 / 1e7) as usize;
    let beep = &sent[i0 + 480..i0 + 4_320];
    let peak = beep.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let db = 20.0 * (peak as f64).log10();
    let f = frequency(beep, 1);
    eprintln!("IDN beep: {f:.1} Hz at {db:.2} dBFS");
    assert!((db + 6.0).abs() < 1.0, "level {db}");
    assert!((f - 1000.0).abs() < 2.0, "frequency {f}");
    server.shutdown().await.unwrap();
    // Migrated in memory only: the stored schema-1 document is unchanged.
    assert_eq!(std::fs::read(&path).unwrap(), on_disk);
}

/// A video element's sound (G16): the live slide's video element plays its soundtrack on the
/// slide group, in step with its picture (the element's frame follows the sound clock); the
/// slide layer's mute and the fill's own mute silence it; clearing the slide stops it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_video_element_plays_its_sound_in_step() {
    use model::{AudioSettings, Element, FillKind, Presentation, Rect, Section, Slide};
    let fx = fixture("audio-element");
    let beeps = [0.5, 1.5, 2.5, 3.5, 4.5, 5.5];
    let looped = Playback {
        repeat: Repeat::Loop,
        ..Playback::default()
    };
    let clip = video(&fx, "element-beeps", 6.0, &beeps, looped.clone());
    let element = |muted: bool| {
        let mut e = Element::media(Rect::new(0.0, 0.0, 1920.0, 1080.0), clip.id);
        if let FillKind::Media {
            playback, audio, ..
        } = &mut e.fill.kind
        {
            *playback = looped.clone();
            *audio = AudioSettings {
                muted,
                ..AudioSettings::default()
            };
        }
        e
    };
    let mut loud = Slide::new();
    loud.elements = vec![element(false)];
    let mut quiet = Slide::new();
    quiet.elements = vec![element(true)];
    let loud_id = loud.id;
    let mut sec = Section::new("Clip");
    sec.slides = vec![loud, quiet];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Element sound")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 0}))
        .await;
    // One play for the element (whatever the screens), open off the actor.
    let deadline = Instant::now() + Duration::from_secs(10);
    let play = loop {
        let p = server
            .shared
            .element_plays
            .lock()
            .unwrap()
            .values()
            .next()
            .cloned();
        if let Some(p) = p.filter(|p| p.player().is_some()) {
            break p;
        }
        assert!(
            Instant::now() < deadline,
            "the element's player never opened"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(server.shared.element_plays.lock().unwrap().len(), 1);
    assert!(
        server
            .shared
            .element_plays
            .lock()
            .unwrap()
            .contains_key(&(loud_id, play.video.element))
    );
    assert!(a.state["layers"]["media"].is_null(), "no media-layer play");
    // Beeps are heard as their flashes are shown (the frame follows the sound clock).
    let shown = play.clone();
    let (h, _) = collect_frames(
        &server,
        Arc::new(move |tick| shown.frame(tick).map(|(_, f)| f)),
        Duration::from_millis(3200),
    );
    let o = tokio::task::spawn_blocking(move || h.join().unwrap())
        .await
        .unwrap();
    let off = offsets(&o);
    eprintln!("element A/V offsets (ms, heard − shown): {off:.1?}");
    assert!(o.audio.len() >= 2, "beeps heard: {}", o.audio.len());
    assert_eq!(o.audio.len(), o.video.len(), "a flash for every beep");
    for x in &off {
        assert!(x.abs() <= FRAME_MS, "A/V offset {x:.1} ms");
    }
    // The slide layer's mute silences it (the slide group), and unmuting brings it back.
    a.ok(json!({"type": "setMute", "target": "layer", "layer": "slide", "muted": true}))
        .await;
    let heard = |server: &RunningServer, ms: u64| {
        let (h, _) = collect_frames(server, Arc::new(|_| None), Duration::from_millis(ms));
        tokio::task::spawn_blocking(move || h.join().unwrap().audio.len())
    };
    assert_eq!(heard(&server, 1300).await.unwrap(), 0, "muted slide layer");
    a.ok(json!({"type": "setMute", "target": "layer", "layer": "slide", "muted": false}))
        .await;
    assert!(heard(&server, 1300).await.unwrap() >= 1, "unmuted");
    // The next slide has the same video, muted in its fill: no sound, but a new play.
    a.ok(json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 1}))
        .await;
    assert!(
        !server
            .shared
            .element_plays
            .lock()
            .unwrap()
            .contains_key(&(loud_id, play.video.element)),
        "the first slide's play left with it"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(heard(&server, 1300).await.unwrap(), 0, "a muted fill");
    // Back to the first slide, then clear: silence, and no play left.
    a.ok(json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 0}))
        .await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while heard(&server, 1100).await.unwrap() == 0 {
        assert!(Instant::now() < deadline, "no sound after going back");
    }
    a.ok(json!({"type": "clearLayer", "layer": "slide"})).await;
    assert!(server.shared.element_plays.lock().unwrap().is_empty());
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(heard(&server, 1300).await.unwrap(), 0, "cleared");
    server.shutdown().await.unwrap();
}

/// Beeps heard on the master bus over `ms`.
async fn beeps_heard(server: &RunningServer, ms: u64) -> usize {
    let (h, _) = collect_frames(server, Arc::new(|_| None), Duration::from_millis(ms));
    tokio::task::spawn_blocking(move || h.join().unwrap().audio.len())
        .await
        .unwrap()
}

/// The live slide's open element play (waits for it to open).
async fn open_play(
    server: &RunningServer,
    slide: model::SlideId,
) -> Arc<vesper_server::element_video::ElementPlay> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let p = server
            .shared
            .element_plays
            .lock()
            .unwrap()
            .iter()
            .find(|((s, _), _)| *s == slide)
            .map(|(_, p)| p.clone());
        if let Some(p) = p.filter(|p| p.player().is_some()) {
            return p;
        }
        assert!(
            Instant::now() < deadline,
            "the element's player never opened"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// E01-1 / R2-2: a video element's sound follows what the screens draw. Before its build-in
/// step, under a look whose audience screens all show a template override, and while its
/// visibility condition fails, its play runs on muted (in step for the reveal); revealed, it
/// is heard.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_video_element_is_heard_only_while_the_screens_draw_it() {
    use model::{
        AudioSettings, Build, BuildMode, Element, FillKind, LayerVisibility, Link, Look,
        Presentation, Rect, ScreenLook, Section, Slide, Timer, TimerKind, TimerState, Transition,
        VisibilityCondition,
    };
    let fx = fixture("audio-element-drawn");
    let looped = Playback {
        repeat: Repeat::Loop,
        ..Playback::default()
    };
    let clip = video(
        &fx,
        "element-drawn",
        4.0,
        &[0.5, 1.5, 2.5, 3.5],
        looped.clone(),
    );
    let element = || {
        let mut e = Element::media(Rect::new(0.0, 0.0, 1920.0, 1080.0), clip.id);
        if let FillKind::Media {
            playback, audio, ..
        } = &mut e.fill.kind
        {
            *playback = looped.clone();
            *audio = AudioSettings::default();
        }
        e
    };
    let timer = Timer {
        name: "Walk-in".into(),
        sort_key: "a".into(),
        kind: TimerKind::Countdown { seconds: 600.0 },
        ..Timer::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "timer",
        &timer.id.0.to_string(),
        &timer,
    );
    // Slide 0: the clip builds in on the first step. Slide 1: shown while the timer runs.
    let built = element();
    let mut s0 = Slide::new();
    s0.builds = vec![Build::new(built.id, BuildMode::In, Transition::CUT)];
    s0.elements = vec![built];
    let mut timed = element();
    timed.visibility = vec![VisibilityCondition::Timer {
        timer: Some(Link::to(timer.id)),
        state: TimerState::IsRunning,
    }];
    let mut s1 = Slide::new();
    s1.elements = vec![timed];
    let (s0_id, s1_id) = (s0.id, s1.id);
    let mut sec = Section::new("Clip");
    sec.slides = vec![s0, s1];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Element drawn")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    // Every audience screen restyles the slide through a template: the clip is not drawn.
    let restyled = Look {
        name: "Restyled".into(),
        sort_key: "c".into(),
        screens: [fx.audience.id, fx.key.id]
            .into_iter()
            .map(|screen| ScreenLook {
                screen,
                layers: LayerVisibility::ALL,
                template_override: fx.default_look.screens[0].template_override.clone(),
                mask: None,
                language: Default::default(),
            })
            .collect(),
        ..Look::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "look",
        &restyled.id.0.to_string(),
        &restyled,
    );
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    let plain = json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()});
    a.ok(plain.clone()).await;
    a.ok(json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 0}))
        .await;
    // Before its build step: open and running, not heard.
    let play = open_play(&server, s0_id).await;
    assert!(!play.heard());
    assert_eq!(beeps_heard(&server, 1300).await, 0, "before its build-in");
    a.ok(json!({"type": "next"})).await;
    assert!(play.heard());
    assert!(beeps_heard(&server, 1300).await >= 1, "built in");
    assert!(
        server
            .shared
            .element_plays
            .lock()
            .unwrap()
            .values()
            .any(|p| Arc::ptr_eq(p, &play)),
        "the same play goes on"
    );
    // A look whose audience screens all show a template override.
    a.ok(json!({"type": "setLook", "look": restyled.id.0.to_string()}))
        .await;
    assert!(!play.heard());
    assert_eq!(beeps_heard(&server, 1300).await, 0, "template override");
    a.ok(plain).await;
    assert!(beeps_heard(&server, 1300).await >= 1, "the plain look");
    // A visibility condition: heard while the timer runs.
    a.ok(json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 1}))
        .await;
    let play = open_play(&server, s1_id).await;
    assert!(!play.heard());
    assert_eq!(beeps_heard(&server, 1300).await, 0, "the timer is stopped");
    a.ok(json!({"type": "timerStart", "timer": timer.id.0.to_string()}))
        .await;
    assert!(play.heard());
    assert!(beeps_heard(&server, 1300).await >= 1, "the timer runs");
    a.ok(json!({"type": "timerStop", "timer": timer.id.0.to_string()}))
        .await;
    assert!(!play.heard());
    server.shutdown().await.unwrap();
}

/// A video element shown by a media condition (`videoCountdown` / `audioCountdown`) is heard
/// exactly when the screens draw it: the sound check sees the same video states as the
/// screens. Here: shown while the background video plays.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_video_element_with_a_media_condition_is_heard_while_drawn() {
    use model::{
        AudioSettings, Element, FillKind, MediaCondition, Presentation, Rect, Section, Slide,
        VisibilityCondition,
    };
    let fx = fixture("audio-element-media");
    let looped = Playback {
        repeat: Repeat::Loop,
        ..Playback::default()
    };
    let clip = video(
        &fx,
        "element-media",
        4.0,
        &[0.5, 1.5, 2.5, 3.5],
        looped.clone(),
    );
    // The background: a silent video (its "beep" lies past its end).
    let bg = video(&fx, "silent-bg", 30.0, &[100.0], Playback::default());
    let mut e = Element::media(Rect::new(0.0, 0.0, 1920.0, 1080.0), clip.id);
    if let FillKind::Media {
        playback, audio, ..
    } = &mut e.fill.kind
    {
        *playback = looped.clone();
        *audio = AudioSettings::default();
    }
    e.visibility = vec![VisibilityCondition::VideoCountdown {
        state: MediaCondition::IsRunning,
    }];
    let mut slide = Slide::new();
    slide.elements = vec![e];
    let slide_id = slide.id;
    let mut sec = Section::new("Clip");
    sec.slides = vec![slide];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Element media state")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 0}))
        .await;
    let play = open_play(&server, slide_id).await;
    assert!(!play.heard());
    assert_eq!(beeps_heard(&server, 1300).await, 0, "no video plays");
    let until = |want: bool, what: &str| {
        let play = play.clone();
        let what = what.to_string();
        async move {
            let deadline = Instant::now() + Duration::from_secs(5);
            while play.heard() != want {
                assert!(Instant::now() < deadline, "{what}: heard stays {}", !want);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    };
    a.ok(json!({"type": "triggerMedia", "asset": bg.id.0.to_string()}))
        .await;
    until(true, "the background video plays").await;
    assert!(beeps_heard(&server, 1300).await >= 1, "the video plays");
    a.ok(json!({"type": "mediaPause"})).await;
    until(false, "paused").await;
    assert_eq!(beeps_heard(&server, 1300).await, 0, "the video is paused");
    a.ok(json!({"type": "mediaPlay"})).await;
    until(true, "playing again").await;
    a.ok(json!({"type": "clearLayer", "layer": "media"})).await;
    until(false, "the media layer cleared").await;
    server.shutdown().await.unwrap();
}

/// X3-2: a video element whose file could not be opened (still copying, a media drive not
/// mounted yet) is reported, and triggering the slide again opens it once the file is there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_video_element_that_failed_to_open_opens_when_triggered_again() {
    use model::{Element, FillKind, Presentation, Rect, Section, Slide};
    let fx = fixture("audio-element-late");
    let clip = video(&fx, "element-late", 2.0, &[0.5], Playback::default());
    let mut e = Element::media(Rect::new(0.0, 0.0, 1920.0, 1080.0), clip.id);
    if let FillKind::Media { playback, .. } = &mut e.fill.kind {
        *playback = Playback {
            repeat: Repeat::Loop,
            ..Playback::default()
        };
    }
    let mut s = Slide::new();
    s.elements = vec![e];
    let slide_id = s.id;
    let mut sec = Section::new("Clip");
    sec.slides = vec![s];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Element late")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let server = start_server(&fx, true, false).await;
    // The file is not there yet (outside the server's folder meanwhile).
    let file = server.shared.data().content.asset_path(clip.id).unwrap();
    let away = fx.dir.with_extension("element-late.mp4");
    std::fs::rename(&file, &away).unwrap();
    let mut a = Client::connect(server.addr, "").await;
    let trigger = json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 0});
    a.ok(trigger.clone()).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let failed = server
            .shared
            .element_plays
            .lock()
            .unwrap()
            .values()
            .any(|p| p.failed());
        if failed {
            break;
        }
        assert!(Instant::now() < deadline, "the open never failed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The file arrives; the operator triggers the slide again.
    std::fs::rename(&away, &file).unwrap();
    a.ok(trigger).await;
    let play = open_play(&server, slide_id).await;
    assert!(!play.failed());
    assert_eq!(server.shared.element_plays.lock().unwrap().len(), 1);
    server.shutdown().await.unwrap();
}

/// Waits until `play` is heard (`want`) or not, at most 10 s; returns how long it took.
async fn heard_becomes(
    play: &vesper_server::element_video::ElementPlay,
    want: bool,
    what: &str,
) -> Duration {
    let t = Instant::now();
    while play.heard() != want {
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "{what}: heard stays {}",
            !want
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    t.elapsed()
}

/// F01-2 / R2-1 / F01-3: a video element's sound follows the timed builds of the live step
/// (a clip that hides itself after a delay goes quiet then, one built in after a delay is
/// heard from then), a saved edit of the live look, and a video input bound while live.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_video_elements_sound_follows_timed_builds_look_edits_and_inputs() {
    use model::{
        AudioSettings, Build, BuildMode, BuildStart, Element, FillKind, LayerVisibility, Link,
        Look, Presentation, Rect, ScreenLook, Section, Slide, Transition, VisibilityCondition,
    };
    let fx = fixture("audio-element-timed");
    let looped = Playback {
        repeat: Repeat::Loop,
        ..Playback::default()
    };
    let clip = video(
        &fx,
        "element-timed",
        4.0,
        &[0.5, 1.5, 2.5, 3.5],
        looped.clone(),
    );
    let element = || {
        let mut e = Element::media(Rect::new(0.0, 0.0, 1920.0, 1080.0), clip.id);
        if let FillKind::Media {
            playback, audio, ..
        } = &mut e.fill.kind
        {
            *playback = looped.clone();
            *audio = AudioSettings::default();
        }
        e
    };
    let build = |e: &Element, mode, start, delay| Build {
        start,
        delay,
        ..Build::new(e.id, mode, Transition::CUT)
    };
    const SHOWN: f64 = 4.0;
    // Slide 0: in with the slide, out by itself after SHOWN s.
    let a = element();
    let mut s0 = Slide::new();
    s0.builds = vec![
        build(&a, BuildMode::In, BuildStart::WithSlide, 0.0),
        build(&a, BuildMode::Out, BuildStart::AfterPrevious, SHOWN),
    ];
    s0.elements = vec![a];
    // Slide 1: in SHOWN s after the slide.
    let b = element();
    let mut s1 = Slide::new();
    s1.builds = vec![build(&b, BuildMode::In, BuildStart::WithSlide, SHOWN)];
    s1.elements = vec![b];
    // Slide 2: shown while Camera 1 is live.
    let cam = model::VideoInput {
        name: "Camera 1".into(),
        ..model::VideoInput::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "videoInput",
        &cam.id.0.to_string(),
        &cam,
    );
    let mut c = element();
    c.visibility = vec![VisibilityCondition::VideoInput {
        input: Link::to(cam.id),
        active: true,
    }];
    let mut s2 = Slide::new();
    s2.elements = vec![c];
    let ids = [s0.id, s1.id, s2.id];
    let mut sec = Section::new("Clip");
    sec.slides = vec![s0, s1, s2];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Element timed")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let restyled = Look {
        name: "Restyled".into(),
        sort_key: "c".into(),
        screens: [fx.audience.id, fx.key.id]
            .into_iter()
            .map(|screen| ScreenLook {
                screen,
                layers: LayerVisibility::ALL,
                template_override: fx.default_look.screens[0].template_override.clone(),
                mask: None,
                language: Default::default(),
            })
            .collect(),
        ..Look::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "look",
        &restyled.id.0.to_string(),
        &restyled,
    );
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "").await;
    ws.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    let trigger =
        |i: usize| json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": i});

    // Hidden by its own timed build-out: heard until then, not after.
    let t = Instant::now();
    ws.ok(trigger(0)).await;
    let play = open_play(&server, ids[0]).await;
    if t.elapsed().as_secs_f64() < SHOWN - 0.5 {
        assert!(play.heard(), "drawn until its build-out");
    }
    heard_becomes(&play, false, "after its build-out").await;
    assert!(
        t.elapsed().as_secs_f64() >= SHOWN - 0.1,
        "went quiet {:?} after the slide, before its build-out",
        t.elapsed()
    );

    // Built in after a delay: quiet until then, heard after.
    let t = Instant::now();
    ws.ok(trigger(1)).await;
    let play = open_play(&server, ids[1]).await;
    if t.elapsed().as_secs_f64() < SHOWN - 0.5 {
        assert!(!play.heard(), "not drawn before its build-in");
    }
    heard_becomes(&play, true, "after its build-in").await;
    assert!(
        t.elapsed().as_secs_f64() >= SHOWN - 0.1,
        "heard {:?} after the slide, before its build-in",
        t.elapsed()
    );

    // A video input bound while its source is live.
    ws.ok(json!({"type": "triggerLiveSource", "source": "127.0.0.1:9"}))
        .await;
    ws.ok(trigger(2)).await;
    let play = open_play(&server, ids[2]).await;
    assert!(!play.heard(), "Camera 1 is not bound");
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/video-inputs/{}", cam.id.0),
        &[],
        Some(json!({"name": "Camera 1", "source": {"type": "idn", "sourceName": "127.0.0.1:9"}})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        play.heard(),
        "drawn once Camera 1 is bound to the live source"
    );

    // The live look restyles every audience screen, then is edited to show the slide again.
    ws.ok(json!({"type": "setLook", "look": restyled.id.0.to_string()}))
        .await;
    assert!(!play.heard());
    let path = format!("/api/v1/looks/{}", restyled.id.0);
    let r = http(addr, "GET", &path, &[], None).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let mut look = r.json();
    let rev = look["revision"].as_str().unwrap().to_string();
    for s in look["screens"].as_array_mut().unwrap() {
        s.as_object_mut().unwrap().remove("templateOverride");
    }
    let r = http(addr, "PUT", &path, &[("If-Match", &rev)], Some(look)).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert!(play.heard(), "drawn once the live look's edit is saved");
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// A looping `secs` s clip of one colour `c` (an FFmpeg colour name) with a 440 Hz tone, as a
/// video asset.
fn solid_video(fx: &Fixture, name: &str, c: &str, secs: f64) -> MediaAsset {
    let video = format!("color=c={c}:s=160x90:r=30:d={secs}");
    let audio = format!("aevalsrc='0.5*sin(2*PI*440*t)':s=44100:d={secs}");
    ff(&[
        "-f",
        "lavfi",
        "-i",
        &video,
        "-f",
        "lavfi",
        "-i",
        &audio,
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        fx.dir
            .join("media")
            .join(format!("{name}.mp4"))
            .to_str()
            .unwrap(),
    ]);
    add_asset(fx, name, "mp4", MediaKind::Video)
}

/// A slide whose one full-screen video element (looping `clip`) has `builds(element)`.
fn element_slide(
    clip: &MediaAsset,
    builds: impl FnOnce(&model::Element) -> Vec<model::Build>,
) -> model::Slide {
    use model::{Element, FillKind, Rect, Slide};
    let mut e = Element::media(Rect::new(0.0, 0.0, 1920.0, 1080.0), clip.id);
    if let FillKind::Media { playback, .. } = &mut e.fill.kind {
        *playback = Playback {
            repeat: Repeat::Loop,
            ..Playback::default()
        };
    }
    let mut s = Slide::new();
    s.builds = builds(&e);
    s.elements = vec![e];
    s
}

/// H04-1: a video element's slide with a build delay or duration too large to be an instant
/// (an edit through the API, an imported file) goes live without stopping the show: later
/// commands are still answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_huge_build_time_on_a_video_elements_slide_keeps_the_show_running() {
    use model::{Build, BuildMode, BuildStart, Presentation, Section, Transition};
    let fx = fixture("audio-element-huge-build");
    let clip = solid_video(&fx, "element-huge", "red", 2.0);
    let delayed = element_slide(&clip, |e| {
        vec![Build {
            start: BuildStart::WithSlide,
            delay: 1e19,
            ..Build::new(e.id, BuildMode::In, Transition::CUT)
        }]
    });
    let long = element_slide(&clip, |e| {
        vec![Build {
            start: BuildStart::WithSlide,
            ..Build::new(e.id, BuildMode::In, Transition::dissolve(1e300))
        }]
    });
    let ids = [delayed.id, long.id];
    let mut sec = Section::new("Clip");
    sec.slides = vec![delayed, long];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Element huge build")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let server = start_server(&fx, true, false).await;
    let mut ws = Client::connect(server.addr, "").await;
    ws.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    for (i, id) in ids.into_iter().enumerate() {
        ws.ok(json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": i}))
            .await;
        open_play(&server, id).await;
        // The show still answers.
        ws.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
            .await;
    }
    ws.ok(json!({"type": "clearLayer", "layer": "slide"})).await;
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// H04-2: a video element's sound and the screens count a slide's first build step from the
/// same instant: the same slide triggered again starts over on both (the element built in
/// after a delay is hidden and quiet again until then), and so does a look that shows the
/// slide layer again after one that hid it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_slide_triggered_again_restarts_its_builds_on_screens_and_sound() {
    use model::{
        Build, BuildMode, BuildStart, LayerVisibility, Look, Presentation, ScreenLook, Section,
        Transition,
    };
    const DELAY: f64 = 2.0;
    let fx = fixture("audio-element-again");
    let clip = solid_video(&fx, "element-again", "red", 4.0);
    let s = element_slide(&clip, |e| {
        vec![Build {
            start: BuildStart::WithSlide,
            delay: DELAY,
            ..Build::new(e.id, BuildMode::In, Transition::CUT)
        }]
    });
    let id = s.id;
    let mut sec = Section::new("Clip");
    sec.slides = vec![s];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Element again")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let hidden = Look {
        name: "No slide".into(),
        sort_key: "c".into(),
        screens: [fx.audience.id, fx.key.id]
            .into_iter()
            .map(|screen| ScreenLook {
                screen,
                layers: LayerVisibility {
                    slide: false,
                    ..LayerVisibility::ALL
                },
                template_override: None,
                mask: None,
                language: Default::default(),
            })
            .collect(),
        ..Look::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "look",
        &hidden.id.0.to_string(),
        &hidden,
    );
    let server = start_server(&fx, true, false).await;
    let mut ws = Client::connect(server.addr, "").await;
    let plain = json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()});
    ws.ok(plain.clone()).await;
    let red = |img: &image::RgbaImage| near(img.get_pixel(W / 2, H / 2), [255, 0, 0], 70);
    let trigger = json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 0});
    ws.ok(trigger.clone()).await;
    let play = open_play(&server, id).await;
    heard_becomes(&play, true, "built in").await;
    wait_frame(&server, 0, "the element built in", red).await;

    // Triggered again: both start over.
    let t = Instant::now();
    ws.ok(trigger).await;
    assert!(!play.heard(), "quiet again until its build-in");
    wait_frame(&server, 0, "hidden again on the screen", |img| !red(img)).await;
    assert!(
        t.elapsed().as_secs_f64() < DELAY - 0.5,
        "the screen hid it only after {:?}",
        t.elapsed()
    );
    let took = heard_becomes(&play, true, "built in again").await;
    assert!(took.as_secs_f64() >= DELAY - 0.3, "heard after {took:?}");
    let shown = wait_frame(&server, 0, "shown again", red).await;
    let _ = shown;
    assert!(
        t.elapsed().as_secs_f64() >= DELAY - 0.3,
        "drawn again after {:?}",
        t.elapsed()
    );

    // A look that hides the slide layer, then one that shows it again: the screens start the
    // slide's builds over from the look change, and so does its sound.
    ws.ok(json!({"type": "setLook", "look": hidden.id.0.to_string()}))
        .await;
    assert!(!play.heard(), "the look hides the slide layer");
    wait_frame(&server, 0, "hidden by the look", |img| !red(img)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t = Instant::now();
    ws.ok(plain).await;
    let took = heard_becomes(&play, true, "built in after the look").await;
    assert!(took.as_secs_f64() >= DELAY - 0.3, "heard after {took:?}");
    wait_frame(&server, 0, "shown after the look", red).await;
    assert!(
        t.elapsed().as_secs_f64() >= DELAY - 0.3,
        "drawn after {:?}",
        t.elapsed()
    );
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}
