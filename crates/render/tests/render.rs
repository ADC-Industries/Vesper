//! Golden-ish rendering tests on synthetic slides: pixel probes with tolerances, no image
//! compares. They run on any adapter, including lavapipe (software Vulkan).

use model::{
    AlphaMode, Anchor, Baseline, Capitalization, CharStyle, Color, Element, ElementKind, Fill,
    FillKind, FitMode, FontSubstitution, FontSubstitutionTable, Layer, MediaAssetId, MediaDisplay,
    Paragraph, ParagraphStyle, Rect, ScaleMode, Shadow, Shape, Size, Slide, Stroke, TextAlign,
    TextBox, TextOutline, TextRun, VerticalAlign,
};
use render::{
    FontMatch, ImageSource, LayerContent, RenderedFrame, Renderer, RendererOptions, ScreenConfig,
};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

fn shared() -> MutexGuard<'static, Renderer> {
    static R: OnceLock<Mutex<Renderer>> = OnceLock::new();
    R.get_or_init(|| {
        let r = Renderer::new(test_options()).expect("renderer");
        eprintln!("test adapter: {}", r.adapter());
        Mutex::new(r)
    })
    .lock()
    .unwrap_or_else(|e| e.into_inner())
}

/// DejaVu only when it is installed in the usual Linux place (deterministic glyphs), else the
/// system fonts.
fn test_options() -> RendererOptions {
    let dejavu = std::path::Path::new("/usr/share/fonts/truetype/dejavu");
    if dejavu.is_dir() {
        RendererOptions {
            system_fonts: false,
            font_dirs: vec![dejavu.to_path_buf()],
            ..RendererOptions::default()
        }
    } else {
        RendererOptions::default()
    }
}

const W: u32 = 960;
const H: u32 = 540;

fn canvas() -> Size {
    Size::new(W as f64, H as f64)
}

fn text_box(text: &str, size: f64) -> TextBox {
    TextBox {
        paragraphs: Paragraph::from_lines(text),
        default_style: CharStyle {
            font_family: Some("DejaVu Sans".into()),
            font_weight: Some(700),
            size: Some(size),
            color: Some(Color::WHITE),
            ..CharStyle::default()
        },
        ..TextBox::default()
    }
}

fn slide_with(elements: Vec<Element>) -> Slide {
    Slide {
        elements,
        ..Slide::new()
    }
}

fn render(slide: &Slide) -> RenderedFrame {
    shared()
        .render_layers(
            ScreenConfig::opaque(W, H),
            &[LayerContent::slide(slide, canvas())],
        )
        .expect("render")
}

fn lum(p: [u8; 4]) -> u32 {
    (p[0] as u32 + p[1] as u32 + p[2] as u32) / 3
}

/// Bounding box (x0, y0, x1, y1) of pixels matching `f`.
fn bbox(f: &RenderedFrame, pred: impl Fn([u8; 4]) -> bool) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..f.height {
        for x in 0..f.width {
            if pred(f.pixel(x, y)) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x0 != u32::MAX).then_some((x0, y0, x1, y1))
}

fn centroid(f: &RenderedFrame, pred: impl Fn([u8; 4]) -> bool) -> (f64, f64, usize) {
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0usize);
    for y in 0..f.height {
        for x in 0..f.width {
            if pred(f.pixel(x, y)) {
                sx += x as f64;
                sy += y as f64;
                n += 1;
            }
        }
    }
    (sx / n.max(1) as f64, sy / n.max(1) as f64, n)
}

fn close(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| (*x as i32 - *y as i32).abs() <= tol)
}

#[test]
fn text_is_centered() {
    let el = Element::text(
        Rect::new(0.0, 0.0, W as f64, H as f64),
        text_box("HHHH", 80.0),
    );
    let f = render(&slide_with(vec![el]));
    assert_eq!(f.data.len(), (W * H * 4) as usize);
    let (x0, y0, x1, y1) = bbox(&f, |p| lum(p) > 128).expect("text drawn");
    let cx = (x0 + x1) as f64 / 2.0;
    let cy = (y0 + y1) as f64 / 2.0;
    assert!((cx - W as f64 / 2.0).abs() < 4.0, "x centre {cx}");
    // cap height sits a little above the line-box centre (descender space below)
    assert!((cy - H as f64 / 2.0).abs() < 12.0, "y centre {cy}");
    assert!(x1 - x0 > 150, "text too small: {}", x1 - x0);
}

/// A run with an absurd font size (a typo in the editor, a bad import) renders without
/// rasterizing glyphs bigger than any texture: one glyph a million px high overflowed the
/// rasterizer's mask size (a panic; in release builds a mask of the wrong size), and sizes
/// just below that needed gigabytes per glyph.
#[test]
fn absurd_font_sizes_do_not_abort() {
    let huge = Element::text(
        Rect::new(0.0, 0.0, W as f64, H as f64),
        text_box("W", 1_000_000.0),
    );
    // Normal text beside it still draws.
    let normal = Element::text(
        Rect::new(0.0, 0.0, W as f64, H as f64),
        text_box("HH", 80.0),
    );
    let f = render(&slide_with(vec![huge, normal]));
    let (x0, _, x1, _) = bbox(&f, |p| lum(p) > 128).expect("the normal-size text");
    assert!(x1 - x0 < W / 2, "only the normal-size text: {x0}..{x1}");
}

#[test]
fn alignment_left_and_right() {
    let mk = |a: TextAlign| {
        let mut tb = text_box("HH", 60.0);
        tb.default_paragraph = ParagraphStyle {
            alignment: Some(a),
            ..ParagraphStyle::default()
        };
        Element::text(Rect::new(100.0, 100.0, 760.0, 300.0), tb)
    };
    let l = bbox(&render(&slide_with(vec![mk(TextAlign::Left)])), |p| {
        lum(p) > 128
    })
    .unwrap();
    let r = bbox(&render(&slide_with(vec![mk(TextAlign::Right)])), |p| {
        lum(p) > 128
    })
    .unwrap();
    assert!(l.0 >= 100 && l.0 < 112, "left edge {l:?}");
    assert!(r.2 <= 860 && r.2 > 845, "right edge {r:?}");
}

fn outline_scan(width: f64, color: Color) -> (usize, usize) {
    let mut tb = text_box("I", 300.0);
    tb.outline = TextOutline {
        enabled: width > 0.0,
        width,
        color,
    };
    let f = render(&slide_with(vec![Element::text(
        Rect::new(0.0, 0.0, W as f64, H as f64),
        tb,
    )]));
    // horizontal scanline through the middle of the "I" stem
    let y = H / 2;
    let row: Vec<[u8; 4]> = (0..W).map(|x| f.pixel(x, y)).collect();
    let white = row.iter().filter(|p| lum(**p) > 230).count();
    let red = row
        .iter()
        .filter(|p| p[0] > 200 && p[1] < 60 && p[2] < 60)
        .count();
    (white, red)
}

#[test]
fn outline_surrounds_glyph_edges() {
    let (white0, red0) = outline_scan(0.0, Color::rgb(1.0, 0.0, 0.0));
    assert!(white0 > 20, "stem visible: {white0}");
    assert_eq!(red0, 0);
    let (white, red) = outline_scan(16.0, Color::rgb(1.0, 0.0, 0.0));
    // centred stroke under the fill: 8 px visible on each side of the stem, fill unchanged
    assert!(
        (14..=20).contains(&red),
        "outline pixels on the scanline: {red}"
    );
    assert!(
        (white as i64 - white0 as i64).abs() <= 2,
        "{white} vs {white0}"
    );
}

#[test]
fn outline_is_on_both_sides_of_stem() {
    let mut tb = text_box("I", 300.0);
    tb.outline = TextOutline {
        enabled: true,
        width: 16.0,
        color: Color::rgb(1.0, 0.0, 0.0),
    };
    let f = render(&slide_with(vec![Element::text(
        Rect::new(0.0, 0.0, W as f64, H as f64),
        tb,
    )]));
    let y = H / 2;
    let is_red = |p: [u8; 4]| p[0] > 200 && p[1] < 60 && p[2] < 60;
    let xs: Vec<u32> = (0..W).filter(|x| lum(f.pixel(*x, y)) > 230).collect();
    let (l, r) = (xs[0], *xs.last().unwrap());
    assert!(is_red(f.pixel(l - 3, y)), "left of stem");
    assert!(is_red(f.pixel(r + 3, y)), "right of stem");
    assert!(lum(f.pixel(l - 14, y)) < 20, "black beyond the outline");
}

fn shadow_offset(angle: f64) -> (f64, f64) {
    let mut tb = text_box("I", 200.0);
    tb.text_shadow = Shadow {
        enabled: true,
        angle,
        offset: 30.0,
        radius: 4.0,
        color: Color::rgb(0.0, 1.0, 0.0),
        opacity: 1.0,
    };
    let f = render(&slide_with(vec![Element::text(
        Rect::new(0.0, 0.0, W as f64, H as f64),
        tb,
    )]));
    let (wx, wy, wn) = centroid(&f, |p| lum(p) > 230);
    let (gx, gy, gn) = centroid(&f, |p| p[1] > 120 && p[0] < 80 && p[2] < 80);
    assert!(wn > 100 && gn > 100, "white {wn}, shadow {gn}");
    (gx - wx, gy - wy)
}

#[test]
fn shadow_315_points_down_right() {
    let (dx, dy) = shadow_offset(315.0);
    assert!(
        dx > 10.0 && dy > 10.0,
        "315° shadow offset ({dx:.1}, {dy:.1})"
    );
    let (dx, dy) = shadow_offset(90.0);
    assert!(
        dx.abs() < 5.0 && dy < -10.0,
        "90° shadow offset ({dx:.1}, {dy:.1})"
    );
}

#[test]
fn shadow_is_blurred() {
    let mut tb = text_box("I", 200.0);
    tb.text_shadow = Shadow {
        enabled: true,
        offset: 40.0,
        radius: 24.0,
        color: Color::rgb(0.0, 1.0, 0.0),
        opacity: 1.0,
        ..Shadow::default()
    };
    let f = render(&slide_with(vec![Element::text(
        Rect::new(0.0, 0.0, W as f64, H as f64),
        tb,
    )]));
    // a blurred shadow has many partially green pixels
    let partial = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = f.pixel(x, y);
            p[0] < 40 && p[1] > 20 && p[1] < 200
        })
        .count();
    assert!(partial > 2000, "soft shadow pixels: {partial}");
}

#[test]
fn the_measurer_lays_out_like_the_renderer_without_a_gpu() {
    let long = "Lanterns along the water wait for the morning tide to turn";
    let mut tb = text_box(long, 80.0);
    tb.fit = FitMode::ShrinkToFit;
    let bounds = Rect::new(280.0, 200.0, 400.0, 100.0);
    let mut m = render::TextMeasurer::new(&test_options());
    let a = m.measure(&tb, bounds);
    let b = shared().measure_text(&tb, bounds);
    assert_eq!((a.lines, a.fit_scale), (b.lines, b.fit_scale));
    assert!((a.block_height - b.block_height).abs() < 0.01);
    // `fits` ignores the box's own shrink: at 80 px the text overflows.
    assert!(!m.fits(&tb, bounds));
    assert!(m.fits(&text_box("Wait", 40.0), bounds));
    // A single word wider than the box does not fit either.
    assert!(!m.fits(&text_box("Lanternslanternslanterns", 80.0), bounds));
}

#[test]
fn shrink_to_fit_reduces_size() {
    let long = "Morning light over the quiet hill and the river runs so slow";
    let mut tb = text_box(long, 80.0);
    let bounds = Rect::new(280.0, 200.0, 400.0, 100.0);
    let mut r = shared();
    let none = r.measure_text(&tb, bounds);
    assert_eq!(none.fit_scale, 1.0);
    assert!(
        none.block_height > 100.0,
        "overflows unshrunk: {}",
        none.block_height
    );
    tb.fit = FitMode::ShrinkToFit;
    let shrunk = r.measure_text(&tb, bounds);
    assert!(shrunk.fit_scale < 0.9, "scale {}", shrunk.fit_scale);
    assert!(
        shrunk.block_height <= 100.5,
        "fits: {}",
        shrunk.block_height
    );
    // never grows
    let mut short = text_box("Hi", 20.0);
    short.fit = FitMode::ShrinkToFit;
    assert_eq!(r.measure_text(&short, bounds).fit_scale, 1.0);
    // scale-to-fit grows short text
    short.fit = FitMode::ScaleToFit;
    assert!(r.measure_text(&short, bounds).fit_scale > 1.5);
    drop(r);

    // pixels: the shrunk text stays inside the box
    let f = render(&slide_with(vec![Element::text(bounds, tb.clone())]));
    let (x0, y0, x1, y1) = bbox(&f, |p| lum(p) > 100).unwrap();
    assert!(
        x0 >= 278 && x1 <= 682 && y0 >= 198 && y1 <= 302,
        "{x0},{y0} {x1},{y1}"
    );
    tb.fit = FitMode::None;
    let f = render(&slide_with(vec![Element::text(bounds, tb)]));
    let (_, y0, _, y1) = bbox(&f, |p| lum(p) > 100).unwrap();
    assert!(y0 < 190 || y1 > 310, "unshrunk text overflows the box");
}

#[test]
fn superscript_is_raised_and_smaller() {
    let mk = |baseline: Baseline| {
        let mut tb = text_box("", 120.0);
        tb.paragraphs = vec![Paragraph {
            runs: vec![TextRun::styled(
                "x",
                CharStyle {
                    baseline: Some(baseline),
                    ..CharStyle::default()
                },
            )],
            ..Paragraph::default()
        }];
        tb.vertical_align = VerticalAlign::Top;
        let f = render(&slide_with(vec![Element::text(
            Rect::new(0.0, 0.0, W as f64, H as f64),
            tb,
        )]));
        bbox(&f, |p| lum(p) > 128).unwrap()
    };
    let normal = mk(Baseline::Normal);
    let sup = mk(Baseline::Superscript);
    let sub = mk(Baseline::Subscript);
    assert!(
        sup.3 + 25 < normal.3,
        "superscript bottom {sup:?} vs {normal:?}"
    );
    assert!(
        (sup.3 - sup.1) < (normal.3 - normal.1),
        "superscript is smaller"
    );
    assert!(
        sub.3 > normal.3 + 5,
        "subscript lowered {sub:?} vs {normal:?}"
    );
}

#[test]
fn all_caps_is_applied() {
    let mk = |text: &str, cap: Option<Capitalization>| {
        let mut tb = text_box(text, 70.0);
        tb.default_style.capitalization = cap;
        render(&slide_with(vec![Element::text(
            Rect::new(0.0, 0.0, W as f64, H as f64),
            tb,
        )]))
    };
    let caps = mk("morning light", Some(Capitalization::AllCaps));
    let upper = mk("MORNING LIGHT", None);
    let lower = mk("morning light", None);
    let diff = |a: &RenderedFrame, b: &RenderedFrame| {
        a.data
            .chunks(4)
            .zip(b.data.chunks(4))
            .filter(|(x, y)| (x[0] as i32 - y[0] as i32).abs() > 8)
            .count()
    };
    assert!(diff(&caps, &upper) < 20, "ALL CAPS equals uppercase text");
    assert!(
        diff(&caps, &lower) > 1000,
        "ALL CAPS differs from lowercase"
    );
}

fn test_image() -> Arc<image::RgbaImage> {
    // 200×100: left half red, right half blue
    Arc::new(image::RgbaImage::from_fn(200, 100, |x, _| {
        if x < 100 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        }
    }))
}

fn image_fill(mode: ScaleMode) -> RenderedFrame {
    let asset = MediaAssetId::new();
    let img = test_image();
    let mut el = Element::media(Rect::new(100.0, 100.0, 300.0, 300.0), asset);
    if let FillKind::Media { display, .. } = &mut el.fill.kind {
        display.scale = mode;
    }
    let slide = slide_with(vec![el]);
    let resolver = move |id: MediaAssetId| {
        (id == asset).then(|| ImageSource::Rgba {
            key: 7,
            image: img.clone(),
        })
    };
    shared()
        .render_layers(
            ScreenConfig::opaque(500, 500),
            &[LayerContent::slide_with_media(
                &slide,
                Size::new(500.0, 500.0),
                &resolver,
            )],
        )
        .unwrap()
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

#[test]
fn image_fill_fit() {
    let f = image_fill(ScaleMode::Fit);
    assert!(close(f.pixel(250, 120), BLACK, 4), "letterbox above");
    assert!(close(f.pixel(130, 250), RED, 4));
    assert!(close(f.pixel(370, 250), BLUE, 4));
    assert!(close(f.pixel(250, 380), BLACK, 4), "letterbox below");
}

#[test]
fn image_fill_fill() {
    let f = image_fill(ScaleMode::Fill);
    assert!(close(f.pixel(120, 120), RED, 4), "covers the top");
    assert!(close(f.pixel(240, 250), RED, 4));
    assert!(close(f.pixel(260, 250), BLUE, 4));
    assert!(close(f.pixel(380, 380), BLUE, 4));
    assert!(close(f.pixel(90, 250), BLACK, 4), "clipped to the element");
}

#[test]
fn image_fill_stretch() {
    let f = image_fill(ScaleMode::Stretch);
    assert!(close(f.pixel(120, 120), RED, 4));
    assert!(close(f.pixel(380, 120), BLUE, 4));
    assert!(close(f.pixel(240, 390), RED, 4));
    assert!(close(f.pixel(260, 390), BLUE, 4));
}

#[test]
fn image_fill_none() {
    let f = image_fill(ScaleMode::None);
    assert!(close(f.pixel(120, 120), BLACK, 4));
    assert!(close(f.pixel(160, 250), RED, 4));
    assert!(close(f.pixel(340, 250), BLUE, 4));
    assert!(close(f.pixel(250, 190), BLACK, 4), "native height only");
}

#[test]
fn media_layer_anchor_and_order() {
    let img = test_image();
    let display = MediaDisplay {
        scale: ScaleMode::Fit,
        anchor: Anchor::Left,
        ..MediaDisplay::default()
    };
    let green = slide_with(vec![Element {
        fill: Fill::color(Color::rgb(0.0, 1.0, 0.0)),
        ..Element::new(Rect::new(0.0, 0.0, 100.0, 100.0))
    }]);
    // slide given first, media second: the media layer still ends up below the slide
    let f = shared()
        .render_layers(
            ScreenConfig::opaque(400, 100),
            &[
                LayerContent::slide(&green, Size::new(400.0, 100.0)),
                LayerContent::media(ImageSource::Rgba { key: 8, image: img }, display),
            ],
        )
        .unwrap();
    assert!(close(f.pixel(50, 50), [0, 255, 0, 255], 4), "slide on top");
    assert!(
        close(f.pixel(150, 50), BLUE, 4),
        "fit, anchored left (200 px wide)"
    );
    assert!(close(f.pixel(300, 50), BLACK, 4), "letterbox on the right");
}

#[test]
fn layers_compose_in_fixed_order() {
    let f = shared()
        .render_layers(
            ScreenConfig::opaque(64, 64),
            &[
                LayerContent::color(Layer::Messages, Color::rgba(0.0, 0.0, 1.0, 0.5)),
                LayerContent::color(Layer::Media, Color::rgb(1.0, 0.0, 0.0)),
                LayerContent::color(Layer::VideoInput, Color::rgb(0.0, 1.0, 0.0)),
                LayerContent::color(Layer::Audio, Color::WHITE),
            ],
        )
        .unwrap();
    // red media over green input, then 50 % blue message on top: linear 50/50 red+blue
    let p = f.pixel(32, 32);
    assert!(close(p, [188, 0, 188, 255], 3), "{p:?}");
}

#[test]
fn alpha_modes() {
    let slide = slide_with(vec![Element {
        fill: Fill::color(Color::rgba(1.0, 0.0, 0.0, 0.5)),
        ..Element::new(Rect::new(0.0, 0.0, 50.0, 100.0))
    }]);
    let layers = [LayerContent::slide(&slide, Size::new(100.0, 100.0))];
    let mut r = shared();

    let pre = r
        .render_layers(ScreenConfig::transparent(100, 100), &layers)
        .unwrap();
    assert_eq!(pre.alpha, AlphaMode::Premultiplied);
    assert!(
        close(pre.pixel(25, 50), [128, 0, 0, 128], 2),
        "{:?}",
        pre.pixel(25, 50)
    );
    assert_eq!(pre.pixel(75, 50), [0, 0, 0, 0], "transparent outside");

    let straight = r
        .render_layers(
            ScreenConfig {
                alpha: AlphaMode::Straight,
                ..ScreenConfig::transparent(100, 100)
            },
            &layers,
        )
        .unwrap();
    assert_eq!(straight.alpha, AlphaMode::Straight);
    assert_eq!(
        straight.pixel(25, 50),
        pre.pixel(25, 50),
        "frames are premultiplied"
    );
    let png = straight.to_straight_rgba();
    assert!(close(png.get_pixel(25, 50).0, [255, 0, 0, 128], 2));

    let opaque = r
        .render_layers(
            ScreenConfig {
                width: 100,
                height: 100,
                alpha: AlphaMode::Opaque,
                background: Color::rgb(0.0, 0.0, 1.0),
            },
            &layers,
        )
        .unwrap();
    assert!(
        close(opaque.pixel(25, 50), [188, 0, 188, 255], 3),
        "{:?}",
        opaque.pixel(25, 50)
    );
    assert!(
        close(opaque.pixel(75, 50), [0, 0, 255, 255], 1),
        "screen background"
    );

    // a transparent background on an opaque screen flattens over black
    let black = r
        .render_layers(ScreenConfig::opaque(100, 100), &layers)
        .unwrap();
    assert!(
        close(black.pixel(25, 50), [188, 0, 0, 255], 3),
        "{:?}",
        black.pixel(25, 50)
    );
    assert_eq!(black.pixel(75, 50), [0, 0, 0, 255]);
}

#[test]
fn shapes_fill_stroke_and_rounding() {
    let el = Element {
        kind: ElementKind::Shape {
            shape: Shape::RoundedRectangle { roundness: 0.25 },
        },
        fill: Fill::color(Color::rgb(0.0, 0.0, 1.0)),
        stroke: Stroke {
            enabled: true,
            width: 10.0,
            color: Color::rgb(1.0, 1.0, 0.0),
            ..Stroke::default()
        },
        ..Element::new(Rect::new(100.0, 100.0, 400.0, 200.0))
    };
    let f = render(&slide_with(vec![el.clone()]));
    assert!(close(f.pixel(300, 200), BLUE, 3), "inside");
    assert!(
        close(f.pixel(300, 100), [255, 255, 0, 255], 3),
        "stroke on the edge"
    );
    assert!(close(f.pixel(102, 102), BLACK, 3), "rounded corner is cut");
    assert!(close(f.pixel(300, 92), BLACK, 3), "outside the stroke");

    // a disabled stroke is not drawn
    let mut no_stroke = el;
    no_stroke.stroke.enabled = false;
    let f = render(&slide_with(vec![no_stroke]));
    assert!(close(f.pixel(300, 101), BLUE, 3), "{:?}", f.pixel(300, 101));
    assert!(close(f.pixel(300, 97), BLACK, 3));
}

#[test]
fn path_shapes_and_rotation() {
    let tri = Element {
        kind: ElementKind::Shape {
            shape: Shape::IsoscelesTriangle,
        },
        fill: Fill::color(Color::rgb(0.0, 1.0, 0.0)),
        ..Element::new(Rect::new(100.0, 100.0, 200.0, 200.0))
    };
    let f = render(&slide_with(vec![tri]));
    assert!(
        close(f.pixel(200, 250), [0, 255, 0, 255], 3),
        "triangle body"
    );
    assert!(close(f.pixel(110, 110), BLACK, 3), "outside the apex");

    // a 400×40 bar rotated 90° becomes a 40×400 bar about the same centre
    let bar = Element {
        rotation: 90.0,
        fill: Fill::color(Color::WHITE),
        ..Element::new(Rect::new(280.0, 250.0, 400.0, 40.0))
    };
    let f = render(&slide_with(vec![bar]));
    assert!(lum(f.pixel(480, 100)) > 240, "vertical bar top");
    assert!(lum(f.pixel(480, 440)) > 240, "vertical bar bottom");
    assert!(lum(f.pixel(300, 270)) < 10, "no longer horizontal");
}

/// Path shapes and dashed strokes larger than the GPU's largest texture (16384 px at most)
/// still draw, instead of failing the whole frame with a GPU validation error.
#[test]
fn path_shapes_larger_than_the_largest_texture_draw() {
    // A triangle 20000 px across with its apex at (480, 100): 200 px below the apex it is
    // 200 px wide.
    let tri = Element {
        kind: ElementKind::Shape {
            shape: Shape::IsoscelesTriangle,
        },
        fill: Fill::color(Color::rgb(0.0, 1.0, 0.0)),
        ..Element::new(Rect::new(-9520.0, 100.0, 20000.0, 20000.0))
    };
    let f = shared()
        .render_layers(
            ScreenConfig::opaque(W, H),
            &[LayerContent::slide(&slide_with(vec![tri]), canvas())],
        )
        .expect("a huge path shape renders");
    dump("huge-triangle", &f);
    assert!(close(f.pixel(480, 300), [0, 255, 0, 255], 3), "inside");
    assert!(close(f.pixel(300, 300), BLACK, 3), "beside the apex");
    assert!(close(f.pixel(480, 50), BLACK, 3), "above the apex");

    // A dashed stroke around a huge rectangle whose left edge crosses the screen.
    let rect = Element {
        stroke: Stroke {
            enabled: true,
            width: 40.0,
            color: Color::WHITE,
            style: model::StrokeStyle::SquareDash,
            ..Stroke::default()
        },
        ..Element::new(Rect::new(480.0, -10000.0, 20000.0, 20000.0))
    };
    let f = shared()
        .render_layers(
            ScreenConfig::opaque(W, H),
            &[LayerContent::slide(&slide_with(vec![rect]), canvas())],
        )
        .expect("a huge dashed stroke renders");
    dump("huge-dashed-rect", &f);
    let column: Vec<u32> = (0..H).map(|y| lum(f.pixel(480, y))).collect();
    assert!(
        column.iter().any(|&l| l > 200) && column.iter().any(|&l| l < 20),
        "dashes along the left edge"
    );
    assert!(close(f.pixel(200, 270), BLACK, 3), "left of the stroke");
    assert!(close(f.pixel(800, 270), BLACK, 3), "inside the rectangle");
}

#[test]
fn element_opacity_and_shadow() {
    let el = Element {
        fill: Fill::color(Color::WHITE),
        opacity: 0.5,
        shadow: Shadow {
            enabled: true,
            offset: 20.0,
            radius: 0.0,
            color: Color::rgb(1.0, 0.0, 0.0),
            opacity: 1.0,
            ..Shadow::default()
        },
        ..Element::new(Rect::new(100.0, 100.0, 200.0, 100.0))
    };
    let f = render(&slide_with(vec![el]));
    let p = f.pixel(150, 150);
    assert!(
        p[0] > 150 && p[1] > 150,
        "half white over half shadow: {p:?}"
    );
    let shadow = f.pixel(305, 205);
    assert!(
        shadow[0] > 100 && shadow[1] < 30,
        "shadow down-right: {shadow:?}"
    );
    assert!(close(f.pixel(95, 95), BLACK, 3));
}

/// A02-7: element opacity fades outlined text as one group: inside the glyphs, the
/// half-faded white text lies over the background, not over the half-faded black outline.
#[test]
fn element_opacity_fades_outlined_text_as_one_group() {
    let mut tb = text_box("I", 300.0);
    tb.outline = TextOutline {
        enabled: true,
        width: 8.0,
        color: Color::BLACK,
    };
    let el = Element {
        opacity: 0.5,
        ..Element::text(Rect::new(0.0, 0.0, W as f64, H as f64), tb)
    };
    let slide = slide_with(vec![el]);
    let f = shared()
        .render_layers(
            ScreenConfig::opaque(W, H),
            &[
                LayerContent::color(Layer::Media, Color::rgb(1.0, 0.0, 0.0)),
                LayerContent::slide(&slide, canvas()),
            ],
        )
        .expect("render");
    // The middle of the "I" stem: half white over red is (255, 188, 188); with the outline
    // showing through it would be much darker red (about 225, 188, 188).
    let p = f.pixel(W / 2, H / 2);
    assert!(p[0] >= 250 && (170..=205).contains(&p[1]), "{p:?}");
}

#[test]
fn odd_size_readback_is_unpadded() {
    let slide = slide_with(vec![Element {
        fill: Fill::color(Color::rgb(0.0, 1.0, 0.0)),
        ..Element::new(Rect::new(200.0, 0.0, 133.0, 77.0))
    }]);
    let f = shared()
        .render_layers(
            ScreenConfig::opaque(333, 77),
            &[LayerContent::slide(&slide, Size::new(333.0, 77.0))],
        )
        .unwrap();
    assert_eq!((f.width, f.height), (333, 77));
    assert_eq!(f.data.len(), 333 * 77 * 4);
    for y in [0, 38, 76] {
        assert!(close(f.pixel(300, y), [0, 255, 0, 255], 2), "row {y}");
        assert!(close(f.pixel(10, y), BLACK, 2), "row {y}");
    }
}

#[test]
fn async_ring_returns_frames_in_order() {
    let mut r = shared();
    let mut screen = r.create_screen(ScreenConfig::opaque(320, 180));
    let colors = [
        Color::rgb(1.0, 0.0, 0.0),
        Color::rgb(0.0, 1.0, 0.0),
        Color::rgb(0.0, 0.0, 1.0),
        Color::WHITE,
        Color::rgb(1.0, 1.0, 0.0),
    ];
    let mut seqs = Vec::new();
    for c in colors {
        seqs.push(
            r.submit(&mut screen, &[LayerContent::color(Layer::Media, c)])
                .unwrap(),
        );
    }
    assert_eq!(seqs, vec![0, 1, 2, 3, 4]);
    let mut got = Vec::new();
    while let Some(f) = r.wait(&mut screen).unwrap() {
        got.push((f.sequence, f.pixel(10, 10)));
        screen.recycle(f);
    }
    assert_eq!(got.len(), 5);
    for (i, (seq, px)) in got.iter().enumerate() {
        assert_eq!(*seq, i as u64);
        let c = colors[i];
        let want = [
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8,
            255,
        ];
        assert!(close(*px, want, 1), "frame {i}: {px:?}");
    }
    assert!(r.poll(&mut screen).unwrap().is_none());
}

#[test]
fn font_resolution_records_misses() {
    let mut r = Renderer::new(RendererOptions {
        font_substitutions: FontSubstitutionTable {
            rules: vec![FontSubstitution {
                family: Some("Calibri".into()),
                replacement_family: "DejaVu Serif".into(),
                ..FontSubstitution::default()
            }],
            ..FontSubstitutionTable::default()
        },
        ..test_options()
    })
    .unwrap();
    let el = |family: &str, ps: Option<&str>, weight: u16, y: f64| {
        let mut tb = text_box("Grace", 40.0);
        tb.default_style.font_family = Some(family.into());
        tb.default_style.font_weight = Some(weight);
        tb.default_style.postscript_name = ps.map(String::from);
        Element::text(Rect::new(0.0, y, 960.0, 100.0), tb)
    };
    let slide = slide_with(vec![
        el("Calibri", Some("Calibri-Bold"), 700, 0.0),
        el("No Such Font", None, 400, 100.0),
        el("Whatever", Some("DejaVuSans-Bold"), 700, 200.0),
        el("dejavu sans", None, 400, 300.0),
    ]);
    r.render_layers(
        ScreenConfig::opaque(W, H),
        &[LayerContent::slide(&slide, canvas())],
    )
    .unwrap();
    let report = r.take_report();
    let calibri = report
        .fonts
        .iter()
        .find(|i| i.requested.family == "Calibri")
        .expect("Calibri reported");
    assert_eq!(calibri.matched, FontMatch::Substituted);
    assert_eq!(calibri.used_family, "DejaVu Serif");
    let missing = report
        .fonts
        .iter()
        .find(|i| i.requested.family == "No Such Font")
        .expect("fallback reported");
    assert_eq!(missing.matched, FontMatch::Fallback);
    assert_eq!(missing.used_family, "DejaVu Sans");
    assert!(
        !report
            .fonts
            .iter()
            .any(|i| i.requested.family == "Whatever" || i.requested.family == "dejavu sans"),
        "exact PostScript and case-insensitive family matches are not issues: {report}"
    );
    assert!(r.take_report().fonts.is_empty(), "take_report resets");
}

#[test]
fn missing_media_is_reported_not_fatal() {
    let mut r = Renderer::new(test_options()).unwrap();
    let slide = slide_with(vec![Element::media(
        Rect::new(0.0, 0.0, 100.0, 100.0),
        MediaAssetId::new(),
    )]);
    let none = |_: MediaAssetId| None;
    let f = r
        .render_layers(
            ScreenConfig::opaque(100, 100),
            &[
                LayerContent::slide_with_media(&slide, Size::new(100.0, 100.0), &none),
                LayerContent::media(
                    ImageSource::path("/nonexistent/background.jpg"),
                    MediaDisplay::default(),
                ),
                LayerContent::media(
                    ImageSource::path("/nonexistent/loop.mp4"),
                    MediaDisplay::default(),
                )
                .on_layer(Layer::Announcements),
            ],
        )
        .unwrap();
    assert_eq!(f.pixel(50, 50), [0, 0, 0, 255]);
    let rep = r.take_report();
    assert_eq!(rep.media.len(), 3, "{rep}");
    assert!(rep.media.iter().any(|m| m.problem.contains("video")));
}

/// A02-5: a slide drawn while its image file was missing is not cached, so the image shows
/// once the file arrives (a media copy still running, a drive mounted late).
#[test]
fn a_slide_drawn_without_its_missing_image_draws_it_once_the_file_arrives() {
    let dir = tmp_dir("arrives");
    let png = dir.join("late.png");
    let _ = std::fs::remove_file(&png);
    let mut r = Renderer::new(test_options()).unwrap();
    let slide = slide_with(vec![Element::media(
        Rect::new(0.0, 0.0, 100.0, 100.0),
        MediaAssetId::new(),
    )]);
    let res = |_: MediaAssetId| Some(ImageSource::path(&png));
    let layers = [LayerContent::slide_with_media(
        &slide,
        Size::new(100.0, 100.0),
        &res,
    )];
    let f = r
        .render_layers(ScreenConfig::opaque(100, 100), &layers)
        .unwrap();
    assert_eq!(f.pixel(50, 50), [0, 0, 0, 255], "not there yet");
    // D03-5: meanwhile the slide is drawn once and kept (an animated screen renders every
    // tick), not laid out again on every render.
    for _ in 0..3 {
        r.render_layers(ScreenConfig::opaque(100, 100), &layers)
            .unwrap();
    }
    let uses: u64 = r.take_report().media.iter().map(|m| m.uses).sum();
    assert_eq!(uses, 1, "drawn once while the file is missing");
    image::RgbImage::from_pixel(100, 100, image::Rgb([0, 255, 0]))
        .save(&png)
        .unwrap();
    let f = r
        .render_layers(ScreenConfig::opaque(100, 100), &layers)
        .unwrap();
    assert_eq!(f.pixel(50, 50), [0, 255, 0, 255], "drawn once it arrived");
    let _ = std::fs::remove_dir_all(dir);
}

/// Rotated text is filtered between the glyph and its transparent surroundings in
/// premultiplied colour: white text over white stays white at its edges, with no dark fringe.
#[test]
fn rotated_text_edges_have_no_dark_fringe() {
    let text = Element {
        rotation: 10.0,
        ..Element::text(
            Rect::new(0.0, 0.0, W as f64, H as f64),
            text_box("Wo", 200.0),
        )
    };
    let slide = Slide {
        background: Some(Color::WHITE),
        ..slide_with(vec![text])
    };
    let f = render(&slide);
    dump("rotated-text-white-on-white", &f);
    let darkest = f.data.as_chunks::<4>().0.iter().map(|p| lum(*p)).min();
    assert!(darkest >= Some(250), "darkest pixel {darkest:?}");

    // For the eye (RENDER_PNG_DIR): the same text over blue shows the edge quality.
    let blue = Slide {
        background: Some(Color::rgb(0.1, 0.2, 0.6)),
        ..slide
    };
    dump("rotated-text-white-on-blue", &render(&blue));
}

/// Saves `f` as `<RENDER_PNG_DIR>/<name>.png` when that variable is set, to look at by eye.
fn dump(name: &str, f: &RenderedFrame) {
    if let Some(dir) = std::env::var_os("RENDER_PNG_DIR") {
        f.save_png(std::path::Path::new(&dir).join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn underline_is_drawn_below_the_baseline() {
    let mk = |underline: Option<model::LineStyle>| {
        let mut tb = text_box("nnnn", 100.0);
        tb.default_style.underline = underline;
        let f = render(&slide_with(vec![Element::text(
            Rect::new(0.0, 0.0, W as f64, H as f64),
            tb,
        )]));
        bbox(&f, |p| lum(p) > 128).unwrap()
    };
    let plain = mk(None);
    let under = mk(Some(model::LineStyle::Single));
    assert!(
        under.3 > plain.3 + 3,
        "underline below the glyphs: {under:?} vs {plain:?}"
    );
    assert!(
        under.3 < plain.3 + 30,
        "close to the baseline: {under:?} vs {plain:?}"
    );
}

#[test]
fn feather_softens_the_edge() {
    let mk = |feather: bool| {
        let el = Element {
            fill: Fill::color(Color::WHITE),
            feather: model::Feather {
                enabled: feather,
                radius: 0.5,
                ..model::Feather::default()
            },
            ..Element::new(Rect::new(100.0, 100.0, 400.0, 200.0))
        };
        render(&slide_with(vec![el]))
    };
    let hard = mk(false);
    let soft = mk(true);
    // 10 px inside the left edge: fully white without feather, partially with
    assert!(lum(hard.pixel(110, 200)) > 250);
    let v = lum(soft.pixel(110, 200));
    assert!(v > 5 && v < 200, "feathered edge: {v}");
    assert!(lum(soft.pixel(300, 200)) > 250, "centre stays solid");
}

#[test]
fn renderer_types_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Renderer>();
    assert_send::<render::ScreenTarget>();
    assert_send::<RenderedFrame>();
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("render-test-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn large_image_files_are_downscaled_on_decode_and_drawn() {
    let dir = tmp_dir("large");
    let png = dir.join("big.png");
    image::RgbImage::from_fn(3000, 2000, |x, _| {
        if x < 1500 {
            image::Rgb([255, 0, 0])
        } else {
            image::Rgb([0, 0, 255])
        }
    })
    .save(&png)
    .unwrap();
    let mut r = shared();
    let f = r
        .render_layers(
            ScreenConfig::opaque(300, 200),
            &[LayerContent::media(
                ImageSource::path(&png),
                MediaDisplay::default(),
            )],
        )
        .unwrap();
    assert_eq!(r.missing_images(), 0);
    assert_eq!(f.pixel(20, 100), [255, 0, 0, 255]);
    assert_eq!(f.pixel(280, 100), [0, 0, 255, 255]);
    // Decoded just covering the 300 × 200 box, not at 3000 × 2000.
    let stats = r.decoder().stats();
    assert!(stats.cached_bytes <= 300 * 200 * 4 * 2, "{stats:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn without_waiting_a_still_decoding_image_is_left_out_then_drawn() {
    let dir = tmp_dir("nowait");
    let png = dir.join("green.png");
    image::RgbImage::from_pixel(1200, 800, image::Rgb([0, 255, 0]))
        .save(&png)
        .unwrap();
    let mut r = Renderer::new(RendererOptions {
        wait_for_images: false,
        ..test_options()
    })
    .unwrap();
    let mut screen = r.create_screen(ScreenConfig::opaque(120, 80));
    let layers = [LayerContent::media(
        ImageSource::path(&png),
        MediaDisplay::default(),
    )];
    r.submit(&mut screen, &layers).unwrap();
    let f = r.wait(&mut screen).unwrap().unwrap();
    assert_eq!(r.missing_images(), 1, "queued, not decoded on this thread");
    assert_eq!(f.pixel(60, 40), [0, 0, 0, 255]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while r.request_images(&layers, (120, 80), render::DecodePriority::Live) > 0 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    r.submit(&mut screen, &layers).unwrap();
    let f = r.wait(&mut screen).unwrap().unwrap();
    assert_eq!(r.missing_images(), 0);
    assert_eq!(f.pixel(60, 40), [0, 255, 0, 255]);
    let _ = std::fs::remove_dir_all(dir);
}

// ---- test patterns ----------------------------------------------------------------------------

fn render_pattern(p: &model::TestPattern, frame: u64, alpha: AlphaMode) -> RenderedFrame {
    let f = render_pattern_inner(p, frame, alpha);
    if let Some(dir) = std::env::var_os("PATTERN_PNG_DIR") {
        let name = serde_json::to_value(p).unwrap()["type"]
            .as_str()
            .unwrap()
            .to_string();
        f.save_png(std::path::Path::new(&dir).join(format!("{name}-{frame}.png")))
            .unwrap();
    }
    f
}

fn render_pattern_inner(p: &model::TestPattern, frame: u64, alpha: AlphaMode) -> RenderedFrame {
    let info = render::patterns::PatternInfo {
        name: "Test".into(),
        fps: 25.0,
        frame,
    };
    let scene = render::patterns::test_pattern(p, (W, H), &info);
    let cfg = ScreenConfig {
        alpha,
        background: Color::TRANSPARENT,
        ..ScreenConfig::opaque(W, H)
    };
    shared()
        .render_layers(cfg, &scene.layers())
        .expect("render")
}

#[test]
fn colour_bars_are_75_percent() {
    let f = render_pattern(&model::TestPattern::ColorBars, 0, AlphaMode::Opaque);
    let bw = W as f64 / 7.0;
    for (i, c) in render::patterns::BARS_75.iter().enumerate() {
        let x = ((i as f64 + 0.5) * bw) as u32;
        let p = f.pixel(x, H / 3);
        assert!(
            close(p, [c[0], c[1], c[2], 255], 2),
            "bar {i}: {p:?} vs {c:?}"
        );
    }
    // 100 % white in the bottom row, second block
    let p = f.pixel((bw * 1.25 * 1.5) as u32, H - 20);
    assert!(close(p, [255, 255, 255, 255], 2), "{p:?}");
}

#[test]
fn grid_gradient_solid_and_alpha_checker() {
    let f = render_pattern(&model::TestPattern::Grid, 0, AlphaMode::Opaque);
    assert!(
        lum(f.pixel(W / 2, H / 2)) > 60,
        "centre cross {:?}",
        f.pixel(W / 2, H / 2)
    );
    assert!(lum(f.pixel(1, H / 2 + 7)) > 200, "white border");
    let f = render_pattern(&model::TestPattern::Gradient, 0, AlphaMode::Opaque);
    assert!(
        lum(f.pixel(4, 10)) < 20 && lum(f.pixel(W - 4, 10)) > 230,
        "white ramp"
    );
    assert!(f.pixel(W - 4, H * 3 / 8)[0] > 230, "red ramp");
    let f = render_pattern(
        &model::TestPattern::Solid {
            color: Color::from_rgb8(10, 200, 30),
        },
        0,
        AlphaMode::Opaque,
    );
    assert!(close(f.pixel(W / 2, H / 2), [10, 200, 30, 255], 2));
    // On a key screen the transparent squares really are transparent.
    let f = render_pattern(&model::TestPattern::AlphaChecker, 0, AlphaMode::Straight);
    let sq = W / 16;
    assert_eq!(f.pixel(sq / 2, sq / 2)[3], 255, "opaque square");
    assert_eq!(f.pixel(sq + sq / 2, sq / 2)[3], 0, "transparent square");
    let a = f.pixel(W / 8, H - 4)[3];
    assert!((50..=80).contains(&a), "25 % patch alpha {a}");
}

#[test]
fn motion_pattern_moves_every_frame() {
    let bar_x = |f: &RenderedFrame| {
        (0..W)
            .find(|&x| lum(f.pixel(x, H / 2)) > 240)
            .expect("bar drawn")
    };
    let a = render_pattern(&model::TestPattern::Motion, 3, AlphaMode::Opaque);
    let b = render_pattern(&model::TestPattern::Motion, 4, AlphaMode::Opaque);
    assert!(bar_x(&b) > bar_x(&a), "{} → {}", bar_x(&a), bar_x(&b));
    assert_ne!(a.data, b.data);
}

#[test]
fn identify_label_is_drawn_on_top() {
    let scene = render::patterns::identify_overlay(
        &["Main".to_string(), "Audience · IDN: Main".to_string()],
        (W, H),
    );
    let bg = LayerContent::color(Layer::Media, Color::rgb(0.0, 0.0, 1.0));
    let mut layers = vec![bg];
    layers.extend(scene.layers());
    let f = shared()
        .render_layers(ScreenConfig::opaque(W, H), &layers)
        .expect("render");
    assert!(close(f.pixel(3, 3), [0, 0, 255, 255], 2), "show underneath");
    let (x0, _, x1, _) = bbox(&f, |p| lum(p) > 200).expect("label text");
    assert!(x0 > W / 8 && x1 < W * 7 / 8, "label centred {x0}..{x1}");
}

/// Renderers made and dropped on other threads while one renders. Each used to create its own
/// Vulkan instance and device, which raced the renderer's GPU calls inside the Vulkan loader
/// and crashed now and then with SIGSEGV (see "One GPU device per process" in the crate docs).
#[test]
fn renderers_made_and_dropped_while_another_renders() {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let churn: Vec<_> = (0..2)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut made = 0;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) && made < 6 {
                    drop(Renderer::new(test_options()).unwrap());
                    made += 1;
                }
            })
        })
        .collect();
    let slide = slide_with(vec![Element::text(
        Rect::new(0.0, 0.0, W as f64, H as f64),
        text_box("Grace", 60.0),
    )]);
    let mut r = Renderer::new(test_options()).unwrap();
    let t0 = std::time::Instant::now();
    while !churn.iter().all(|h| h.is_finished()) && t0.elapsed().as_secs() < 60 {
        r.render_layers(
            ScreenConfig::opaque(160, 90),
            &[LayerContent::slide(&slide, canvas())],
        )
        .unwrap();
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for h in churn {
        h.join().unwrap();
    }
}

fn render_at(slide: &Slide, time: Option<f64>) -> RenderedFrame {
    shared()
        .render_layers(
            ScreenConfig::opaque(W, H),
            &[LayerContent {
                layer: Layer::Slide,
                source: render::LayerSource::Slide {
                    slide,
                    canvas: canvas(),
                    media: None,
                    time,
                    videos: Vec::new(),
                },
                opacity: 1.0,
            }],
        )
        .expect("render")
}

#[test]
fn scrolling_text_moves_on_the_clock_inside_its_box() {
    // A 600 × 100 ticker box at (100, 200) with a blue fill; the text enters from the right
    // at 100 px/s and loops 50 px apart.
    let mut e = Element::text(
        Rect::new(100.0, 200.0, 600.0, 100.0),
        text_box("TICKER", 60.0),
    );
    e.fill = Fill::enabled(FillKind::Color {
        color: Color::rgb(0.0, 0.0, 1.0),
    });
    e.scroll = model::TextScroll {
        enabled: true,
        direction: model::ScrollDirection::Left,
        speed: 100.0,
        start: model::ScrollStart::Offscreen,
        repeat: true,
        gap: 50.0,
    };
    let slide = slide_with(vec![e]);
    let white = |p: [u8; 4]| p[0] > 200 && p[1] > 200 && p[2] > 200;
    // Just live: no text yet; the box's fill stays put.
    let f = render_at(&slide, Some(0.0));
    assert!(bbox(&f, white).is_none());
    assert_eq!(f.pixel(400, 250)[..3], [0, 0, 255]);
    // 2 s: the strip's start is 200 px into the box from the right (its ink 24 px later).
    let f2 = render_at(&slide, Some(2.0));
    let (x0, y0, x1, y1) = bbox(&f2, white).expect("text after 2 s");
    assert!((x0 as i32 - (100 + 400 + 24)).abs() <= 8, "x0 {x0}");
    assert!(
        x1 < 700 && y0 >= 200 && y1 < 300,
        "clipped: {x0},{y0},{x1},{y1}"
    );
    // A second later it has moved 100 px left.
    let f3 = render_at(&slide, Some(3.0));
    let (x0b, ..) = bbox(&f3, white).unwrap();
    assert!((x0 as i32 - x0b as i32 - 100).abs() <= 2, "{x0} → {x0b}");
    // Much later the loop keeps the box filled, and nothing is drawn outside it.
    for t in [40.0, 41.3, 100.7] {
        let f = render_at(&slide, Some(t));
        let (x0, y0, x1, y1) = bbox(&f, white).expect("looping");
        assert!(x0 >= 100 && x1 < 700 && y0 >= 200 && y1 < 300, "t {t}");
    }
    // A still picture (thumbnail) shows the text where it is laid out: like a plain element.
    let still = render_at(&slide, None);
    let mut plain = slide.clone();
    plain.elements[0].scroll.enabled = false;
    let p = render(&plain);
    let (a, b) = (bbox(&still, white).unwrap(), bbox(&p, white).unwrap());
    assert!(
        (a.0 as i32 - b.0 as i32).abs() <= 2 && (a.2 as i32 - b.2 as i32).abs() <= 2,
        "{a:?} vs {b:?}"
    );
}

#[test]
fn scrolling_text_up_wraps_to_its_box() {
    let mut e = Element::text(
        Rect::new(200.0, 100.0, 300.0, 200.0),
        text_box("One two three four five six seven", 40.0),
    );
    e.scroll = model::TextScroll {
        enabled: true,
        direction: model::ScrollDirection::Up,
        speed: 50.0,
        start: model::ScrollStart::Offscreen,
        repeat: false,
        gap: 0.0,
    };
    let slide = slide_with(vec![e]);
    let white = |p: [u8; 4]| p[0] > 200 && p[1] > 200 && p[2] > 200;
    let f = render_at(&slide, Some(2.0));
    // 100 px up into the 200 px box: text only in its lower half, within its width.
    let (x0, y0, x1, y1) = bbox(&f, white).expect("text after 2 s");
    assert!(x0 >= 200 && x1 < 500, "{x0}..{x1}");
    assert!(y0 >= 200 - 2 + 24 - 8 && y1 < 300, "{y0}..{y1}");
    // Once through (no repeat), it is gone.
    assert!(bbox(&render_at(&slide, Some(60.0)), white).is_none());
}

/// D03-3: a strip longer than the largest texture is cut off there, not stretched over its
/// whole length.
#[test]
fn scrolling_text_past_the_largest_texture_is_cut_off() {
    // About 23 000 px of text in a box the width of the screen (1 screen px per canvas px).
    let mut e = Element::text(
        Rect::new(0.0, 200.0, W as f64, 100.0),
        text_box(&"W".repeat(400), 60.0),
    );
    e.scroll = model::TextScroll {
        enabled: true,
        direction: model::ScrollDirection::Left,
        speed: 1000.0,
        start: model::ScrollStart::Onscreen,
        repeat: false,
        gap: 0.0,
    };
    let slide = slide_with(vec![e]);
    let white = |p: [u8; 4]| p[0] > 200 && p[1] > 200 && p[2] > 200;
    // The strip starts 24 px before the box, moving 1000 px/s: 10 000 px in, text.
    assert!(bbox(&render_at(&slide, Some(10.0)), white).is_some());
    // 17 000 px in is past 16 384 px: nothing (stretched, the text would still be there).
    assert!(bbox(&render_at(&slide, Some(17.0)), white).is_none());
}

/// D03-4: a faded scrolling element whose text has an outline (rendered as one group) still
/// shows its text when it is centred, right-aligned or vertically centred.
#[test]
fn faded_outlined_scrolling_text_shows_whatever_its_alignment() {
    let light = |p: [u8; 4]| p[0] > 150 && p[1] > 150 && p[2] > 150;
    for (direction, align, valign) in [
        (
            model::ScrollDirection::Left,
            TextAlign::Center,
            VerticalAlign::Top,
        ),
        (
            model::ScrollDirection::Left,
            TextAlign::Right,
            VerticalAlign::Top,
        ),
        (
            model::ScrollDirection::Up,
            TextAlign::Center,
            VerticalAlign::Middle,
        ),
        (
            model::ScrollDirection::Up,
            TextAlign::Left,
            VerticalAlign::Bottom,
        ),
    ] {
        let mut tb = text_box("TICKER", 60.0);
        tb.outline = TextOutline {
            enabled: true,
            width: 4.0,
            color: Color::BLACK,
        };
        tb.vertical_align = valign;
        for p in tb.paragraphs.iter_mut() {
            p.style.alignment = Some(align);
        }
        let mut e = Element::text(Rect::new(100.0, 200.0, 600.0, 100.0), tb);
        e.opacity = 0.8;
        e.scroll = model::TextScroll {
            enabled: true,
            direction,
            speed: 100.0,
            start: model::ScrollStart::Offscreen,
            repeat: true,
            gap: 50.0,
        };
        let slide = slide_with(vec![e]);
        let t = if direction == model::ScrollDirection::Up {
            0.6
        } else {
            2.0
        };
        let (x0, y0, x1, y1) = bbox(&render_at(&slide, Some(t)), light)
            .unwrap_or_else(|| panic!("{direction:?} {align:?} {valign:?}: no text"));
        assert!(
            x0 >= 100 && x1 < 700 && y0 >= 200 && y1 < 300,
            "{direction:?}"
        );
    }
}

#[test]
fn double_stroke_draws_two_lines_with_a_gap() {
    for shape in [Shape::Rectangle, Shape::Ellipse] {
        let mut e = Element::new(Rect::new(100.0, 100.0, 400.0, 200.0));
        e.kind = ElementKind::Shape {
            shape: shape.clone(),
        };
        e.stroke = Stroke {
            enabled: true,
            width: 30.0,
            color: Color::rgb(1.0, 0.0, 0.0),
            style: model::StrokeStyle::Double,
            ..Stroke::default()
        };
        let f = render(&slide_with(vec![e.clone()]));
        // Left edge at x = 100, mid-height: lines centred at 90 and 110, 10 px wide each.
        let red = |x: u32| f.pixel(x, 200)[0];
        assert!(
            red(90) > 240 && red(110) > 240,
            "{shape:?}: {} {}",
            red(90),
            red(110)
        );
        assert!(red(100) < 10, "{shape:?}: gap {}", red(100));
        assert!(red(80) < 10 && red(120) < 10, "{shape:?}: outside");
        // A solid stroke of the same width covers the gap.
        e.stroke.style = model::StrokeStyle::Solid;
        let f = render(&slide_with(vec![e]));
        assert!(f.pixel(100, 200)[0] > 240, "{shape:?}: solid");
    }
}
