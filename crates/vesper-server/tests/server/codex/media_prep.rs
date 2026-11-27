//! Media preparation (`docs/MEDIA-PREP-PLAN.md`): an awkward upload is analysed (loudness,
//! gain, poster frame) and converted to the house format with its original kept; a house-format
//! upload is only analysed; the trim; going back to the original; the same bytes again find the
//! asset; conversions wait while something is live. Media are synthetic (made with FFmpeg's
//! lavfi here; the server itself needs no `ffmpeg` tool: IDN probes and converts).

use super::media::{sha_hex, upload};
use crate::common::*;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};
use vesper_server::codex::CodexOptions;
use vesper_server::codex::prep::child::{Converter, TestHook};
use vesper_server::{RunningServer, ServerConfig, start};

/// The `vesper` binary: conversions run in a child process of it (`vesper media-convert`).
pub(super) fn vesper() -> std::path::PathBuf {
    env!("CARGO_BIN_EXE_vesper").into()
}

async fn start_prep(fx: &Fixture) -> RunningServer {
    start_prep_with(fx, TestHook::default()).await
}

/// A server preparing media, its conversions in child processes with `test`'s hooks.
async fn start_prep_with(fx: &Fixture, test: TestHook) -> RunningServer {
    start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: true,
        mdns: false,
        site: vesper_server::site::SiteConfig::off(),
        codex: CodexOptions {
            media_prep: true,
            converter: Converter {
                program: Some(vesper()),
                test,
            },
            ..CodexOptions::default()
        },
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts")
}

pub(super) fn ff(args: &[&str]) {
    let out = std::process::Command::new("ffmpeg")
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

/// −23 dBFS, 1 kHz, both channels: −23 LUFS.
const TONE: &str = "aevalsrc=0.0708*sin(2*PI*1000*t)|0.0708*sin(2*PI*1000*t):d=2";

/// A 2 s MPEG-4 Part 2 clip, black for its first 0.8 s, with a 44.1 kHz tone at −23 LUFS:
/// two reasons to convert it.
pub(super) fn awkward(path: &Path) {
    awkward_sized(path, "320x180");
}

/// [`awkward`] at another size (other bytes).
fn awkward_sized(path: &Path, size: &str) {
    ff(&[
        "-f",
        "lavfi",
        "-i",
        &format!(
            "testsrc=s={size}:r=25:d=2,drawbox=x=0:y=0:w=iw:h=ih:color=black:t=fill:enable='lt(t,0.8)'"
        ),
        "-f",
        "lavfi",
        "-i",
        &format!("{TONE}:s=44100"),
        "-c:v",
        "mpeg4",
        "-q:v",
        "4",
        "-c:a",
        "aac",
        "-ar",
        "44100",
        "-shortest",
        path.to_str().unwrap(),
    ]);
}

/// A 2 s house-format clip (H.264, AAC 48 kHz).
fn house(path: &Path) {
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=s=320x180:r=25:d=2",
        "-f",
        "lavfi",
        "-i",
        &format!("{TONE}:s=48000"),
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-ar",
        "48000",
        "-shortest",
        path.to_str().unwrap(),
    ]);
}

/// A 2 s interlaced H.264 clip (top field first, as the file states), AAC 48 kHz.
fn interlaced(path: &Path) {
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=s=320x180:r=25:d=2",
        "-f",
        "lavfi",
        "-i",
        &format!("{TONE}:s=48000"),
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-flags",
        "+ilme+ildct",
        "-x264opts",
        "tff=1",
        "-field_order",
        "tt",
        "-c:a",
        "aac",
        "-ar",
        "48000",
        "-shortest",
        path.to_str().unwrap(),
    ]);
}

async fn asset(addr: SocketAddr, id: &str) -> Value {
    get_json(addr, &format!("/api/v1/media/{id}")).await["asset"].clone()
}

/// Waits until `done` holds for the asset (at most 3 minutes: a conversion under load).
async fn until(addr: SocketAddr, id: &str, what: &str, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let a = asset(addr, id).await;
        if done(&a) {
            return a;
        }
        let st = get_json(addr, "/api/v1/codex/media-prep").await;
        assert!(Instant::now() < deadline, "{what}: {a}\n{st}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn call(addr: SocketAddr, method: &str, path: &str, body: Option<Value>) -> Resp {
    http(addr, method, path, &[], body).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn uploads_are_analysed_and_awkward_videos_converted() {
    let fx = fixture("media-prep");
    let server = start_prep(&fx).await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let st = get_json(addr, "/api/v1/codex/media-prep").await;
    assert_eq!(st["settings"]["enabled"], true, "{st}");
    assert_eq!(st["settings"]["loudnessTarget"], -16.0);
    assert!(
        st["converter"]
            .as_str()
            .is_some_and(|v| v.starts_with("IDN media conversion, FFmpeg")),
        "{st}"
    );
    assert!(
        st["encoder"].as_str().is_some_and(|e| !e.is_empty()),
        "{st}"
    );
    // The new settings' defaults (owner, 2026-11-26), and what this box has.
    let s = &st["settings"];
    assert_eq!(
        (
            &s["whileLive"],
            &s["priority"],
            &s["quality"],
            &s["maxSize"],
            &s["encoder"],
            &s["video"]
        ),
        (
            &json!("lowPriority"),
            &json!("low"),
            &json!("standard"),
            &json!("keep"),
            &json!("auto"),
            &json!("h264")
        ),
        "{st}"
    );
    assert!(
        st["gpuEncoder"].is_string() || st["cpuEncoder"].is_string(),
        "{st}"
    );
    // Either codec's encoders (the saved codec's are H.264's), for a codec chosen before Save.
    let by = &st["codecEncoders"];
    assert_eq!(
        (&by["h264"]["gpu"], &by["h264"]["cpu"]),
        (&st["gpuEncoder"], &st["cpuEncoder"]),
        "{st}"
    );
    assert!(by["hevc"].is_object(), "{st}");

    let tmp = fx.dir.join("src");
    std::fs::create_dir_all(&tmp).unwrap();
    awkward(&tmp.join("Awkward.mov"));
    house(&tmp.join("Good.mp4"));

    // The awkward one: analysed, then converted; the original kept.
    let bytes = std::fs::read(tmp.join("Awkward.mov")).unwrap();
    let orig_sha = sha_hex(&bytes);
    let done = upload(addr, &main, "Awkward.mov", &bytes).await;
    let id = done["asset"]["id"].as_str().unwrap().to_string();
    let a = until(addr, &id, "converted", |a| {
        a["prep"]["original"].is_object()
    })
    .await;
    let p = &a["prep"];
    let o = &p["original"];
    assert_eq!(o["location"]["sha256"], orig_sha.as_str(), "{a}");
    let reasons: Vec<&str> = o["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert!(
        reasons.contains(&"MPEG-4 Part 2 video isn't decoded by the GPU"),
        "{reasons:?}"
    );
    assert!(
        reasons.contains(&"sound at 44.1 kHz (the house rate is 48 kHz)"),
        "{reasons:?}"
    );
    assert_eq!(o["format"], "H.264 320×180 25 fps, AAC 48 kHz");
    assert_eq!(o["probe"]["codec"], "mpeg4");
    let new_sha = a["location"]["sha256"].as_str().unwrap().to_string();
    assert_ne!(new_sha, orig_sha);
    assert_eq!(p["analysed"], new_sha.as_str());
    assert_eq!(a["probe"]["codec"], "h264", "{a}");
    assert_eq!(a["probe"]["frameRate"], json!({"num": 25, "den": 1}));
    // −23 LUFS measured; +7 dB to −16.
    let l = p["loudness"].as_f64().unwrap();
    assert!((l + 23.0).abs() < 0.6, "{l}");
    let g = p["gainDb"].as_f64().unwrap();
    assert!((g - 7.0).abs() < 0.6, "{g}");
    // The poster isn't in the black first 0.8 s.
    assert!(p["posterAt"].as_f64().unwrap() >= 0.8, "{p}");
    assert!(
        fx.dir
            .join(format!("codex/thumbs/{new_sha}-320.jpg"))
            .is_file()
    );
    // The copy plays at 48 kHz, and is served as the asset's file.
    let path = server
        .shared
        .library
        .resolve(a["location"]["id"].as_str().unwrap())
        .unwrap();
    assert_eq!(path.file_name().unwrap(), "Awkward (prepared).mp4");
    let snd = media::probe_audio(&path).unwrap().unwrap();
    assert_eq!(snd.sample_rate, 48_000);
    let r = call(addr, "GET", &format!("/api/v1/media/{id}/file"), None).await;
    assert_eq!(r.status, 200);
    // The original stays in the library.
    assert!(server.shared.library.by_sha(&orig_sha).is_some());

    // The same bytes again: the same asset, still playing its prepared copy (not relinked
    // back to the original, which wiped its preparation).
    let again = upload(addr, &main, "Awkward again.mov", &bytes).await;
    assert_eq!(again["asset"]["id"], id.as_str(), "{again}");
    let a = asset(addr, &id).await;
    assert_eq!(a["location"]["sha256"], new_sha.as_str(), "{a}");
    assert!(a["prep"]["original"].is_object(), "{a}");
    assert!(a["prep"]["gainDb"].is_number(), "{a}");

    // The copy is in the house format: progressive, a constant 25 fps, H.264 8-bit 4:2:0.
    let v = media::probe(&path).unwrap();
    assert_eq!(
        (v.pixel_format.as_str(), v.bit_depth, v.variable_rate),
        ("yuv420p", 8, false),
        "{v:?}"
    );
    assert!((v.avg_fps - 25.0).abs() < 0.01, "{v:?}");

    // An interlaced clip (the file says so): converted, deinterlaced.
    interlaced(&tmp.join("Fields.mp4"));
    let fields = std::fs::read(tmp.join("Fields.mp4")).unwrap();
    let done = upload(addr, &main, "Fields.mp4", &fields).await;
    let fid = done["asset"]["id"].as_str().unwrap().to_string();
    let f = until(addr, &fid, "interlaced converted", |a| {
        a["prep"]["original"].is_object()
    })
    .await;
    let reasons = f["prep"]["original"]["reasons"].clone();
    assert_eq!(reasons, json!(["interlaced"]), "{f}");
    let path = server
        .shared
        .library
        .resolve(f["location"]["id"].as_str().unwrap())
        .unwrap();
    let v = media::probe(&path).unwrap();
    assert!(!v.interlaced, "{v:?}");

    // A house-format clip: analysed, not converted.
    let good = std::fs::read(tmp.join("Good.mp4")).unwrap();
    let done = upload(addr, &main, "Good.mp4", &good).await;
    let gid = done["asset"]["id"].as_str().unwrap().to_string();
    let g = until(addr, &gid, "analysed", |a| {
        a["prep"]["analysed"].is_string()
    })
    .await;
    assert_eq!(g["prep"]["analysed"], sha_hex(&good).as_str(), "{g}");
    assert!(g["prep"]["original"].is_null());
    assert!(g["prep"]["loudness"].is_number());

    // The trim.
    let base = format!("/api/v1/codex/collections/{main}/media/{gid}");
    let r = call(
        addr,
        "PUT",
        &format!("{base}/trim"),
        Some(json!({"trimIn": 0.5, "trimOut": 1.5})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["asset"]["prep"]["trimIn"], 0.5);
    assert_eq!(r.json()["asset"]["prep"]["trimOut"], 1.5);
    for bad in [
        json!({"trimIn": 1.5, "trimOut": 1.0}),
        json!({"trimOut": 99.0}),
        json!({"trimIn": -1.0}),
    ] {
        let r = call(addr, "PUT", &format!("{base}/trim"), Some(bad.clone())).await;
        assert_eq!(r.status, 400, "{bad}");
    }
    let r = call(addr, "PUT", &format!("{base}/trim"), Some(json!({}))).await;
    assert_eq!(r.status, 200);
    assert!(r.json()["asset"]["prep"]["trimIn"].is_null());
    // Pictures aren't prepared.
    let st = get_json(addr, "/api/v1/codex/media-prep").await;
    assert!(
        st["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|j| j["step"] == "done"),
        "{st}"
    );

    // Back to the original: not converted again by itself.
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/collections/{main}/media/{id}/original"),
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let a = r.json()["asset"].clone();
    assert_eq!(a["location"]["sha256"], orig_sha.as_str(), "{a}");
    assert!(a["prep"]["original"].is_null());
    assert_eq!(a["prep"]["analysed"], orig_sha.as_str());
    let r = call(
        addr,
        "POST",
        &format!("/api/v1/codex/collections/{main}/media/{id}/original"),
        None,
    )
    .await;
    assert_eq!(r.status, 409);

    // Settings: kept in range, admins only (localhost is trusted here).
    let r = call(
        addr,
        "PUT",
        "/api/v1/codex/media-prep",
        Some(json!({"enabled": false, "loudnessTarget": -99, "maxJobs": 9, "whileLive": "lowPriority",
            "priority": "normal", "quality": "high", "videoKbps": 250000, "maxSize": "1080p", "encoder": "cpu"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let s = r.json();
    assert_eq!(
        (
            &s["enabled"],
            &s["loudnessTarget"],
            &s["maxJobs"],
            &s["whileLive"]
        ),
        (
            &json!(false),
            &json!(-30.0),
            &json!(4),
            &json!("lowPriority")
        )
    );
    assert_eq!(
        (
            &s["priority"],
            &s["quality"],
            &s["videoKbps"],
            &s["maxSize"],
            &s["encoder"]
        ),
        (
            &json!("normal"),
            &json!("high"),
            &json!(60_000),
            &json!("1080p"),
            &json!("cpu")
        ),
        "{s}"
    );
    assert!(fx.dir.join("codex/media-prep.json").is_file());
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conversions_wait_while_something_is_live() {
    let fx = fixture("media-prep-live");
    let server = start_prep(&fx).await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    // Not the default any more (owner, 2026-11-26): chosen.
    let r = call(
        addr,
        "PUT",
        "/api/v1/codex/media-prep",
        Some(json!({"whileLive": "wait"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let list = get_json(addr, &format!("/api/v1/codex/collections/{main}/media")).await;
    let blue = list
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["asset"]["name"] == "Blue background")
        .map(|m| m["asset"]["id"].as_str().unwrap().to_string())
        .expect("the fixture's background");
    let r = call(
        addr,
        "POST",
        "/api/v1/commands",
        Some(json!({"type": "triggerMedia", "asset": blue})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));

    let tmp = fx.dir.join("src");
    std::fs::create_dir_all(&tmp).unwrap();
    awkward(&tmp.join("Late.mov"));
    let bytes = std::fs::read(tmp.join("Late.mov")).unwrap();
    let done = upload(addr, &main, "Late.mov", &bytes).await;
    let id = done["asset"]["id"].as_str().unwrap().to_string();
    // Analysed while live, then the conversion waits.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let st = get_json(addr, "/api/v1/codex/media-prep").await;
        assert_eq!(st["live"], true);
        let job = st["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["asset"] == id.as_str())
            .cloned();
        if job.as_ref().is_some_and(|j| j["step"] == "waitingForLive") {
            break;
        }
        assert!(Instant::now() < deadline, "{st}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let a = asset(addr, &id).await;
    assert!(
        a["prep"]["loudness"].is_number(),
        "analysed while live: {a}"
    );
    assert!(a["prep"]["original"].is_null());
    tokio::time::sleep(Duration::from_millis(800)).await;
    let st = get_json(addr, "/api/v1/codex/media-prep").await;
    assert_eq!(st["jobs"][0]["step"], "waitingForLive", "{st}");

    // Waiting conversions hold no job thread either (four of them): with five waiting, an
    // upload is still analysed (H02-3).
    let mut more = Vec::new();
    for (i, size) in ["384x216", "448x252", "512x288", "576x324"]
        .iter()
        .enumerate()
    {
        let name = format!("Late{i}.mov");
        awkward_sized(&tmp.join(&name), size);
        let bytes = std::fs::read(tmp.join(&name)).unwrap();
        let done = upload(addr, &main, &name, &bytes).await;
        more.push(done["asset"]["id"].as_str().unwrap().to_string());
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let st = get_json(addr, "/api/v1/codex/media-prep").await;
        let waiting = st["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|j| j["step"] == "waitingForLive")
            .count();
        if waiting == 5 {
            break;
        }
        assert!(Instant::now() < deadline, "{st}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // The waiting conversions take no job slot (one by default) and no thread: another upload
    // is analysed meanwhile.
    house(&tmp.join("Good.mp4"));
    let good = std::fs::read(tmp.join("Good.mp4")).unwrap();
    let done = upload(addr, &main, "Good.mp4", &good).await;
    let gid = done["asset"]["id"].as_str().unwrap().to_string();
    until(addr, &gid, "analysed while another waits", |a| {
        a["prep"]["analysed"].is_string()
    })
    .await;
    let st = get_json(addr, "/api/v1/codex/media-prep").await;
    let late = st["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["asset"] == id.as_str())
        .cloned()
        .unwrap();
    assert_eq!(late["step"], "waitingForLive", "{st}");

    // Nothing live: it converts.
    let r = call(
        addr,
        "POST",
        "/api/v1/commands",
        Some(json!({"type": "clearAll"})),
    )
    .await;
    assert_eq!(r.status, 200);
    until(addr, &id, "converted after the show", |a| {
        a["prep"]["original"].is_object()
    })
    .await;
    for m in &more {
        until(addr, m, "converted after the show", |a| {
            a["prep"]["original"].is_object()
        })
        .await;
    }

    // A waiting conversion cut off by a restart is picked up again.
    let r = call(
        addr,
        "POST",
        "/api/v1/commands",
        Some(json!({"type": "triggerMedia", "asset": blue})),
    )
    .await;
    assert_eq!(r.status, 200);
    awkward_sized(&tmp.join("Later.mov"), "352x198");
    let bytes = std::fs::read(tmp.join("Later.mov")).unwrap();
    let done = upload(addr, &main, "Later.mov", &bytes).await;
    let later = done["asset"]["id"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let st = get_json(addr, "/api/v1/codex/media-prep").await;
        let waits = st["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["asset"] == later.as_str() && j["step"] == "waitingForLive");
        if waits {
            break;
        }
        assert!(Instant::now() < deadline, "{st}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    server.shutdown().await.unwrap();
    assert!(fx.dir.join("codex/media-prep-queue.json").is_file());
    // The old server's preparation threads see the stop within half a second (they aren't
    // joined); two servers never share a data folder outside tests.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let server = start_prep(&fx).await;
    let addr = server.addr;
    until(addr, &later, "converted after a restart", |a| {
        a["prep"]["original"].is_object()
    })
    .await;
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// The fixture's background put live (something is live on the box).
async fn go_live(addr: SocketAddr, main: &str) {
    let list = get_json(addr, &format!("/api/v1/codex/collections/{main}/media")).await;
    let blue = list
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["asset"]["name"] == "Blue background")
        .map(|m| m["asset"]["id"].as_str().unwrap().to_string())
        .expect("the fixture's background");
    let r = call(
        addr,
        "POST",
        "/api/v1/commands",
        Some(json!({"type": "triggerMedia", "asset": blue})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
}

/// The default (owner, 2026-11-26): media imported during a service are converted at the
/// lowest priority, not after it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn by_default_conversions_go_on_at_the_lowest_priority_while_live() {
    let fx = fixture("media-prep-live-low");
    let server = start_prep(&fx).await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    go_live(addr, &main).await;
    let tmp = fx.dir.join("src");
    std::fs::create_dir_all(&tmp).unwrap();
    awkward(&tmp.join("Late.mov"));
    let bytes = std::fs::read(tmp.join("Late.mov")).unwrap();
    let done = upload(addr, &main, "Late.mov", &bytes).await;
    let id = done["asset"]["id"].as_str().unwrap().to_string();
    until(addr, &id, "converted while live", |a| {
        a["prep"]["original"].is_object()
    })
    .await;
    let st = get_json(addr, "/api/v1/codex/media-prep").await;
    assert_eq!(st["live"], true, "still live: {st}");
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// A crash inside the converter ends only its child process: the job fails with a note, the
/// asset keeps its analysis, and the server goes on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_converter_crash_fails_the_job_not_the_server() {
    let fx = fixture("media-prep-crash");
    let server = start_prep_with(
        &fx,
        TestHook {
            crash: true,
            ..TestHook::default()
        },
    )
    .await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let tmp = fx.dir.join("src");
    std::fs::create_dir_all(&tmp).unwrap();
    awkward(&tmp.join("Crashy.mov"));
    let bytes = std::fs::read(tmp.join("Crashy.mov")).unwrap();
    let done = upload(addr, &main, "Crashy.mov", &bytes).await;
    let id = done["asset"]["id"].as_str().unwrap().to_string();
    let a = until(addr, &id, "the crash noted", |a| {
        a["prep"]["error"].is_string()
    })
    .await;
    let e = a["prep"]["error"].as_str().unwrap();
    assert!(e.contains("the converter crashed"), "{e}");
    assert!(a["prep"]["original"].is_null(), "{a}");
    assert!(a["prep"]["loudness"].is_number(), "analysed before: {a}");
    let st = get_json(addr, "/api/v1/codex/media-prep").await;
    let job = st["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["asset"] == id.as_str())
        .cloned()
        .unwrap();
    assert_eq!(job["step"], "failed", "{st}");
    // Nothing of the conversion is left in the work folder.
    let work = fx.dir.join("codex/prep").join(&id);
    assert!(!work.exists(), "{}", work.display());
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// The converter child of a job, found by its configuration file below `dir`.
#[cfg(target_os = "linux")]
fn converter_child(dir: &Path) -> Option<i32> {
    let dir = dir.to_string_lossy().into_owned();
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let pid: i32 = e.file_name().to_str()?.parse().ok()?;
        let cmd = std::fs::read(e.path().join("cmdline")).ok()?;
        let cmd = String::from_utf8_lossy(&cmd);
        (cmd.contains("media-convert") && cmd.contains(&dir)).then_some(pid)
    })
}

/// A service stop (systemd signals every process of the service; Ctrl-C the whole terminal
/// group) during a conversion isn't a failure: the child outlives the signal, the server's
/// stop ends it, the asset keeps no error and the job is picked up at the next start (R1-1).
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_service_stop_during_a_conversion_keeps_the_job() {
    let fx = fixture("media-prep-sigterm");
    let server = start_prep_with(
        &fx,
        TestHook {
            stall: true,
            ..TestHook::default()
        },
    )
    .await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let tmp = fx.dir.join("src");
    std::fs::create_dir_all(&tmp).unwrap();
    awkward(&tmp.join("Stopped.mov"));
    let bytes = std::fs::read(tmp.join("Stopped.mov")).unwrap();
    let done = upload(addr, &main, "Stopped.mov", &bytes).await;
    let id = done["asset"]["id"].as_str().unwrap().to_string();
    // Converting (stalled mid-file by the test hook).
    let deadline = Instant::now() + Duration::from_secs(120);
    let pid = loop {
        if let Some(pid) = converter_child(&fx.dir.join("codex/prep"))
            && get_json(addr, "/api/v1/codex/media-prep").await["jobs"][0]["step"] == "converting"
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "no conversion started");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    // The stop signals reach the child first (as systemd's, before the server's own stop).
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: a signal to this test's own child process, by its id.
        assert_eq!(unsafe { libc::kill(pid, sig) }, 0);
    }
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(
        Path::new(&format!("/proc/{pid}/cmdline")).exists(),
        "the converter died of a stop signal"
    );
    let a = asset(addr, &id).await;
    assert!(a["prep"]["error"].is_null(), "{a}");
    server.shutdown().await.unwrap();
    assert!(fx.dir.join("codex/media-prep-queue.json").is_file());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !Path::new(&format!("/proc/{pid}/cmdline")).exists(),
        "the server's stop ended the converter"
    );

    // The next start converts it.
    let server = start_prep(&fx).await;
    let addr = server.addr;
    let a = until(addr, &id, "converted after the restart", |a| {
        a["prep"]["original"].is_object()
    })
    .await;
    assert!(a["prep"]["error"].is_null(), "{a}");
    let _ = std::fs::remove_dir_all(&fx.dir);
}
