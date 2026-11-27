//! Media preparation's conversions in a child process (`vesper media-convert`,
//! `vesper_codex::prep::child`): progress as it goes, a cancel kills it and leaves nothing, a
//! crash is a failure with a note, and the priority it runs at (Linux: read from `/proc`).
//! Media are synthetic (FFmpeg's lavfi).

use super::media_prep::{awkward, ff, vesper};
use crate::common::*;
use media::convert::{ConvertError, ConvertOptions};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use vesper_server::codex::prep::child::{Job, Level, Running, TestHook};

/// A 30 s MPEG-4 Part 2 clip with a 44.1 kHz tone (long enough to report progress).
fn long(path: &Path) {
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=s=320x180:r=25:d=30",
        "-f",
        "lavfi",
        "-i",
        "sine=f=440:d=30:sample_rate=44100",
        "-c:v",
        "mpeg4",
        "-q:v",
        "5",
        "-c:a",
        "aac",
        "-shortest",
        path.to_str().unwrap(),
    ]);
}

fn job(src: &Path, out: &Path, level: Level, test: TestHook) -> Job {
    Job {
        test,
        ..Job::new(src, out, &ConvertOptions::default(), level)
    }
}

/// The files left in `dir`.
fn files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// The nice value of every thread of process `pid`.
#[cfg(target_os = "linux")]
fn nice_of(pid: u32) -> Vec<i64> {
    threads_of(pid).into_iter().map(|(_, n)| n).collect()
}

/// Every thread of process `pid`: its name and nice value.
#[cfg(target_os = "linux")]
fn threads_of(pid: u32) -> Vec<(String, i64)> {
    std::fs::read_dir(format!("/proc/{pid}/task"))
        .unwrap()
        .flatten()
        .filter_map(|t| std::fs::read_to_string(t.path().join("stat")).ok())
        .map(|stat| {
            let name = stat[stat.find('(').unwrap() + 1..stat.rfind(')').unwrap()].to_string();
            // After the command's ")": state is field 3, nice field 19.
            let rest = &stat[stat.rfind(')').unwrap() + 2..];
            (
                name,
                rest.split_whitespace().nth(16).unwrap().parse().unwrap(),
            )
        })
        .collect()
}

/// The calling thread's nice value.
#[cfg(target_os = "linux")]
fn own_nice() -> i64 {
    let stat = std::fs::read_to_string("/proc/thread-self/stat").unwrap();
    let rest = &stat[stat.rfind(')').unwrap() + 2..];
    rest.split_whitespace().nth(16).unwrap().parse().unwrap()
}

fn scratch(name: &str) -> (Fixture, PathBuf, PathBuf) {
    let fx = fixture(name);
    let src = fx.dir.join("src");
    let work = fx.dir.join("work");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    (fx, src, work)
}

#[test]
fn a_conversion_in_a_child_process_reports_progress_and_finishes() {
    let (fx, src, work) = scratch("media-convert-child");
    long(&src.join("Long.mov"));
    let out = work.join("Long (prepared).mp4");
    let j = job(&src.join("Long.mov"), &out, Level::Low, TestHook::default());
    let running = Running::spawn(&vesper(), &j, &work.join("convert.json")).unwrap();
    assert_ne!(running.pid(), std::process::id());
    let mut seen = Vec::new();
    let made = running
        .wait(&|| false, &|| false, |x| seen.push(x))
        .expect("converted");
    assert!(!seen.is_empty(), "no progress reported");
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    assert!(seen.iter().all(|x| (0.0..=1.0).contains(x)), "{seen:?}");
    assert_eq!((made.width, made.height), (320, 180));
    assert!(made.audio);
    assert!((made.duration - 30.0).abs() < 0.5, "{made:?}");
    // The output and nothing else (the configuration is removed).
    assert_eq!(files(&work), vec!["Long (prepared).mp4"]);
    let snd = media::probe_audio(&out).unwrap().unwrap();
    assert_eq!(snd.sample_rate, 48_000);
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[test]
fn a_cancelled_conversion_is_killed_and_leaves_nothing() {
    let (fx, src, work) = scratch("media-convert-cancel");
    long(&src.join("Long.mov"));
    let out = work.join("Long (prepared).mp4");
    let stall = TestHook {
        stall: true,
        ..TestHook::default()
    };
    let j = job(&src.join("Long.mov"), &out, Level::Low, stall);
    let running = Running::spawn(&vesper(), &j, &work.join("convert.json")).unwrap();
    let pid = running.pid();
    let started = Cell::new(false);
    #[cfg(target_os = "linux")]
    let niced = Cell::new(Vec::new());
    let r = running.wait(&|| started.get(), &|| false, |_| {
        // Its first progress: it stalls there (the test hook), mid-file.
        #[cfg(target_os = "linux")]
        niced.set(nice_of(pid));
        started.set(true);
    });
    assert!(matches!(r, Err(ConvertError::Cancelled)), "{r:?}");
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "the child is gone"
    );
    assert!(files(&work).is_empty(), "{:?}", files(&work));
    // Low: every thread of it at nice 19 (the threads IDN started too).
    #[cfg(target_os = "linux")]
    {
        let n = niced.take();
        assert!(n.len() > 1 && n.iter().all(|v| *v == 19), "{n:?}");
    }
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[test]
fn a_crash_in_the_child_is_a_failure_with_a_note() {
    let (fx, src, work) = scratch("media-convert-crash");
    awkward(&src.join("Clip.mov"));
    let out = work.join("Clip (prepared).mp4");
    let crash = TestHook {
        crash: true,
        ..TestHook::default()
    };
    let j = job(&src.join("Clip.mov"), &out, Level::Low, crash);
    let running = Running::spawn(&vesper(), &j, &work.join("convert.json")).unwrap();
    let r = running.wait(&|| false, &|| false, |_| {});
    let Err(ConvertError::Failed(e)) = r else {
        panic!("{r:?}");
    };
    assert!(e.contains("the converter crashed"), "{e}");
    #[cfg(unix)]
    assert!(e.contains("SIGABRT"), "{e}");
    // What it wrote last before it ended, read to the end of its stderr (R1-3).
    assert!(
        e.ends_with("Its last words: media-convert: crashing on purpose (test hook)"),
        "{e}"
    );
    assert!(files(&work).is_empty(), "{:?}", files(&work));
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// Normal priority: the child runs as the thread that started it; once something goes live it
/// is lowered to the lowest (every thread of it).
#[cfg(target_os = "linux")]
#[test]
fn a_normal_conversion_is_lowered_once_something_goes_live() {
    let (fx, src, work) = scratch("media-convert-lower");
    long(&src.join("Long.mov"));
    let out = work.join("Long (prepared).mp4");
    let stall = TestHook {
        stall: true,
        ..TestHook::default()
    };
    let j = job(&src.join("Long.mov"), &out, Level::Normal, stall);
    let mine = own_nice();
    let running = Running::spawn(&vesper(), &j, &work.join("convert.json")).unwrap();
    let pid = running.pid();
    let started = Cell::new(false);
    let before = Cell::new(Vec::new());
    let lowered = Cell::new(0u32);
    let after = Cell::new(Vec::new());
    let r = running.wait(
        &|| {
            // Asked before `lower` each round: two rounds after the lowering, look and stop.
            if lowered.get() > 0 {
                lowered.set(lowered.get() + 1);
            }
            if lowered.get() > 2 {
                after.set(nice_of(pid));
                return true;
            }
            false
        },
        &|| {
            if started.get() && lowered.get() == 0 {
                lowered.set(1);
            }
            started.get()
        },
        |_| {
            before.set(threads_of(pid));
            started.set(true);
        },
    );
    assert!(matches!(r, Err(ConvertError::Cancelled)), "{r:?}");
    let (b, a) = (before.take(), after.take());
    // Its own threads and FFmpeg's decoders as the test; the GPU driver's background
    // threads (Mesa's shader cache, its trace queues) lower themselves whatever the process.
    let ours: Vec<_> = b
        .iter()
        .filter(|(n, _)| n == "vesper" || n == "stdin" || n.starts_with("av:"))
        .collect();
    assert!(
        ours.len() > 2 && ours.iter().all(|(_, v)| *v == mine),
        "normal: as the test ({mine}): {b:?}"
    );
    assert!(a.len() > 1 && a.iter().all(|v| *v == 19), "lowered: {a:?}");
    assert!(files(&work).is_empty(), "{:?}", files(&work));
    let _ = std::fs::remove_dir_all(&fx.dir);
}
