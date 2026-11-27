//! The summary: the run's numbers against pass/fail thresholds (`summary.txt`).

use crate::probe::AvPhase;
use crate::watch::Sample;
use std::fmt::Write;

/// Pass/fail limits (command-line options).
#[derive(Clone, Debug)]
pub struct Thresholds {
    /// Resident memory growth after the warm-up, MB per hour (least-squares slope).
    pub rss_slope_mb_h: f64,
    /// Open files more than after the warm-up.
    pub fd_growth: i64,
    /// Threads more than after the warm-up.
    pub thread_growth: i64,
    /// Late or dropped frames (late ticks + late renders + dropped, every screen), per minute,
    /// in the worst minute.
    pub late_per_min: f64,
    /// Largest |A/V offset| at the output, ms.
    pub av_ms: f64,
    /// Change of the mean A/V offset over the run (median of the last third of the plays
    /// against the first third), ms.
    pub av_drift_ms: f64,
    /// WebSocket heartbeat round trip (median of a sample), ms.
    pub ws_ms: f64,
    /// Longest stall of a running recording's growth, s.
    pub rec_stall_s: f64,
    /// Longest time the camera took to be live again after it came back, s.
    pub camera_return_s: f64,
}

/// Everything the summary reports besides the samples.
pub struct Run<'a> {
    pub duration_s: f64,
    /// The duration asked for (`--duration`).
    pub planned_s: f64,
    /// A summary of what is in so far, written while the run goes on (`summary-so-far.txt`).
    pub interim: bool,
    /// Why the run stopped before its end (the operator's Ctrl-C or SIGTERM, or the server
    /// stopped cleanly from outside).
    pub early_stop: Option<String>,
    pub samples: &'a [Sample],
    pub phases: &'a [AvPhase],
    pub commands: u64,
    pub command_errors: u64,
    pub last_errors: Vec<String>,
    pub drops: u64,
    pub lost: u64,
    pub camera_returns: Vec<Option<f64>>,
    pub probe_reconnects: u64,
    pub camera_errors: u64,
    pub recording: bool,
    pub captions: Option<String>,
    pub scripture: bool,
    pub server_exit: Option<String>,
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Least-squares slope of `y` over `x`.
pub fn slope(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    if sxx == 0.0 { 0.0 } else { sxy / sxx }
}

/// The summary text and whether every check passed.
pub fn summary(run: &Run, th: &Thresholds) -> (String, bool) {
    let mut out = String::new();
    let mut pass = true;
    let mut check = |out: &mut String, name: &str, ok: bool, detail: String| {
        pass &= ok;
        let _ = writeln!(
            out,
            "[{}] {name}: {detail}",
            if ok { "PASS" } else { "FAIL" }
        );
    };
    let alive: Vec<&Sample> = run.samples.iter().filter(|s| s.alive).collect();
    // Caches (decoded images, glyphs, thumbnails, slide layers) fill over the first few
    // Sundays: the warm-up is a quarter of the run, at most 15 minutes.
    let warm = (run.duration_s * 0.25).min(900.0);
    let steady: Vec<&Sample> = alive.iter().copied().filter(|s| s.t >= warm).collect();
    let how = if run.interim {
        format!(
            "SO FAR, {:.0} of {:.0} planned min (the run goes on)",
            run.duration_s / 60.0,
            run.planned_s / 60.0
        )
    } else if let Some(why) = &run.early_stop {
        format!(
            "STOPPED EARLY BY OPERATOR after {:.0} of {:.0} planned min ({why})",
            run.duration_s / 60.0,
            run.planned_s / 60.0
        )
    } else {
        format!("{:.0} min", run.duration_s / 60.0)
    };
    let _ = writeln!(
        out,
        "Vesper soak: {how}, {} samples ({} after a {:.0} s warm-up)\n",
        run.samples.len(),
        steady.len(),
        warm
    );

    // Stopped by the operator: the samples up to the stop count, the stop itself isn't a
    // failure (the server may have been stopped before the last sample).
    let answered = run.samples.last().is_some_and(|s| s.alive)
        || (run.early_stop.is_some() && !alive.is_empty());
    check(
        &mut out,
        "server ran to the end",
        run.server_exit.is_none() && answered,
        run.server_exit.clone().unwrap_or_else(|| {
            if run.interim {
                "answered every sample so far".into()
            } else if run.early_stop.is_some() {
                "answered every sample until the operator stopped the run".into()
            } else {
                "answered every sample".into()
            }
        }),
    );

    // Memory, files, threads.
    let pts: Vec<(f64, f64)> = steady.iter().map(|s| (s.t / 3600.0, s.rss_mb)).collect();
    let k = slope(&pts);
    let (first, last) = (
        steady.first().map(|s| s.rss_mb).unwrap_or(0.0),
        steady.last().map(|s| s.rss_mb).unwrap_or(0.0),
    );
    let peak = alive.iter().map(|s| s.rss_mb).fold(0.0, f64::max);
    check(
        &mut out,
        "memory growth",
        k <= th.rss_slope_mb_h,
        format!(
            "{k:.1} MB/h (limit {:.0}); {first:.0} MB after warm-up, {last:.0} MB at the end, peak {peak:.0} MB{}",
            th.rss_slope_mb_h,
            heap_line(&steady)
        ),
    );
    let ends = |f: &dyn Fn(&Sample) -> f64| {
        let n = steady.len().min(5);
        let mut a: Vec<f64> = steady.iter().take(n).map(|s| f(s)).collect();
        let mut b: Vec<f64> = steady.iter().rev().take(n).map(|s| f(s)).collect();
        (median(&mut a), median(&mut b))
    };
    let (f0, f1) = ends(&|s| s.fds as f64);
    check(
        &mut out,
        "open files",
        (f1 - f0) as i64 <= th.fd_growth,
        format!("{f0:.0} → {f1:.0} (growth limit {})", th.fd_growth),
    );
    let (t0, t1) = ends(&|s| s.threads as f64);
    check(
        &mut out,
        "threads",
        (t1 - t0) as i64 <= th.thread_growth,
        format!("{t0:.0} → {t1:.0} (growth limit {})", th.thread_growth),
    );

    // Frame pacing: the worst minute.
    let late = |s: &Sample| (s.late_ticks + s.late_renders + s.frames_dropped) as f64;
    let mut worst = (0.0f64, 0.0f64);
    for (i, a) in alive.iter().enumerate() {
        if let Some(b) = alive[i..].iter().find(|b| b.t - a.t >= 60.0) {
            let per_min = (late(b) - late(a)) / ((b.t - a.t) / 60.0);
            if per_min > worst.0 {
                worst = (per_min, a.t);
            }
        }
    }
    let (l0, l1) = (
        alive.first().map(|s| late(s)).unwrap_or(0.0),
        alive.last().map(|s| late(s)).unwrap_or(0.0),
    );
    let ticks = alive.last().map(|s| s.ticks).unwrap_or(0) as f64;
    let last = alive.last().cloned().cloned().unwrap_or_default();
    check(
        &mut out,
        "frame pacing",
        worst.0 <= th.late_per_min,
        format!(
            "worst minute {:.1} late/dropped frames (from {:.0} s; limit {:.0}/min); total {:.0} of {ticks:.0} ticks (late ticks {}, late renders {}, dropped {}); output receiver: {} frames, {} timecode gaps",
            worst.0,
            worst.1,
            th.late_per_min,
            l1 - l0,
            last.late_ticks,
            last.late_renders,
            last.frames_dropped,
            last.rx_frames,
            last.rx_gaps
        ),
    );

    // A/V sync at the output.
    let measured: Vec<&AvPhase> = run.phases.iter().filter(|p| p.offsets.len() >= 5).collect();
    if measured.is_empty() {
        check(
            &mut out,
            "A/V sync",
            false,
            format!(
                "not measured ({} sync clip plays, none with 5 paired beeps)",
                run.phases.len()
            ),
        );
    } else {
        let worst = measured
            .iter()
            .filter_map(|p| p.worst())
            .max_by(|a, b| a.abs().total_cmp(&b.abs()))
            .unwrap_or(0.0);
        let means: Vec<f64> = measured.iter().filter_map(|p| p.mean()).collect();
        // A play's mean moves within a frame (where the clip's frames fall on the output's
        // ticks): drift is the median of the last third of the plays against the first third.
        let third = means.len().div_ceil(3);
        let mut head: Vec<f64> = means.iter().take(third).copied().collect();
        let mut tail: Vec<f64> = means.iter().rev().take(third).copied().collect();
        let drift = median(&mut tail) - median(&mut head);
        check(
            &mut out,
            "A/V offset",
            worst.abs() <= th.av_ms,
            format!(
                "worst {worst:+.1} ms over {} plays (limit ±{:.0}); means {}",
                measured.len(),
                th.av_ms,
                means
                    .iter()
                    .map(|m| format!("{m:+.1}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
        check(
            &mut out,
            "A/V drift",
            drift.abs() <= th.av_drift_ms,
            format!(
                "{drift:+.1} ms, the last third of the plays against the first (medians; limit ±{:.0})",
                th.av_drift_ms
            ),
        );
    }

    // Recording.
    if run.recording {
        let mut stall = 0.0f64;
        let mut since: Option<(f64, u64)> = None;
        for s in &alive {
            if s.rec_state != "live" {
                since = None;
                continue;
            }
            match since {
                Some((t, b)) if s.rec_total_bytes <= b => stall = stall.max(s.t - t),
                _ => since = Some((s.t, s.rec_total_bytes)),
            }
        }
        let failed = alive.iter().any(|s| s.rec_state == "failed");
        check(
            &mut out,
            "recording",
            stall <= th.rec_stall_s && !failed && last.rec_state == "live",
            format!(
                "{:.0} MB written, longest stall {stall:.0} s (limit {:.0}), state at the end {:?}, {} dropped",
                last.rec_total_bytes as f64 / 1_048_576.0,
                th.rec_stall_s,
                last.rec_state,
                last.rec_dropped
            ),
        );
    } else {
        let _ = writeln!(out, "[SKIP] recording: no encoder on this box");
    }

    // Operators.
    let mut p50s: Vec<f64> = alive.iter().map(|s| s.ping_p50).collect();
    let worst_p50 = p50s.iter().copied().fold(0.0, f64::max);
    let ping_max = alive.iter().map(|s| s.ping_max).fold(0.0, f64::max);
    let cmd_max = alive.iter().map(|s| s.cmd_max).fold(0.0, f64::max);
    check(
        &mut out,
        "WebSocket latency",
        worst_p50 <= th.ws_ms,
        format!(
            "heartbeat median {:.1} ms, worst sample median {worst_p50:.1} ms (limit {:.0}), slowest {ping_max:.0} ms; slowest command {cmd_max:.0} ms",
            median(&mut p50s),
            th.ws_ms
        ),
    );
    let err_rate = run.command_errors as f64 / run.commands.max(1) as f64;
    check(
        &mut out,
        "commands",
        err_rate <= 0.005,
        format!(
            "{} sent, {} refused or unanswered ({:.2} %, limit 0.5 %)",
            run.commands,
            run.command_errors,
            err_rate * 100.0
        ),
    );
    for e in &run.last_errors {
        let _ = writeln!(out, "         {e}");
    }
    check(
        &mut out,
        "operator connections",
        run.lost == 0,
        format!(
            "{} planned drops and reconnects, {} closed by the server",
            run.drops, run.lost
        ),
    );

    // The camera coming back.
    let worst_return = run
        .camera_returns
        .iter()
        .map(|r| r.unwrap_or(f64::INFINITY))
        .fold(0.0, f64::max);
    check(
        &mut out,
        "camera drop and return",
        !run.camera_returns.is_empty() && worst_return <= th.camera_return_s,
        format!(
            "{} returns, slowest back to live {} (limit {:.0} s)",
            run.camera_returns.len(),
            if worst_return.is_finite() {
                format!("{worst_return:.1} s")
            } else {
                "never".into()
            },
            th.camera_return_s
        ),
    );

    let _ = writeln!(
        out,
        "\nAlso: probe reconnects {}, camera start errors {}, audio underruns {}, scripture {}, captions {}",
        run.probe_reconnects,
        run.camera_errors,
        last.xruns,
        if run.scripture {
            "from a Bible translation"
        } else {
            "synthetic (no translation on the server)"
        },
        run.captions
            .as_deref()
            .unwrap_or("skipped (no model given)")
    );
    let mut loads: Vec<f64> = alive.iter().map(|s| s.load1).collect();
    let max_load = loads.iter().copied().fold(0.0, f64::max);
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    let _ = writeln!(
        out,
        "Machine load (1 min): median {:.1}, max {max_load:.1} on {cpus} CPUs{}",
        median(&mut loads),
        if max_load > cpus as f64 * 0.75 {
            ": other work on the box competed with the server, so frame pacing and A/V numbers \
             are pessimistic"
        } else {
            ""
        }
    );
    let _ = writeln!(out, "\n{}", enough_data(&alive, warm, th));
    let _ = writeln!(
        out,
        "\n{}: {}{}",
        if run.interim {
            "RESULT SO FAR"
        } else {
            "RESULT"
        },
        if pass { "PASS" } else { "FAIL" },
        if run.early_stop.is_some() && !run.interim {
            " (stopped early by operator)"
        } else {
            ""
        }
    );
    (out, pass)
}

/// Where the steady part's memory went: the heap in use, held free, and the caches (start →
/// end, slopes), when the server reports them.
fn heap_line(steady: &[&Sample]) -> String {
    let (Some(a), Some(b)) = (steady.first(), steady.last()) else {
        return String::new();
    };
    if b.heap_mb == 0.0 {
        return String::new();
    }
    let k = |f: &dyn Fn(&Sample) -> f64| {
        slope(
            &steady
                .iter()
                .map(|s| (s.t / 3600.0, f(s)))
                .collect::<Vec<_>>(),
        )
    };
    let rust = if b.rust_heap_mb > 0.0 {
        format!(
            "; Rust heap (mimalloc) {:.0} → {:.0} MB ({:+.1} MB/h), the heap numbers before it C libraries only",
            a.rust_heap_mb,
            b.rust_heap_mb,
            k(&|s| s.rust_heap_mb)
        )
    } else {
        String::new()
    };
    format!(
        "; heap in use {:.0} → {:.0} MB ({:+.1} MB/h), held free {:.0} → {:.0} MB ({:+.1} MB/h), caches {:.0} → {:.0} MB{rust}",
        a.heap_mb,
        b.heap_mb,
        k(&|s| s.heap_mb),
        a.heap_free_mb,
        b.heap_free_mb,
        k(&|s| s.heap_free_mb),
        a.caches_mb,
        b.caches_mb
    )
}

/// The phases of a Sunday that every judgement needs at least twice.
const PHASES: [&str; 7] = [
    "pre-service",
    "camera",
    "worship",
    "scripture",
    "video",
    "sermon",
    "closing",
];

/// Whether the samples so far are enough to judge the run (`docs/SOAK-PLAN.md`): every phase
/// seen at least twice, a steady part (after the warm-up) of at least 20 minutes, and the
/// memory slope settled (the steady part's slope and its second half's within 10 MB/h, or a
/// quarter, of each other).
pub fn enough_data(alive: &[&Sample], warm: f64, th: &Thresholds) -> String {
    let mut runs: Vec<(&str, usize)> = PHASES.iter().map(|p| (*p, 0)).collect();
    let mut prev = "";
    for s in alive {
        if s.phase != prev {
            prev = &s.phase;
            if let Some(r) = runs.iter_mut().find(|r| r.0 == s.phase) {
                r.1 += 1;
            }
        }
    }
    let few: Vec<String> = runs
        .iter()
        .filter(|r| r.1 < 2)
        .map(|r| format!("{} {}×", r.0, r.1))
        .collect();
    let steady: Vec<&Sample> = alive.iter().copied().filter(|s| s.t >= warm).collect();
    let steady_min = match (steady.first(), steady.last()) {
        (Some(a), Some(b)) => (b.t - a.t) / 60.0,
        _ => 0.0,
    };
    let rss = |v: &[&Sample]| {
        slope(
            &v.iter()
                .map(|s| (s.t / 3600.0, s.rss_mb))
                .collect::<Vec<_>>(),
        )
    };
    let whole = rss(&steady);
    let half = rss(&steady[steady.len() / 2..]);
    let settled = (whole - half).abs() <= 10.0f64.max(whole.abs() * 0.25);
    let late = |s: &Sample| (s.late_ticks + s.late_renders + s.frames_dropped) as f64;
    let per_min = |v: &[&Sample]| match (v.first(), v.last()) {
        (Some(a), Some(b)) if b.t > a.t => (late(b) - late(a)) / ((b.t - a.t) / 60.0),
        _ => 0.0,
    };
    let mid = steady.len() / 2;
    let (l1, l2) = (
        per_min(&steady[..mid.max(1).min(steady.len())]),
        per_min(&steady[mid..]),
    );
    let mut reasons = Vec::new();
    if !few.is_empty() {
        reasons.push(format!("phases seen fewer than twice: {}", few.join(", ")));
    }
    if steady_min < 20.0 {
        reasons.push(format!("{steady_min:.0} steady min (want 20)"));
    }
    if !settled {
        reasons.push("the memory slope has not settled".into());
    }
    format!(
        "Enough data: {} (phases {}; {steady_min:.0} steady min; memory slope {whole:.1} MB/h, \
         second half {half:.1} MB/h, limit {:.0}; late/dropped frames {l1:.1}/min then {l2:.1}/min)",
        if reasons.is_empty() {
            "yes".to_string()
        } else {
            format!("not yet: {}", reasons.join("; "))
        },
        runs.iter()
            .map(|r| format!("{} {}×", r.0, r.1))
            .collect::<Vec<_>>()
            .join(", "),
        th.rss_slope_mb_h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slope_of_a_line() {
        let pts: Vec<(f64, f64)> = (0..10).map(|i| (i as f64, 3.0 * i as f64 + 1.0)).collect();
        assert!((slope(&pts) - 3.0).abs() < 1e-9);
        assert_eq!(slope(&[(1.0, 1.0)]), 0.0);
    }

    fn th() -> Thresholds {
        Thresholds {
            rss_slope_mb_h: 40.0,
            fd_growth: 20,
            thread_growth: 10,
            late_per_min: 30.0,
            av_ms: 45.0,
            av_drift_ms: 20.0,
            ws_ms: 250.0,
            rec_stall_s: 30.0,
            camera_return_s: 20.0,
        }
    }

    /// `minutes` of samples every 5 s, cycling through the Sunday's phases (30 s each), memory
    /// flat at 500 MB.
    fn samples(minutes: f64) -> Vec<Sample> {
        (0..(minutes * 12.0) as usize)
            .map(|i| Sample {
                t: i as f64 * 5.0,
                phase: PHASES[(i / 6) % PHASES.len()].to_string(),
                alive: true,
                rss_mb: 500.0,
                ..Sample::default()
            })
            .collect()
    }

    fn run<'a>(samples: &'a [Sample], planned_s: f64) -> Run<'a> {
        Run {
            duration_s: samples.last().map_or(0.0, |s| s.t),
            planned_s,
            interim: false,
            early_stop: None,
            samples,
            phases: &[],
            commands: 10,
            command_errors: 0,
            last_errors: Vec::new(),
            drops: 0,
            lost: 0,
            camera_returns: vec![Some(1.0)],
            probe_reconnects: 0,
            camera_errors: 0,
            recording: false,
            captions: None,
            scripture: false,
            server_exit: None,
        }
    }

    /// A run the operator stopped: marked as such, "ran to the end" passes on the samples up
    /// to the stop, even when the server was gone at the last one.
    #[test]
    fn an_early_stop_is_marked_and_not_a_failure() {
        let mut s = samples(12.0);
        s.last_mut().unwrap().alive = false;
        let mut r = run(&s, 3600.0);
        r.early_stop = Some("SIGINT to the harness".into());
        let (text, _) = summary(&r, &th());
        assert!(
            text.contains("STOPPED EARLY BY OPERATOR after 12 of 60 planned min (SIGINT"),
            "{text}"
        );
        assert!(text.contains("[PASS] server ran to the end: answered every sample until the operator stopped the run"), "{text}");
        assert!(text.contains("(stopped early by operator)"), "{text}");
        // Without the stop, the same samples fail it.
        let (text, _) = summary(&run(&s, 3600.0), &th());
        assert!(text.contains("[FAIL] server ran to the end"), "{text}");
    }

    #[test]
    fn interim_summaries_say_so() {
        let s = samples(10.0);
        let mut r = run(&s, 3600.0);
        r.interim = true;
        let (text, _) = summary(&r, &th());
        assert!(text.contains("SO FAR, 10 of 60 planned min"), "{text}");
        assert!(text.contains("RESULT SO FAR:"), "{text}");
    }

    /// Enough data: every phase twice, 20 steady minutes, a settled memory slope.
    #[test]
    fn enough_data_needs_phases_steady_minutes_and_a_settled_slope() {
        let s = samples(40.0);
        let alive: Vec<&Sample> = s.iter().collect();
        let e = enough_data(&alive, 600.0, &th());
        assert!(e.starts_with("Enough data: yes"), "{e}");

        // Too short a steady part, and too few phases.
        let s = samples(4.0);
        let alive: Vec<&Sample> = s.iter().collect();
        let e = enough_data(&alive, 60.0, &th());
        assert!(
            e.contains("not yet") && e.contains("closing 1×") && e.contains("want 20"),
            "{e}"
        );

        // Memory still climbing faster at the end than over the whole steady part.
        let mut s = samples(40.0);
        for x in &mut s {
            x.rss_mb = 500.0 + (x.t / 60.0 - 20.0).max(0.0).powi(2);
        }
        let alive: Vec<&Sample> = s.iter().collect();
        let e = enough_data(&alive, 600.0, &th());
        assert!(e.contains("the memory slope has not settled"), "{e}");
    }
}
