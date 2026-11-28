//! Live captions (`docs/CAPTIONS-PLAN.md`): the configuration API, lines from another
//! captioner (the `external` source) on the captions layer, max lines, clearing, looks and
//! the hold. The speech engine itself (whisper) needs a model of 75 MB or more and speech to
//! hear (no speech synthesiser here): `engine_runs_on_the_program_bus` (CPU) and
//! `engine_runs_on_the_gpu_or_says_why_not` run it when
//! `VESPER_TEST_WHISPER_DIR` names a folder holding `ggml-base.en.bin`. whisper runs in IDN's
//! worker process, the `vesper` binary (`vesper --idn-captions-worker`), as in the server.

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

/// The band colour the tests use (opaque green: easy to count).
const BAND: [u8; 3] = [0, 255, 0];

async fn put_config(addr: std::net::SocketAddr, cfg: Value) -> Resp {
    http(addr, "PUT", "/api/v1/captions/config", &[], Some(cfg)).await
}

async fn push(addr: std::net::SocketAddr, text: &str, fin: bool) -> Resp {
    http(
        addr,
        "POST",
        "/api/v1/captions/lines",
        &[],
        Some(json!({"text": text, "final": fin})),
    )
    .await
}

fn band_pixels(img: &image::RgbaImage) -> usize {
    count(img, H / 2, H, BAND, 12)
}

fn lines(state: &Value) -> Vec<(String, bool)> {
    state["captions"]["lines"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|l| {
                    (
                        l["text"].as_str().unwrap_or("").to_string(),
                        l["final"].as_bool().unwrap_or(false),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn captions_layer_with_lines_from_another_captioner() {
    let fx = fixture("captions");
    // A stage screen (third in sort order): captions only with `stage` on.
    let stage = model::Screen {
        name: "Stage".into(),
        sort_key: "c".into(),
        size: model::Size::new(W as f64, H as f64),
        frame_rate: model::FrameRate::FPS30,
        kind: model::ScreenKind::Stage { layout: None },
        ..model::Screen::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "screen",
        &stage.id.0.to_string(),
        &stage,
    );
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;

    // ---- the configuration ------------------------------------------------------------------
    let cfg = get_json(addr, "/api/v1/captions/config").await;
    assert_eq!(
        cfg.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["schemaVersion"],
        "captions are off by default (every field at its default)"
    );
    assert_eq!(ws.state["captions"]["status"], "off");
    assert_eq!(ws.state["captions"]["enabled"], false);
    let models = get_json(addr, "/api/v1/captions/models").await;
    let names: Vec<&str> = models
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"base.en") && names.contains(&"large-v3-turbo"),
        "{models}"
    );
    assert!(models[0]["sizeMb"].as_u64().unwrap() > 10);
    assert_eq!(models[0]["downloaded"], false);

    assert_eq!(push(addr, "Too soon", true).await.status, 409, "off");
    for (bad, why) in [
        (json!({"maxLines": 9}), "maxLines"),
        (json!({"model": "huge"}), "model"),
        (
            json!({"language": "es", "model": "base.en"}),
            "English only",
        ),
        (
            json!({"source": {"type": "liveSource", "name": " "}}),
            "live source",
        ),
        (json!({"style": {"size": 2}}), "size"),
    ] {
        let r = put_config(addr, bad.clone()).await;
        assert_eq!(r.status, 400, "{bad}");
        assert!(
            r.json()["error"]["message"].as_str().unwrap().contains(why),
            "{bad}: {:?}",
            r.json()
        );
    }

    let on = json!({
        "enabled": true,
        "source": {"type": "external"},
        "maxLines": 2,
        "style": {"background": {"r": 0, "g": 1, "b": 0}, "size": 40},
    });
    let r = put_config(addr, on.clone()).await;
    assert_eq!(r.status, 200, "{:?}", r.json());
    assert_eq!(r.json()["enabled"], true);
    assert_eq!(
        get_json(addr, "/api/v1/captions/config").await,
        r.json(),
        "stored"
    );
    ws.wait_state("running", |_, s| {
        s["captions"]["enabled"] == true && s["captions"]["status"] == "running"
    })
    .await;

    // ---- lines on the screen ----------------------------------------------------------------
    let before = wait_frame(&server, 0, "nothing yet", |i| band_pixels(i) == 0).await;
    assert_eq!(band_pixels(&before), 0);
    assert_eq!(push(addr, "Welcome to the", false).await.status, 204);
    ws.wait_state("draft", |_, s| {
        lines(s) == vec![("Welcome to the".to_string(), false)]
    })
    .await;
    let drawn = wait_frame(&server, 0, "the band", |i| band_pixels(i) > 2000).await;
    // The band sits along the bottom: nothing of it in the top half.
    assert_eq!(count(&drawn, 0, H / 2, BAND, 12), 0, "bottom only");
    assert!(
        count(&drawn, H / 2, H, WHITE, 60) > 50,
        "the text is drawn over the band"
    );
    // The key screen draws the layer too (every layer on every screen without a look).
    wait_frame(&server, 1, "key screen", |i| band_pixels(i) > 2000).await;
    // Not on the stage screen, until `stage` is on.
    assert_eq!(band_pixels(&latest(&server, 2).unwrap()), 0, "not on stage");
    let mut with_stage = on.clone();
    with_stage["stage"] = json!(true);
    assert_eq!(put_config(addr, with_stage).await.status, 200);
    wait_frame(&server, 2, "on stage too", |i| band_pixels(i) > 2000).await;
    assert_eq!(put_config(addr, on.clone()).await.status, 200);
    wait_frame(&server, 2, "off stage again", |i| band_pixels(i) == 0).await;
    assert_eq!(
        lines(&ws.state).len(),
        1,
        "a configuration change keeps the lines"
    );

    // A draft is corrected in place, then made final; the next lines push it up, and only
    // the last two stay.
    assert_eq!(
        push(addr, "Welcome to the service", false).await.status,
        204
    );
    assert_eq!(
        push(addr, "Welcome to the service.", true).await.status,
        204
    );
    assert_eq!(push(addr, "Please stand.", true).await.status, 204);
    assert_eq!(push(addr, "Let us pray", false).await.status, 204);
    ws.wait_state("two lines", |_, s| {
        lines(s)
            == vec![
                ("Please stand.".to_string(), true),
                ("Let us pray".to_string(), false),
            ]
    })
    .await;
    // Two lines make a taller band than one.
    let one = band_pixels(&drawn);
    wait_frame(&server, 0, "a taller band for two lines", |i| {
        band_pixels(i) > one + one / 4
    })
    .await;

    // ---- clearLayer, looks ------------------------------------------------------------------
    ws.ok(json!({"type": "clearLayer", "layer": "captions"}))
        .await;
    wait_frame(&server, 0, "cleared", |i| band_pixels(i) == 0).await;
    ws.wait_state("no lines", |_, s| lines(s).is_empty()).await;
    assert_eq!(
        ws.state["captions"]["status"], "running",
        "captions keep running"
    );

    let screens = get_json(addr, "/api/v1/screens").await;
    let audience = screens
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "Audience")
        .unwrap()["id"]
        .clone();
    let look = http(
        addr,
        "POST",
        "/api/v1/looks",
        &[],
        Some(json!({"name": "No captions", "screens": [
            {"screen": audience, "layers": {"captions": false}}]})),
    )
    .await;
    assert_eq!(look.status, 201, "{:?}", look.json());
    let look_id = look.json()["id"].clone();
    ws.ok(json!({"type": "setLook", "look": look_id})).await;
    assert_eq!(push(addr, "Amen.", true).await.status, 204);
    wait_frame(&server, 1, "the key screen still shows them", |i| {
        band_pixels(i) > 2000
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        band_pixels(&latest(&server, 0).unwrap()),
        0,
        "the look hides captions on the audience screen"
    );
    ws.ok(json!({"type": "setLook", "look": null})).await;
    wait_frame(&server, 0, "back with every layer", |i| {
        band_pixels(i) > 2000
    })
    .await;

    // ---- the hold: lines leave the screens after 8 s without a new one -----------------------
    let started = std::time::Instant::now();
    wait_frame(&server, 0, "held lines leave", |i| band_pixels(i) == 0).await;
    assert!(started.elapsed() >= Duration::from_secs(5), "held a while");
    assert!(!lines(&ws.state).is_empty(), "the state keeps them");

    // ---- off --------------------------------------------------------------------------------
    let mut off = on.clone();
    off["enabled"] = json!(false);
    assert_eq!(put_config(addr, off).await.status, 200);
    ws.wait_state("off", |_, s| {
        s["captions"]["status"] == "off" && lines(s).is_empty()
    })
    .await;
    assert_eq!(push(addr, "Late", true).await.status, 409);
    server.shutdown().await.unwrap();

    // The configuration is kept for the next start.
    let server = start_server(&fx, true, false).await;
    let cfg = get_json(server.addr, "/api/v1/captions/config").await;
    assert_eq!(cfg["source"]["type"], "external");
    assert_eq!(
        cfg["maxLines"],
        json!(null),
        "2 is the default (not written)"
    );
    assert_eq!(cfg.get("enabled"), None);
    server.shutdown().await.unwrap();
}

/// The speech engine on the program bus with a real model: it loads (from the data folder's
/// model cache) and runs on the CPU (`processor: "cpu"`). Needs `VESPER_TEST_WHISPER_DIR` (a
/// folder with `ggml-base.en.bin`); skipped otherwise.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_runs_on_the_program_bus() {
    let Some(models) = std::env::var_os("VESPER_TEST_WHISPER_DIR") else {
        eprintln!("skipped: VESPER_TEST_WHISPER_DIR is not set");
        return;
    };
    let (server, ws, _workers) =
        engine_server("captions-engine", &models, json!({"processor": "cpu"})).await;
    let addr = server.addr;
    let c = &ws.state["captions"];
    assert_eq!(c["status"], "running", "{c}");
    assert_eq!(c["model"], "base.en");
    assert!(c["device"].as_str().unwrap().starts_with("CPU"), "{c}");
    assert_eq!(c["gpu"], false, "{c}");
    assert!(c["note"].is_null(), "{c}");
    let models = get_json(addr, "/api/v1/captions/models").await;
    assert!(
        models
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["name"] == "base.en" && m["downloaded"] == true)
    );
    // whisper in its worker process: a child of this one, the `vesper` binary.
    #[cfg(target_os = "linux")]
    assert!(
        !captions_workers().is_empty(),
        "no `vesper --idn-captions-worker` child"
    );
    // Silence on the bus: no lines.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(lines(&ws.state).is_empty(), "{}", ws.state["captions"]);
    server.shutdown().await.unwrap();
}

/// The speech engine with `processor: "gpu"`: on the box's GPU (Vulkan, in the worker) when
/// it has one, else on the CPU with a note saying why; either way captions run. Needs
/// `VESPER_TEST_WHISPER_DIR`, as above.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_runs_on_the_gpu_or_says_why_not() {
    let Some(models) = std::env::var_os("VESPER_TEST_WHISPER_DIR") else {
        eprintln!("skipped: VESPER_TEST_WHISPER_DIR is not set");
        return;
    };
    let (server, ws, _workers) =
        engine_server("captions-engine-gpu", &models, json!({"processor": "gpu"})).await;
    let c = &ws.state["captions"];
    eprintln!("captions on: {c}");
    assert_eq!(c["status"], "running", "{c}");
    let device = c["device"].as_str().unwrap();
    if c["gpu"] == true {
        assert!(!device.starts_with("CPU"), "{c}");
        assert!(c["note"].is_null(), "{c}");
    } else {
        assert!(device.starts_with("CPU"), "{c}");
        assert!(c["note"].as_str().is_some_and(|n| n.contains("CPU")), "{c}");
    }
    #[cfg(target_os = "linux")]
    assert!(!captions_workers().is_empty(), "no worker");
    server.shutdown().await.unwrap();
}

/// A server whose captions run `base.en` (from `models`) with the settings `more` (the
/// processor, the source), once the engine is running or has failed (the state then in the
/// client).
async fn engine_server(
    name: &str,
    models: &std::ffi::OsStr,
    more: Value,
) -> (
    vesper_server::RunningServer,
    Client,
    tokio::sync::MutexGuard<'static, ()>,
) {
    let workers = WORKERS.lock().await;
    let fx = fixture(name);
    let cache = fx.dir.join("cache").join("whisper");
    std::fs::create_dir_all(&cache).unwrap();
    let file = std::path::Path::new(&models).join("ggml-base.en.bin");
    assert!(file.is_file(), "{} is missing", file.display());
    #[cfg(unix)]
    std::os::unix::fs::symlink(&file, cache.join("ggml-base.en.bin")).unwrap();
    #[cfg(not(unix))]
    std::fs::copy(&file, cache.join("ggml-base.en.bin")).unwrap();
    let server = start_server_with(&fx, |c| c.captions.worker = Some(vesper())).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let mut cfg = json!({"enabled": true, "model": "base.en"});
    cfg.as_object_mut()
        .unwrap()
        .extend(more.as_object().unwrap().clone());
    let r = put_config(addr, cfg).await;
    assert_eq!(r.status, 200, "{:?}", r.json());
    ws.wait_state("the engine runs", |_, s| {
        s["captions"]["status"] == "running" || s["captions"]["status"] == "error"
    })
    .await;
    (server, ws, workers)
}

/// IDN Beta21's `Engine::tend`: with no sound to step with (a network source that sends
/// nothing), a worker that ends is still noticed and started again on IDN's schedule (1 s
/// after it ended), not only once sound comes. Needs `VESPER_TEST_WHISPER_DIR`, as above.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_that_ends_in_silence_starts_again() {
    let Some(models) = std::env::var_os("VESPER_TEST_WHISPER_DIR") else {
        eprintln!("skipped: VESPER_TEST_WHISPER_DIR is not set");
        return;
    };
    let source = json!({"source": {"type": "liveSource", "name": "NOWHERE (Synthetic Mic)"},
                        "processor": "cpu"});
    let (server, ws, _workers) = engine_server("captions-engine-tend", &models, source).await;
    assert_eq!(
        ws.state["captions"]["status"], "running",
        "{}",
        ws.state["captions"]
    );
    // Ours alone (a worker of the test before may still be ending).
    let end = std::time::Instant::now() + Duration::from_secs(10);
    let mut before = captions_workers();
    while before.len() != 1 && std::time::Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(100)).await;
        before = captions_workers();
    }
    assert_eq!(before.len(), 1, "one worker: {before:?}");
    let killed = std::process::Command::new("kill")
        .args(["-KILL", &before[0].to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    // Started again within IDN's first wait (1 s) and a few of our steps (0.5 s each).
    let end = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let now = captions_workers();
        if now.iter().any(|p| !before.contains(p)) {
            break;
        }
        assert!(
            std::time::Instant::now() < end,
            "no new worker after the first one ended: {now:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    server.shutdown().await.unwrap();
}

/// Held by each test that starts a captions worker: they count this process's worker children,
/// so two at once would see each other's.
static WORKERS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The `vesper` binary: IDN's captions worker in the server (`vesper --idn-captions-worker`).
fn vesper() -> std::path::PathBuf {
    env!("CARGO_BIN_EXE_vesper").into()
}

/// This process's children that are captions workers (their PIDs).
#[cfg(target_os = "linux")]
fn captions_workers() -> Vec<u32> {
    let me = std::process::id().to_string();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            // `pid (comm) state ppid …`: the fields after the name's closing parenthesis.
            let ppid = stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().nth(1));
            let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            ppid == Some(me.as_str())
                && String::from_utf8_lossy(&cmdline).contains("--idn-captions-worker")
        })
        .collect()
}

/// A speech engine whose worker can't load its model (a damaged file here; the same path
/// as a worker that ends twice or is too slow) shows the error in the captions status and
/// is tried again; the server goes on (the show answers, captions turn off cleanly). Runs
/// without a real model: the test's file is used as it is.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_engine_whose_worker_fails_leaves_the_server_running() {
    let _workers = WORKERS.lock().await;
    let fx = fixture("captions-worker-fails");
    let model = fx.dir.join("ggml-base.en.bin");
    std::fs::write(&model, b"not a whisper model: synthetic bytes for the test").unwrap();
    let server = start_server_with(&fx, |c| {
        c.captions.worker = Some(vesper());
        c.captions.model_file = Some(model.clone());
    })
    .await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let r = put_config(addr, json!({"enabled": true, "model": "base.en"})).await;
    assert_eq!(r.status, 200, "{:?}", r.json());
    ws.wait_state("the engine fails", |_, s| {
        s["captions"]["status"] == "error"
    })
    .await;
    let c = ws.state["captions"].clone();
    let detail = c["detail"].as_str().unwrap_or_default();
    assert!(
        detail.starts_with("loading the base.en model: ")
            && detail.ends_with("(trying again in 5 s)"),
        "{c}"
    );
    // The server goes on: the API answers.
    let cfg = get_json(addr, "/api/v1/captions/config").await;
    assert_eq!(cfg["enabled"], true, "{cfg}");
    // Tried again after the first wait, failing the same way (the second wait is longer).
    ws.wait_state("the second try", |_, s| {
        s["captions"]["detail"]
            .as_str()
            .is_some_and(|d| d.ends_with("(trying again in 30 s)"))
    })
    .await;
    // Turned off: no error left, no worker left behind.
    let r = put_config(addr, json!({"enabled": false})).await;
    assert_eq!(r.status, 200, "{:?}", r.json());
    ws.wait_state("captions off", |_, s| s["captions"]["status"] == "off")
        .await;
    server.shutdown().await.unwrap();
    #[cfg(target_os = "linux")]
    {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while !captions_workers().is_empty() && std::time::Instant::now() < end {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(
            captions_workers(),
            Vec::<u32>::new(),
            "a worker was left running"
        );
    }
}
