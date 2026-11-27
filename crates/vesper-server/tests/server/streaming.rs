//! Recording and streaming a screen (`crate::gateways`, docs/RECORD-STREAM-PLAN.md): a few
//! seconds of a test pattern and a test tone recorded and checked with FFmpeg's probe and a
//! decoded picture; streams to local RTMP and SRT listeners (the `ffmpeg` tool, when the
//! machine has it); stream keys never come back. Made-up keys only.

use crate::common::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const KEY: &str = "made-up-key-0001";

async fn status(addr: std::net::SocketAddr) -> Value {
    get_json(addr, "/api/v1/streaming/status").await
}

/// Whether the box can encode H.264 (a GPU's or the CPU's); tests that need it say so and
/// pass otherwise.
async fn can_encode(addr: std::net::SocketAddr) -> bool {
    let s = get_json(addr, "/api/v1/streaming").await;
    let ok = s["encoders"]
        .as_array()
        .is_some_and(|e| e.iter().any(|e| e["codec"] == "h264"));
    if !ok {
        eprintln!("SKIPPED: no H.264 encoder on this machine");
    }
    ok
}

async fn command(addr: std::net::SocketAddr, c: Value) {
    let r = http(addr, "POST", "/api/v1/commands", &[], Some(c)).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
}

/// Waits until `pred` holds for the status (at most `secs`).
async fn wait_status(
    addr: std::net::SocketAddr,
    secs: u64,
    pred: impl Fn(&Value) -> bool,
) -> Value {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let s = status(addr).await;
        if pred(&s) {
            return s;
        }
        assert!(Instant::now() < end, "status never got there: {s}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn files_in(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.retain(|p| p.extension().is_some_and(|e| e == ext));
    v.sort();
    v
}

/// Checks a recorded or received file: H.264 at the screen's size with AAC sound, about
/// `min_s` seconds or more, and a picture that isn't flat (the colour bars).
fn check_file(path: &Path, min_s: f64) {
    // The process's FFmpeg, never a second probe: a probe makes a Vulkan instance and
    // device of its own, which can crash other tests' rendering in the Vulkan loader (M-4).
    let m = idn_ffmpeg::media::probe(path, media::machine_ffmpeg()).expect("the file opens");
    let v = m.video.as_ref().expect("a video stream");
    assert_eq!(v.codec, "h264", "{m:?}");
    assert_eq!((v.width, v.height), (W as usize, H as usize), "{m:?}");
    let a = m.audio.as_ref().expect("a sound stream");
    assert_eq!(a.codec, "aac", "{m:?}");
    assert_eq!(a.sample_rate, 48_000);
    let thumb = media::thumbnail(path, 0.5, 320).expect("a picture decodes");
    if let Some(d) = thumb.duration {
        assert!(d >= min_s, "{} lasts {d} s", path.display());
    }
    let (mut lo, mut hi) = (255u8, 0u8);
    for p in thumb.rgb.as_chunks::<3>().0 {
        let l = p[1];
        lo = lo.min(l);
        hi = hi.max(l);
    }
    assert!(hi - lo > 100, "the picture is flat ({lo}–{hi})");
}

#[tokio::test(flavor = "multi_thread")]
async fn records_a_screen_with_its_sound_and_splits() {
    let fx = fixture("rec");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    if !can_encode(addr).await {
        return;
    }
    let screen = fx.audience.id.0.to_string();
    command(
        addr,
        json!({"type": "setTestPattern", "screen": screen, "pattern": {"type": "colorBars"}}),
    )
    .await;
    command(addr, json!({"type": "playTestTone", "seconds": 20.0})).await;

    let r = http(
        addr,
        "POST",
        "/api/v1/recording/start",
        &[],
        Some(json!({"screen": screen})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["recording"]["screenName"], fx.audience.name);
    // A second start is refused.
    let again = http(addr, "POST", "/api/v1/recording/start", &[], None).await;
    assert_eq!(again.status, 409);

    wait_status(addr, 15, |s| {
        s["recording"]["state"] == "live" && s["recording"]["bytes"].as_u64() > Some(50_000)
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let split = http(addr, "POST", "/api/v1/recording/split", &[], None).await;
    assert_eq!(
        split.status,
        200,
        "{}",
        String::from_utf8_lossy(&split.body)
    );
    assert_eq!(split.json()["recording"]["part"], 2);
    wait_status(addr, 15, |s| s["recording"]["state"] == "live").await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let s = status(addr).await;
    assert!(s["recording"]["seconds"].as_f64().unwrap() >= 4.0, "{s}");
    assert!(s["recording"]["freeBytes"].as_u64().unwrap() > 0);

    let stop = http(addr, "POST", "/api/v1/recording/stop", &[], None).await;
    assert_eq!(stop.status, 200);
    assert!(stop.json()["recording"].is_null());
    assert_eq!(
        http(addr, "POST", "/api/v1/recording/stop", &[], None)
            .await
            .status,
        409
    );

    let files = files_in(&fx.dir.join("recordings"), "mp4");
    assert_eq!(files.len(), 2, "{files:?}");
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with(&fx.audience.name), "{name}");
        check_file(f, 1.5);
    }
}

/// A split's new part is encoded like the first (F05-2): the recording's codec, not the
/// gateway's default H.264. Needs an HEVC encoder (GPU or CPU); skipped otherwise.
#[tokio::test(flavor = "multi_thread")]
async fn a_split_part_keeps_the_recording_codec() {
    let fx = fixture("rec-split-codec");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let s = get_json(addr, "/api/v1/streaming").await;
    if !s["encoders"]
        .as_array()
        .is_some_and(|e| e.iter().any(|e| e["codec"] == "hevc"))
    {
        eprintln!("SKIPPED: no HEVC encoder on this machine");
        return;
    }
    let r = http(
        addr,
        "PUT",
        "/api/v1/streaming/recording",
        &[],
        Some(json!({"codec": "hevc", "videoMbps": 4.0, "audioKbps": 96})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let screen = fx.audience.id.0.to_string();
    command(
        addr,
        json!({"type": "setTestPattern", "screen": screen, "pattern": {"type": "colorBars"}}),
    )
    .await;
    let r = http(addr, "POST", "/api/v1/recording/start", &[], None).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    wait_status(addr, 15, |s| {
        s["recording"]["state"] == "live" && s["recording"]["bytes"].as_u64() > Some(20_000)
    })
    .await;
    let split = http(addr, "POST", "/api/v1/recording/split", &[], None).await;
    assert_eq!(
        split.status,
        200,
        "{}",
        String::from_utf8_lossy(&split.body)
    );
    wait_status(addr, 15, |s| {
        s["recording"]["part"] == 2
            && s["recording"]["state"] == "live"
            && s["recording"]["bytes"].as_u64() > Some(20_000)
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let stop = http(addr, "POST", "/api/v1/recording/stop", &[], None).await;
    assert_eq!(stop.status, 200);
    let files = files_in(&fx.dir.join("recordings"), "mp4");
    assert_eq!(files.len(), 2, "{files:?}");
    for f in &files {
        let m = idn_ffmpeg::media::probe(f, media::machine_ffmpeg()).expect("the file opens");
        assert_eq!(
            m.video.as_ref().map(|v| v.codec.as_str()),
            Some("hevc"),
            "{}: {m:?}",
            f.display()
        );
    }
    server.shutdown().await.unwrap();
}

/// Stopping the server finishes a recording in progress before `shutdown` returns (X3-1):
/// the file is closed (the MKV gets its duration and cues), nothing writes to it afterwards,
/// and it plays.
#[tokio::test(flavor = "multi_thread")]
async fn stopping_the_server_finishes_the_recording() {
    let fx = fixture("rec-shutdown");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    if !can_encode(addr).await {
        return;
    }
    let r = http(
        addr,
        "PUT",
        "/api/v1/streaming/recording",
        &[],
        Some(json!({"container": "mkv"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let screen = fx.audience.id.0.to_string();
    command(
        addr,
        json!({"type": "setTestPattern", "screen": screen, "pattern": {"type": "colorBars"}}),
    )
    .await;
    command(addr, json!({"type": "playTestTone", "seconds": 20.0})).await;
    let r = http(
        addr,
        "POST",
        "/api/v1/recording/start",
        &[],
        Some(json!({"screen": screen})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    wait_status(addr, 15, |s| {
        s["recording"]["state"] == "live" && s["recording"]["bytes"].as_u64() > Some(50_000)
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    server.shutdown().await.unwrap();
    let files = files_in(&fx.dir.join("recordings"), "mkv");
    assert_eq!(files.len(), 1, "{files:?}");
    let size = std::fs::metadata(&files[0]).unwrap().len();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        std::fs::metadata(&files[0]).unwrap().len(),
        size,
        "the recording was still being written after the server stopped"
    );
    let m = idn_ffmpeg::media::probe(&files[0], media::machine_ffmpeg()).expect("the file opens");
    let d = m.duration.expect("the finished file states its duration");
    assert!(d >= Duration::from_millis(1500), "{d:?}");
    check_file(&files[0], 1.5);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recording_needs_free_space() {
    let fx = fixture("rec-space");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let r = http(
        addr,
        "PUT",
        "/api/v1/streaming/recording",
        &[],
        Some(json!({"minFreeGb": 9999.0, "container": "mkv"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["recording"]["container"], "mkv");
    let start = http(addr, "POST", "/api/v1/recording/start", &[], None).await;
    assert_eq!(
        start.status,
        507,
        "{}",
        String::from_utf8_lossy(&start.body)
    );
    assert_eq!(start.json()["error"]["code"], "insufficientStorage");
    // Bad settings are refused.
    for bad in [
        json!({"videoMbps": 0.0}),
        json!({"folder": "relative/path"}),
        json!({"audioKbps": 10}),
    ] {
        let r = http(addr, "PUT", "/api/v1/streaming/recording", &[], Some(bad)).await;
        assert_eq!(r.status, 400);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_keys_stay_on_the_box() {
    let fx = fixture("stream-keys");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let r = http(
        addr,
        "POST",
        "/api/v1/streaming/targets",
        &[],
        Some(json!({"name": "Platform", "kind": "rtmp", "url": "rtmp://127.0.0.1:1/live2", "key": KEY})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let t = r.json();
    assert_eq!(t["hasKey"], true);
    assert!(!String::from_utf8_lossy(&r.body).contains(KEY));
    let id = t["id"].as_str().unwrap().to_string();
    let all = http(addr, "GET", "/api/v1/streaming", &[], None).await;
    assert!(!String::from_utf8_lossy(&all.body).contains(KEY));
    let saved = std::fs::read_to_string(fx.dir.join("machine/streaming.json")).unwrap();
    assert!(saved.contains(KEY), "the key is kept on the box");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(fx.dir.join("machine/streaming.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "only the server's user reads it");
    }

    // Edited without a key: kept. With "": cleared, and then it can't start.
    let path = format!("/api/v1/streaming/targets/{id}");
    let put = |body: Value| http(addr, "PUT", &path, &[], Some(body));
    let r =
        put(json!({"name": "Platform A", "kind": "rtmp", "url": "rtmp://127.0.0.1:1/live2"})).await;
    assert_eq!(r.json()["hasKey"], true);
    let r = put(
        json!({"name": "Platform A", "kind": "rtmp", "url": "rtmp://127.0.0.1:1/live2", "key": ""}),
    )
    .await;
    assert_eq!(r.json()["hasKey"], false);
    let start = http(
        addr,
        "POST",
        &format!("/api/v1/streaming/targets/{id}/start"),
        &[],
        None,
    )
    .await;
    assert_eq!(start.status, 400);
    let r = put(json!({"name": "Platform A", "kind": "rtmp", "url": "rtmp://127.0.0.1:1/live2", "key": KEY})).await;
    assert_eq!(r.json()["hasKey"], true);

    // Bad targets.
    for bad in [
        json!({"name": "X", "kind": "rtmp", "url": "http://192.0.2.1/live"}),
        json!({"name": "", "kind": "rtmp", "url": "rtmp://192.0.2.1/live"}),
        json!({"name": "X", "kind": "srt", "url": "rtmp://192.0.2.1/live"}),
        json!({"name": "X", "kind": "rtmp", "url": "rtmp://192.0.2.1/live", "videoMbps": 500.0}),
    ] {
        let r = http(addr, "POST", "/api/v1/streaming/targets", &[], Some(bad)).await;
        assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    }

    // An SRT passphrase is hidden, and sending the hidden URL back keeps it.
    let srt = http(
        addr,
        "POST",
        "/api/v1/streaming/targets",
        &[],
        Some(json!({"name": "Overflow", "kind": "srt", "url": "srt://192.0.2.7:9000?passphrase=madeuppass99"})),
    )
    .await
    .json();
    assert_eq!(srt["url"], "srt://192.0.2.7:9000?passphrase=***");
    let sid = srt["id"].as_str().unwrap().to_string();
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/streaming/targets/{sid}"),
        &[],
        Some(json!({"name": "Overflow room", "kind": "srt", "url": "srt://192.0.2.7:9000?passphrase=***"})),
    )
    .await;
    assert_eq!(r.status, 200);
    let saved = std::fs::read_to_string(fx.dir.join("machine/streaming.json")).unwrap();
    assert!(saved.contains("madeuppass99"));
    assert!(
        !String::from_utf8_lossy(&http(addr, "GET", "/api/v1/streaming", &[], None).await.body)
            .contains("madeuppass99")
    );
    // An edited copy of the shown URL is never saved with `***` as the passphrase (F10-1):
    // for another server it is refused (F05-3: the secret stays with its server) ...
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/streaming/targets/{sid}"),
        &[],
        Some(json!({"name": "Overflow room", "kind": "srt", "url": "srt://192.0.2.7:9001?passphrase=***"})),
    )
    .await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    // ... and on the same server the saved passphrase is kept with the edit.
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/streaming/targets/{sid}"),
        &[],
        Some(json!({"name": "Overflow room", "kind": "srt", "url": "srt://192.0.2.7:9000?passphrase=***&latency=200"})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let saved = std::fs::read_to_string(fx.dir.join("machine/streaming.json")).unwrap();
    assert!(
        saved.contains("srt://192.0.2.7:9000?passphrase=madeuppass99&latency=200"),
        "{saved}"
    );
    assert!(!saved.contains("passphrase=***"), "{saved}");
    let r = http(
        addr,
        "POST",
        "/api/v1/streaming/targets",
        &[],
        Some(json!({"name": "Copy", "kind": "srt", "url": "srt://192.0.2.8:9000?passphrase=***"})),
    )
    .await;
    assert_eq!(r.status, 400);
    let r = http(
        addr,
        "PUT",
        &format!("/api/v1/streaming/targets/{sid}"),
        &[],
        Some(json!({"name": "Overflow room", "kind": "srt", "url": "srt://192.0.2.7:9001?passphrase=othermadeup77"})),
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["url"], "srt://192.0.2.7:9001?passphrase=***");

    if can_encode(addr).await {
        // Nothing listens there: it keeps trying, and the key shows nowhere.
        let r = http(
            addr,
            "POST",
            &format!("/api/v1/streaming/targets/{id}/start"),
            &[],
            None,
        )
        .await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let s = wait_status(addr, 15, |s| s["streams"][0]["state"] == "retrying").await;
        assert!(!s.to_string().contains(KEY), "{s}");
        assert_eq!(
            http(
                addr,
                "POST",
                &format!("/api/v1/streaming/targets/{id}/start"),
                &[],
                None
            )
            .await
            .status,
            409
        );
        let r = http(
            addr,
            "POST",
            &format!("/api/v1/streaming/targets/{id}/stop"),
            &[],
            None,
        )
        .await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json()["streams"], json!([]));
    }
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/streaming/targets/{id}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204);
    let all = get_json(addr, "/api/v1/streaming").await;
    assert_eq!(all["targets"].as_array().unwrap().len(), 1);
}

/// A local `ffmpeg` listener receiving `url` into `out` (none when the tool is missing).
fn listener(url: &str, extra: &[&str], out: &Path) -> Option<std::process::Child> {
    let mut c = std::process::Command::new("ffmpeg");
    c.args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(extra)
        .args(["-i", url, "-c", "copy"])
        .arg(out)
        .stdin(std::process::Stdio::null());
    c.spawn().ok()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_to_rtmp_and_srt_listeners() {
    let fx = fixture("stream-live");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    if !can_encode(addr).await {
        return;
    }
    let screen = fx.audience.id.0.to_string();
    command(
        addr,
        json!({"type": "setTestPattern", "screen": screen, "pattern": {"type": "colorBars"}}),
    )
    .await;
    command(addr, json!({"type": "playTestTone", "seconds": 20.0})).await;
    let out = fx.dir.join("received");
    std::fs::create_dir_all(&out).unwrap();
    let (rp, sp) = (free_port(), free_port());
    let rtmp_file = out.join("rtmp.flv");
    let srt_file = out.join("srt.ts");
    let Some(mut rtmp) = listener(
        &format!("rtmp://127.0.0.1:{rp}/live/{KEY}"),
        &["-listen", "1"],
        &rtmp_file,
    ) else {
        eprintln!("SKIPPED: no ffmpeg tool for the listeners");
        return;
    };
    let mut srt = listener(
        &format!("srt://127.0.0.1:{sp}?mode=listener"),
        &[],
        &srt_file,
    )
    .expect("ffmpeg");
    tokio::time::sleep(Duration::from_millis(800)).await;

    let mut ids = Vec::new();
    for t in [
        json!({"name": "RTMP here", "kind": "rtmp", "url": format!("rtmp://127.0.0.1:{rp}/live"), "key": KEY, "screen": screen}),
        json!({"name": "SRT here", "kind": "srt", "url": format!("srt://127.0.0.1:{sp}"), "screen": screen}),
    ] {
        let r = http(addr, "POST", "/api/v1/streaming/targets", &[], Some(t)).await;
        assert_eq!(r.status, 200);
        let id = r.json()["id"].as_str().unwrap().to_string();
        let r = http(
            addr,
            "POST",
            &format!("/api/v1/streaming/targets/{id}/start"),
            &[],
            None,
        )
        .await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        ids.push(id);
    }
    let s = wait_status(addr, 20, |s| {
        s["streams"].as_array().is_some_and(|a| {
            a.len() == 2
                && a.iter()
                    .all(|x| x["state"] == "live" && x["bytes"].as_u64() > Some(100_000))
        })
    })
    .await;
    assert!(!s.to_string().contains(KEY));
    tokio::time::sleep(Duration::from_secs(3)).await;
    for id in &ids {
        let r = http(
            addr,
            "POST",
            &format!("/api/v1/streaming/targets/{id}/stop"),
            &[],
            None,
        )
        .await;
        assert_eq!(r.status, 200);
    }
    // The listeners end when their senders close.
    for (child, name) in [(&mut rtmp, "rtmp"), (&mut srt, "srt")] {
        let end = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() && Instant::now() < end {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if child.try_wait().unwrap().is_none() {
            eprintln!("{name} listener still running: stopping it");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    check_file(&rtmp_file, 2.0);
    check_file(&srt_file, 2.0);
}

/// A saved stream key or SRT passphrase is only ever sent to the server it was entered for:
/// a client that points a target at another server (a paired client, or a typo) without
/// entering the secret again gets no secret for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_saved_stream_secret_goes_only_to_its_server() {
    let fx = fixture("stream-key-server");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let saved = || std::fs::read_to_string(fx.dir.join("machine/streaming.json")).unwrap();
    let r = http(
        addr,
        "POST",
        "/api/v1/streaming/targets",
        &[],
        Some(json!({"name": "Platform", "kind": "rtmp", "url": "rtmp://live.example.com/live2", "key": KEY})),
    )
    .await;
    let id = r.json()["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/streaming/targets/{id}");
    let put = |url: &str| {
        let body = json!({"name": "Platform", "kind": "rtmp", "url": url});
        http(addr, "PUT", &path, &[], Some(body))
    };
    // The same server (another application path, a host name in capitals): kept.
    let r = put("rtmp://live.example.com/live").await;
    assert_eq!(r.json()["hasKey"], true);
    let r = put("rtmp://LIVE.Example.com/live").await;
    assert_eq!(r.json()["hasKey"], true);
    // Another server, port or scheme: the key is entered again. Left out, the change is
    // refused (H05-1: the saved key is never dropped without a word); `""` saves the target
    // without a key.
    for url in [
        "rtmp://attacker.example/live",
        "rtmp://live.example.com:1936/live",
        "rtmps://live.example.com/live",
    ] {
        let r = http(
            addr,
            "PUT",
            &path,
            &[],
            Some(json!({"name": "Platform", "kind": "rtmp", "url": "rtmp://live.example.com/live", "key": KEY})),
        )
        .await;
        assert_eq!(r.json()["hasKey"], true);
        let r = put(url).await;
        assert_eq!(r.status, 400, "{url}: {}", String::from_utf8_lossy(&r.body));
        assert!(
            String::from_utf8_lossy(&r.body).contains("stream key again"),
            "{}",
            String::from_utf8_lossy(&r.body)
        );
        assert!(saved().contains(KEY), "{url}: the key is kept");
        assert!(!saved().contains(url), "{url}: the target is unchanged");
        let body = json!({"name": "Platform", "kind": "rtmp", "url": url, "key": ""});
        let r = http(addr, "PUT", &path, &[], Some(body)).await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        assert_eq!(r.json()["hasKey"], false, "{url}");
        assert!(!saved().contains(KEY), "{url}: the key is gone");
    }
    // A new key for a new server.
    let body = json!({"name": "Platform", "kind": "rtmp", "url": "rtmps://other.example.com/live", "key": "made-up-new-key"});
    let r = http(addr, "PUT", &path, &[], Some(body)).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["hasKey"], true);

    // SRT: hidden values are kept for the same server (another option added), refused for
    // another one; a new passphrase can be entered for it.
    let srt = http(
        addr,
        "POST",
        "/api/v1/streaming/targets",
        &[],
        Some(json!({"name": "Overflow", "kind": "srt", "url": "srt://192.0.2.7:9000?passphrase=madeuppass99&streamid=made-up-id"})),
    )
    .await
    .json();
    let spath = format!("/api/v1/streaming/targets/{}", srt["id"].as_str().unwrap());
    let sput = |url: &str| {
        let body = json!({"name": "Overflow", "kind": "srt", "url": url});
        http(addr, "PUT", &spath, &[], Some(body))
    };
    let r = sput("srt://192.0.2.7:9000?passphrase=***&streamid=***&latency=200000").await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(
        r.json()["url"],
        "srt://192.0.2.7:9000?passphrase=***&streamid=***&latency=***"
    );
    assert!(
        saved().contains(
            "srt://192.0.2.7:9000?passphrase=madeuppass99&streamid=made-up-id&latency=200000"
        ),
        "{}",
        saved()
    );
    let r = sput("srt://attacker.example:9000?passphrase=***&streamid=***").await;
    assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    assert!(!saved().contains("attacker.example"));
    let r = sput("srt://192.0.2.8:9000?passphrase=anotherpass11").await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert!(
        !saved().contains("madeuppass99"),
        "the old passphrase is gone"
    );
    server.shutdown().await.unwrap();
}
