//! A live source's sound (G7 stage 3 item 3) end to end, with an in-process IDN source as the
//! camera sending pictures and sound with the same timecodes: the video-input layer's source
//! plays its sound on the program bus (null audio output), its flashes and beeps arrive
//! together (within a frame on the null output; at most a few frames late), the layer's mute and
//! clearing the layer silence it.
//!
//! ```text
//! sg render -c "cargo test -p vesper-server --test server live_sound:: -- --nocapture"
//! ```

use crate::common::*;
use output::{AudioSink, FrameSink, IdnSenderConfig, IdnSink};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use vesper_server::RunningServer;

/// A camera at 30 fps: black pictures with a white one every second, each picture with its
/// 1600 samples of 48 kHz stereo sound (a 1 kHz beep at −6 dBFS with the white one, silence
/// otherwise) carrying the picture's timecode. Returns its `127.0.0.1:port`.
fn camera(stop: Arc<AtomicBool>) -> (String, std::thread::JoinHandle<()>) {
    let mut sink = IdnSink::new(IdnSenderConfig::new(format!(
        "Sound Cam {}",
        std::process::id()
    )))
    .unwrap();
    let addr = format!("127.0.0.1:{}", sink.port());
    let mut sound = sink.audio_sender();
    let h = std::thread::spawn(move || {
        let (w, h) = (320u32, 180u32);
        let period = Duration::from_nanos(33_333_333);
        let t0 = Instant::now();
        let mut n = 0u32;
        let mut px = vec![0u8; (w * h * 4) as usize];
        while !stop.load(Ordering::Relaxed) {
            let flash = n % 30 == 15;
            let v = if flash { 255 } else { 0 };
            for p in px.as_chunks_mut::<4>().0 {
                p.copy_from_slice(&[v, v, v, 255]);
            }
            let tc = n as i64 * 333_333;
            let block: Vec<f32> = (0..1600)
                .flat_map(|i| {
                    let s = if flash {
                        0.5 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin()
                    } else {
                        0.0
                    };
                    [s, s]
                })
                .collect();
            let _ = sound.send(&block, 2, 48_000, tc);
            let _ = sink.send(&output::Frame {
                width: w,
                height: h,
                pixels: &px,
                alpha: output::AlphaMode::Opaque,
                frame_rate: output::FrameRate::FPS30,
                timecode: tc,
            });
            n += 1;
            if let Some(d) = (t0 + period * n).checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
        }
    });
    (addr, h)
}

async fn command(addr: std::net::SocketAddr, body: serde_json::Value) {
    let r = http(addr, "POST", "/api/v1/commands", &[], Some(body)).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
}

/// Beep onsets heard on the master bus and flash onsets shown on screen 0 (the instant the
/// screen's newest frame turned white, plus one 30 fps tick: frames are rendered a tick ahead)
/// over `dur`.
async fn onsets(server: &RunningServer, dur: Duration) -> (Vec<Instant>, Vec<Instant>) {
    let tap = server.shared.sound.engine.tap();
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    let heard = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut last: Option<Instant> = None;
        while !st.load(Ordering::Relaxed) {
            let Some(b) = tap.recv_timeout(Duration::from_millis(50)) else {
                continue;
            };
            for i in 0..b.frames {
                if b.data[i * b.channels].abs() > 0.05 {
                    let at = b.heard_at + Duration::from_secs_f64(i as f64 / 48_000.0);
                    if last.is_none_or(|l| at.duration_since(l) > Duration::from_millis(200)) {
                        out.push(at);
                    }
                    last = Some(at);
                }
            }
            tap.recycle(b);
        }
        out
    });
    let mut shown = Vec::new();
    let mut was = false;
    let mut seq = None;
    let end = Instant::now() + dur;
    while Instant::now() < end {
        let f = server.shared.screens()[0].latest.lock().unwrap().clone();
        if let Some(f) = f
            && Some(f.sequence) != seq
        {
            seq = Some(f.sequence);
            let white = f.pixel(f.width / 2, f.height / 2)[1] > 200;
            if white && !was {
                shown.push(Instant::now() + Duration::from_nanos(33_333_333));
            }
            was = white;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    stop.store(true, Ordering::Relaxed);
    let heard = tokio::task::spawn_blocking(move || heard.join().unwrap())
        .await
        .unwrap();
    (heard, shown)
}

async fn heard_for(server: &RunningServer, ms: u64) -> usize {
    let tap = server.shared.sound.engine.tap();
    tokio::task::spawn_blocking(move || {
        let end = Instant::now() + Duration::from_millis(ms);
        let mut loud = 0;
        while Instant::now() < end {
            if let Some(b) = tap.recv_timeout(Duration::from_millis(50)) {
                loud += b.data.iter().filter(|v| v.abs() > 0.05).count();
                tap.recycle(b);
            }
        }
        loud
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_source_plays_its_sound_with_its_picture() {
    let fx = fixture("live-sound");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let stop = Arc::new(AtomicBool::new(false));
    let (source, cam) = camera(stop.clone());
    command(addr, json!({"type": "triggerLiveSource", "source": source})).await;
    wait_frame(&server, 0, "the camera's picture", |i| {
        near(i.get_pixel(4, 4), [0, 0, 0], 30)
    })
    .await;
    // Sound comes (one beep a second).
    let deadline = Instant::now() + Duration::from_secs(10);
    while heard_for(&server, 1100).await == 0 {
        assert!(
            Instant::now() < deadline,
            "the source's sound was never heard"
        );
    }
    // Beeps are heard with their flashes: within about a frame, at most a few frames late.
    // Each beep is paired with the flash within half a beep period of it (a flash the test's
    // polling missed under load leaves its beep unpaired instead of pairing it with the next
    // second's flash).
    let (heard, shown) = onsets(&server, Duration::from_millis(4200)).await;
    let offsets: Vec<f64> = heard
        .iter()
        .filter_map(|a| {
            shown
                .iter()
                .map(|v| audio::secs_between(*v, *a) * 1000.0)
                .min_by(|x, y| x.abs().total_cmp(&y.abs()))
                .filter(|x| x.abs() < 500.0)
        })
        .collect();
    let stats = server
        .shared
        .live_sound
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| *p.stats.lock().unwrap())
        .expect("a live sound lane");
    eprintln!("live A/V offsets (ms, heard − shown): {offsets:.1?}; {stats:?}");
    assert!(heard.len() >= 3, "{} beeps heard", heard.len());
    assert!(shown.len() >= 3, "{} flashes shown", shown.len());
    assert!(
        offsets.len() >= 3,
        "{} beeps heard with a flash",
        offsets.len()
    );
    for x in &offsets {
        assert!((-40.0..=150.0).contains(x), "A/V offset {x:.1} ms");
    }
    // The drift loop steers: it locks on once (perhaps twice, as the pictures' transit
    // settles) and never jumps while it has sound. Running dry (an underrun: the sender or the
    // receive thread starved, as on a loaded machine) unlocks it, and each underrun costs one
    // lock more; a jump would be a lock without one.
    assert!(stats.locks <= 2 + stats.underruns, "{stats:?}");
    // The layer's mute (its own group) silences it; the state shows the level.
    command(
        addr,
        json!({"type": "setMute", "target": "layer", "layer": "videoInput", "muted": true}),
    )
    .await;
    let st = get_json(addr, "/api/v1/state").await;
    assert_eq!(
        st["state"]["audio"]["layers"]["videoInput"]["muted"],
        json!(true)
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(heard_for(&server, 1300).await, 0, "muted layer");
    command(
        addr,
        json!({"type": "setMute", "target": "layer", "layer": "videoInput", "muted": false}),
    )
    .await;
    assert!(heard_for(&server, 1300).await > 0, "unmuted");
    // Cleared: silence, no lane.
    command(addr, json!({"type": "clearLayer", "layer": "videoInput"})).await;
    assert!(server.shared.live_sound.lock().unwrap().is_none());
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(heard_for(&server, 1300).await, 0, "cleared");
    stop.store(true, Ordering::Relaxed);
    cam.join().unwrap();
    server.shutdown().await.unwrap();
}

/// One ADTS frame's AudioSpecificConfig (AAC object type, sampling frequency index, channel
/// configuration) and its raw frame.
fn adts_frames(adts: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 7 <= adts.len() {
        let h = &adts[i..];
        assert!(h[0] == 0xFF && h[1] & 0xF0 == 0xF0, "ADTS sync at {i}");
        let header = if h[1] & 1 == 1 { 7 } else { 9 };
        let object = (h[2] >> 6) + 1;
        let rate = (h[2] >> 2) & 0x0F;
        let channels = ((h[2] & 1) << 2) | (h[3] >> 6);
        let len = (((h[3] & 3) as usize) << 11) | ((h[4] as usize) << 3) | ((h[5] as usize) >> 5);
        let config = vec![
            (object << 3) | (rate >> 1),
            ((rate & 1) << 7) | (channels << 3),
        ];
        out.push((config, h[header..len].to_vec()));
        i += len;
    }
    out
}

/// NDI|HX sound: AAC frames (AudioSpecificConfig + one raw frame, as
/// `InputEvent::CompressedAudio` delivers them) decode with the media crate's FFmpeg adapter
/// into sound blocks with their timecodes: a second of a 1 kHz stereo tone comes back as about
/// a second of a 1 kHz tone; a broken frame is skipped and the next one decodes; a new config
/// (mono) opens a new decoder. The AAC is made by the system FFmpeg here (skipped without it).
#[test]
fn aac_sound_decodes_into_sound_blocks() {
    let dir = std::env::temp_dir().join(format!("vesper-aac-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let make = |channels: u32, name: &str| -> Option<Vec<u8>> {
        let path = dir.join(name);
        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg("sine=frequency=1000:sample_rate=48000:duration=1")
            .args([
                "-ac",
                &channels.to_string(),
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-f",
                "adts",
            ])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        ok.then(|| std::fs::read(&path).unwrap())
    };
    let Some(stereo) = make(2, "stereo.aac") else {
        eprintln!("skipped: no ffmpeg with an AAC encoder here");
        return;
    };
    let mono = make(1, "mono.aac").unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let decoder = output::LiveDecoder(media::network_decoder());
    let mut aac = output::AacSound::new(&decoder);
    let frames = adts_frames(&stereo);
    assert!(frames.len() > 40, "{} AAC frames", frames.len());
    let tick = |n: usize| (n as i64) * 1024 * 10_000_000 / 48_000;
    let mut blocks = Vec::new();
    for (n, (config, data)) in frames.iter().enumerate() {
        if n == 10 {
            // A broken frame: skipped, the stream carries on.
            assert!(aac.decode(config, &[0xFF; 40], 48_000, 0).is_none());
        }
        if let Some(s) = aac.decode(config, data, 48_000, tick(n)) {
            blocks.push(s);
        }
    }
    assert!(
        blocks.len() + 3 >= frames.len(),
        "{} of {} frames",
        blocks.len(),
        frames.len()
    );
    assert!(
        blocks
            .iter()
            .all(|b| b.channels.len() == 2 && b.sample_rate == 48_000)
    );
    assert!(blocks.windows(2).all(|w| w[1].timecode > w[0].timecode));
    let left: Vec<f32> = blocks.iter().flat_map(|b| b.channels[0].clone()).collect();
    assert!(
        (46_000..=52_000).contains(&left.len()),
        "{} samples for a second",
        left.len()
    );
    // The tone: 1 kHz (rising zero crossings over the middle half second), at the sine
    // source's level (amplitude 1/8, RMS 0.088; 3 dB less once FFmpeg makes it stereo).
    let mid = &left[12_000..36_000];
    let rising = mid.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    assert!((495..=505).contains(&rising), "{rising} cycles in 0.5 s");
    let rms = (mid.iter().map(|v| v * v).sum::<f32>() / mid.len() as f32).sqrt();
    assert!((0.05..0.1).contains(&rms), "RMS {rms}");

    // Mono: another config, a new decoder.
    let mono = adts_frames(&mono);
    assert_ne!(mono[0].0, frames[0].0);
    let mut got = None;
    for (c, d) in mono.iter().take(6) {
        got = aac.decode(c, d, 48_000, 0).or(got);
    }
    assert_eq!(got.expect("mono sound").channels.len(), 1);
}
