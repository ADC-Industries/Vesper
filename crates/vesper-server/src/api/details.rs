//! `GET /server/details` (G9): this Vesper's own numbers (version, OS, CPU, GPU, uptime, the
//! products it runs). Other machines of the site are described by the Site Manager's device
//! list (`SiteDevice`: IDN's build and services, and since IDN Beta8 `machine`, what their
//! Nodes report: system, CPU, GPUs, memory, start). `GET /server/stats`: its health
//! counters (frame pacing, video decoders, the process's memory, threads and files, the heap's
//! use and the caches' sizes), read by the soak harness (`tools/soak`, `docs/SOAK-PLAN.md`).

use super::*;
use protocol::{CacheSize, ProcessStats, ScreenPacing, ServerDetails, ServerStats, VideoStats};
use std::sync::atomic::Ordering;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/server/details", get(details))
        .route("/server/stats", get(stats))
}

async fn stats(State(app): State<AppState>) -> ApiResult<ServerStats> {
    let sh = app.shared.clone();
    let thumbs = app.thumbs.clone();
    let s = tokio::task::spawn_blocking(move || {
        let screens = sh
            .screens()
            .iter()
            .map(|rt| {
                let st = crate::lock(&rt.stats).clone();
                let frames_dropped = crate::lock(&rt.outputs)
                    .iter()
                    .map(|o| o.stats.dropped.load(Ordering::Relaxed))
                    .sum();
                ScreenPacing {
                    id: rt.id,
                    name: rt.screen().name.clone(),
                    ticks: st.ticks,
                    late_ticks: st.late_ticks,
                    late_renders: st.late_renders,
                    renders: st.renders,
                    max_render_ms: st.max_render_ms,
                    p95_render_ms: st.percentiles().map(|p| p.1),
                    frames_dropped,
                }
            })
            .collect();
        let videos = crate::lock(&sh.videos)
            .values()
            .map(|v| {
                let p = v.player.stats();
                VideoStats {
                    name: v.name.clone(),
                    shown: p.shown,
                    skipped: p.skipped,
                    late: p.late,
                    errors: p.errors,
                }
            })
            .collect();
        let mut caches: Vec<CacheSize> = crate::lock(&sh.render_caches)
            .iter()
            .map(|c| CacheSize {
                name: c.name.to_string(),
                entries: c.entries,
                bytes: c.bytes,
            })
            .collect();
        for (name, (entries, bytes)) in [
            ("slide thumbnails (CPU)", thumbs.mem_size()),
            (
                "video thumbnails (CPU)",
                crate::lock(&sh.video_thumbs).size(),
            ),
        ] {
            caches.push(CacheSize {
                name: name.into(),
                entries,
                bytes,
            });
        }
        ServerStats {
            uptime_seconds: sh.started.elapsed().as_secs_f64(),
            process: process_stats(),
            ws_clients: sh.events.receiver_count() as u32,
            screens,
            videos,
            audio_xruns: sh.sound.engine.xruns(),
            caches,
        }
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    ok(s)
}

/// The process's memory, threads and open files (Linux: `/proc/self`).
fn process_stats() -> ProcessStats {
    if !cfg!(target_os = "linux") {
        return ProcessStats::default();
    }
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status.lines().find_map(|l| {
            let v = l.strip_prefix(name)?.strip_prefix(':')?;
            v.split_whitespace().next()?.parse::<u64>().ok()
        })
    };
    let (heap_in_use_bytes, heap_free_bytes) = heap();
    ProcessStats {
        rss_bytes: field("VmRSS").map(|kb| kb * 1024),
        threads: field("Threads").map(|n| n as u32),
        open_files: std::fs::read_dir("/proc/self/fd")
            .ok()
            .map(|d| d.count() as u32),
        heap_in_use_bytes,
        heap_free_bytes,
        rust_heap_bytes: crate::heap::rust_heap(),
    }
}

/// The heap's bytes in use and held free (see [`crate::heap`]).
fn heap() -> (Option<u64>, Option<u64>) {
    crate::heap::usage().map_or((None, None), |(u, f)| (Some(u), Some(f)))
}

async fn details(State(app): State<AppState>) -> ApiResult<ServerDetails> {
    let sh = app.shared.clone();
    let d = tokio::task::spawn_blocking(move || {
        let mut products = vec!["Vesper".to_string()];
        if sh.codex_is_host() {
            products.push("Codex".into());
        }
        if sh.site.handle().is_some() {
            products.push("Site Manager".into());
        }
        ServerDetails {
            name: sh.auth.identity().name,
            version: crate::VERSION.to_string(),
            os: os_name(),
            cpu: cpu_model(),
            cpus: std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(1),
            gpu: crate::lock(&sh.adapter).clone(),
            uptime_seconds: sh.started.elapsed().as_secs(),
            products,
        }
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    ok(d)
}

/// The OS's name for people (Linux: `PRETTY_NAME` of `/etc/os-release`).
fn os_name() -> String {
    if cfg!(target_os = "linux")
        && let Ok(t) = std::fs::read_to_string("/etc/os-release")
        && let Some(n) = t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME="))
    {
        let n = n.trim().trim_matches('"').trim();
        if !n.is_empty() {
            return n.to_string();
        }
    }
    match std::env::consts::OS {
        "linux" => "Linux".into(),
        "windows" => "Windows".into(),
        "macos" => "macOS".into(),
        o => o.to_string(),
    }
}

/// The CPU's model (Linux: `/proc/cpuinfo`; Windows: `PROCESSOR_IDENTIFIER`).
fn cpu_model() -> Option<String> {
    if cfg!(target_os = "linux") {
        let t = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        return t
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                (k.trim() == "model name")
                    .then(|| v.split_whitespace().collect::<Vec<_>>().join(" "))
            })
            .filter(|s| !s.is_empty());
    }
    std::env::var("PROCESSOR_IDENTIFIER")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
