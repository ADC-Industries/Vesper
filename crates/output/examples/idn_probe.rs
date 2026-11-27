//! Receives a network video source with the IDN Open Protocol receiver (compatible with
//! NDI®) and reports what arrives (verification tool).
//!
//! ```text
//! cargo run --release -p output --example idn_probe -- <source> [--frames N] [--png out.png]
//! cargo run --release -p output --example idn_probe -- --list [--wait SECS]
//! ```
//!
//! `<source>` is the full `"MACHINE (SOURCE)"` name, just `SOURCE`, or `ip:port` (connects
//! directly, without discovery).

use anyhow::{Context, bail};
use output::{AlphaMode, Frame, FrameRate, IdnFinder, IdnReceiver};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let mut name = None;
    let mut frames = 30usize;
    let mut png = None;
    let mut list = false;
    let mut wait = 3.0f64;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--frames" => frames = it.next().context("--frames N")?.parse()?,
            "--png" => png = Some(it.next().context("--png FILE")?),
            "--wait" => wait = it.next().context("--wait SECS")?.parse()?,
            "--list" => list = true,
            _ if name.is_none() => name = Some(a),
            _ => bail!("unexpected argument {a}"),
        }
    }
    if list {
        let finder = IdnFinder::new()?;
        let mut sources = finder.sources(Duration::from_secs_f64(wait));
        sources.sort_by(|a, b| a.name.cmp(&b.name));
        for s in &sources {
            println!(
                "{}  ({}; {}; groups {})",
                s.name,
                s.address.as_deref().unwrap_or("-"),
                if s.is_idn { "IDN" } else { "other" },
                if s.groups.is_empty() {
                    "-".to_string()
                } else {
                    s.groups.join(",")
                }
            );
        }
        println!("{} source(s)", sources.len());
        return Ok(());
    }
    let name = name.context("source name required")?;
    let t = Instant::now();
    // `ip:port` connects directly; anything else is looked up by name.
    let target = if name.parse::<std::net::SocketAddr>().is_ok() {
        name.clone()
    } else {
        let finder = IdnFinder::new()?;
        let source = finder
            .find_by_name(&name, Duration::from_secs(10))
            .with_context(|| format!("source {name:?} not found within 10 s"))?;
        println!(
            "found {:?} at {} after {:.1} s",
            source.name,
            source.address.as_deref().unwrap_or("?"),
            t.elapsed().as_secs_f64()
        );
        source.name
    };
    let mut rx = IdnReceiver::connect(&target)?;
    let mut got = 0usize;
    let mut first: Option<Instant> = None;
    let mut last = None;
    let deadline = Instant::now() + Duration::from_secs(10 + frames as u64 / 10);
    while got < frames && Instant::now() < deadline {
        if let Some(f) = rx.capture_video(Duration::from_secs(2))? {
            first.get_or_insert_with(Instant::now);
            got += 1;
            last = Some(f);
        }
    }
    let Some(f) = last else {
        bail!("no video frames received");
    };
    let secs = first.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
    let (mut amin, mut amax) = (255u8, 0u8);
    for y in (0..f.height).step_by(4) {
        for x in (0..f.width).step_by(4) {
            let a = f.pixel_rgba(x, y)[3];
            amin = amin.min(a);
            amax = amax.max(a);
        }
    }
    println!(
        "received {got} frames from {}: {}x{} {} @ {}/{} (measured {:.1} fps), alpha {amin}..{amax}, centre pixel {:?}",
        rx.connected_to().unwrap_or("?"),
        f.width,
        f.height,
        f.fourcc_str(),
        f.frame_rate.0,
        f.frame_rate.1,
        if secs > 0.0 && got > 1 {
            (got - 1) as f64 / secs
        } else {
            0.0
        },
        f.pixel_rgba(f.width / 2, f.height / 2)
    );
    if let Some(path) = png {
        // Straight RGBA/RGBX → premultiplied RGBA, which is what `write_png` takes.
        let mut rgba = Vec::with_capacity(f.data.len());
        for y in 0..f.height {
            for x in 0..f.width {
                let [r, g, b, a] = f.pixel_rgba(x, y);
                let pm = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
                rgba.extend_from_slice(&[pm(r), pm(g), pm(b), a]);
            }
        }
        output::write_png(
            &path,
            &Frame {
                width: f.width,
                height: f.height,
                pixels: &rgba,
                alpha: if f.fourcc_str() == "RGBX" {
                    AlphaMode::Opaque
                } else {
                    AlphaMode::Straight
                },
                frame_rate: FrameRate::FPS30,
                timecode: 0,
            },
        )?;
        println!("wrote {path}");
    }
    Ok(())
}
