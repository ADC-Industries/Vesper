//! Video playback and transitions through the real server: triggering videos from a bin,
//! transport commands, retrigger rules, end behaviours, fades, dissolves on the slide and
//! media layers, a retrigger in the middle of a dissolve, a push timed on the screen clock, and
//! frame pacing on a screen.
//! Videos are synthetic (system `ffmpeg` CLI): frame `n` shows `n` in binary as blocks across
//! the top quarter.

use crate::common::*;
use model::{
    AssetLocation, EndBehavior, Fade, MediaAsset, MediaBin, MediaBinItem, MediaBinItemId,
    MediaBinKind, MediaKind, MediaLayer, MediaTrigger, Playback, Repeat, Retrigger,
};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};
use vesper_server::RunningServer;

const BITS: u32 = 12;

/// A counter video (`frames` frames at `fps`, 640×360, H.264) at `path`.
fn counter_video(path: &Path, fps: u32, frames: u32) {
    let src = format!(
        "color=c=gray:s=192x108:r={fps},format=yuv420p,\
         geq=lum='if(lt(Y,27),if(bitand(N,pow(2,{BITS}-1-floor(X/16))),235,16),126)':cb=128:cr=128,\
         scale=640:360:flags=neighbor"
    );
    let out = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &src,
        ])
        .args([
            "-frames:v",
            &frames.to_string(),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ])
        .args([
            "-pix_fmt",
            "yuv420p",
            "-g",
            &fps.to_string(),
            "-colorspace",
            "bt709",
        ])
        .arg(path)
        .output()
        .expect("system ffmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The counter shown on a rendered 640×360 frame (`None` when the blocks are not there).
fn read_counter(img: &image::RgbaImage) -> Option<u32> {
    let mut n = 0;
    for bit in 0..BITS {
        let x = (bit * 16 + 8) * 640 / 192;
        let p = img.get_pixel(x, 45).0;
        // Blocks are white (255) or black (0); anything else is not the counter (a blend).
        let on = match p[0] {
            0..=40 => false,
            215..=255 => true,
            _ => return None,
        };
        n = (n << 1) | on as u32;
    }
    // The grey area below.
    let g = img.get_pixel(320, 300).0[0];
    (100..=160).contains(&g).then_some(n)
}

/// Grey level below the counter (the video's mid grey ≈ 128 at full opacity).
fn grey(img: &image::RgbaImage) -> u8 {
    img.get_pixel(320, 300).0[0]
}

struct Clip {
    asset: MediaAsset,
}

/// Adds a counter video asset (with its own bin item and playback settings).
fn add_clip(
    fx: &Fixture,
    name: &str,
    fps: u32,
    frames: u32,
    playback: Playback,
) -> (Clip, MediaBinItem) {
    counter_video(
        &fx.dir.join("media").join(format!("{name}.mp4")),
        fps,
        frames,
    );
    let asset = MediaAsset {
        name: name.into(),
        kind: MediaKind::Video,
        location: AssetLocation::Managed {
            sha256: name.into(),
            extension: "mp4".into(),
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
    let item = MediaBinItem {
        id: MediaBinItemId::new(),
        name: None,
        enabled: true,
        trigger: MediaTrigger {
            playback,
            ..MediaTrigger::new(asset.id, MediaLayer::Background)
        },
    };
    (Clip { asset }, item)
}

fn write_bin(fx: &Fixture, items: Vec<MediaBinItem>) {
    let bin = MediaBin {
        name: "Videos".into(),
        kind: MediaBinKind::Manual { items },
        ..MediaBin::default()
    };
    write_doc(&fx.dir, "portable", "mediaBin", &bin.id.0.to_string(), &bin);
}

fn once(end: EndBehavior) -> Playback {
    Playback {
        end_behavior: end,
        ..Playback::default()
    }
}

fn media(state: &Value) -> &Value {
    &state["layers"]["media"]
}

/// Frames of screen 0 as they are published, for `dur` (sequence numbers must not repeat).
async fn record(server: &RunningServer, dur: Duration) -> Vec<(Instant, image::RgbaImage)> {
    record_seq(server, dur)
        .await
        .into_iter()
        .map(|(t, _, f)| (t, f))
        .collect()
}

/// [`record`] with each frame's sequence on its screen (consecutive renders have consecutive
/// sequences: a gap is a frame this sampler missed, not one the server skipped).
async fn record_seq(
    server: &RunningServer,
    dur: Duration,
) -> Vec<(Instant, u64, image::RgbaImage)> {
    let mut out = Vec::new();
    let mut last = None;
    let end = Instant::now() + dur;
    while Instant::now() < end {
        let f = server.shared.screens()[0].latest.lock().unwrap().clone();
        if let Some(f) = f
            && Some(f.sequence) != last
        {
            last = Some(f.sequence);
            out.push((Instant::now(), f.sequence, f.to_straight_rgba()));
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transport_commands_update_state_and_picture() {
    let fx = fixture("video-transport");
    let (clip, item) = add_clip(&fx, "clip", 25, 100, once(EndBehavior::HoldLastFrame));
    write_bin(&fx, vec![item]);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;

    a.ok(json!({"type": "triggerMedia", "asset": clip.asset.id.0.to_string()}))
        .await;
    let m = media(&a.state);
    assert_eq!(m["kind"], "video", "{m}");
    assert_eq!(m["online"], true);
    assert_eq!(m["playback"]["state"], "playing");
    let d = m["playback"]["duration"].as_f64().unwrap();
    assert!((d - 4.0).abs() < 0.05, "duration {d}");
    let t = &a.state["transport"];
    assert_eq!(t["layer"], "media");
    assert_eq!(t["name"], "clip");
    assert_eq!(t["rate"], 1.0);
    assert!(t["hw"].is_boolean());
    // The picture plays: counters advance.
    let img = wait_frame(&server, 0, "video on screen", |img| {
        read_counter(img).is_some_and(|n| n >= 10)
    })
    .await;
    assert!(read_counter(&img).unwrap() < 60);

    // Pause: the state says so, the position and the picture hold.
    a.ok(json!({"type": "mediaPause"})).await;
    assert_eq!(a.state["transport"]["state"], "paused");
    let p0 = a.state["transport"]["position"].as_f64().unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let n0 = read_counter(&latest(&server, 0).unwrap());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        read_counter(&latest(&server, 0).unwrap()),
        n0,
        "paused picture holds"
    );
    a.ok(json!({"type": "mediaToggle"})).await; // → playing
    assert_eq!(a.state["transport"]["state"], "playing");
    a.ok(json!({"type": "mediaToggle"})).await; // → paused
    let p1 = a.state["transport"]["position"].as_f64().unwrap();
    assert!((p1 - p0).abs() < 0.2, "{p0} → {p1}");

    // Seek while paused: the picture shows that frame (within one frame).
    a.ok(json!({"type": "mediaSeek", "seconds": 2.0})).await;
    assert!((a.state["transport"]["position"].as_f64().unwrap() - 2.0).abs() < 0.01);
    wait_frame(&server, 0, "frame 50 after the seek", |img| {
        read_counter(img).is_some_and(|n| (49..=51).contains(&n))
    })
    .await;
    // Skip back 1.5 s.
    a.ok(json!({"type": "mediaSkip", "seconds": -1.5})).await;
    assert!((a.state["transport"]["position"].as_f64().unwrap() - 0.5).abs() < 0.01);
    wait_frame(&server, 0, "frame 12 after the skip", |img| {
        read_counter(img).is_some_and(|n| (11..=13).contains(&n))
    })
    .await;

    // Play: positions are broadcast while it plays, at most 4 a second.
    a.ok(json!({"type": "mediaPlay"})).await;
    let start_rev = a.revision;
    let t0 = Instant::now();
    let mut positions = Vec::new();
    while t0.elapsed() < Duration::from_secs(2) {
        let m = a.next_any().await;
        if m["type"] == "state" {
            positions.push(m["state"]["transport"]["position"].as_f64().unwrap());
        }
        a.absorb(&m);
    }
    let n = (a.revision - start_rev) as usize;
    assert!((5..=10).contains(&n), "{n} states in 2 s: {positions:?}");
    assert!(positions.windows(2).all(|w| w[1] > w[0]), "{positions:?}");

    // Restart: back near the in point, playing.
    a.ok(json!({"type": "mediaRestart"})).await;
    assert!(a.state["transport"]["position"].as_f64().unwrap() < 0.1);
    assert_eq!(a.state["transport"]["state"], "playing");
    wait_frame(&server, 0, "restarted", |img| {
        read_counter(img).is_some_and(|n| n < 10)
    })
    .await;

    // Stop: the layer clears.
    a.ok(json!({"type": "mediaStop"})).await;
    assert!(media(&a.state).is_null());
    assert!(a.state["transport"].is_null());
    let r = a.command(json!({"type": "mediaPlay"})).await;
    assert_eq!(r["error"]["code"], "conflict");
    // Players are released once nothing shows them.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(server.shared.videos.lock().unwrap().is_empty());
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn end_behaviour_and_retrigger_rules() {
    let fx = fixture("video-end");
    let (hold, i1) = add_clip(&fx, "hold", 25, 25, once(EndBehavior::HoldLastFrame));
    let (clear, i2) = add_clip(&fx, "clear", 25, 25, once(EndBehavior::Clear));
    let lp = |r: Retrigger| Playback {
        repeat: Repeat::Loop,
        retrigger: r,
        ..Playback::default()
    };
    let (auto, i3) = add_clip(&fx, "auto", 25, 50, lp(Retrigger::Automatic));
    let (always, i4) = add_clip(&fx, "always", 25, 50, lp(Retrigger::Always));
    let (never, i5) = add_clip(&fx, "never", 25, 50, lp(Retrigger::Never));
    let (black, i6) = add_clip(&fx, "black", 25, 25, once(EndBehavior::Black));
    write_bin(&fx, vec![i1, i2, i3, i4, i5, i6]);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    let trig = |c: &Clip| json!({"type": "triggerMedia", "asset": c.asset.id.0.to_string()});

    // Hold the last frame: stopped, still on screen.
    a.ok(trig(&hold)).await;
    a.wait_state("hold ends", |_, s| s["transport"]["state"] == "stopped")
        .await;
    assert_eq!(media(&a.state)["asset"], hold.asset.id.0.to_string());
    let img = wait_frame(&server, 0, "last frame held", |img| {
        read_counter(img) == Some(24)
    })
    .await;
    assert!(grey(&img) > 100);
    // Retrigger (automatic) of an ended video restarts it.
    a.ok(trig(&hold)).await;
    assert_eq!(a.state["transport"]["state"], "playing");
    assert!(a.state["transport"]["position"].as_f64().unwrap() < 0.2);

    // Clear at the end: the layer empties.
    a.ok(trig(&clear)).await;
    a.wait_state("clear ends", |_, s| media(s).is_null()).await;
    wait_frame(&server, 0, "nothing on screen", |img| grey(img) < 10).await;

    // Black at the end: black, layer stays.
    a.ok(trig(&black)).await;
    a.wait_state("black ends", |_, s| s["transport"]["state"] == "stopped")
        .await;
    wait_frame(&server, 0, "black", |img| {
        img.get_pixel(320, 300).0[..3] == [0, 0, 0] && read_counter(img).is_none()
    })
    .await;

    // Retrigger: automatic keeps playing, always restarts, never ignores.
    for (clip, restarts) in [(&auto, false), (&always, true), (&never, false)] {
        a.ok(trig(clip)).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        a.ok(trig(clip)).await;
        let pos = a.state["transport"]["position"].as_f64().unwrap();
        if restarts {
            assert!(pos < 0.3, "{}: restarts ({pos})", clip.asset.name);
        } else {
            assert!(pos > 0.6, "{}: keeps playing ({pos})", clip.asset.name);
        }
        assert_eq!(a.state["transport"]["loop"], true);
    }
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fades_in_and_out() {
    let fx = fixture("video-fade");
    let fade = |s: Option<f64>| Fade {
        enabled: true,
        seconds: s,
    };
    let pb = Playback {
        fade_in: fade(Some(1.0)),
        // No length: the show default (0.5 s).
        fade_out: fade(None),
        ..Playback::default()
    };
    let (clip, item) = add_clip(&fx, "fade", 25, 75, pb);
    write_bin(&fx, vec![item]);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "triggerMedia", "asset": clip.asset.id.0.to_string()}))
        .await;
    let v = server
        .shared
        .videos
        .lock()
        .unwrap()
        .values()
        .next()
        .cloned()
        .unwrap();
    let start = Instant::now() - Duration::from_secs_f64(v.time_at(Instant::now()));
    let at = |s: f64| start + Duration::from_secs_f64(s);
    // Fade in over 1 s (linear), out over the last 0.5 s of the 3 s clip.
    for (t, want) in [
        (0.0, 0.0),
        (0.25, 0.25),
        (0.5, 0.5),
        (1.5, 1.0),
        (2.75, 0.5),
        (2.95, 0.1),
    ] {
        let o = v.view(at(t)).opacity as f64;
        assert!((o - want).abs() < 0.03, "opacity at {t}: {o} (want {want})");
    }
    // On screen: dim at first, full later.
    let early = record(&server, Duration::from_millis(400)).await;
    let late = wait_frame(&server, 0, "full grey", |img| grey(img) > 120).await;
    assert!(
        grey(&early[0].1) < grey(&late),
        "{} vs {}",
        grey(&early[0].1),
        grey(&late)
    );
    // Clearing a video with a fade-out dissolves over its length.
    a.ok(json!({"type": "clearLayer", "layer": "media"})).await;
    let tr = &a.state["transitions"];
    assert_eq!(tr[0]["layer"], "media", "{tr}");
    assert_eq!(tr[0]["duration"], 0.5);
    server.shutdown().await.unwrap();
}

/// White pixels in the text band.
fn text_white(img: &image::RgbaImage) -> usize {
    count(img, 80, 280, WHITE, 40)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slide_dissolve_and_retrigger_mid_transition() {
    let fx = fixture("video-dissolve");
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    let song = fx.second.id.0.to_string();
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 0}))
        .await;
    let full = wait_frame(&server, 0, "B1", |img| text_white(img) > 100).await;
    let n_full = text_white(&full);

    // Clear with a 1 s dissolve: frames in between are partly there.
    a.ok(json!({"type": "setTransitionOverride", "layer": "slide",
                "transition": {"kind": {"type": "dissolve"}, "duration": 1.0}}))
        .await;
    a.ok(json!({"type": "clearLayer", "layer": "slide"})).await;
    assert_eq!(a.state["transitions"][0]["duration"], 1.0);
    let frames = record(&server, Duration::from_millis(1300)).await;
    // The text's brightest pixel through the dissolve: from 255 down to 0, gradually.
    let peak = |img: &image::RgbaImage| {
        (80..280)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .map(|(x, y)| img.get_pixel(x, y).0[0])
            .max()
            .unwrap()
    };
    let peaks: Vec<u8> = frames.iter().map(|(_, f)| peak(f)).collect();
    assert!(peaks.len() >= 20, "{} frames", peaks.len());
    assert!(
        peaks.windows(2).all(|w| w[1] <= w[0].saturating_add(2)),
        "{peaks:?}"
    );
    assert!(
        peaks.iter().any(|p| (150..=220).contains(p)),
        "a mid-dissolve frame: {peaks:?}"
    );
    assert_eq!(*peaks.last().unwrap(), 0);
    // Midway (0.5 s) the white text is at 50 % in linear light: sRGB ≈ 188.
    let t0 = frames[0].0;
    let mid = frames
        .iter()
        .min_by_key(|(t, _)| ((*t - t0).as_secs_f64() * 1000.0 - 500.0).abs() as i64)
        .unwrap();
    let p = peak(&mid.1);
    assert!((150..=225).contains(&p), "midpoint peak {p}");

    // Trigger, then trigger again mid-way: no jump between consecutive frames.
    a.ok(
        json!({"type": "triggerSlide", "presentation": song, "index": 0,
                "transition": {"kind": {"type": "dissolve"}, "duration": 0.6}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = record(&server, Duration::from_millis(60)).await;
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 1}))
        .await;
    let after = record(&server, Duration::from_millis(300)).await;
    let last_before = &before.last().unwrap().1;
    let first_after = &after[0].1;
    let diff = |x: &image::RgbaImage, y: &image::RgbaImage| {
        x.pixels()
            .zip(y.pixels())
            .map(|(p, q)| (p.0[0] as i32 - q.0[0] as i32).unsigned_abs() as u64)
            .sum::<u64>() as f64
            / (W * H) as f64
    };
    let step = diff(last_before, first_after);
    let whole =
        diff(&full, &latest(&server, 0).unwrap()).max(diff(&full, &image::RgbaImage::new(W, H)));
    assert!(
        step < whole * 0.2,
        "jump at the retrigger: {step:.2} (full change {whole:.2})"
    );
    let _ = n_full;
    server.shutdown().await.unwrap();
}

/// A push plays on the screen clock: the text moves out to the left, steadily, and is gone
/// once the transition's duration has passed (not before, not much later).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn push_completes_in_its_duration_on_the_screen_clock() {
    let fx = fixture("video-push");
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    let song = fx.second.id.0.to_string();
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 0}))
        .await;
    let full = wait_frame(&server, 0, "B1", |img| text_white(img) > 100).await;
    // Leftmost and rightmost white pixel of the text band.
    let span = |img: &image::RgbaImage| {
        let xs: Vec<u32> = (80..280)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .filter(|&(x, y)| img.get_pixel(x, y).0[0] > 200)
            .map(|(x, _)| x)
            .collect();
        Some((*xs.iter().min()?, *xs.iter().max()?))
    };
    let (_, right0) = span(&full).unwrap();

    a.ok(json!({"type": "setTransitionOverride", "layer": "slide",
                "transition": {"kind": {"type": "push", "direction": "left"},
                               "duration": 0.8, "easing": "linear"}}))
        .await;
    let start = Instant::now();
    a.ok(json!({"type": "clearLayer", "layer": "slide"})).await;
    assert_eq!(a.state["transitions"][0]["kind"]["type"], "push");
    let frames = record(&server, Duration::from_millis(1200)).await;
    assert!(frames.len() >= 15, "{} frames", frames.len());
    // The text's right edge moves left, never back.
    let rights: Vec<(f64, Option<u32>)> = frames
        .iter()
        .map(|(t, f)| ((*t - start).as_secs_f64(), span(f).map(|s| s.1)))
        .collect();
    let shown: Vec<u32> = rights.iter().filter_map(|r| r.1).collect();
    assert!(
        shown.windows(2).all(|w| w[1] <= w[0] + 2),
        "moves left: {rights:?}"
    );
    // It moves at the push's speed (the screen width in 0.8 s, linear).
    for (t, r) in &rights {
        if let Some(r) = r {
            let want = right0 as f64 - W as f64 * t / 0.8;
            assert!(
                (*r as f64 - want).abs() < 50.0,
                "at {t:.3} s: {r} vs {want:.0}: {rights:?}"
            );
        }
    }
    assert!(rights.iter().any(|r| r.1.is_some_and(|x| x < right0 - 100)));
    // The screen renders every tick until the push ends, then stops: its last new frame
    // comes when the duration has passed.
    let last = rights.last().unwrap().0;
    assert!(
        (0.7..1.0).contains(&last),
        "last frame at {last:.3} s: {rights:?}"
    );
    assert!(span(&frames.last().unwrap().1).is_none());
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn video_frames_are_paced_on_the_screen_clock() {
    let fx = fixture("video-pace");
    // 25 fps media on the 30 fps Audience screen, looping.
    let (clip, item) = add_clip(
        &fx,
        "pace",
        25,
        125,
        Playback {
            repeat: Repeat::Loop,
            ..Playback::default()
        },
    );
    write_bin(&fx, vec![item]);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerMedia", "asset": clip.asset.id.0.to_string()}))
        .await;
    wait_frame(&server, 0, "video", |img| read_counter(img).is_some()).await;
    let late = || {
        let st = server.shared.screens()[0].stats.lock().unwrap().clone();
        st.late_renders + st.late_ticks
    };
    let late_before = late();
    let frames = record_seq(&server, Duration::from_secs(3)).await;
    // Ticks the server itself reports as late (a loaded machine): each may skip a picture.
    let late = late() - late_before;
    let counters: Vec<(u64, u32)> = frames
        .iter()
        .filter_map(|(_, seq, f)| read_counter(f).map(|c| (*seq, c)))
        .collect();
    // 25 new pictures a second (the 30 fps clock repeats one in six; repeats are not
    // re-rendered). Counted by the screen's render sequence, so pictures this test's sampler
    // missed under load still count (M-3).
    let renders = counters.last().map_or(0, |l| l.0 + 1 - counters[0].0);
    assert!(
        renders + late >= 70,
        "{renders} frames in 3 s ({late} late ticks)"
    );
    // Each new picture is the next source frame (wrapping at the loop), judged between
    // consecutive renders: no source frame is skipped. (A render of the same picture again,
    // which a loaded machine can cause, skips nothing.)
    let skips = counters
        .windows(2)
        .filter(|w| w[1].0 == w[0].0 + 1 && w[1].1 != w[0].1 && w[1].1 != (w[0].1 + 1) % 125)
        .count();
    eprintln!(
        "pacing: {renders} new frames in 3 s ({} seen), {skips} skipped/out of order, {late} late ticks/renders",
        counters.len(),
    );
    assert!(
        skips as u64 <= 2 + late,
        "{skips} source frames skipped ({late} late): {counters:?}"
    );
    server.shutdown().await.unwrap();
}

/// A slide element whose media fill is a video plays it in its box, timed from when the slide
/// went live, without a media-layer play.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn video_element_plays_on_its_slide() {
    use model::{Presentation, Rect, Section, Slide};
    let fx = fixture("video-element");
    let (clip, _) = add_clip(&fx, "element", 30, 300, Playback::default());
    // The element covers the canvas, so the counter reads as on the media layer.
    let mut e = model::Element::media(Rect::new(0.0, 0.0, 1920.0, 1080.0), clip.asset.id);
    if let model::FillKind::Media { display, .. } = &mut e.fill.kind {
        display.scale = model::ScaleMode::Stretch;
    }
    let mut slide = Slide::new();
    slide.elements = vec![e];
    let mut sec = Section::new("Clip");
    sec.slides = vec![slide];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Clip slide")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 0}))
        .await;
    // The player opens off the render thread; then frames follow the clock.
    wait_frame(&server, 0, "a counter", |img| read_counter(img).is_some()).await;
    assert!(media(&a.state).is_null(), "no media-layer play");
    // Measured on the slide's play once it is open: until then the render thread may draw a
    // muted player of its own, timed from when the slide went live, which the play takes over
    // from (M-3: under load that hand-over fell inside the measured second; the play now
    // starts at the slide's time, see `video_element_starts_at_the_slides_time`).
    let deadline = Instant::now() + Duration::from_secs(10);
    while !server
        .shared
        .element_plays
        .lock()
        .unwrap()
        .values()
        .any(|p| p.player().is_some())
    {
        assert!(Instant::now() < deadline, "the slide's play never opened");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let frames = record(&server, Duration::from_millis(1000)).await;
    let counts: Vec<(Instant, u32)> = frames
        .iter()
        .filter_map(|(t, f)| read_counter(f).map(|n| (*t, n)))
        .collect();
    assert!(counts.len() >= 15, "{} counted frames", counts.len());
    assert!(counts.windows(2).all(|w| w[1].1 >= w[0].1), "{counts:?}");
    let (first, last) = (counts[0], counts[counts.len() - 1]);
    let rate = (last.1 - first.1) as f64 / (last.0 - first.0).as_secs_f64();
    assert!(
        (20.0..40.0).contains(&rate),
        "{rate:.1} frames/s: {counts:?}"
    );
    server.shutdown().await.unwrap();
}

/// A counter video with sound (see [`counter_video`]; a 1 kHz beep for 0.1 s at each of
/// `beeps`, s) added as asset `name`.
fn counter_av_clip(fx: &Fixture, name: &str, fps: u32, frames: u32, beeps: &[f64]) -> MediaAsset {
    let dir = fx.dir.join("media");
    let picture = dir.join(format!("{name}-picture.mp4"));
    counter_video(&picture, fps, frames);
    let on = beeps
        .iter()
        .map(|b| format!("between(t,{b:.3},{:.3})", b + 0.0999))
        .collect::<Vec<_>>()
        .join("+");
    let sound = format!(
        "aevalsrc='0.5*sin(2*PI*1000*t)*({on})':s=44100:d={}",
        frames as f64 / fps as f64
    );
    let out = dir.join(format!("{name}.mp4"));
    crate::audio::ff(&[
        "-i",
        picture.to_str().unwrap(),
        "-f",
        "lavfi",
        "-i",
        &sound,
        "-c:v",
        "copy",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        out.to_str().unwrap(),
    ]);
    std::fs::remove_file(&picture).unwrap();
    crate::audio::add_asset(fx, name, "mp4", MediaKind::Video)
}

/// The owner's decision for the hand-over from a muted player to the play with sound (M-3):
/// a video element's play starts at the slide's time (the seconds since the slide went live,
/// the muted players' clock), not at its in point. A looping clip with sound is added to the
/// live slide 2.5 s after it went live (past one pass): the picture on the screen follows the
/// slide's time and never goes backwards, and its beeps are heard at their place on the same
/// timeline. A clip that does not repeat and ended before its play opened holds its last frame
/// and is not heard.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn video_element_starts_at_the_slides_time() {
    use model::{AudioSettings, FillKind, Layer, Presentation, Rect, Section, Slide};
    let fx = fixture("video-element-offset");
    const FPS: f64 = 30.0;
    let beeps = [0.25, 1.25];
    let looping = counter_av_clip(&fx, "offset-loop", 30, 60, &beeps);
    let ending = counter_av_clip(&fx, "offset-once", 30, 60, &beeps);
    let element = |asset: &MediaAsset, repeat: Repeat| {
        let mut e = model::Element::media(Rect::new(0.0, 0.0, 1920.0, 1080.0), asset.id);
        if let FillKind::Media {
            display,
            playback,
            audio,
            ..
        } = &mut e.fill.kind
        {
            display.scale = model::ScaleMode::Stretch;
            *playback = Playback {
                repeat,
                ..Playback::default()
            };
            *audio = AudioSettings::default();
        }
        e
    };
    let (s0, s1) = (Slide::new(), Slide::new());
    let ids = [s0.id, s1.id];
    let mut sec = Section::new("Clip");
    sec.slides = vec![s0, s1];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Clip later")
    };
    let pid = p.id.0.to_string();
    write_doc(&fx.dir, "portable", "presentation", &pid, &p);
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    let add = |slide: model::SlideId, e: model::Element| {
        let pid = pid.clone();
        async move {
            let r = http(
                addr,
                "POST",
                &format!("/api/v1/presentations/{pid}/edits"),
                &[],
                Some(json!({"clientId": "test", "ops": [
                    {"type": "addElement", "slide": slide.0.to_string(), "element": e}
                ]})),
            )
            .await;
            assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        }
    };
    let open_play = |slide: model::SlideId| {
        let shared = server.shared.clone();
        async move {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let p = shared
                    .element_plays
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|((s, _), _)| *s == slide)
                    .map(|(_, p)| p.clone());
                if let Some(p) = p.filter(|p| p.player().is_some()) {
                    return (p, Instant::now());
                }
                assert!(Instant::now() < deadline, "the play never opened");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    };
    let went_live = |server: &RunningServer| server.shared.state().changes[&Layer::Slide].at;

    // The looping clip, added past its first pass while the screen and the sound are recorded.
    a.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 0}))
        .await;
    let since = went_live(&server);
    tokio::time::sleep_until((since + Duration::from_millis(2500)).into()).await;
    let (heard, _) = crate::audio::collect_frames(
        &server,
        std::sync::Arc::new(|_| None),
        Duration::from_millis(3500),
    );
    let (frames, (play, opened)) =
        tokio::join!(record(&server, Duration::from_millis(3500)), async {
            add(ids[0], element(&looping, Repeat::Loop)).await;
            open_play(ids[0]).await
        });
    let heard = tokio::task::spawn_blocking(move || heard.join().unwrap().audio)
        .await
        .unwrap();
    let pass = play.player().unwrap().region_len().expect("pass length");
    assert!((pass - 2.0).abs() < 0.01, "pass {pass}");
    let slide_time = |at: Instant| at.saturating_duration_since(since).as_secs_f64();
    eprintln!(
        "the play opened at {:.2} s of the slide",
        slide_time(opened)
    );
    // The picture: each counter on the slide's timeline (unwrapped to the nearest pass).
    let pictures: Vec<(f64, f64)> = frames
        .iter()
        .filter_map(|(t, f)| read_counter(f).map(|n| (slide_time(*t), n as f64 / FPS)))
        .map(|(expect, pic)| (expect, pic + ((expect - pic) / pass).round() * pass))
        .collect();
    assert!(pictures.len() >= 30, "{} counted frames", pictures.len());
    // Sampled after it was rendered (for a present instant a tick ahead): within a few frames.
    let lags: Vec<f64> = pictures.iter().map(|(e, p)| e - p).collect();
    let worst = lags.iter().fold(0.0f64, |m, l| m.max(l.abs()));
    eprintln!(
        "picture off the slide's time: worst {:.0} ms over {} frames, first shown at {:.2} s",
        worst * 1000.0,
        pictures.len(),
        pictures[0].1
    );
    assert!(
        worst < 0.2,
        "the picture is off the slide's time: {lags:.3?}"
    );
    let back: Vec<_> = pictures
        .windows(2)
        .filter(|w| w[1].1 < w[0].1 - 1e-6)
        .collect();
    assert!(back.is_empty(), "the picture went backwards: {back:?}");
    // The sound: each beep heard at its place on the same timeline (the sound is the clock).
    // A beep the play opened in the middle of is cut, not an onset: from just after it opened.
    let off: Vec<f64> = heard
        .iter()
        .filter(|h| **h > opened + Duration::from_millis(250))
        .map(|h| {
            let t = slide_time(*h).rem_euclid(pass);
            beeps
                .iter()
                .map(|b| t - b)
                .min_by(|x, y| x.abs().total_cmp(&y.abs()))
                .unwrap()
                * 1000.0
        })
        .collect();
    eprintln!("beeps heard off the slide's time (ms): {off:.1?}");
    assert!(off.len() >= 2, "beeps heard: {}", off.len());
    assert!(
        off.iter().all(|x| x.abs() <= 1.5 * 1000.0 / FPS),
        "sound off the picture's timeline: {off:.1?}"
    );

    // A clip that plays once, added after its end: its last frame, no sound.
    a.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 1}))
        .await;
    let since = went_live(&server);
    tokio::time::sleep_until((since + Duration::from_millis(2500)).into()).await;
    add(ids[1], element(&ending, Repeat::Once)).await;
    let (play, _) = open_play(ids[1]).await;
    let v = play.player().unwrap();
    assert!(
        v.time_at(Instant::now()) >= v.end().unwrap(),
        "ended when it opened"
    );
    let (heard, _) = crate::audio::collect_frames(
        &server,
        std::sync::Arc::new(|_| None),
        Duration::from_millis(1300),
    );
    let heard = tokio::task::spawn_blocking(move || heard.join().unwrap().audio)
        .await
        .unwrap();
    assert!(heard.is_empty(), "an ended clip is heard: {heard:?}");
    wait_frame(&server, 0, "the last frame", |img| {
        read_counter(img) == Some(59)
    })
    .await;
    server.shutdown().await.unwrap();
}

/// Scrolling text moves on the screen clock from when its slide goes live: the screen renders
/// every tick, and the text enters from the right at its speed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scrolling_text_runs_on_the_screen_clock() {
    use model::{Presentation, Rect, Section, Slide};
    let fx = fixture("video-scroll");
    let mut e = text_element(
        Rect::new(0.0, 400.0, 1920.0, 300.0),
        "NEWS",
        model::Color::WHITE,
    );
    // 600 canvas px/s = 200 px/s on the 640 px screen.
    e.scroll = model::TextScroll {
        enabled: true,
        speed: 600.0,
        start: model::ScrollStart::Offscreen,
        repeat: false,
        ..model::TextScroll::default()
    };
    let mut slide = Slide::new();
    slide.elements = vec![e];
    let mut sec = Section::new("Ticker");
    sec.slides = vec![slide];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        ..Presentation::new("Ticker")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "setLook", "look": fx.plain_look.id.0.to_string()}))
        .await;
    a.ok(json!({"type": "setTransitionOverride", "layer": "slide",
                "transition": {"kind": {"type": "cut"}}}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": p.id.0.to_string(), "index": 0}))
        .await;
    let frames = record(&server, Duration::from_millis(1500)).await;
    assert!(
        frames.len() >= 25,
        "renders every tick: {} frames",
        frames.len()
    );
    let left = |img: &image::RgbaImage| {
        (130..235)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .filter(|&(x, y)| img.get_pixel(x, y).0[0] > 200)
            .map(|(x, _)| x)
            .min()
    };
    let seen: Vec<(Instant, u32)> = frames
        .iter()
        .filter_map(|(t, f)| left(f).map(|x| (*t, x)))
        .collect();
    assert!(seen.len() >= 10, "{} frames with text", seen.len());
    // Entering from the right: it starts near the right edge and only moves left.
    assert!(seen[0].1 > W - 120, "first seen at {}", seen[0].1);
    assert!(seen.windows(2).all(|w| w[1].1 <= w[0].1));
    // At about 200 px/s.
    let (a0, b0) = (seen[0], seen[seen.len() - 1]);
    let speed = (a0.1 - b0.1) as f64 / (b0.0 - a0.0).as_secs_f64();
    assert!((160.0..240.0).contains(&speed), "{speed:.0} px/s");
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transition_resolution_order() {
    use model::{Presentation, Section, Slide, Transition, TransitionKind, VendorEffect};
    let fx = fixture("video-resolve");
    // A song with a presentation transition (vendor: plays as a dissolve) and one slide with
    // its own.
    let mut own = Slide::new();
    own.transition = Some(Transition::dissolve(0.3));
    let mut sec = Section::new("V");
    sec.slides = vec![own, Slide::new()];
    let p = Presentation {
        library: Some(fx.library.id),
        sections: vec![sec],
        transition: Some(Transition {
            kind: TransitionKind::Vendor(VendorEffect {
                name: "Color Warp".into(),
                ..Default::default()
            }),
            duration: 0.7,
            ..Default::default()
        }),
        ..Presentation::new("Transitions")
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    let pid = p.id.0.to_string();
    let second = fx.second.id.0.to_string();
    let slide_tr = |s: &Value| {
        s["transitions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["layer"] == "slide")
            .map(|t| {
                (
                    t["kind"]["type"].as_str().unwrap().to_string(),
                    t["duration"].as_f64().unwrap(),
                )
            })
    };
    let d = |kind: &str, s: f64| Some((kind.to_string(), s));
    // The slide's own.
    a.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 0}))
        .await;
    assert_eq!(slide_tr(&a.state), d("dissolve", 0.3));
    // The presentation's (vendor → dissolve).
    a.ok(json!({"type": "triggerSlide", "presentation": pid, "index": 1}))
        .await;
    assert_eq!(slide_tr(&a.state), d("dissolve", 0.7));
    // The command's beats both.
    a.ok(
        json!({"type": "triggerSlide", "presentation": pid, "index": 0,
                "transition": {"kind": {"type": "dissolve"}, "duration": 0.9}}),
    )
    .await;
    assert_eq!(slide_tr(&a.state), d("dissolve", 0.9));
    // No slide / presentation transition: the layer override, then the show default (cut).
    a.ok(json!({"type": "setTransitionOverride", "layer": "slide",
                "transition": {"kind": {"type": "dissolve"}, "duration": 1.2}}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": second, "index": 0}))
        .await;
    assert_eq!(slide_tr(&a.state), d("dissolve", 1.2));
    a.ok(json!({"type": "setTransitionOverride", "layer": "slide", "transition": null}))
        .await;
    a.ok(json!({"type": "triggerSlide", "presentation": second, "index": 1}))
        .await;
    assert_eq!(slide_tr(&a.state), None, "the fixture's default cuts");
    let r = a
        .command(json!({"type": "setTransitionOverride", "layer": "audio", "transition": null}))
        .await;
    assert_eq!(r["error"]["code"], "badRequest");
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loop_on_and_off_continues_the_playing_video() {
    let fx = fixture("video-loop");
    let looping = Playback {
        repeat: Repeat::Loop,
        ..Playback::default()
    };
    let (clip, item) = add_clip(&fx, "loopme", 25, 50, looping);
    write_bin(&fx, vec![item]);
    let server = start_server(&fx, true, false).await;
    let mut a = Client::connect(server.addr, "").await;
    a.ok(json!({"type": "triggerMedia", "asset": clip.asset.id.0.to_string()}))
        .await;
    assert_eq!(a.state["transport"]["loop"], true);
    tokio::time::sleep(Duration::from_millis(800)).await;
    // Loop off: it goes on from where it is, then ends (holding its last frame).
    a.ok(json!({"type": "mediaSetLoop", "loop": false})).await;
    assert_eq!(a.state["transport"]["loop"], false);
    let pos = a.state["transport"]["position"].as_f64().unwrap();
    assert!(pos > 0.5, "continues, not restarted ({pos})");
    assert_eq!(a.state["transport"]["state"], "playing");
    a.wait_state("plays to its end", |_, s| {
        s["transport"]["state"] == "stopped"
    })
    .await;
    wait_frame(&server, 0, "last frame held", |img| {
        read_counter(img) == Some(49)
    })
    .await;
    // Loop on for the ended video: it plays again, and again.
    a.ok(json!({"type": "mediaSetLoop", "loop": true, "layer": "media"}))
        .await;
    assert_eq!(a.state["transport"]["loop"], true);
    a.wait_state("plays again", |_, s| s["transport"]["state"] == "playing")
        .await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    a.ok(json!({"type": "mediaPause"})).await;
    assert_eq!(
        a.state["transport"]["state"], "paused",
        "still going after 2.5 s"
    );
    // Paused: the toggle keeps it paused.
    a.ok(json!({"type": "mediaSetLoop", "loop": false})).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    a.ok(json!({"type": "mediaSetLoop", "loop": false})).await;
    assert_eq!(a.state["transport"]["state"], "paused");
    // Nothing playing on the audio layer.
    let r = a
        .command(json!({"type": "mediaSetLoop", "loop": true, "layer": "audio"}))
        .await;
    assert_eq!(r["error"]["code"], "conflict", "{r}");
    server.shutdown().await.unwrap();
}
