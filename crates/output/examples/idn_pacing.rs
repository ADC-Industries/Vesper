//! Receives a network video source with the IDN Open Protocol receiver (compatible with
//! NDI®) for a while and measures frame pacing end to end (verification tool): inter-frame
//! arrival intervals (jitter), and, for a test video that shows its frame
//! number as 12 binary blocks across the top quarter (white = 1), which source frames arrived:
//! repeats and skips.
//!
//! ```text
//! cargo run --release -p output --example idn_pacing -- <source> --seconds 30 [--src-fps 25]
//!     [--csv out.csv] [--png-dir dir --png-every 30]
//! ```
//!
//! With `--src-fps` the expected pattern is checked: a source at 25 fps on a 30 fps output
//! repeats one frame in six and skips none. `<source>` is the full `"MACHINE (SOURCE)"` name,
//! just `SOURCE`, or `ip:port`.

use anyhow::{Context, bail};
use output::{AlphaMode, Frame, FrameRate, IdnFinder, IdnReceiver};
use std::io::Write;
use std::time::{Duration, Instant};

const BITS: u32 = 12;

fn counter(f: &output::ReceivedFrame) -> Option<u32> {
    let (w, h) = (f.width, f.height);
    let mut n = 0;
    for bit in 0..BITS {
        let x = (bit * 16 + 8) * w / 192;
        let y = h * 12 / 108;
        let p = f.pixel_rgba(x, y);
        let on = match p[0] {
            0..=40 => false,
            215..=255 => true,
            _ => return None,
        };
        n = (n << 1) | on as u32;
    }
    Some(n)
}

fn percentile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((s.len() - 1) as f64 * q).round() as usize]
}

fn main() -> anyhow::Result<()> {
    let mut name = None;
    let mut seconds = 30.0f64;
    let mut src_fps: Option<f64> = None;
    let mut csv = None;
    let mut png_dir = None;
    let mut png_every = 0usize;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seconds" => seconds = it.next().context("--seconds S")?.parse()?,
            "--src-fps" => src_fps = Some(it.next().context("--src-fps F")?.parse()?),
            "--csv" => csv = Some(it.next().context("--csv FILE")?),
            "--png-dir" => png_dir = Some(it.next().context("--png-dir DIR")?),
            "--png-every" => png_every = it.next().context("--png-every N")?.parse()?,
            _ if name.is_none() => name = Some(a),
            _ => bail!("unexpected argument {a}"),
        }
    }
    let name = name.context("source name required")?;
    let source = if name.parse::<std::net::SocketAddr>().is_ok() {
        name.clone()
    } else {
        IdnFinder::new()?
            .find_by_name(&name, Duration::from_secs(10))
            .with_context(|| format!("source {name:?} not found within 10 s"))?
            .name
    };
    let mut rx = IdnReceiver::connect(&source)?;
    // Let the connection settle.
    let warm = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < warm {
        rx.capture_video(Duration::from_millis(200))?;
    }
    let start = Instant::now();
    let end = start + Duration::from_secs_f64(seconds);
    let mut rows: Vec<(f64, i64, Option<u32>)> = Vec::new();
    let mut size = (0, 0, String::new(), (0, 0));
    while Instant::now() < end {
        let Some(f) = rx.capture_video(Duration::from_secs(2))? else {
            continue;
        };
        let t = start.elapsed().as_secs_f64() * 1000.0;
        let c = counter(&f);
        size = (f.width, f.height, f.fourcc_str(), f.frame_rate);
        if let Some(dir) = &png_dir
            && png_every > 0
            && rows.len().is_multiple_of(png_every)
        {
            let mut rgba = Vec::with_capacity(f.data.len());
            for y in 0..f.height {
                for x in 0..f.width {
                    let [r, g, b, a] = f.pixel_rgba(x, y);
                    let pm = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
                    rgba.extend_from_slice(&[pm(r), pm(g), pm(b), a]);
                }
            }
            let path = format!("{dir}/frame-{:05}.png", rows.len());
            output::write_png(
                &path,
                &Frame {
                    width: f.width,
                    height: f.height,
                    pixels: &rgba,
                    alpha: AlphaMode::Opaque,
                    frame_rate: FrameRate::FPS30,
                    timecode: 0,
                },
            )?;
        }
        rows.push((t, f.timecode, c));
    }
    if rows.len() < 2 {
        bail!("only {} frames received", rows.len());
    }
    if let Some(path) = &csv {
        let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(w, "recv_ms,timecode,counter")?;
        for (t, tc, c) in &rows {
            writeln!(
                w,
                "{t:.3},{tc},{}",
                c.map(|c| c.to_string()).unwrap_or_default()
            )?;
        }
    }
    let intervals: Vec<f64> = rows.windows(2).map(|w| w[1].0 - w[0].0).collect();
    let mean = intervals.iter().sum::<f64>() / intervals.len() as f64;
    let sd =
        (intervals.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / intervals.len() as f64).sqrt();
    let tcs: Vec<i64> = rows.windows(2).map(|w| w[1].1 - w[0].1).collect();
    let tc_gaps = tcs
        .iter()
        .filter(|d| (**d as f64 / 1e4 - mean).abs() > mean * 0.5)
        .count();
    println!(
        "{}: {} frames in {:.1} s ({}x{} {} @ {}/{}): {:.2} fps",
        source,
        rows.len(),
        seconds,
        size.0,
        size.1,
        size.2,
        size.3.0,
        size.3.1,
        (rows.len() - 1) as f64 / ((rows.last().unwrap().0 - rows[0].0) / 1000.0)
    );
    println!(
        "arrival interval ms: mean {mean:.2}, sd {sd:.2}, p50 {:.2}, p95 {:.2}, p99 {:.2}, min {:.2}, max {:.2}; sender timecode gaps {tc_gaps}",
        percentile(&intervals, 0.5),
        percentile(&intervals, 0.95),
        percentile(&intervals, 0.99),
        percentile(&intervals, 0.0),
        percentile(&intervals, 1.0),
    );
    let counters: Vec<u32> = rows.iter().filter_map(|r| r.2).collect();
    if counters.len() > 1 {
        let max = counters.iter().copied().max().unwrap_or(0);
        let mut repeats = 0;
        let mut steps = 0;
        let mut skips = Vec::new();
        let mut wraps = 0;
        for w in counters.windows(2) {
            if w[1] == w[0] {
                repeats += 1;
            } else if w[1] == w[0] + 1 {
                steps += 1;
            } else if w[1] < w[0] && w[1] <= 2 && w[0] + 3 >= max {
                wraps += 1; // the loop point
                if w[1] != 0 {
                    skips.push((w[0], w[1]));
                }
            } else {
                skips.push((w[0], w[1]));
            }
        }
        println!(
            "source frames: {} readable, {} distinct steps, {} repeats, {} loop wraps, {} skips/out of order {:?}",
            counters.len(),
            steps,
            repeats,
            wraps,
            skips.len(),
            &skips[..skips.len().min(10)]
        );
        if let Some(sf) = src_fps {
            let out_fps = rows.len() as f64 / seconds;
            let expect_repeat = (1.0 - sf / out_fps).max(0.0);
            println!(
                "expected repeat share {:.3} (source {sf} fps on ~{out_fps:.1} fps output), measured {:.3}",
                expect_repeat,
                repeats as f64 / (counters.len() - 1) as f64
            );
        }
    } else {
        println!("no frame counter in the picture");
    }
    Ok(())
}
