//! Times every pixel conversion on 1080p frames with three kinds of content.
//!
//! ```text
//! cargo run -p output --release --example convert_bench
//! ```

use std::time::Instant;

use output::convert::{PixelFormat, convert};

fn frame(w: u32, h: u32, alpha: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let a = alpha(x, y) as u32;
            let pm = |c: u32| ((c * a + 127) / 255) as u8;
            v.extend_from_slice(&[pm(x % 256), pm(y % 256), pm((x ^ y) % 256), a as u8]);
        }
    }
    v
}

fn main() {
    let (w, h) = (1920u32, 1080u32);
    let contents = [
        ("opaque", frame(w, h, |_, _| 255)),
        // A lower third: transparent except an opaque band with soft edges.
        (
            "lower third",
            frame(w, h, |_, y| match y {
                0..=799 => 0,
                800..=815 => ((y - 800) * 16) as u8,
                _ => 255,
            }),
        ),
        ("alpha ramp", frame(w, h, |x, _| (x * 255 / w) as u8)),
    ];
    let formats = [
        PixelFormat::BgraPremultiplied,
        PixelFormat::Bgrx,
        PixelFormat::Bgra,
        PixelFormat::Rgba,
        PixelFormat::Rgb,
        PixelFormat::Uyvy,
        PixelFormat::Uyva,
    ];
    let mut dst = Vec::new();
    let n = 30;
    println!("{w}x{h}, {n} iterations each (ms per frame)");
    print!("{:<18}", "");
    for (name, _) in &contents {
        print!("{name:>13}");
    }
    println!();
    for format in formats {
        print!("{:<18}", format!("{format:?}"));
        for (_, src) in &contents {
            convert(src, w, h, format, &mut dst).unwrap();
            let start = Instant::now();
            for _ in 0..n {
                convert(src, w, h, format, &mut dst).unwrap();
            }
            let ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
            print!("{ms:>13.2}");
        }
        println!();
    }
}
