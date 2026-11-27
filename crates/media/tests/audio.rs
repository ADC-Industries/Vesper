//! Audio decoding and resampling with synthetic files made by the system `ffmpeg` CLI: sine
//! tones at known frequencies (44.1 kHz → 48 kHz), a stereo left / right identification tone,
//! WAV / MP3 / AAC, a ramp whose value is its own timestamp (positions after seeks and
//! loops), a video's soundtrack, and a sample-for-sample comparison with FFmpeg's own decode
//! and resample (what the decoder before the IDN port produced, bit for bit).

mod common;

use common::TempDir;
use media::{AUDIO_RATE, AudioOptions, AudioTap, Loops, Player, PlayerOptions, open_audio};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn ff(args: &[&str]) {
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

/// A tone file: `expr` is an `aevalsrc` expression (channels separated by `|`).
fn tone(path: &Path, expr: &str, rate: u32, secs: f64, codec: &[&str]) {
    let src = format!("aevalsrc={expr}:s={rate}:d={secs}");
    let mut args = vec!["-f", "lavfi", "-i", &src];
    args.extend_from_slice(codec);
    let p = path.to_string_lossy().into_owned();
    args.push(&p);
    ff(&args);
}

/// Reads `frames` frames at `pos`, waiting (up to 5 s) until the decoder delivered them.
fn read_wait(tap: &mut AudioTap, pos: i64, frames: usize) -> Vec<f32> {
    let ch = tap.channels();
    let mut out = vec![0.0; frames * ch];
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let ready = tap
            .buffered()
            .is_some_and(|(s, e)| s <= pos && e >= pos + frames as i64)
            || tap.end().is_some_and(|e| e <= pos + frames as i64);
        if ready || Instant::now() > deadline {
            break;
        }
        // Release what lies before `pos` (a reader that jumps ahead more than the ring holds).
        tap.read(pos, &mut out, 0);
        std::thread::sleep(Duration::from_millis(2));
    }
    tap.read(pos, &mut out, frames);
    out
}

/// Frequency of one channel by counting rising zero crossings (with interpolation).
fn frequency(x: &[f32], ch: usize, c: usize) -> f64 {
    let s: Vec<f32> = x.iter().skip(c).step_by(ch).copied().collect();
    let mut crossings = Vec::new();
    for i in 1..s.len() {
        if s[i - 1] < 0.0 && s[i] >= 0.0 {
            crossings.push((i - 1) as f64 + (-s[i - 1] / (s[i] - s[i - 1])) as f64);
        }
    }
    assert!(crossings.len() > 10, "no tone on channel {c}");
    let n = crossings.len() - 1;
    n as f64 * AUDIO_RATE as f64 / (crossings[n] - crossings[0])
}

fn peak(x: &[f32], ch: usize, c: usize) -> f32 {
    x.iter()
        .skip(c)
        .step_by(ch)
        .fold(0.0f32, |m, v| m.max(v.abs()))
}

fn opts() -> AudioOptions {
    AudioOptions::default()
}

#[test]
fn a_44k1_sine_stays_1khz_after_resampling_in_every_codec() {
    let dir = TempDir::new("audio-sine");
    for (ext, codec) in [
        ("wav", &["-c:a", "pcm_s16le"][..]),
        ("mp3", &["-c:a", "libmp3lame", "-b:a", "192k"][..]),
        ("m4a", &["-c:a", "aac", "-b:a", "192k"][..]),
    ] {
        let p = dir.path().join(format!("sine.{ext}"));
        tone(&p, "0.5*sin(2*PI*1000*t)", 44_100, 3.0, codec);
        let info = media::probe_audio(&p).unwrap().unwrap();
        assert_eq!(info.sample_rate, 44_100, "{ext}");
        assert!(
            (info.duration.unwrap() - 3.0).abs() < 0.1,
            "{ext}: {info:?}"
        );
        let (_dec, mut tap) = open_audio(&p, opts(), None).unwrap().unwrap();
        assert_eq!(tap.channels(), 2);
        // One second from the middle.
        let x = read_wait(&mut tap, AUDIO_RATE as i64 / 2, AUDIO_RATE as usize);
        for c in 0..2 {
            let f = frequency(&x, 2, c);
            assert!((f - 1000.0).abs() < 0.5, "{ext} ch{c}: {f} Hz");
            // Mono is copied to both channels at the same level.
            let a = peak(&x, 2, c);
            assert!((a - 0.5).abs() < 0.03, "{ext} ch{c}: peak {a}");
        }
    }
}

#[test]
fn stereo_left_and_right_stay_apart() {
    let dir = TempDir::new("audio-lr");
    let p = dir.path().join("lr.wav");
    tone(
        &p,
        "0.4*sin(2*PI*440*t)|0.2*sin(2*PI*880*t)",
        44_100,
        2.0,
        &["-c:a", "pcm_s16le"],
    );
    let (_dec, mut tap) = open_audio(&p, opts(), None).unwrap().unwrap();
    let x = read_wait(&mut tap, 4800, 48_000);
    assert!((frequency(&x, 2, 0) - 440.0).abs() < 0.5);
    assert!((frequency(&x, 2, 1) - 880.0).abs() < 0.5);
    assert!((peak(&x, 2, 0) - 0.4).abs() < 0.02);
    assert!((peak(&x, 2, 1) - 0.2).abs() < 0.02);
}

/// A ramp whose value is `t / 10` (its own timestamp in tenths of seconds).
fn ramp(dir: &TempDir, secs: f64) -> std::path::PathBuf {
    let p = dir.path().join("ramp.wav");
    tone(&p, "t/10", 48_000, secs, &["-c:a", "pcm_f32le"]);
    p
}

fn value_time(x: &[f32], ch: usize, i: usize) -> f64 {
    x[i * ch] as f64 * 10.0
}

#[test]
fn seeking_lands_on_the_exact_sample() {
    let dir = TempDir::new("audio-seek");
    let p = ramp(&dir, 6.0);
    let (_dec, mut tap) = open_audio(&p, opts(), None).unwrap().unwrap();
    let x = read_wait(&mut tap, 0, 480);
    assert!(value_time(&x, 2, 0).abs() < 1e-3);
    for t in [3.25f64, 1.0, 5.5, 0.1] {
        let s = (t * AUDIO_RATE as f64) as i64;
        tap.seek(s);
        let x = read_wait(&mut tap, s, 480);
        let v = value_time(&x, 2, 0);
        assert!((v - t).abs() < 0.0005, "seek to {t}: reads {v}");
        let v = value_time(&x, 2, 479);
        assert!((v - (t + 479.0 / 48_000.0)).abs() < 0.0005);
    }
    // Starting at a timeline time.
    let (_d, mut tap) = open_audio(
        &p,
        AudioOptions {
            start: 2.0,
            ..opts()
        },
        None,
    )
    .unwrap()
    .unwrap();
    let x = read_wait(&mut tap, 96_000, 10);
    assert!((value_time(&x, 2, 0) - 2.0).abs() < 0.0005);
    // The end is known once decoded.
    let x = read_wait(&mut tap, 6 * 48_000 - 100, 200);
    assert!((value_time(&x, 2, 0) - (6.0 - 100.0 / 48_000.0)).abs() < 0.001);
    assert_eq!(x[150 * 2], 0.0, "silence after the end");
    assert_eq!(tap.end(), Some(6 * 48_000));
}

#[test]
fn loops_restart_at_the_in_point_with_a_fixed_pass_length() {
    let dir = TempDir::new("audio-loop");
    let p = ramp(&dir, 4.0);
    // In 1.0 → out 2.5: passes of 1.5 s, three of them.
    let (_dec, mut tap) = open_audio(
        &p,
        AudioOptions {
            in_point: 1.0,
            out_point: Some(2.5),
            loops: Loops::Count(3),
            ..opts()
        },
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(tap.region_len(), Some(72_000));
    for k in 0..3i64 {
        let x = read_wait(&mut tap, k * 72_000, 480);
        assert!((value_time(&x, 2, 0) - 1.0).abs() < 0.0005, "pass {k}");
        let x = read_wait(&mut tap, k * 72_000 + 71_000, 1000);
        assert!((value_time(&x, 2, 999) - (2.5 - 1.0 / 48_000.0)).abs() < 0.0005);
    }
    let x = read_wait(&mut tap, 3 * 72_000, 100);
    assert!(x.iter().all(|v| *v == 0.0));
    assert_eq!(tap.end(), Some(3 * 72_000));
    // A caller-fixed pass length pads a short stream with silence. (Fails with IDN
    // v0.2.0-Alpha91: read on without a seek, the next play starts where the sound ends
    // instead of at the pass length. Reported to the IDN maintainers; left failing until fixed.)
    let (_dec, mut tap) = open_audio(
        &p,
        AudioOptions {
            loops: Loops::Forever,
            ..opts()
        },
        Some(5.0),
    )
    .unwrap()
    .unwrap();
    let x = read_wait(&mut tap, 4 * 48_000 + 10, 100);
    assert!(x.iter().all(|v| *v == 0.0), "padding");
    let x = read_wait(&mut tap, 5 * 48_000, 100);
    assert!(value_time(&x, 2, 0).abs() < 0.0005, "second pass");
    // Seeking into a later pass.
    tap.seek(11 * 48_000);
    let x = read_wait(&mut tap, 11 * 48_000, 10);
    assert!((value_time(&x, 2, 0) - 1.0).abs() < 0.0005);
}

#[test]
fn a_player_decodes_its_videos_soundtrack() {
    let dir = TempDir::new("audio-video");
    let p = dir.path().join("av.mp4");
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=320x180:r=30:d=3",
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.5*sin(2*PI*1000*t):s=44100:d=3.5",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        p.to_str().unwrap(),
    ]);
    let player = Player::open(
        &p,
        PlayerOptions {
            audio: Some(2),
            loops: Loops::Count(2),
            mode: Some(media::DecodeMode::Software),
            ..PlayerOptions::default()
        },
    )
    .unwrap();
    // Picture and sound both loop at the file's duration (3.5 s: the longer soundtrack's).
    let file_len = media::probe(&p).unwrap().duration.unwrap();
    assert!((file_len - 3.5).abs() < 0.04, "{file_len}");
    assert!((player.region_len().unwrap() - file_len).abs() < 1e-9);
    let mut tap = player.take_audio().expect("soundtrack");
    assert!(player.take_audio().is_none());
    let len = tap.region_len().unwrap();
    assert!((len as f64 / 48_000.0 - player.region_len().unwrap()).abs() < 1e-3);
    let x = read_wait(&mut tap, 24_000, 24_000);
    assert!((frequency(&x, 2, 0) - 1000.0).abs() < 1.0);
    // Second pass sounds too.
    let x = read_wait(&mut tap, len + 4_800, 9_600);
    assert!(peak(&x, 2, 0) > 0.4);
    // No sound requested: none decoded.
    let silent = Player::open(&p, PlayerOptions::default()).unwrap();
    assert!(silent.take_audio().is_none());
    assert!((silent.region_len().unwrap() - file_len).abs() < 0.04);
    // A file without sound.
    let q = dir.path().join("v.mp4");
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=s=320x180:r=30:d=1",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        q.to_str().unwrap(),
    ]);
    assert!(media::probe_audio(&q).unwrap().is_none());
    let v = Player::open(
        &q,
        PlayerOptions {
            audio: Some(2),
            ..PlayerOptions::default()
        },
    )
    .unwrap();
    assert!(v.take_audio().is_none());
}

/// FFmpeg's own decode of `path`, resampled to 48 kHz mono float (`ffmpeg -ar 48000`).
fn ffmpeg_reference(path: &Path) -> Vec<f32> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-ar", "48000", "-ac", "1", "-f", "f32le", "pipe:1"])
        .output()
        .expect("system ffmpeg");
    assert!(out.status.success());
    out.stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

/// The whole timeline of a tap (until its end), left channel.
fn read_all(tap: &mut AudioTap) -> Vec<f32> {
    let mut all = Vec::new();
    let mut pos = 0i64;
    while tap.end().is_none_or(|e| pos < e) && pos < 60 * 48_000 {
        let x = read_wait(tap, pos, 4800);
        all.extend(x.iter().step_by(tap.channels()));
        pos += 4800;
    }
    all.truncate(tap.end().unwrap_or(0) as usize);
    all
}

/// A chirp (no period, so any offset shows), 3 s at 44.1 kHz mono, as WAV and MP3, with
/// FFmpeg's own decode of it resampled to 48 kHz.
fn chirps(dir: &TempDir) -> Vec<(&'static str, std::path::PathBuf, Vec<f32>)> {
    [
        ("wav", &["-c:a", "pcm_s16le"][..]),
        ("mp3", &["-c:a", "libmp3lame", "-b:a", "192k"][..]),
    ]
    .into_iter()
    .map(|(ext, codec)| {
        let p = dir.path().join(format!("chirp.{ext}"));
        tone(&p, "0.5*sin(2*PI*(100+300*t)*t)", 44_100, 3.0, codec);
        let reference = ffmpeg_reference(&p);
        assert_eq!(reference.len(), 144_000, "{ext}");
        (ext, p, reference)
    })
    .collect()
}

/// Every sample as FFmpeg decodes and resamples it, to the last one (bit for bit, as the
/// decoder before the IDN port did). Fails with IDN v0.2.0-Alpha91: the last samples are
/// missing (17 of a WAV, 1202 of an MP3). Reported to the IDN maintainers; left failing.
#[test]
fn file_sound_matches_ffmpeg_sample_for_sample() {
    let dir = TempDir::new("audio-exact");
    let mut failures = Vec::new();
    for (ext, p, reference) in chirps(&dir) {
        let (_d, mut tap) = open_audio(&p, opts(), None).unwrap().unwrap();
        let got = read_all(&mut tap);
        assert_eq!(got.len(), reference.len(), "{ext}: length");
        let bad: Vec<usize> = (0..got.len())
            .filter(|&k| (got[k] - reference[k]).abs() > 1e-6)
            .collect();
        if !bad.is_empty() {
            failures.push(format!(
                "{ext}: {} samples differ from FFmpeg's decode, first {:?} last {:?}",
                bad.len(),
                bad.first(),
                bad.last()
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// From an in point (0.7 s): FFmpeg's samples from there on, after the decoder's warm-up (an
/// MP3 frame needs the one before it: 25 ms). Fails with IDN v0.2.0-Alpha91 for MP3 (the
/// sound starts about 1100 samples late). Reported to the IDN maintainers; left failing.
#[test]
fn in_point_matches_ffmpeg_sample_for_sample() {
    let dir = TempDir::new("audio-exact-in");
    let mut failures = Vec::new();
    for (ext, p, reference) in chirps(&dir) {
        let (_d, mut tap) = open_audio(
            &p,
            AudioOptions {
                in_point: 0.7,
                ..opts()
            },
            None,
        )
        .unwrap()
        .unwrap();
        let got = read_all(&mut tap);
        let skip = 33_600;
        assert_eq!(
            got.len(),
            reference.len() - skip,
            "{ext}: length from 0.7 s"
        );
        // Up to the end FFmpeg's resampler flushes (see the test above).
        let bad: Vec<usize> = (1200..got.len() - 1300)
            .filter(|&k| (got[k] - reference[skip + k]).abs() > 0.01)
            .collect();
        if !bad.is_empty() {
            failures.push(format!(
                "{ext} from 0.7 s: {} samples off, first {:?}",
                bad.len(),
                bad.first()
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
