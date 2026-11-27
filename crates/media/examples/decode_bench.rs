//! Decode throughput of one file (measurement tool): frames per second and CPU time per
//! frame, GPU first or software, without rendering.
//!
//! ```text
//! cargo run --release -p media --example decode_bench -- <file> [--software] [--frames N]
//!     [--realtime]
//! ```

use media::{DecodeMode, Loops, Player, PlayerOptions};
use std::time::{Duration, Instant};

fn cpu_ms() -> f64 {
    let s = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let f: Vec<&str> = s
        .rsplit(')')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    let t: u64 = [11, 12]
        .iter()
        .filter_map(|i| f.get(*i).and_then(|v| v.parse::<u64>().ok()))
        .sum();
    t as f64 * 10.0
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

fn main() -> anyhow::Result<()> {
    let mut file = None;
    let mut mode = DecodeMode::Auto;
    let mut frames = 600u32;
    let mut realtime = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--software" => mode = DecodeMode::Software,
            "--frames" => frames = it.next().unwrap_or_default().parse()?,
            "--realtime" => realtime = true,
            _ => file = Some(a),
        }
    }
    let file = file.ok_or_else(|| anyhow::anyhow!("file?"))?;
    let p = Player::open(
        file.as_ref(),
        PlayerOptions {
            loops: Loops::Forever,
            mode: Some(mode),
            ..Default::default()
        },
    )?;
    let fps = p.info().fps;
    let c0 = cpu_ms();
    let t0 = Instant::now();
    let mut rss = Vec::new();
    for n in 0..frames {
        let t = n as f64 / fps;
        if realtime {
            // Paced like a screen: ask at the frame's time, never wait.
            let due = t0 + Duration::from_secs_f64(t);
            if let Some(d) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(d);
            }
            p.frame_for(t);
        } else {
            p.wait_frame_for(t, Duration::from_secs(5));
        }
        if n.is_multiple_of(300) {
            rss.push(format!("{:.0}", rss_mb()));
        }
    }
    println!("RSS MB every 300 frames: {}", rss.join(" "));
    let wall = t0.elapsed().as_secs_f64();
    let cpu = cpu_ms() - c0;
    let s = p.stats();
    println!(
        "{file}: {frames} frames in {wall:.2} s = {:.0} fps; CPU {:.2} ms/frame; hw expected={} late={}",
        frames as f64 / wall,
        cpu / frames as f64,
        s.hw,
        s.late
    );
    Ok(())
}
