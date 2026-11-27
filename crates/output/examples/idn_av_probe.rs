//! Measures the A/V offset of a network video source (received with the IDN Open Protocol
//! receiver, compatible with NDI®) showing white flash frames with beeps at the same media
//! times: `cargo run --release -p output --example idn_av_probe -- <source> <seconds>`.
//! Prints the offset (audio − video timecode) of every beep and the audio level.
//!
//! `<source>` is the full `"MACHINE (SOURCE)"` name, just `SOURCE`, or `ip:port`.

use output::{IdnFinder, IdnReceiver, Received};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let name = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "Program Feed".into());
    let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let target = if name.parse::<std::net::SocketAddr>().is_ok() {
        name.clone()
    } else {
        IdnFinder::new()?
            .find_by_name(&name, Duration::from_secs(10))
            .ok_or_else(|| anyhow::anyhow!("{name} not found"))?
            .name
    };
    let mut rx = IdnReceiver::connect(&target)?;
    let end = Instant::now() + Duration::from_secs(secs);
    let (mut video_on, mut was) = (Vec::new(), false);
    let mut audio: Vec<f32> = Vec::new();
    let mut first_tc = None;
    let (mut vframes, mut aframes) = (0u64, 0u64);
    while Instant::now() < end {
        match rx.capture(Duration::from_millis(200))? {
            Some(Received::Video(f)) => {
                vframes += 1;
                let flash = f.pixel_rgba(f.width / 2, f.height / 2)[0] > 128;
                if flash && !was {
                    video_on.push(f.timecode);
                }
                was = flash;
            }
            Some(Received::Audio(a)) => {
                aframes += 1;
                let t0 = *first_tc.get_or_insert(a.timecode);
                let at = ((a.timecode - t0) as f64 * a.sample_rate as f64 / 1e7).round() as usize;
                if audio.len() < at + a.samples {
                    audio.resize(at + a.samples, 0.0);
                }
                audio[at..at + a.samples].copy_from_slice(a.channel(0));
            }
            None => {}
        }
    }
    let t0 = first_tc.unwrap_or(0);
    let mut onsets = Vec::new();
    let mut last: Option<usize> = None;
    for (i, v) in audio.iter().enumerate() {
        if v.abs() > 0.05 {
            if last.is_none_or(|l| i - l > 2400) {
                onsets.push(i);
            }
            last = Some(i);
        }
    }
    let mut offs = Vec::new();
    for i in &onsets {
        let tc = t0 + (*i as i64 * 10_000_000) / 48_000;
        if let Some(o) = video_on
            .iter()
            .map(|v| (tc - v) as f64 / 10_000.0)
            .min_by(|a, b| a.abs().total_cmp(&b.abs()))
        {
            offs.push(o);
        }
    }
    println!(
        "video frames {vframes}, audio frames {aframes}, beeps {}, flashes {}",
        onsets.len(),
        video_on.len()
    );
    println!("A/V offsets (ms, audio - video): {offs:.1?}");
    if !offs.is_empty() {
        let max = offs.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let mean = offs.iter().sum::<f64>() / offs.len() as f64;
        println!("mean {mean:.1} ms, max |offset| {max:.1} ms");
    }
    let peak = audio.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!("audio peak {:.2} dBFS", 20.0 * (peak as f64).log10());
    Ok(())
}
