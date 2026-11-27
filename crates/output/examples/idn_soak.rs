//! Soak / CPU measurement for one IDN Open Protocol output at 1920×1080, 30 fps, with a motion
//! test pattern (scrolling bars, a moving box and the 12-bit frame counter `idn_pacing` reads):
//!
//! ```text
//! cargo run --release -p output --example idn_soak -- \
//!     [--seconds 300] [--name "Soak"] [--alpha] [--audio]
//! ```
//!
//! Every 10 s it prints the process CPU (user + system, % of one core), resident memory, frames
//! sent, late ticks, the slowest `send` call and, for IDN, the library's counters (replaced =
//! frames the encoder could not keep up with). Receive it meanwhile from another process, e.g.
//! `cargo run --release -p output --example idn_pacing -- "<name>" --seconds 280`, to check
//! for stalls, repeats and skips on the receiving side.

use anyhow::{Context, Result, bail};
use output::{AlphaMode, AudioSink, Frame, FrameRate, FrameSink, IdnSenderConfig, IdnSink};
use std::time::{Duration, Instant};

const W: u32 = 1920;
const H: u32 = 1080;
const FPS: u32 = 30;

/// (user + system CPU seconds, resident set in MB) of this process.
fn usage() -> (f64, f64) {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let after = stat.rsplit(')').next().unwrap_or("");
    let f: Vec<&str> = after.split_whitespace().collect();
    let tick = 100.0; // USER_HZ on Linux
    let cpu = f.get(11).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0)
        + f.get(12).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let rss_kb = status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    (cpu / tick, rss_kb / 1024.0)
}

struct Pattern {
    row: Vec<u8>,
    buf: Vec<u8>,
}

impl Pattern {
    fn new() -> Self {
        // Two screen widths of smooth colour bars, scrolled by 8 px per frame.
        let mut row = Vec::with_capacity((2 * W * 4) as usize);
        for x in 0..2 * W {
            let t = (x % W) as f32 / W as f32 * 6.0;
            let c = |o: f32| (((t + o) * std::f32::consts::PI / 3.0).sin() * 0.5 + 0.5) * 220.0;
            row.extend_from_slice(&[c(0.0) as u8, c(2.0) as u8, c(4.0) as u8, 255]);
        }
        Pattern {
            row,
            buf: vec![0; (W * H * 4) as usize],
        }
    }

    fn frame(&mut self, n: u32) -> &[u8] {
        let shift = ((n * 8) % W) as usize * 4;
        let line = (W * 4) as usize;
        for y in 0..H as usize {
            self.buf[y * line..(y + 1) * line].copy_from_slice(&self.row[shift..shift + line]);
        }
        // A white box bouncing across the middle.
        let bx = (n * 12 % (2 * (W - 200))) as i64;
        let bx = if bx > (W - 200) as i64 {
            2 * (W - 200) as i64 - bx
        } else {
            bx
        } as u32;
        self.fill(bx, bx + 200, H / 2 - 100, H / 2 + 100, 255);
        // The counter idn_pacing reads: 12 blocks, white = 1.
        for bit in 0..12u32 {
            let on = (n >> (11 - bit)) & 1 == 1;
            let x0 = bit * 16 * W / 192 + 4;
            let x1 = (bit * 16 + 16) * W / 192 - 4;
            self.fill(x0, x1, H * 6 / 108, H * 18 / 108, if on { 255 } else { 0 });
        }
        &self.buf
    }

    fn fill(&mut self, x0: u32, x1: u32, y0: u32, y1: u32, v: u8) {
        for y in y0..y1 {
            let r = (y * W * 4) as usize;
            for x in x0..x1 {
                let i = r + (x * 4) as usize;
                self.buf[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
    }
}

fn main() -> Result<()> {
    let mut seconds = 300.0f64;
    let mut name = "Soak".to_string();
    let mut alpha = false;
    let mut with_audio = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seconds" => seconds = it.next().context("--seconds S")?.parse()?,
            "--name" => name = it.next().context("--name N")?,
            "--alpha" => alpha = true,
            "--audio" => with_audio = true,
            _ => bail!("unexpected argument {a}"),
        }
    }
    let mut cfg = IdnSenderConfig::new(&name);
    cfg.alpha = alpha;
    let mut sink = IdnSink::new(cfg)?;
    eprintln!("IDN source {:?} on port {}", sink.full_name(), sink.port());
    let mut audio = sink.audio_sender();
    let mut pattern = Pattern::new();
    let tone: Vec<f32> = (0..1600 * 2)
        .map(|i| ((i / 2) as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0).sin() * 0.1)
        .collect();
    let start = Instant::now();
    let (cpu0, rss0) = usage();
    let mut report = (Instant::now(), cpu0);
    let mut n: u32 = 0;
    let mut late = 0u64;
    let mut errors = 0u64;
    let mut slowest = Duration::ZERO;
    let mut cpu_samples = Vec::new();
    let mut rss_samples = Vec::new();
    println!("t_s,cpu_pct,rss_mb,frames,late,errors,slowest_send_ms,receivers,library_stats");
    while start.elapsed().as_secs_f64() < seconds {
        let px = pattern.frame(n);
        let t = Instant::now();
        if with_audio {
            let _ = audio.send(&tone, 2, 48_000, n as i64 * 333_333);
        }
        let frame = Frame {
            width: W,
            height: H,
            pixels: px,
            alpha: if alpha {
                AlphaMode::Premultiplied
            } else {
                AlphaMode::Opaque
            },
            frame_rate: FrameRate { num: FPS, den: 1 },
            timecode: n as i64 * 333_333,
        };
        let r = sink.send(&frame);
        slowest = slowest.max(t.elapsed());
        if let Err(e) = r {
            errors += 1;
            eprintln!("send error: {e:#}");
        }
        n += 1;
        let due = start + Duration::from_nanos(n as u64 * 1_000_000_000 / FPS as u64);
        match due.checked_duration_since(Instant::now()) {
            Some(d) => std::thread::sleep(d),
            None => late += 1,
        }
        if report.0.elapsed() >= Duration::from_secs(10) {
            let (cpu, rss) = usage();
            let pct = (cpu - report.1) / report.0.elapsed().as_secs_f64() * 100.0;
            report = (Instant::now(), cpu);
            cpu_samples.push(pct);
            rss_samples.push(rss);
            let (rx, lib) = (sink.connections(), format!("{:?}", sink.stats()));
            println!(
                "{:.0},{pct:.1},{rss:.1},{n},{late},{errors},{:.2},{rx},{lib}",
                start.elapsed().as_secs_f64(),
                slowest.as_secs_f64() * 1000.0
            );
        }
    }
    let (cpu1, rss1) = usage();
    // Steady state: skip the first 10 s window.
    let steady: Vec<f64> = cpu_samples.iter().skip(1).copied().collect();
    let avg = steady.iter().sum::<f64>() / steady.len().max(1) as f64;
    println!(
        "SOAK backend=idn name={name:?} alpha={alpha} audio={with_audio} seconds={:.0} frames={n} late={late} errors={errors} slowest_send_ms={:.2} cpu_avg_pct={avg:.1} cpu_total_s={:.1} rss_start_mb={rss0:.1} rss_after_10s_mb={:.1} rss_end_mb={rss1:.1} rss_max_mb={:.1}",
        start.elapsed().as_secs_f64(),
        slowest.as_secs_f64() * 1000.0,
        cpu1 - cpu0,
        rss_samples.first().copied().unwrap_or(0.0),
        rss_samples.iter().copied().fold(0.0, f64::max),
    );
    println!(
        "IDN library stats: {:?}, audio dropped {}",
        sink.stats(),
        sink.audio_dropped()
    );
    drop(audio);
    drop(sink);
    Ok(())
}
