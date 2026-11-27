//! Decoding, pacing, looping and seeking on synthetic videos (generated with the system
//! `ffmpeg` CLI). Hardware tests need VA-API on /dev/dri/renderD128 (run with `sg render`);
//! without access they are skipped with a message. (`hw` is whether GPU decoding is expected:
//! IDN's reader does not say whether it fell back to software.)

mod common;

use common::*;
use media::{DecodeMode, Loops, Player, PlayerOptions};
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(20);

fn opts(mode: DecodeMode) -> PlayerOptions {
    PlayerOptions {
        mode: Some(mode),
        ..Default::default()
    }
}

/// Decodes the first `n` frames at their own timestamps and checks the counter (and `hw`,
/// when given).
fn check_frames(p: &Player, fps: f64, n: u32, hw: Option<bool>) {
    for i in 0..n {
        let t = i as f64 / fps;
        let f = p.wait_frame_for(t, WAIT).expect("frame");
        assert_eq!(read_counter(&f), i, "frame {i} at t={t}: {f:?}");
        assert!((f.t - t).abs() < 1e-3, "t {} vs {t}", f.t);
        if let Some(hw) = hw {
            assert_eq!(f.hw, hw, "frame {i}: hw");
        }
        assert!(grey_level(&f) > 100, "frame {i} is dark");
    }
}

fn vaapi_or_skip(test: &str) -> bool {
    if !render_node_accessible() {
        eprintln!("{test}: /dev/dri/renderD128 not accessible (run under `sg render`); skipped");
        return false;
    }
    assert!(
        media::hardware_available(),
        "{test}: render node accessible but VA-API failed to open"
    );
    true
}

#[test]
fn h264_and_hevc_decode_with_vaapi() {
    if !vaapi_or_skip("h264_and_hevc_decode_with_vaapi") {
        return;
    }
    let dir = TempDir::new("hw");
    for (codec, name) in [
        (Codec::H264, "h264.mp4"),
        (Codec::Hevc, "hevc.mp4"),
        (Codec::Hevc10, "hevc10.mp4"),
    ] {
        let path = counter_video(dir.path(), name, 1920, 1080, (30, 1), 45, codec, 15);
        let p = Player::open(&path, opts(DecodeMode::Auto)).unwrap();
        check_frames(&p, 30.0, 45, Some(true));
        let info = p.info();
        assert!(info.hw, "{name}: {info:?}");
        assert_eq!((info.width, info.height), (1920, 1080));
        assert!((info.fps - 30.0).abs() < 0.01);
        assert_eq!(info.matrix, media::Matrix::Bt709);
        let f = p.wait_frame_for(0.0, WAIT).unwrap();
        let want = match codec {
            Codec::Hevc10 => media::PlaneFormat::P010,
            _ => media::PlaneFormat::Nv12,
        };
        assert_eq!(f.format, want, "{name}");
        let st = p.stats();
        assert!(
            st.hw && st.errors == 0 && st.fallback.is_none(),
            "{name}: {st:?}"
        );
        eprintln!(
            "{name}: {} decoded, GPU expected ({:?})",
            info.codec,
            media::hardware_decoder()
        );
    }
}

#[test]
fn software_decode_when_forced_or_unsupported() {
    let dir = TempDir::new("sw");
    for (codec, name) in [(Codec::H264, "h264.mp4"), (Codec::Hevc, "hevc.mp4")] {
        let path = counter_video(dir.path(), name, 1280, 720, (25, 1), 30, codec, 10);
        let p = Player::open(&path, opts(DecodeMode::Software)).unwrap();
        check_frames(&p, 25.0, 30, Some(false));
        assert!(!p.info().hw);
        assert!(!p.stats().hw);
    }
    // H.264 4:2:2 has no VA-API profile: Auto falls back to software per stream (when VA-API
    // exists at all) and the chroma is repacked to 4:2:0. Whether it fell back is not visible
    // (IDN's reader does not say; reported to the IDN maintainers), so only the pictures are
    // checked.
    let path = counter_video(
        dir.path(),
        "h264-422.mp4",
        640,
        360,
        (25, 1),
        20,
        Codec::H264_422,
        10,
    );
    let p = Player::open(&path, opts(DecodeMode::Auto)).unwrap();
    check_frames(&p, 25.0, 20, None);
    assert_eq!(p.stats().errors, 0);
}

#[test]
fn pacing_25fps_media_on_30fps_screen() {
    let dir = TempDir::new("p25");
    // 10 s of 25 fps media = 250 frames; sampled by a 30 fps screen for 10 s = 300 ticks.
    let path = counter_video(
        dir.path(),
        "c25.mp4",
        640,
        360,
        (25, 1),
        250,
        Codec::H264,
        50,
    );
    let p = Player::open(&path, PlayerOptions::default()).unwrap();
    let mut seen = Vec::new();
    for n in 0..300u32 {
        let t = n as f64 / 30.0;
        let f = p.wait_frame_for(t, WAIT).expect("frame");
        let idx = read_counter(&f);
        // The nearest frame (ties, which fall on exact half frames, may go either way).
        let exact = t * 25.0;
        assert!(
            (idx as f64 - exact).abs() <= 0.5 + 1e-6,
            "tick {n}: frame {idx} for t={t:.4} (exact {exact:.3})"
        );
        seen.push(idx);
    }
    // Every source frame is shown, in order: 250 distinct frames over 300 ticks, each step
    // 0 (a repeat) or 1, so exactly 50 repeats and no skipped frame.
    let repeats = seen.windows(2).filter(|w| w[1] == w[0]).count();
    assert!(
        seen.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1),
        "{seen:?}"
    );
    assert_eq!(seen.first(), Some(&0));
    assert_eq!(seen.last(), Some(&249));
    assert_eq!(repeats, 50);
    // One repeat in every six ticks.
    for chunk in seen.windows(2).collect::<Vec<_>>().chunks(6) {
        assert!(chunk.iter().filter(|w| w[1] == w[0]).count() <= 2);
    }
    let st = p.stats();
    assert_eq!(st.skipped, 0, "{st:?}");
    assert_eq!(st.shown, 250);
}

#[test]
fn pacing_60fps_media_on_30fps_screen() {
    let dir = TempDir::new("p60");
    let path = counter_video(
        dir.path(),
        "c60.mp4",
        640,
        360,
        (60, 1),
        240,
        Codec::H264,
        60,
    );
    let p = Player::open(&path, PlayerOptions::default()).unwrap();
    for n in 0..120u32 {
        let f = p.wait_frame_for(n as f64 / 30.0, WAIT).expect("frame");
        assert_eq!(read_counter(&f), 2 * n, "tick {n}");
    }
    let st = p.stats();
    assert_eq!(st.shown, 120);
    assert!(st.skipped >= 115, "{st:?}");
}

#[test]
fn loop_point_is_seamless() {
    let dir = TempDir::new("loop");
    // 2 s at 25 fps = 50 frames, played 3 times, sampled at 30 Hz.
    let path = counter_video(dir.path(), "c.mp4", 640, 360, (25, 1), 50, Codec::H264, 25);
    let p = Player::open(
        &path,
        PlayerOptions {
            loops: Loops::Count(3),
            ..Default::default()
        },
    )
    .unwrap();
    let mut last_t = -1.0;
    for n in 0..180u32 {
        let t = n as f64 / 30.0;
        let f = p.wait_frame_for(t, WAIT).expect("frame");
        assert!(
            grey_level(&f) > 100,
            "black frame at tick {n} (t={t:.3}): {f:?}"
        );
        let idx = read_counter(&f) as f64;
        let want = (t * 25.0).round() % 50.0;
        let ok = idx == want || (t * 25.0 - (t * 25.0).floor() - 0.5).abs() < 1e-6;
        assert!(ok, "tick {n}: frame {idx}, want {want} (t={t:.4})");
        assert!(f.t >= last_t, "timeline went back at tick {n}");
        last_t = f.t;
        assert!(
            (f.t - t).abs() <= 0.5 / 25.0 + 1e-6,
            "tick {n}: t {} vs {t}",
            f.t
        );
    }
    assert_eq!(p.region_len().map(|l| (l * 1000.0).round()), Some(2000.0));
    let end = p.wait_frame_for(6.5, WAIT).unwrap();
    assert_eq!(read_counter(&end), 49, "holds the last frame");
    assert!(p.finished_at(6.0) && !p.finished_at(5.95));
    assert_eq!(p.stats().skipped, 0);
}

#[test]
fn in_and_out_points_loop() {
    let dir = TempDir::new("inout");
    let path = counter_video(dir.path(), "c.mp4", 640, 360, (25, 1), 75, Codec::H264, 25);
    // Frames 10..30 (0.4 s .. 1.2 s), forever.
    let p = Player::open(
        &path,
        PlayerOptions {
            in_point: 0.4,
            out_point: Some(1.2),
            loops: Loops::Forever,
            ..Default::default()
        },
    )
    .unwrap();
    for n in 0..100u32 {
        let t = n as f64 / 25.0;
        let f = p.wait_frame_for(t, WAIT).expect("frame");
        assert_eq!(read_counter(&f), 10 + n % 20, "tick {n}");
    }
}

#[test]
fn seek_lands_within_one_frame() {
    let dir = TempDir::new("seek");
    let path = counter_video(dir.path(), "c.mp4", 640, 360, (25, 1), 300, Codec::H264, 50);
    let p = Player::open(&path, PlayerOptions::default()).unwrap();
    assert_eq!(read_counter(&p.wait_frame_for(0.0, WAIT).unwrap()), 0);
    for target in [7.37, 0.52, 11.2, 3.0, 9.98, 0.0, 6.04] {
        p.seek(target);
        let f = p.wait_frame_for(target, WAIT).expect("frame after seek");
        let idx = read_counter(&f);
        let exact = target * 25.0;
        assert!(
            (idx as f64 - exact).abs() <= 0.5 + 1e-6,
            "seek {target}: frame {idx} (exact {exact})"
        );
        assert!((f.t - target).abs() <= 0.5 / 25.0 + 1e-6);
        // Plays on from there.
        let g = p.wait_frame_for(target + 0.2, WAIT).unwrap();
        assert_eq!(read_counter(&g), idx + 5, "after seek {target}");
    }
}

#[test]
fn frame_for_never_blocks_and_counts_late() {
    let dir = TempDir::new("late");
    let path = counter_video(dir.path(), "c.mp4", 640, 360, (25, 1), 50, Codec::H264, 25);
    let p = Player::open(&path, PlayerOptions::default()).unwrap();
    let first = p.wait_frame_for(0.0, WAIT).unwrap();
    assert_eq!(read_counter(&first), 0);
    // Far ahead of the decoder: the newest decoded frame comes back at once, counted late.
    let t0 = std::time::Instant::now();
    let f = p.frame_for(1.9).unwrap();
    assert!(t0.elapsed() < Duration::from_millis(50));
    assert!(read_counter(&f) < 49);
    assert!(p.stats().late >= 1);
}

#[test]
fn thumbnail_and_probe() {
    let dir = TempDir::new("thumb");
    let path = counter_video(dir.path(), "c.mp4", 640, 360, (25, 1), 100, Codec::H264, 25);
    let info = media::probe(&path).unwrap();
    assert_eq!(info.codec, "h264");
    assert!((info.duration.unwrap() - 4.0).abs() < 0.05, "{info:?}");
    let th = media::thumbnail(&path, 0.1, 160).unwrap();
    assert_eq!((th.width, th.height), (160, 90));
    // The picture at 10 %: frame 10 (its counter's first block row, in RGB).
    let mut n = 0;
    for bit in 0..BITS {
        let x = ((bit * 16 + 8) * 160 / 192) as usize;
        let y = (90 * 12 / 108) as usize;
        n = (n << 1) | (th.rgb[(y * 160 + x) * 3] > 128) as u32;
    }
    assert_eq!(n, 10, "thumbnail shows frame {n}");
    // Fails with IDN v0.2.0-Alpha91: a thumbnail's `file_time` is 0 whatever it shows.
    // Reported to the IDN maintainers; left failing until fixed.
    assert!((th.at - 0.4).abs() <= 0.04 + 1e-6, "{}", th.at);
    assert_eq!(th.rgb.len(), 160 * 90 * 3);
}
