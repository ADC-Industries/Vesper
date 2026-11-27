//! The watcher: every few seconds it reads the server's health counters
//! (`GET /api/v1/server/stats`), the recording's status, the camera's status as the server sees
//! it, the operators' round trips and the probe's counters, and appends a row to `soak.csv`.

use crate::http;
use crate::operators::WsMetrics;
use crate::probe::ProbeStats;
use serde_json::Value;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One row of `soak.csv`.
#[derive(Clone, Debug, Default)]
pub struct Sample {
    pub t: f64,
    pub phase: String,
    pub alive: bool,
    pub rss_mb: f64,
    pub threads: u32,
    pub fds: u32,
    pub ws_clients: u32,
    pub screens: u32,
    pub ticks: u64,
    pub late_ticks: u64,
    pub late_renders: u64,
    pub frames_dropped: u64,
    pub videos: u32,
    pub video_late: u64,
    pub xruns: u64,
    pub rec_state: String,
    pub rec_bytes: u64,
    pub rec_total_bytes: u64,
    pub rec_dropped: u64,
    pub camera: String,
    pub ping_p50: f64,
    pub ping_max: f64,
    pub cmd_p50: f64,
    pub cmd_max: f64,
    pub cmd_errors: u64,
    pub rx_frames: u64,
    pub rx_gaps: u64,
    /// The machine's 1-minute load average (other work on the box skews the frame numbers).
    pub load1: f64,
    /// Heap in use and held free by the allocator, MB (Linux with glibc; else 0): resident
    /// memory growing with the free part flat is a leak, with the free part growing is
    /// fragmentation.
    pub heap_mb: f64,
    pub heap_free_mb: f64,
    /// The server's caches, MB (bytes counted; `caches.csv` has each).
    pub caches_mb: f64,
    /// Memory the server's Rust allocator (mimalloc) has committed, MB (0 on glibc's malloc;
    /// with it, `heap_mb`/`heap_free_mb` count only the C libraries).
    pub rust_heap_mb: f64,
}

const HEADER: &str = "t_s,phase,alive,rss_mb,threads,fds,ws_clients,screens,ticks,late_ticks,late_renders,frames_dropped,videos,video_late,audio_xruns,rec_state,rec_bytes,rec_total_bytes,rec_dropped,camera,ping_p50_ms,ping_max_ms,cmd_p50_ms,cmd_max_ms,cmd_errors,rx_frames,rx_gaps,load1,heap_mb,heap_free_mb,caches_mb,rust_heap_mb";

impl Sample {
    fn row(&self) -> String {
        format!(
            "{:.1},{},{},{:.1},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.1},{:.1},{:.1},{:.1},{},{},{},{:.2},{:.1},{:.1},{:.1},{:.1}",
            self.t,
            self.phase,
            self.alive as u8,
            self.rss_mb,
            self.threads,
            self.fds,
            self.ws_clients,
            self.screens,
            self.ticks,
            self.late_ticks,
            self.late_renders,
            self.frames_dropped,
            self.videos,
            self.video_late,
            self.xruns,
            self.rec_state,
            self.rec_bytes,
            self.rec_total_bytes,
            self.rec_dropped,
            self.camera,
            self.ping_p50,
            self.ping_max,
            self.cmd_p50,
            self.cmd_max,
            self.cmd_errors,
            self.rx_frames,
            self.rx_gaps,
            self.load1,
            self.heap_mb,
            self.heap_free_mb,
            self.caches_mb,
            self.rust_heap_mb,
        )
    }
}

/// What the watcher reads besides the server.
pub struct Watched {
    pub addr: SocketAddr,
    pub t0: Instant,
    pub every: Duration,
    pub phase: Arc<Mutex<String>>,
    pub camera: Arc<Mutex<Option<String>>>,
    pub ws: Arc<WsMetrics>,
    pub probe: Arc<ProbeStats>,
    pub recordings: PathBuf,
    pub stop: Arc<AtomicBool>,
}

fn p50_max(v: &mut Vec<f64>) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    v.sort_by(f64::total_cmp);
    let r = (v[v.len() / 2], v[v.len() - 1]);
    v.clear();
    r
}

/// Runs until `stop`; every sample is appended to `csv` and kept in `samples`; the caches'
/// sizes go to `caches.csv` beside it.
pub fn spawn(
    w: Watched,
    csv: &Path,
    samples: Arc<Mutex<Vec<Sample>>>,
) -> anyhow::Result<std::thread::JoinHandle<()>> {
    let mut file = std::fs::File::create(csv)?;
    writeln!(file, "{HEADER}")?;
    let mut caches = std::fs::File::create(csv.with_file_name("caches.csv"))?;
    writeln!(caches, "t_s,cache,entries,bytes")?;
    Ok(std::thread::Builder::new()
        .name("watch".into())
        .spawn(move || {
            let mut next = Instant::now();
            // Bytes of the recording parts finished before the current one.
            let mut finished_parts = 0u64;
            let mut current: Option<(String, u64)> = None;
            while !w.stop.load(Ordering::Relaxed) {
                next += w.every;
                let (s, sizes) = sample(&w, &mut finished_parts, &mut current);
                let _ = writeln!(file, "{}", s.row());
                let _ = file.flush();
                for (name, entries, bytes) in sizes {
                    let _ = writeln!(caches, "{:.1},{name},{entries},{bytes}", s.t);
                }
                let _ = caches.flush();
                samples.lock().unwrap().push(s);
                while Instant::now() < next && !w.stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        })?)
}

/// A cache's name, entries and bytes.
type CacheRow = (String, u64, u64);

fn sample(
    w: &Watched,
    finished: &mut u64,
    current: &mut Option<(String, u64)>,
) -> (Sample, Vec<CacheRow>) {
    let mut sizes = Vec::new();
    let mut s = Sample {
        t: w.t0.elapsed().as_secs_f64(),
        phase: w.phase.lock().unwrap().clone(),
        ..Sample::default()
    };
    if let Ok(st) = http::get(w.addr, "/api/v1/server/stats") {
        s.alive = true;
        let p = &st["process"];
        s.rss_mb = p["rssBytes"].as_f64().unwrap_or(0.0) / 1_048_576.0;
        s.threads = p["threads"].as_u64().unwrap_or(0) as u32;
        s.fds = p["openFiles"].as_u64().unwrap_or(0) as u32;
        s.heap_mb = p["heapInUseBytes"].as_f64().unwrap_or(0.0) / 1_048_576.0;
        s.heap_free_mb = p["heapFreeBytes"].as_f64().unwrap_or(0.0) / 1_048_576.0;
        s.rust_heap_mb = p["rustHeapBytes"].as_f64().unwrap_or(0.0) / 1_048_576.0;
        for c in st["caches"].as_array().into_iter().flatten() {
            let name = c["name"].as_str().unwrap_or("?").replace(',', ";");
            let bytes = c["bytes"].as_u64().unwrap_or(0);
            s.caches_mb += bytes as f64 / 1_048_576.0;
            sizes.push((name, c["entries"].as_u64().unwrap_or(0), bytes));
        }
        s.ws_clients = st["wsClients"].as_u64().unwrap_or(0) as u32;
        s.xruns = st["audioXruns"].as_u64().unwrap_or(0);
        let screens = st["screens"].as_array().cloned().unwrap_or_default();
        s.screens = screens.len() as u32;
        let sum = |k: &str| {
            screens
                .iter()
                .map(|x| x[k].as_u64().unwrap_or(0))
                .sum::<u64>()
        };
        s.ticks = sum("ticks");
        s.late_ticks = sum("lateTicks");
        s.late_renders = sum("lateRenders");
        s.frames_dropped = sum("framesDropped");
        let videos = st["videos"].as_array().cloned().unwrap_or_default();
        s.videos = videos.len() as u32;
        s.video_late = videos.iter().map(|v| v["late"].as_u64().unwrap_or(0)).sum();
    }
    if let Ok(st) = http::get(w.addr, "/api/v1/streaming/status") {
        let r = &st["recording"];
        if r.is_object() {
            s.rec_state = r["state"].as_str().unwrap_or("").to_string();
            s.rec_bytes = r["bytes"].as_u64().unwrap_or(0);
            s.rec_dropped = r["dropped"].as_u64().unwrap_or(0);
            let file = r["file"].as_str().unwrap_or("").to_string();
            // A new part: count the last one's bytes and delete it (the disk is shared).
            if let Some((f, bytes)) = current.as_ref()
                && *f != file
            {
                *finished += *bytes;
                remove_finished(&w.recordings, &file);
            }
            *current = Some((file, s.rec_bytes));
        } else {
            s.rec_state = "off".into();
        }
        s.rec_total_bytes = *finished + s.rec_bytes;
    }
    if let Some(name) = w.camera.lock().unwrap().clone()
        && let Ok(src) = http::get(w.addr, "/api/v1/sources")
    {
        s.camera = src["sources"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["name"] == Value::String(name.clone())))
            .and_then(|x| x["status"].as_str())
            .unwrap_or("absent")
            .to_string();
    }
    (s.ping_p50, s.ping_max) = p50_max(&mut w.ws.pings.lock().unwrap());
    (s.cmd_p50, s.cmd_max) = p50_max(&mut w.ws.commands.lock().unwrap());
    s.cmd_errors = w.ws.command_errors.load(Ordering::Relaxed);
    s.rx_frames = w.probe.frames.load(Ordering::Relaxed);
    s.rx_gaps = w.probe.gaps.load(Ordering::Relaxed);
    s.load1 = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|l| l.split_whitespace().next()?.parse().ok())
        .unwrap_or(0.0);
    (s, sizes)
}

/// Deletes every recording in `dir` except `keep` (the part being written).
fn remove_finished(dir: &Path, keep: &str) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().map(|n| n.to_string_lossy().to_string());
        if p.is_file() && name.is_some_and(|n| !keep.ends_with(&n)) {
            let _ = std::fs::remove_file(&p);
        }
    }
}
