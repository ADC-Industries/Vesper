//! Sends a few seconds of a synthetic moving test pattern to one sink, for checking an output
//! by eye (e.g. on a receiver on the network).
//!
//! ```text
//! cargo run -p output --example sink_demo -- png   <dir>          # every 30th frame
//! cargo run -p output --example sink_demo -- ffplay                # preview window
//! cargo run -p output --example sink_demo -- record <file.mp4>     # H.264 recording
//! cargo run -p output --example sink_demo -- idn   <source name>   # IDN Open Protocol source
//! ```
//!
//! Add `--alpha` to send a lower-third style frame with a transparent top.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use model::{AlphaMode, FrameRate};
use output::{FfmpegSink, Frame, FrameSink, IdnSenderConfig, IdnSink, PngSink};

fn render(w: u32, h: u32, t: u32, alpha: bool, buf: &mut Vec<u8>) {
    buf.clear();
    let bar = (t * 8) % w;
    for y in 0..h {
        for x in 0..w {
            let a: u32 = if !alpha || y > h * 2 / 3 { 255 } else { 0 };
            let (r, g, b) = if x.abs_diff(bar) < 16 {
                (255, 255, 255)
            } else {
                // Eight vertical colour bars.
                match x * 8 / w {
                    0 => (192, 192, 192),
                    1 => (192, 192, 0),
                    2 => (0, 192, 192),
                    3 => (0, 192, 0),
                    4 => (192, 0, 192),
                    5 => (192, 0, 0),
                    6 => (0, 0, 192),
                    _ => (16, 16, 16),
                }
            };
            let pm = |c: u32| ((c * a + 127) / 255) as u8;
            buf.extend_from_slice(&[pm(r), pm(g), pm(b), a as u8]);
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let alpha = args.iter().any(|a| a == "--alpha");
    let pos: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect();
    let mut sink: Box<dyn FrameSink> = match pos.as_slice() {
        ["png", dir] => Box::new(PngSink::every_nth(dir, 30)?),
        ["ffplay"] => Box::new(FfmpegSink::preview("output sink_demo")),
        ["record", file] => Box::new(FfmpegSink::record(file)),
        ["idn", name] => Box::new(IdnSink::new(IdnSenderConfig::new(*name))?),
        _ => bail!("usage: sink_demo (png <dir> | ffplay | record <file> | idn <name>) [--alpha]"),
    };
    let (w, h) = (1280, 720);
    let rate = FrameRate::FPS30;
    let period = Duration::from_secs_f64(1.0 / rate.as_f64());
    let mut buf = Vec::new();
    let start = Instant::now();
    println!("sending 5 s of {w}x{h}@30 to {}", sink.name());
    for t in 0..150u32 {
        render(w, h, t, alpha, &mut buf);
        let frame = Frame {
            width: w,
            height: h,
            pixels: &buf,
            alpha: if alpha {
                AlphaMode::Straight
            } else {
                AlphaMode::Opaque
            },
            frame_rate: rate,
            timecode: (start.elapsed().as_nanos() / 100) as i64,
        };
        sink.send(&frame)?;
        let next = start + period * (t + 1);
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    drop(sink);
    println!("done");
    Ok(())
}
