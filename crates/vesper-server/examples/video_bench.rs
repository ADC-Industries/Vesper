//! Video playout benchmark (measurement tool): N screens, each showing a different looping
//! video, rendered the way the server's render thread does it (one renderer, every screen
//! submitted per tick, then read back), paced by a 30 fps clock. Reports per-screen render
//! times, late ticks, decoder counters and process CPU / memory.
//!
//! ```text
//! cargo run --release -p vesper-server --example video_bench -- --screens 4 --seconds 30 \
//!     [--software-decode] a.mp4 b.mp4 c.mp4 d.mp4
//! ```
//!
//! The server itself has one media layer (every screen shows the same background), so this
//! harness is how "four screens, four different 1080p backgrounds" is measured.

use anyhow::{Context, bail};
use media::{Loops, Player, PlayerOptions};
use model::MediaDisplay;
use render::{LayerContent, Renderer, RendererOptions, ScreenConfig};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn cpu_ticks() -> u64 {
    let s = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let after = s.rsplit(')').next().unwrap_or("");
    let f: Vec<&str> = after.split_whitespace().collect();
    // utime, stime are fields 14, 15 (1-based); after the ')' they are at 11, 12 (0-based).
    f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0)
        + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0)
}

fn rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
        / 1024.0
}

fn pct(v: &[f64], q: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s.get(((s.len().max(1) - 1) as f64 * q).round() as usize)
        .copied()
        .unwrap_or(0.0)
}

fn main() -> anyhow::Result<()> {
    let mut screens = 1usize;
    let mut seconds = 30.0f64;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--screens" => screens = it.next().context("--screens N")?.parse()?,
            "--seconds" => seconds = it.next().context("--seconds S")?.parse()?,
            "--software-decode" => media::set_decode_mode(media::DecodeMode::Software),
            _ => files.push(a.into()),
        }
    }
    if files.is_empty() {
        bail!("give at least one video file");
    }
    let clk = 100.0; // USER_HZ
    let mut r = Renderer::new(RendererOptions {
        system_fonts: false,
        wait_for_images: false,
        ..Default::default()
    })?;
    println!("adapter: {}", r.adapter());
    println!(
        "ffmpeg: {}, decode mode {:?}",
        media::ffmpeg_version(),
        media::decode_mode()
    );
    let players: Vec<Player> = (0..screens)
        .map(|i| {
            Player::open(
                &files[i % files.len()],
                PlayerOptions {
                    loops: Loops::Forever,
                    ..Default::default()
                },
            )
        })
        .collect::<Result<_, _>>()?;
    for p in &players {
        let _ = p.wait_frame_for(0.0, Duration::from_secs(2));
        let i = p.info();
        println!(
            "  {}: {} {}x{} {:.2} fps hw={}",
            p.path().display(),
            i.codec,
            i.width,
            i.height,
            i.fps,
            i.hw
        );
    }
    let mut targets: Vec<_> = (0..screens)
        .map(|_| r.create_screen(ScreenConfig::opaque(1920, 1080)))
        .collect();
    let display = MediaDisplay {
        scale: model::ScaleMode::Fill,
        ..Default::default()
    };
    let period = Duration::from_nanos(1_000_000_000 / 30);
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); screens];
    let mut late = 0u64;
    let mut ticks = 0u64;
    let mut rss = Vec::new();
    let start = Instant::now();
    let cpu0 = cpu_ticks();
    let mut next = start;
    while start.elapsed().as_secs_f64() < seconds {
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
            continue;
        }
        let present = next + period;
        let t_media = (present - start).as_secs_f64();
        let frames: Vec<_> = players.iter().map(|p| p.frame_for(t_media)).collect();
        let t0 = Instant::now();
        let mut submitted = Vec::new();
        for (i, (f, target)) in frames.iter().zip(targets.iter_mut()).enumerate() {
            let Some(f) = f else { continue };
            let (y, ys) = f.plane(0);
            let (uv, uvs) = f.plane(1);
            let fr = render::VideoFrameRef {
                stream: players[i].id(),
                seq: f.seq,
                width: f.width,
                height: f.height,
                planes: match f.format {
                    media::PlaneFormat::Nv12 => render::VideoPlanes::Nv12,
                    media::PlaneFormat::P010 => render::VideoPlanes::P010,
                },
                y,
                y_stride: ys,
                uv,
                uv_stride: uvs,
                matrix: match f.matrix {
                    media::Matrix::Bt601 => render::VideoMatrix::Bt601,
                    media::Matrix::Bt709 => render::VideoMatrix::Bt709,
                    media::Matrix::Bt2020 => render::VideoMatrix::Bt2020,
                },
                full_range: f.full_range,
                rotation: f.rotation,
            };
            r.submit(target, &[LayerContent::video(fr, display.clone())])?;
            submitted.push(i);
        }
        for i in submitted {
            if r.wait(&mut targets[i])?.is_some() {
                times[i].push(t0.elapsed().as_secs_f64() * 1000.0);
            }
        }
        if Instant::now() > present {
            late += 1;
        }
        ticks += 1;
        if ticks.is_multiple_of(30) {
            rss.push(rss_mb());
        }
        next += period;
        if next < Instant::now() {
            next = Instant::now();
        }
    }
    let wall = start.elapsed().as_secs_f64();
    let cpu = (cpu_ticks() - cpu0) as f64 / clk / wall * 100.0;
    println!(
        "{screens} screens, {wall:.1} s, {ticks} ticks, late ticks {late}, CPU {cpu:.0}% (of one core), RSS start {:.0} MB end {:.0} MB max {:.0} MB",
        rss.first().copied().unwrap_or(0.0),
        rss.last().copied().unwrap_or(0.0),
        rss.iter().copied().fold(0.0, f64::max)
    );
    let series: Vec<String> = rss.iter().step_by(10).map(|v| format!("{v:.0}")).collect();
    println!("RSS MB every 10 s: {}", series.join(" "));
    for (i, t) in times.iter().enumerate() {
        let s = players[i].stats();
        println!(
            "  screen {i}: render ms p50 {:.2} p95 {:.2} max {:.2} | decode hw={} decoded {} shown {} skipped {} late {} {:.2} ms/frame",
            pct(t, 0.5),
            pct(t, 0.95),
            pct(t, 1.0),
            s.hw,
            s.decoded,
            s.shown,
            s.skipped,
            s.late,
            s.decode_ms
        );
    }
    Ok(())
}
