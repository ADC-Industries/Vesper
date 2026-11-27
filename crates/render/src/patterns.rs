//! Test patterns and the "identify screens" label.
//!
//! Both are built as ordinary [`Slide`]s (elements in screen pixels: the canvas is the screen
//! size) plus a few solid rectangles ([`LayerSource::Rects`]), so they go through the same
//! GPU pipeline, colour handling and alpha delivery as the show. The static part of a pattern
//! is one cached slide; the moving part of [`TestPattern::Motion`] is plain rectangles, so a
//! pattern that changes every frame costs no new slide texture per frame.
//!
//! Colours are sRGB straight alpha like every model colour, so a 75 % bar (`191/255`) comes
//! out of the renderer as 191 (±1 rounding).

use crate::{LayerContent, LayerSource};
use model::{
    CharStyle, Color, Element, ElementKind, Fill, FillKind, FitMode, Gradient, GradientStop, Layer,
    Paragraph, Rect, Shape, Size, Slide, Stroke, TestPattern, TextBox,
};

/// What a pattern is drawn for: shown in its caption.
#[derive(Clone, Debug, Default)]
pub struct PatternInfo {
    /// Screen name.
    pub name: String,
    /// Frames per second of the screen (the motion pattern's sweep and counter).
    pub fps: f64,
    /// Frame number (the motion pattern; ignored by the static ones).
    pub frame: u64,
}

/// A drawable pattern or overlay: owns its slide and rectangles; borrow them as layers with
/// [`Scene::layers`].
#[derive(Clone, Debug)]
pub struct Scene {
    /// Static part (cached by the renderer like any slide).
    pub slide: Slide,
    /// Canvas size of `slide` (the screen size).
    pub canvas: Size,
    /// Solid rectangles drawn above the slide, screen px.
    pub rects: Vec<(Rect, Color)>,
    /// Compositor layer.
    pub layer: Layer,
}

impl Scene {
    /// The scene as compositor layers (slide, then rectangles).
    pub fn layers(&self) -> Vec<LayerContent<'_>> {
        let mut v = vec![LayerContent::slide(&self.slide, self.canvas).on_layer(self.layer)];
        if !self.rects.is_empty() {
            v.push(LayerContent {
                layer: self.layer,
                source: LayerSource::Rects(self.rects.clone()),
                opacity: 1.0,
            });
        }
        v
    }
}

/// Whether a pattern changes every frame (render it on every tick).
pub fn is_animated(p: &TestPattern) -> bool {
    matches!(p, TestPattern::Motion)
}

fn rgb8(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

fn rect_el(r: Rect, c: Color) -> Element {
    Element {
        fill: Fill::color(c),
        ..Element::new(r)
    }
}

fn outline_el(r: Rect, shape: Shape, width: f64, c: Color) -> Element {
    Element {
        kind: ElementKind::Shape { shape },
        stroke: Stroke {
            enabled: true,
            width,
            color: c,
            ..Stroke::default()
        },
        ..Element::new(r)
    }
}

fn gradient_el(r: Rect, from: Color, to: Color) -> Element {
    Element {
        fill: Fill::enabled(FillKind::Gradient {
            gradient: Gradient {
                stops: vec![
                    GradientStop {
                        color: from,
                        position: 0.0,
                        midpoint: 0.5,
                    },
                    GradientStop {
                        color: to,
                        position: 1.0,
                        midpoint: 0.5,
                    },
                ],
                ..Gradient::default()
            },
        }),
        ..Element::new(r)
    }
}

/// A centred text element (DejaVu Sans Bold), shrunk to fit its box.
pub fn text_el(r: Rect, text: &str, size: f64, c: Color) -> Element {
    Element::text(
        r,
        TextBox {
            paragraphs: Paragraph::from_lines(text),
            default_style: CharStyle {
                font_family: Some("DejaVu Sans".into()),
                font_weight: Some(700),
                size: Some(size),
                color: Some(c),
                ..CharStyle::default()
            },
            fit: FitMode::ShrinkToFit,
            ..TextBox::default()
        },
    )
}

fn caption(info: &PatternInfo, what: &str, w: f64, h: f64) -> String {
    let fps = if (info.fps - info.fps.round()).abs() < 0.005 {
        format!("{}", info.fps.round())
    } else {
        format!("{:.2}", info.fps)
    };
    let name = if info.name.is_empty() {
        String::new()
    } else {
        format!("{} · ", info.name)
    };
    format!("{name}{what} · {}×{} @ {fps}", w as u32, h as u32)
}

/// The 75 % SMPTE-style bar colours, left to right (sRGB 8-bit).
pub const BARS_75: [[u8; 3]; 7] = [
    [191, 191, 191],
    [191, 191, 0],
    [0, 191, 191],
    [0, 191, 0],
    [191, 0, 191],
    [191, 0, 0],
    [0, 0, 191],
];

/// Builds a test pattern for a `width × height` screen.
pub fn test_pattern(
    pattern: &TestPattern,
    (width, height): (u32, u32),
    info: &PatternInfo,
) -> Scene {
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let mut els: Vec<Element> = Vec::new();
    let mut rects: Vec<(Rect, Color)> = Vec::new();
    let label_size = (h / 22.0).max(10.0);
    match pattern {
        TestPattern::ColorBars | TestPattern::Unknown => {
            let bw = w / 7.0;
            let (top, mid) = (h * 0.67, h * 0.08);
            for (i, c) in BARS_75.iter().enumerate() {
                let x0 = (i as f64 * bw).round();
                let x1 = ((i + 1) as f64 * bw).round();
                els.push(rect_el(
                    Rect::new(x0, 0.0, x1 - x0, top),
                    rgb8(c[0], c[1], c[2]),
                ));
            }
            // Castellations: the reverse-order blue / black row.
            let castle = [
                [0, 0, 191],
                [0, 0, 0],
                [191, 0, 191],
                [0, 0, 0],
                [0, 191, 191],
                [0, 0, 0],
                [191, 191, 191],
            ];
            for (i, c) in castle.iter().enumerate() {
                let x0 = (i as f64 * bw).round();
                let x1 = ((i + 1) as f64 * bw).round();
                els.push(rect_el(
                    Rect::new(x0, top, x1 - x0, mid),
                    rgb8(c[0], c[1], c[2]),
                ));
            }
            // Bottom: -I, 100 % white, +Q, black, PLUGE (0 / 4 % / 8 %), black.
            let y = top + mid;
            let bh = h - y;
            let wide = bw * 5.0 / 4.0;
            let bottom = [[0, 33, 76], [255, 255, 255], [50, 0, 106], [0, 0, 0]];
            for (i, c) in bottom.iter().enumerate() {
                let x0 = (i as f64 * wide).round();
                let x1 = ((i + 1) as f64 * wide).round();
                els.push(rect_el(
                    Rect::new(x0, y, x1 - x0, bh),
                    rgb8(c[0], c[1], c[2]),
                ));
            }
            let px = (5.0 * bw).round();
            let third = bw / 3.0;
            for (i, v) in [0u8, 10, 20].iter().enumerate() {
                let x0 = (px + i as f64 * third).round();
                let x1 = (px + (i + 1) as f64 * third).round();
                els.push(rect_el(Rect::new(x0, y, x1 - x0, bh), rgb8(*v, *v, *v)));
            }
            let x0 = (6.0 * bw).round();
            els.push(rect_el(Rect::new(x0, y, w - x0, bh), Color::BLACK));
            els.push(text_el(
                Rect::new(0.0, y + bh * 0.55, w * 0.55, bh * 0.4),
                &caption(info, "Colour bars 75 %", w, h),
                label_size * 0.8,
                rgb8(140, 140, 140),
            ));
        }
        TestPattern::Grid => {
            els.push(rect_el(Rect::new(0.0, 0.0, w, h), Color::BLACK));
            let line = (h / 540.0).round().max(1.0);
            let grey = rgb8(110, 110, 110);
            let cell = h / 9.0;
            let (cx, cy) = (w / 2.0, h / 2.0);
            let mut k = 0.0;
            while cx - k * cell >= 0.0 {
                for x in [cx - k * cell, cx + k * cell] {
                    els.push(rect_el(
                        Rect::new((x - line / 2.0).round(), 0.0, line, h),
                        grey,
                    ));
                }
                k += 1.0;
            }
            let mut k = 0.0;
            while cy - k * cell >= 0.0 {
                for y in [cy - k * cell, cy + k * cell] {
                    els.push(rect_el(
                        Rect::new(0.0, (y - line / 2.0).round(), w, line),
                        grey,
                    ));
                }
                k += 1.0;
            }
            // Border, safe areas, circle, centre cross.
            els.push(outline_el(
                Rect::new(line, line, w - 2.0 * line, h - 2.0 * line),
                Shape::Rectangle,
                line * 2.0,
                Color::WHITE,
            ));
            let safe = |f: f64| Rect::new(w * (1.0 - f) / 2.0, h * (1.0 - f) / 2.0, w * f, h * f);
            els.push(outline_el(
                safe(0.93),
                Shape::Rectangle,
                line * 2.0,
                rgb8(255, 210, 0),
            ));
            els.push(outline_el(
                safe(0.90),
                Shape::Rectangle,
                line * 2.0,
                rgb8(0, 200, 255),
            ));
            let d = h * 0.9;
            els.push(outline_el(
                Rect::new(cx - d / 2.0, cy - d / 2.0, d, d),
                Shape::Ellipse,
                line * 3.0,
                Color::WHITE,
            ));
            let arm = h * 0.06;
            els.push(rect_el(
                Rect::new(cx - arm, cy - line, arm * 2.0, line * 2.0),
                rgb8(255, 40, 40),
            ));
            els.push(rect_el(
                Rect::new(cx - line, cy - arm, line * 2.0, arm * 2.0),
                rgb8(255, 40, 40),
            ));
            els.push(text_el(
                Rect::new(w * 0.15, cy + h * 0.12, w * 0.7, label_size * 1.8),
                &caption(info, "Grid", w, h),
                label_size,
                Color::WHITE,
            ));
            els.push(text_el(
                Rect::new(
                    w * 0.15,
                    cy + h * 0.12 + label_size * 1.8,
                    w * 0.7,
                    label_size * 1.4,
                ),
                "action safe 93 % (yellow) · title safe 90 % (blue)",
                label_size * 0.6,
                rgb8(180, 180, 180),
            ));
        }
        TestPattern::Gradient => {
            els.push(rect_el(Rect::new(0.0, 0.0, w, h), Color::BLACK));
            let band = h / 4.0;
            let ends = [
                Color::WHITE,
                Color::rgb(1.0, 0.0, 0.0),
                Color::rgb(0.0, 1.0, 0.0),
                Color::rgb(0.0, 0.0, 1.0),
            ];
            for (i, c) in ends.iter().enumerate() {
                let y0 = (i as f64 * band).round();
                let y1 = ((i + 1) as f64 * band).round();
                els.push(gradient_el(
                    Rect::new(0.0, y0, w, y1 - y0),
                    Color::BLACK,
                    *c,
                ));
            }
            els.push(text_el(
                Rect::new(w * 0.02, h - label_size * 1.8, w * 0.6, label_size * 1.6),
                &caption(info, "Ramps", w, h),
                label_size * 0.7,
                rgb8(200, 200, 200),
            ));
        }
        TestPattern::Solid { color } => {
            els.push(rect_el(Rect::new(0.0, 0.0, w, h), *color));
        }
        TestPattern::AlphaChecker => {
            // Opaque white / fully transparent squares, then a white alpha ramp band and
            // 25 / 50 / 75 % grey patches. Transparent areas show the screen background
            // (black on opaque outputs).
            let n = 16.0;
            let sq = w / n;
            let rows = ((h * 0.6) / sq).floor().max(1.0);
            for r in 0..rows as u32 {
                for c in 0..n as u32 {
                    if (r + c) % 2 == 0 {
                        let x0 = (c as f64 * sq).round();
                        let x1 = ((c + 1) as f64 * sq).round();
                        let y0 = (r as f64 * sq).round();
                        let y1 = ((r + 1) as f64 * sq).round();
                        els.push(rect_el(Rect::new(x0, y0, x1 - x0, y1 - y0), Color::WHITE));
                    }
                }
            }
            let y = (rows * sq).round();
            let ramp_h = ((h - y) * 0.5).round();
            els.push(gradient_el(
                Rect::new(0.0, y, w, ramp_h),
                Color::rgba(1.0, 1.0, 1.0, 0.0),
                Color::WHITE,
            ));
            let py = y + ramp_h;
            let pw = w / 4.0;
            for (i, a) in [0.25f32, 0.5, 0.75, 1.0].iter().enumerate() {
                let x0 = (i as f64 * pw).round();
                let x1 = ((i + 1) as f64 * pw).round();
                els.push(rect_el(
                    Rect::new(x0, py, x1 - x0, h - py),
                    Color::rgba(1.0, 0.0, 0.0, *a),
                ));
            }
            els.push(text_el(
                Rect::new(0.0, py, w, h - py),
                &caption(info, "Alpha: red at 25 / 50 / 75 / 100 %", w, h),
                label_size * 0.7,
                Color::WHITE,
            ));
        }
        TestPattern::Motion => {
            els.push(rect_el(Rect::new(0.0, 0.0, w, h), rgb8(40, 40, 40)));
            // Tick marks: one per frame position of the sweep.
            let fps = info.fps.max(1.0);
            let steps = fps.round().max(1.0);
            let bar_w = (w / steps).max(2.0);
            for i in 0..steps as u32 {
                let x = (i as f64 * (w - bar_w) / (steps - 1.0).max(1.0)).round();
                els.push(rect_el(
                    Rect::new(x, h * 0.18, 1.0f64.max(h / 540.0), h * 0.04),
                    rgb8(120, 120, 120),
                ));
            }
            els.push(text_el(
                Rect::new(0.0, h * 0.02, w, label_size * 1.8),
                &caption(info, "Motion", w, h),
                label_size,
                Color::WHITE,
            ));
            els.push(text_el(
                Rect::new(0.0, h * 0.72, w, label_size * 1.2),
                "the bar crosses once per second; the counter below counts frames in binary",
                label_size * 0.55,
                rgb8(170, 170, 170),
            ));
            // Moving part.
            let pos = info.frame % steps as u64;
            let x = (pos as f64 * (w - bar_w) / (steps - 1.0).max(1.0)).round();
            rects.push((Rect::new(x, h * 0.25, bar_w, h * 0.42), Color::WHITE));
            let bits = 16u32;
            let cw = (w * 0.8 / bits as f64).floor();
            let x0 = ((w - cw * bits as f64) / 2.0).round();
            let y0 = (h * 0.82).round();
            let ch = (h * 0.08).round().max(2.0);
            for b in 0..bits {
                let on = info.frame >> (bits - 1 - b) & 1 == 1;
                let c = if on { Color::WHITE } else { rgb8(70, 70, 70) };
                let x = x0 + b as f64 * cw;
                rects.push((Rect::new(x + 2.0, y0, cw - 4.0, ch), c));
            }
        }
    }
    Scene {
        slide: Slide {
            elements: els,
            ..Slide::new()
        },
        canvas: Size::new(w, h),
        rects,
        layer: Layer::Slide,
    }
}

/// The "identify screens" label: `lines[0]` large (the screen name), the rest smaller,
/// centred in a dark box with a thin white border, above everything else.
pub fn identify_overlay(lines: &[String], (width, height): (u32, u32)) -> Scene {
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let bw = w * 0.7;
    let bh = h * 0.5;
    let (x, y) = ((w - bw) / 2.0, (h - bh) / 2.0);
    let border = (h / 270.0).round().max(2.0);
    let mut els = vec![
        rect_el(Rect::new(x, y, bw, bh), Color::rgba(0.0, 0.0, 0.0, 0.85)),
        outline_el(
            Rect::new(x, y, bw, bh),
            Shape::Rectangle,
            border,
            Color::WHITE,
        ),
    ];
    let title = lines.first().map(String::as_str).unwrap_or("");
    els.push(text_el(
        Rect::new(x + bw * 0.05, y + bh * 0.06, bw * 0.9, bh * 0.45),
        title,
        h / 7.0,
        Color::WHITE,
    ));
    if lines.len() > 1 {
        els.push(text_el(
            Rect::new(x + bw * 0.05, y + bh * 0.52, bw * 0.9, bh * 0.42),
            &lines[1..].join("\n"),
            h / 22.0,
            Color::rgb(1.0, 0.85, 0.2),
        ));
    }
    Scene {
        slide: Slide {
            elements: els,
            ..Slide::new()
        },
        canvas: Size::new(w, h),
        rects: Vec::new(),
        // The top-most layer: above every layer, captions included.
        layer: Layer::Captions,
    }
}
