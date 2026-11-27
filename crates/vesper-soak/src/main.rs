//! `vesper-soak`: the service-day soak harness (`docs/SOAK-PLAN.md`).
//!
//! It builds a synthetic library, starts a release-built Vesper server on it (at low priority,
//! with its own data folder and ports, no mDNS announcement, no displays), and plays a simulated
//! Sunday on a loop through the public REST and WebSocket API: pre-service announcements over a
//! motion loop, a camera (a synthetic IDN source sent by the harness) that is unplugged and
//! plugged in again, songs advancing every few seconds, Scripture, a video with sound, a sermon,
//! recording, captions when a model is given, two operators editing and triggering, and
//! operators' connections dropped and made again. Meanwhile it watches the server (memory,
//! files, threads, frame pacing, recording growth, WebSocket round trips) and the A/V sync at an
//! output, writes `soak.csv` every few seconds and `summary.txt` with pass/fail at the end.
//!
//! ```text
//! sg render -c "target/release/vesper-soak --duration 1h"
//! ```
//!
//! Ctrl-C (or SIGTERM) stops a run early with a summary ([`stop`]); `summary-so-far.txt` is
//! written every [`INTERIM_SECS`] while it runs.

mod camera;
mod http;
mod library;
mod operators;
mod probe;
mod report;
mod stop;
mod watch;

use anyhow::{Context, Result, bail};
use clap::Parser;
use library::{AV_SECS, Rng};
use operators::{Operator, WsMetrics};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Soak a Vesper server with a simulated Sunday on a loop.
#[derive(Parser, Debug)]
#[command(name = "vesper-soak", version, about)]
struct Args {
    /// How long to run (`90s`, `20m`, `1h`, `3.5h`).
    #[arg(long, default_value = "10m", value_parser = parse_duration)]
    duration: Duration,
    /// Audience screens, each with an IDN output (the first embeds the program audio and is
    /// the one the A/V probe receives).
    #[arg(long, default_value_t = 1)]
    outputs: u32,
    /// Screen size (every screen; 30 fps).
    #[arg(long, default_value = "1280x720")]
    size: String,
    /// The server binary (default: `$CARGO_TARGET_DIR/release/vesper`, else
    /// `target/release/vesper`).
    #[arg(long)]
    server: Option<PathBuf>,
    /// Folder for this run (data folder, server log, CSV, summary). Default: `soak-runs/<time>`.
    #[arg(long)]
    out: Option<PathBuf>,
    /// The server's API address.
    #[arg(long, default_value = "127.0.0.1:5655")]
    listen: SocketAddr,
    /// The server's audio device (`""` = none: the null output keeps the audio clock; a card's
    /// name, e.g. `hw:1,0`, plays on it).
    #[arg(long, default_value = "")]
    audio_device: String,
    /// Bible translations (`*.vbible`); default `target/scripture` when it exists.
    #[arg(long)]
    scripture_dir: Option<PathBuf>,
    /// A whisper model file (`ggml-<name>.bin`): captions run on the program audio. Without
    /// one, captions are skipped.
    #[arg(long)]
    captions_model: Option<PathBuf>,
    /// Niceness of the server (and of this harness).
    #[arg(long, default_value_t = 10)]
    nice: i32,
    /// Seconds between samples.
    #[arg(long, default_value_t = 5)]
    sample_secs: u64,
    /// Keep the data folder and recordings afterwards.
    #[arg(long)]
    keep_data: bool,
    /// Seconds between interim summaries (`summary-so-far.txt`).
    #[arg(long, default_value_t = INTERIM_SECS)]
    interim_secs: u64,
    /// Limit: memory growth after the warm-up, MB per hour.
    #[arg(long, default_value_t = 40.0)]
    max_rss_mb_per_hour: f64,
    /// Limit: open files more than after the warm-up.
    #[arg(long, default_value_t = 20)]
    max_fd_growth: i64,
    /// Limit: threads more than after the warm-up.
    #[arg(long, default_value_t = 10)]
    max_thread_growth: i64,
    /// Limit: late or dropped frames in the worst minute (every screen together).
    #[arg(long, default_value_t = 30.0)]
    max_late_per_min: f64,
    /// Limit: |A/V offset| at the output, ms.
    #[arg(long, default_value_t = 45.0)]
    max_av_ms: f64,
    /// Limit: change of the mean A/V offset over the run (last third of the plays against the
    /// first), ms.
    #[arg(long, default_value_t = 20.0)]
    max_av_drift_ms: f64,
    /// Limit: WebSocket heartbeat round trip (a sample's median), ms.
    #[arg(long, default_value_t = 250.0)]
    max_ws_ms: f64,
}

/// Seconds between interim summaries (`summary-so-far.txt`, `--interim-secs`).
const INTERIM_SECS: u64 = 600;

/// Operator connections the server closed this close before it stopped (or the operator stopped
/// the run) are the shutdown's, not losses.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(15);

fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let (num, mul) = match s.chars().last() {
        Some('h') => (&s[..s.len() - 1], 3600.0),
        Some('m') => (&s[..s.len() - 1], 60.0),
        Some('s') => (&s[..s.len() - 1], 1.0),
        _ => (s, 1.0),
    };
    let v: f64 = num.parse().map_err(|_| format!("bad duration {s:?}"))?;
    if v <= 0.0 {
        return Err("the duration must be positive".into());
    }
    Ok(Duration::from_secs_f64(v * mul))
}

/// What the scenario threads share.
struct Ctx {
    addr: SocketAddr,
    end: Instant,
    stop: Arc<AtomicBool>,
    phase: Arc<Mutex<String>>,
    ws: Arc<WsMetrics>,
    server: Mutex<Child>,
    server_exit: Mutex<Option<String>>,
    /// Why the run stopped before its end (the operator's signal, or the server stopped
    /// cleanly by someone), and when.
    early: Mutex<Option<(String, Instant)>>,
}

impl Ctx {
    /// The run goes on: before the end, the server alive.
    fn running(&self) -> bool {
        if self.stop.load(Ordering::Relaxed)
            || Instant::now() >= self.end
            || self.early.lock().unwrap().is_some()
        {
            return false;
        }
        if let Some(sig) = stop::requested() {
            self.stop_early(format!("{sig} to the harness"));
            return false;
        }
        if let Ok(Some(st)) = self.server.lock().unwrap().try_wait() {
            if st.success() {
                // The server exits 0 only on a shutdown signal (a crash or panic is never
                // 0): someone stopped it, as an operator would stop the run.
                self.stop_early(format!("the server was stopped from outside ({st})"));
            } else {
                *self.server_exit.lock().unwrap() = Some(format!("the server exited: {st}"));
            }
            self.stop.store(true, Ordering::Relaxed);
            return false;
        }
        true
    }

    fn stop_early(&self, why: String) {
        let mut e = self.early.lock().unwrap();
        if e.is_none() {
            eprintln!("[{}] stopping early: {why}", now_hms());
            *e = Some((why, Instant::now()));
        }
    }

    /// Operator connections the server closed, apart from those its shutdown closed.
    fn lost(&self) -> u64 {
        let stopped = self.early.lock().unwrap().as_ref().map(|e| e.1);
        let lost = self.ws.lost_at.lock().unwrap();
        lost.iter()
            .filter(|t| stopped.is_none_or(|s| **t + SHUTDOWN_GRACE < s))
            .count() as u64
    }

    /// Waits `secs`, or less when the run ends; `false` once it has.
    fn wait(&self, secs: f64) -> bool {
        let until = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < until {
            if !self.running() {
                return false;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.running()
    }

    fn set_phase(&self, p: &str) {
        *self.phase.lock().unwrap() = p.to_string();
        eprintln!("[{}] {p}", now_hms());
    }
}

fn now_hms() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{:02}:{:02}:{:02} UTC", s / 3600 % 24, s / 60 % 60, s % 60)
}

fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("vesper-soak: {e:#}");
            std::process::exit(2);
        }
    }
}

fn run() -> Result<bool> {
    let args = Args::parse();
    stop::install();
    let (w, h) = args
        .size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .context("--size WxH")?;
    // This harness at the low priority asked for (absolute, while it has one thread); the
    // server inherits it. (A `nice -n` on top of a reniced harness added up: the server ran at
    // 19, not 10, until lane P, 2026-11-26.)
    #[cfg(unix)]
    // SAFETY: a plain syscall on this process.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, args.nice);
    }
    let server_bin = match &args.server {
        Some(p) => p.clone(),
        None => std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target"))
            .join("release/vesper"),
    };
    if !server_bin.is_file() {
        bail!(
            "no server binary at {} (build it with `cargo build --release -p vesper-server`, or pass --server)",
            server_bin.display()
        );
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("soak-runs").join(format!("soak-{stamp}")));
    std::fs::create_dir_all(&out)?;
    let out = out.canonicalize()?;
    let data = out.join("data");
    let recordings = new_recordings_dir(&out)?;
    eprintln!("soak folder {}", out.display());

    let tag = format!("{}", std::process::id());
    let lib = library::build(&data, args.outputs, (w, h), &tag)?;
    let captions_model = match &args.captions_model {
        Some(m) => {
            let file = m
                .file_name()
                .context("model file")?
                .to_string_lossy()
                .to_string();
            let name = file
                .strip_prefix("ggml-")
                .and_then(|n| n.strip_suffix(".bin"))
                .context("a captions model is named ggml-<name>.bin")?
                .to_string();
            let dir = data.join("cache/whisper");
            std::fs::create_dir_all(&dir)?;
            std::fs::copy(m, dir.join(&file)).context("copy the captions model")?;
            Some(name)
        }
        None => None,
    };

    // The server.
    let mut cmd = Command::new(server_bin.canonicalize()?);
    // A process group of its own: the terminal's Ctrl-C reaches only the harness, which
    // stops the server after the summary.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.arg("--data")
        .arg(&data)
        .arg("--listen")
        .arg(args.listen.to_string())
        .args(["--no-mdns", "--no-displays"])
        .arg("--log-file")
        .arg(out.join("server.log"))
        .arg("--site-discovery-port")
        .arg((args.listen.port() + 1).to_string())
        .arg("--site-api-port")
        .arg((args.listen.port() + 2).to_string())
        .arg("--audio-device")
        .arg(&args.audio_device)
        .current_dir(&out);
    let scripture = args
        .scripture_dir
        .clone()
        .or_else(|| Some(PathBuf::from("target/scripture")).filter(|p| p.is_dir()));
    if let Some(s) = &scripture {
        cmd.arg("--scripture-dir").arg(s.canonicalize()?);
    }
    if std::env::var_os("RUST_LOG").is_none() {
        cmd.env("RUST_LOG", "info");
    }
    let child = cmd.spawn().context("start the server")?;
    let pid = child.id();
    stop::set_server(pid);
    eprintln!("server pid {pid} on {}", args.listen);
    let t0 = Instant::now();
    let ctx = Arc::new(Ctx {
        addr: args.listen,
        end: t0 + args.duration,
        stop: Arc::new(AtomicBool::new(false)),
        phase: Arc::new(Mutex::new("start".into())),
        ws: Arc::new(WsMetrics::default()),
        server: Mutex::new(child),
        server_exit: Mutex::new(None),
        early: Mutex::new(None),
    });
    let result = soak(&args, &ctx, &lib, &out, &recordings, captions_model, t0);
    // Stop the server (SIGTERM, then SIGKILL after 20 s).
    let _ = Command::new("kill").arg(pid.to_string()).output();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut server = ctx.server.lock().unwrap();
        match server.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) if Instant::now() > deadline => {
                // (The lock is held: taking it again here hung the harness.)
                eprintln!("the server did not stop in 20 s: killing it");
                let _ = server.kill();
            }
            Ok(None) => {}
        }
        drop(server);
        std::thread::sleep(Duration::from_millis(200));
    }
    if !args.keep_data {
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&recordings);
    }
    result
}

/// Sets the run up, plays Sundays until the end and writes the summary.
fn soak(
    args: &Args,
    ctx: &Arc<Ctx>,
    lib: &library::Library,
    out: &Path,
    recordings: &Path,
    captions_model: Option<String>,
    t0: Instant,
) -> Result<bool> {
    // Wait for the API.
    let up = Instant::now() + Duration::from_secs(90);
    loop {
        if http::get(ctx.addr, "/api/v1/server").is_ok() {
            break;
        }
        if !ctx.running() || Instant::now() > up {
            bail!(
                "the server did not answer (see {})",
                out.join("server.log").display()
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    // Recording (when the box can encode).
    let streaming = http::get(ctx.addr, "/api/v1/streaming").unwrap_or(Value::Null);
    let recording = streaming["encoders"]
        .as_array()
        .is_some_and(|e| e.iter().any(|c| c["codec"] == "h264"))
        && http::send(
            ctx.addr,
            "PUT",
            "/api/v1/streaming/recording",
            &json!({
                "screen": lib.screens[0].id,
                "container": "mp4",
                "codec": "h264",
                "videoMbps": 2.0,
                "audioKbps": 128,
                "splitMinutes": 10,
                "folder": recordings.to_string_lossy(),
                "minFreeGb": 5.0
            }),
        )
        .and_then(|_| http::send(ctx.addr, "POST", "/api/v1/recording/start", &json!({})))
        .map_err(|e| eprintln!("recording: {e:#}"))
        .is_ok();

    // Captions (when a model was given).
    let captions = captions_model.and_then(|m| {
        http::send(
            ctx.addr,
            "PUT",
            "/api/v1/captions/config",
            &json!({"enabled": true, "model": m}),
        )
        .map_err(|e| eprintln!("captions: {e:#}"))
        .ok()
        .map(|_| m)
    });

    // Scripture from a translation, when the server has one.
    let bible: Option<String> = http::get(ctx.addr, "/api/v1/scripture/translations")
        .ok()
        .and_then(|t| t[0]["id"].as_str().map(String::from))
        .and_then(|tr| {
            http::send(
                ctx.addr,
                "POST",
                "/api/v1/scripture/presentations",
                &json!({"translation": tr, "reference": "Psalm 23", "then": "none"}),
            )
            .map_err(|e| eprintln!("scripture: {e:#}"))
            .ok()
        })
        .and_then(|c| c["presentation"].as_str().map(String::from));

    // The camera, the probe, the watcher.
    let camera = camera::Camera::start(format!("Soak Camera {}", std::process::id()));
    let camera_name = camera.name(Duration::from_secs(10));
    let camera_full = Arc::new(Mutex::new(camera_name.clone()));
    let av = Arc::new(AtomicBool::new(false));
    let probe_stats = Arc::new(probe::ProbeStats::default());
    let probe = probe::spawn(
        lib.outputs[0].clone(),
        t0,
        av.clone(),
        ctx.stop.clone(),
        probe_stats.clone(),
    );
    let samples = Arc::new(Mutex::new(Vec::new()));
    let watcher = watch::spawn(
        watch::Watched {
            addr: ctx.addr,
            t0,
            every: Duration::from_secs(args.sample_secs.max(1)),
            phase: ctx.phase.clone(),
            camera: camera_full,
            ws: ctx.ws.clone(),
            probe: probe_stats.clone(),
            recordings: recordings.to_path_buf(),
            stop: ctx.stop.clone(),
        },
        &out.join("soak.csv"),
        samples.clone(),
    )?;

    // The second operator.
    let editor = {
        let ctx = ctx.clone();
        let songs: Vec<(String, Vec<String>)> = lib
            .songs
            .iter()
            .map(|p| {
                (
                    p.id.0.to_string(),
                    p.sections
                        .iter()
                        .flat_map(|s| s.slides.iter().map(|sl| sl.id.0.to_string()))
                        .collect(),
                )
            })
            .collect();
        let playlist = lib.playlist.id.0.to_string();
        std::thread::Builder::new()
            .name("operator b".into())
            .spawn(move || operator_b(&ctx, &songs, &playlist))?
    };

    let th = report::Thresholds {
        rss_slope_mb_h: args.max_rss_mb_per_hour,
        fd_growth: args.max_fd_growth,
        thread_growth: args.max_thread_growth,
        late_per_min: args.max_late_per_min,
        av_ms: args.max_av_ms,
        av_drift_ms: args.max_av_drift_ms,
        ws_ms: args.max_ws_ms,
        rec_stall_s: 30.0,
        camera_return_s: 20.0,
    };
    let camera_returns: Mutex<Vec<Option<f64>>> = Mutex::new(Vec::new());
    // The summary of what is in so far (`interim`) or of the whole run.
    let summarize = |interim: bool| {
        // Every lock taken and let go on its own line: a guard held through the `Run` literal
        // while `ctx.lost()` took the same lock again hung the first interim summary.
        let samples = samples.lock().unwrap().clone();
        let phases = probe_stats.phases.lock().unwrap().clone();
        let early_stop = ctx.early.lock().unwrap().as_ref().map(|e| e.0.clone());
        let last_errors = ctx.ws.last_errors.lock().unwrap().clone();
        let lost = ctx.lost();
        let returns = camera_returns.lock().unwrap().clone();
        let server_exit = ctx.server_exit.lock().unwrap().clone();
        let run = report::Run {
            duration_s: t0.elapsed().as_secs_f64(),
            planned_s: args.duration.as_secs_f64(),
            interim,
            early_stop,
            samples: &samples,
            phases: &phases,
            commands: ctx.ws.commands_sent.load(Ordering::Relaxed),
            command_errors: ctx.ws.command_errors.load(Ordering::Relaxed),
            last_errors,
            drops: ctx.ws.drops.load(Ordering::Relaxed),
            lost,
            camera_returns: returns,
            probe_reconnects: probe_stats.reconnects.load(Ordering::Relaxed),
            camera_errors: camera.errors.load(Ordering::Relaxed),
            recording,
            captions: captions.clone(),
            scripture: bible.is_some(),
            server_exit,
        };
        (report::summary(&run, &th), phases)
    };

    // Operator A plays Sundays; every few minutes the summary so far.
    let sundays_done = AtomicBool::new(false);
    std::thread::scope(|sc| {
        sc.spawn(|| {
            let every = Duration::from_secs(args.interim_secs.max(10));
            let mut next = Instant::now() + every;
            while !sundays_done.load(Ordering::Relaxed) && !ctx.stop.load(Ordering::Relaxed) {
                if Instant::now() >= next {
                    next += every;
                    let ((text, _), _) = summarize(true);
                    if let Err(e) = std::fs::write(out.join("summary-so-far.txt"), text) {
                        eprintln!("summary-so-far.txt: {e}");
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        });
        let mut cycle = 0u32;
        while ctx.running() {
            cycle += 1;
            if let Err(e) = sunday(
                ctx,
                lib,
                cycle,
                &camera,
                camera_name.as_deref(),
                bible.as_deref(),
                &av,
                &camera_returns,
            ) {
                eprintln!("cycle {cycle}: {e:#}");
                if !ctx.wait(5.0) {
                    break;
                }
            }
        }
        sundays_done.store(true, Ordering::Relaxed);
    });
    av.store(false, Ordering::Relaxed);
    ctx.set_phase("end");
    // Let the watcher take a last sample (the server still answers when the operator stopped
    // the run), then stop everything.
    std::thread::sleep(Duration::from_secs(args.sample_secs.max(1) + 1));
    ctx.stop.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    // Only after the watcher's last sample: a sample of the stopped recording ("off") would
    // fail the recording check.
    if recording {
        let _ = http::send(ctx.addr, "POST", "/api/v1/recording/stop", &json!({}));
    }
    let _ = editor.join();
    let _ = probe.join();

    let ((text, pass), phases) = summarize(false);
    drop(camera);
    let mut av_csv = String::from("t_s,beeps,flashes,paired,mean_ms,worst_ms\n");
    for p in &phases {
        av_csv.push_str(&format!(
            "{:.1},{},{},{},{},{}\n",
            p.t,
            p.beeps,
            p.flashes,
            p.offsets.len(),
            p.mean().map(|m| format!("{m:.2}")).unwrap_or_default(),
            p.worst().map(|m| format!("{m:.2}")).unwrap_or_default()
        ));
    }
    std::fs::write(out.join("av.csv"), av_csv)?;
    std::fs::write(out.join("summary.txt"), &text)?;
    println!("{text}");
    println!("CSV: {}", out.join("soak.csv").display());
    Ok(pass)
}

/// Sends a command as operator `op`; refusals are counted (and logged) by the metrics.
fn cmd(op: &Operator, c: Value) -> bool {
    match op.command(c) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("  {e:#}");
            false
        }
    }
}

/// Triggers playlist item `i` of the service playlist.
fn item(op: &Operator, lib: &library::Library, i: usize) -> bool {
    let items = lib.items();
    cmd(
        op,
        json!({"type": "triggerPlaylistItem", "playlist": lib.playlist.id, "item": items[i].id}),
    )
}

/// `next` `n` times, `every` seconds apart.
fn advance(ctx: &Ctx, op: &Operator, n: usize, every: f64) -> bool {
    for _ in 0..n {
        if !ctx.wait(every) {
            return false;
        }
        cmd(op, json!({"type": "next"}));
    }
    ctx.wait(every)
}

fn connect(ctx: &Ctx, name: &str) -> Result<Operator> {
    let mut last = None;
    for _ in 0..10 {
        match Operator::connect(ctx.addr, name, ctx.ws.clone()) {
            Ok(o) => return Ok(o),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("connect")))
}

/// One simulated Sunday (about 4½ minutes) by operator A.
#[allow(clippy::too_many_arguments)]
fn sunday(
    ctx: &Ctx,
    lib: &library::Library,
    cycle: u32,
    camera: &camera::Camera,
    camera_name: Option<&str>,
    bible: Option<&str>,
    av: &AtomicBool,
    camera_returns: &Mutex<Vec<Option<f64>>>,
) -> Result<()> {
    let mut a = connect(ctx, "Soak Operator A")?;

    ctx.set_phase("pre-service");
    cmd(
        &a,
        json!({"type": "triggerMedia", "asset": lib.loop_video.id}),
    );
    item(&a, lib, 0);
    if !advance(ctx, &a, 5, 6.0) {
        return Ok(());
    }

    ctx.set_phase("camera");
    cmd(&a, json!({"type": "clearLayer", "layer": "slide"}));
    cmd(&a, json!({"type": "clearLayer", "layer": "media"}));
    if let Some(name) = camera_name {
        cmd(&a, json!({"type": "triggerLiveSource", "source": name}));
    }
    if !ctx.wait(8.0) {
        return Ok(());
    }
    if cycle % 2 == 1 && camera_name.is_some() {
        // Unplugged while live, and plugged in again 10 s later.
        ctx.set_phase("camera-drop");
        camera.on.store(false, Ordering::Relaxed);
        if !ctx.wait(10.0) {
            camera.on.store(true, Ordering::Relaxed);
            return Ok(());
        }
        camera.on.store(true, Ordering::Relaxed);
        let back = Instant::now();
        let mut live = None;
        while ctx.running() && back.elapsed() < Duration::from_secs(30) {
            let status = http::get(ctx.addr, "/api/v1/sources").ok().and_then(|s| {
                s["sources"].as_array().and_then(|a| {
                    a.iter()
                        .find(|x| x["name"].as_str() == camera_name)
                        .and_then(|x| x["status"].as_str().map(String::from))
                })
            });
            if status.as_deref() == Some("live") {
                live = Some(back.elapsed().as_secs_f64());
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if !ctx.running() {
            return Ok(());
        }
        eprintln!("  camera back live after {live:?} s");
        camera_returns.lock().unwrap().push(live);
        ctx.set_phase("camera");
        if !ctx.wait(5.0) {
            return Ok(());
        }
    } else if !ctx.wait(15.0) {
        return Ok(());
    }

    ctx.set_phase("worship");
    for (k, i) in (1..=3).enumerate() {
        cmd(
            &a,
            json!({"type": "triggerMedia", "asset": lib.images[k % lib.images.len()].id}),
        );
        item(&a, lib, i);
        if !advance(ctx, &a, 8, 3.0) {
            return Ok(());
        }
    }

    // Operator A's network drops for 3 s.
    ctx.set_phase("operator-drop");
    a.drop_connection();
    if !ctx.wait(3.0) {
        return Ok(());
    }
    a = connect(ctx, "Soak Operator A")?;

    ctx.set_phase("scripture");
    cmd(&a, json!({"type": "clearLayer", "layer": "media"}));
    match bible {
        Some(p) => {
            cmd(
                &a,
                json!({"type": "triggerSlide", "presentation": p, "index": 0}),
            );
        }
        None => {
            item(&a, lib, 4);
        }
    }
    if !advance(ctx, &a, 4, 5.0) {
        return Ok(());
    }

    ctx.set_phase("video");
    cmd(&a, json!({"type": "clearLayer", "layer": "slide"}));
    av.store(true, Ordering::Relaxed);
    cmd(
        &a,
        json!({"type": "triggerMedia", "asset": lib.av_video.id}),
    );
    let played = ctx.wait(AV_SECS as f64 + 1.0);
    av.store(false, Ordering::Relaxed);
    if !played {
        return Ok(());
    }
    cmd(&a, json!({"type": "clearLayer", "layer": "media"}));

    ctx.set_phase("sermon");
    item(&a, lib, 5);
    cmd(
        &a,
        json!({"type": "stageMessage", "text": "Five minutes left", "flash": true}),
    );
    if !advance(ctx, &a, 4, 6.0) {
        return Ok(());
    }
    cmd(&a, json!({"type": "stageMessageClear"}));

    ctx.set_phase("closing");
    cmd(
        &a,
        json!({"type": "triggerMedia", "asset": lib.loop_video.id}),
    );
    item(&a, lib, 6);
    if !advance(ctx, &a, 4, 3.0) {
        return Ok(());
    }
    cmd(&a, json!({"type": "clearAll"}));
    ctx.wait(3.0);
    Ok(())
}

/// Operator B: edits song slides (the live one too), browses thumbnails and the playlist,
/// warms thumbnails, and loses its connection for 5 s every 90 s.
fn operator_b(ctx: &Ctx, songs: &[(String, Vec<String>)], playlist: &str) {
    let mut rng = Rng::new(0xb0b);
    let mut op = None;
    let mut since = Instant::now();
    let mut n = 0u64;
    while ctx.running() {
        if op.is_none() {
            op = connect(ctx, "Soak Operator B").ok();
            since = Instant::now();
        }
        if since.elapsed() >= Duration::from_secs(90)
            && let Some(o) = op.take()
        {
            o.drop_connection();
            if !ctx.wait(5.0) {
                break;
            }
            continue;
        }
        n += 1;
        let (pres, slides) = &songs[rng.below(songs.len())];
        let slide = &slides[rng.below(slides.len())];
        match n % 4 {
            0 | 2 => {
                let text = format!("{}\n{}", library::line(&mut rng), library::line(&mut rng));
                let edit = json!({
                    "ops": [{"type": "setSlideText", "slide": slide, "text": text}],
                    "clientId": "soak-operator-b"
                });
                if let Err(e) = http::send(
                    ctx.addr,
                    "POST",
                    &format!("/api/v1/presentations/{pres}/edits"),
                    &edit,
                ) {
                    eprintln!("  operator B edit: {e:#}");
                    ctx.ws.command_errors.fetch_add(1, Ordering::Relaxed);
                }
                ctx.ws.commands_sent.fetch_add(1, Ordering::Relaxed);
            }
            1 => {
                if let Some(o) = &op {
                    cmd(
                        o,
                        json!({"type": "prefetchThumbnails", "presentation": pres, "width": 320}),
                    );
                }
            }
            _ => {
                let _ = http::request(
                    ctx.addr,
                    "GET",
                    &format!("/api/v1/presentations/{pres}/slides/{slide}/thumbnail?w=320"),
                    None,
                );
                let _ = http::get(ctx.addr, &format!("/api/v1/playlists/{playlist}"));
            }
        }
        if !ctx.wait(3.0) {
            break;
        }
    }
}

/// The run's recordings folder in `out`, made new: its files are deleted as parts finish and at
/// the end, so never a folder that was there (a user's own recordings).
fn new_recordings_dir(out: &Path) -> Result<PathBuf> {
    let recordings = out.join("recordings");
    if recordings.exists() {
        bail!(
            "{} exists already: give --out a new folder (the run deletes its recordings)",
            recordings.display()
        );
    }
    std::fs::create_dir_all(&recordings)?;
    Ok(recordings)
}

#[cfg(test)]
mod tests {
    /// H05-2 (bug run 6): a `recordings` folder already in `--out` is refused, never used.
    #[test]
    fn an_existing_recordings_folder_is_refused() {
        let out = std::env::temp_dir().join(format!("vesper-soak-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let r = super::new_recordings_dir(&out).unwrap();
        assert!(r.is_dir());
        std::fs::write(r.join("mine.mp4"), b"x").unwrap();
        let e = super::new_recordings_dir(&out).unwrap_err();
        assert!(e.to_string().contains("exists already"), "{e}");
        assert!(r.join("mine.mp4").is_file());
        let _ = std::fs::remove_dir_all(&out);
    }
}
