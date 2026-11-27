//! Measures sustained frame throughput of the async readback path on several screens.
//!
//! ```text
//! throughput [--screens 3] [--frames 120] [--size 1920x1080] [--change-every 30] [--software]
//! ```
//!
//! Every screen shows a synthetic background image plus a lyric slide (outline + shadow); the
//! slide changes every `--change-every` frames (a slide-cache miss that re-lays out and
//! re-rasterizes text). Frames are submitted without waiting and collected as they finish,
//! as an output thread would.

use model::{
    CharStyle, Color, Element, FitMode, MediaDisplay, Paragraph, Rect, ScaleMode, Shadow, Size,
    Slide, TextBox, TextOutline,
};
use render::{ImageSource, LayerContent, Renderer, RendererOptions, ScreenConfig};
use std::sync::Arc;
use std::time::Instant;

fn lyric(i: usize) -> Slide {
    const LINES: [&str; 4] = [
        "Morning light over the quiet hill\nAnd the river runs so slow.",
        "I once was lost but now am found\nWas blind but now I see",
        "Twas grace that taught my heart to fear\nAnd grace my fears relieved",
        "How precious did that grace appear\nThe hour I first believed",
    ];
    let tb = TextBox {
        paragraphs: Paragraph::from_lines(LINES[i % LINES.len()]),
        default_style: CharStyle {
            font_family: Some("DejaVu Sans".into()),
            font_weight: Some(700),
            size: Some(110.0),
            color: Some(Color::WHITE),
            ..CharStyle::default()
        },
        fit: FitMode::ShrinkToFit,
        outline: TextOutline {
            enabled: true,
            ..TextOutline::default()
        },
        text_shadow: Shadow {
            enabled: true,
            offset: 15.0,
            radius: 8.0,
            ..Shadow::default()
        },
        ..TextBox::default()
    };
    Slide {
        elements: vec![Element::text(Rect::new(55.0, 55.0, 1810.0, 970.0), tb)],
        ..Slide::new()
    }
}

fn main() -> anyhow::Result<()> {
    let mut screens = 3usize;
    let mut frames = 120usize;
    let mut size = (1920u32, 1080u32);
    let mut change_every = 30usize;
    let mut opts = RendererOptions::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--screens" => screens = it.next().unwrap_or_default().parse()?,
            "--frames" => frames = it.next().unwrap_or_default().parse()?,
            "--change-every" => {
                change_every = it.next().unwrap_or_default().parse::<usize>()?.max(1)
            }
            "--size" => {
                let v = it.next().unwrap_or_default();
                let (w, h) = v
                    .split_once('x')
                    .ok_or_else(|| anyhow::anyhow!("--size WxH"))?;
                size = (w.parse()?, h.parse()?);
            }
            "--software" => opts.adapter = render::AdapterPreference::Software,
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    let mut r = Renderer::new(opts)?;
    println!("adapter: {}", r.adapter());
    let bg = Arc::new(image::RgbaImage::from_fn(1920, 1080, |x, y| {
        image::Rgba([(x / 8) as u8, (y / 5) as u8, 128, 255])
    }));
    let display = MediaDisplay {
        scale: ScaleMode::Fill,
        ..MediaDisplay::default()
    };
    let slides: Vec<Slide> = (0..4).map(lyric).collect();
    let mut targets: Vec<_> = (0..screens)
        .map(|_| r.create_screen(ScreenConfig::opaque(size.0, size.1)))
        .collect();

    let start = Instant::now();
    let mut received = 0usize;
    for f in 0..frames {
        let slide = &slides[(f / change_every) % slides.len()];
        for t in targets.iter_mut() {
            let layers = [
                LayerContent::media(
                    ImageSource::Rgba {
                        key: 1,
                        image: bg.clone(),
                    },
                    display.clone(),
                ),
                LayerContent::slide(slide, Size::HD),
            ];
            r.submit(t, &layers)?;
            while let Some(frame) = r.poll(t)? {
                received += 1;
                t.recycle(frame);
            }
        }
    }
    for t in targets.iter_mut() {
        while let Some(frame) = r.wait(t)? {
            received += 1;
            t.recycle(frame);
        }
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{screens} screens × {frames} frames at {}x{}: {received} frames in {secs:.2} s = {:.1} frames/s total, {:.1} fps per screen",
        size.0,
        size.1,
        received as f64 / secs,
        frames as f64 / secs
    );
    Ok(())
}
