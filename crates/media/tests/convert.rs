//! What IDN's converter (`media::convert`, media preparation) writes, checked on a synthetic
//! clip made by the test (lavfi, the system `ffmpeg` CLI): what live playback wants of a
//! prepared file.
//!
//! - **Fast start:** the MP4's index (`moov`) before the media data (`mdat`), so a player (or
//!   another machine fetching it) can start without reading to the end. Read from the file's
//!   top-level boxes here.
//! - **Keyframes about every 2 s:** a seek or a loop lands on a keyframe nearby (a cue that
//!   starts mid-file, a loop point), so it starts at once and cleanly. Read with `ffprobe`
//!   (the packets' key flags, no decoding).
//!
//! Each encoder this machine has is checked (the CPU's, and the GPU's where there is one; run
//! with `sg render` for VA-API). A regression test: should IDN's converter stop doing either,
//! the change goes to the IDN maintainers as a black-box note, never fixed up after the conversion.

mod common;

use common::*;
use media::convert::{ConvertOptions, convert};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

/// The longest keyframe interval live playback wants (about 2 s, some room for a rate's
/// rounding).
const MAX_KEYFRAME_GAP: f64 = 2.5;

/// A 12 s, 640×360, 30 fps clip with sound, its keyframes 10 s apart and its index at the end
/// (FFmpeg's MP4 default), so neither is inherited from the source.
fn source(dir: &Path) -> PathBuf {
    let path = dir.join("source.mp4");
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=12"])
        .args(["-f", "lavfi", "-i", "sine=f=440:r=44100:d=12"])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-g", "300", "-keyint_min", "300", "-sc_threshold", "0"])
        .args(["-c:a", "aac", "-shortest"])
        .arg(&path)
        .output()
        .expect("running the system ffmpeg");
    assert!(
        out.status.success(),
        "making the source clip: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    path
}

/// The file's top-level MP4 boxes, in order (`size` 1: a 64-bit size follows; 0: to the end).
fn top_level_boxes(path: &Path) -> Vec<String> {
    let data = std::fs::read(path).unwrap();
    let mut boxes = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as u64;
        let kind = String::from_utf8_lossy(&data[at + 4..at + 8]).into_owned();
        let size = match size {
            0 => (data.len() - at) as u64,
            1 => {
                assert!(at + 16 <= data.len(), "box {kind} at {at}: truncated size");
                u64::from_be_bytes(data[at + 8..at + 16].try_into().unwrap())
            }
            s => s,
        };
        assert!(size >= 8, "box {kind} at {at}: size {size}");
        boxes.push(kind);
        at += size as usize;
    }
    boxes
}

/// The video's keyframe times (s, ascending) and the clip's end (the last packet's end), from
/// `ffprobe`'s packets.
fn keyframes(path: &Path) -> (Vec<f64>, f64) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-show_entries", "packet=pts_time,duration_time,flags"])
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .expect("running the system ffprobe");
    assert!(
        out.status.success(),
        "ffprobe: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut keys = Vec::new();
    let mut end = 0f64;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let f: Vec<&str> = line.split(',').collect();
        let (Some(t), Some(d), Some(flags)) = (
            f.first().and_then(|s| s.parse::<f64>().ok()),
            f.get(1).and_then(|s| s.parse::<f64>().ok()),
            f.get(2),
        ) else {
            continue;
        };
        if flags.contains('K') {
            keys.push(t);
        }
        end = end.max(t + d);
    }
    keys.sort_by(f64::total_cmp);
    (keys, end)
}

/// The longest stretch without a keyframe, the clip's end included (a loop or seek near the
/// end).
fn longest_gap(keys: &[f64], end: f64) -> f64 {
    keys.windows(2)
        .map(|w| w[1] - w[0])
        .chain(keys.last().map(|k| end - k))
        .fold(0f64, f64::max)
}

#[test]
fn mp4_box_order_and_gaps() {
    // The checks' own helpers, on known input.
    assert_eq!(longest_gap(&[0.0, 2.0, 4.0], 5.0), 2.0);
    assert_eq!(longest_gap(&[0.0, 2.0], 5.5), 3.5);
    let dir = TempDir::new("boxes");
    let p = dir.path().join("b.mp4");
    let mut data = Vec::new();
    for (kind, len) in [(b"ftyp", 16u32), (b"moov", 8), (b"mdat", 24)] {
        data.extend(len.to_be_bytes());
        data.extend(kind);
        data.resize(data.len() + len as usize - 8, 0);
    }
    // A 64-bit `mdat`, then one running to the end.
    data.extend(1u32.to_be_bytes());
    data.extend(b"mdat");
    data.extend(20u64.to_be_bytes());
    data.extend([0; 4]);
    data.extend(0u32.to_be_bytes());
    data.extend(b"free");
    data.extend([0; 3]);
    std::fs::write(&p, data).unwrap();
    assert_eq!(
        top_level_boxes(&p),
        ["ftyp", "moov", "mdat", "mdat", "free"]
    );
}

/// Converts the source with each encoder here and checks the box order and the keyframes
/// (IDN Beta16: `moov` first and a keyframe every 2.0 s, with libx264 and VA-API; its
/// documentation doesn't promise either, so this keeps it so).
#[test]
fn converted_files_start_fast_and_seek_cleanly() {
    let dir = TempDir::new("convert");
    let src = source(dir.path());
    let (gpu, cpu) = media::convert::encoders(false);
    assert!(cpu.is_some() || gpu.is_some(), "no H.264 encoder here");
    let mut problems = Vec::new();
    for use_gpu in [false, true] {
        if (if use_gpu { &gpu } else { &cpu }).is_none() {
            eprintln!(
                "no {} H.264 encoder here",
                if use_gpu { "GPU" } else { "CPU" }
            );
            continue;
        }
        let out = dir
            .path()
            .join(format!("out-{}.mp4", if use_gpu { "gpu" } else { "cpu" }));
        let opts = ConvertOptions {
            gpu: use_gpu,
            ..Default::default()
        };
        let r = convert(&src, &out, &opts, |_| {}, &AtomicBool::new(false))
            .unwrap_or_else(|e| panic!("converting (gpu {use_gpu}): {e}"));
        let boxes = top_level_boxes(&out);
        let (keys, end) = keyframes(&out);
        let gap = longest_gap(&keys, end);
        eprintln!(
            "{}: boxes {boxes:?}; keyframes at {keys:?}, end {end:.3} s, longest gap {gap:.3} s",
            r.encoder
        );
        let at = |k: &str| boxes.iter().position(|b| b == k);
        let (moov, mdat) = (at("moov"), at("mdat"));
        assert!(
            moov.is_some() && mdat.is_some(),
            "{}: boxes {boxes:?}",
            r.encoder
        );
        if moov > mdat {
            problems.push(format!(
                "{}: the index (moov) is after the media data: {boxes:?}",
                r.encoder
            ));
        }
        assert!((end - 12.0).abs() < 0.5, "{}: {end} s long", r.encoder);
        if gap > MAX_KEYFRAME_GAP {
            problems.push(format!(
                "{}: keyframes up to {gap:.2} s apart (want about 2 s): {keys:?}",
                r.encoder
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
