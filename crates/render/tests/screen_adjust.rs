//! Screen adjustments (colour correction, corner pin) on the output pass: identity settings
//! leave frames bit for bit unchanged, the colour maths matches a CPU reference, corner-pinned
//! corners land where asked, and alpha survives. The frames are also written as PNGs to
//! `target/tmp/screen-adjust/` to look at.

use model::{
    Color, ColorCorrection, CornerOffset, CornerPin, Layer, MediaDisplay, ScaleMode, Size,
};
use render::{ImageSource, LayerContent, RenderedFrame, Renderer, RendererOptions, ScreenConfig};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

fn shared() -> MutexGuard<'static, Renderer> {
    static R: OnceLock<Mutex<Renderer>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Renderer::new(RendererOptions::default()).expect("renderer")))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

const W: u32 = 640;
const H: u32 = 360;

fn save(f: &RenderedFrame, name: &str) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("screen-adjust");
    let _ = std::fs::create_dir_all(&dir);
    let _ = f.save_png(dir.join(format!("{name}.png")));
}

/// A test card: a grey field with a horizontal ramp in the middle band, and a 24 px square
/// of its own colour in each corner (red top left, green top right, blue bottom right,
/// yellow bottom left).
fn card() -> Arc<image::RgbaImage> {
    Arc::new(image::RgbaImage::from_fn(W, H, |x, y| {
        let s = 24;
        let (l, t, r, b) = (x < s, y < s, x >= W - s, y >= H - s);
        let px = match (l, t, r, b) {
            (true, true, _, _) => [255, 0, 0],
            (_, true, true, _) => [0, 255, 0],
            (_, _, true, true) => [0, 0, 255],
            (true, _, _, true) => [255, 255, 0],
            _ if (H / 3..2 * H / 3).contains(&y) => {
                let v = (x * 255 / (W - 1)) as u8;
                [v, v, v]
            }
            _ => [96, 128, 160],
        };
        image::Rgba([px[0], px[1], px[2], 255])
    }))
}

fn card_layer(img: &Arc<image::RgbaImage>) -> LayerContent<'static> {
    LayerContent::media(
        ImageSource::Rgba {
            key: 0xC0DE,
            image: img.clone(),
        },
        MediaDisplay {
            scale: ScaleMode::Stretch,
            ..MediaDisplay::default()
        },
    )
}

/// Renders `layers` on a new screen target with the given adjustments.
fn render_with(
    config: ScreenConfig,
    layers: &[LayerContent<'_>],
    color: &ColorCorrection,
    pin: &CornerPin,
) -> RenderedFrame {
    let mut r = shared();
    let mut t = r.create_screen(config);
    t.set_adjustments(color, pin);
    r.submit(&mut t, layers).expect("submit");
    r.wait(&mut t).expect("wait").expect("frame")
}

fn pin(tl: (f32, f32), tr: (f32, f32), br: (f32, f32), bl: (f32, f32)) -> CornerPin {
    let o = |(x, y)| CornerOffset { x, y };
    CornerPin {
        enabled: true,
        top_left: o(tl),
        top_right: o(tr),
        bottom_right: o(br),
        bottom_left: o(bl),
    }
}

#[test]
fn identity_settings_are_bit_identical() {
    let img = card();
    let layers = [card_layer(&img)];
    let cfg = ScreenConfig::opaque(W, H);
    let plain = shared().render_layers(cfg, &layers).unwrap();
    // A disabled pin with offsets and an identity colour: the plain output pass.
    let mut off = pin((40.0, 10.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0));
    off.enabled = false;
    let same = render_with(cfg, &layers, &ColorCorrection::IDENTITY, &off);
    assert_eq!(plain.data, same.data, "identity adjustments change nothing");

    // Changing the settings and back on one target, too.
    let mut r = shared();
    let mut t = r.create_screen(cfg);
    let warm = ColorCorrection {
        red: 1.2,
        ..ColorCorrection::IDENTITY
    };
    assert!(t.set_adjustments(&warm, &CornerPin::default()));
    r.submit(&mut t, &layers).unwrap();
    let warmed = r.wait(&mut t).unwrap().unwrap();
    assert_ne!(warmed.data, plain.data);
    assert!(t.set_adjustments(&ColorCorrection::IDENTITY, &CornerPin::default()));
    assert!(!t.set_adjustments(&ColorCorrection::IDENTITY, &CornerPin::default()));
    r.submit(&mut t, &layers).unwrap();
    let back = r.wait(&mut t).unwrap().unwrap();
    assert_eq!(back.data, plain.data);

    // A transparent screen as well.
    let tcfg = ScreenConfig::transparent(W, H);
    let half = [LayerContent::color(
        Layer::Slide,
        Color::rgba(0.2, 0.6, 0.9, 0.5),
    )];
    let plain = r.render_layers(tcfg, &half).unwrap();
    drop(r);
    let same = render_with(tcfg, &half, &ColorCorrection::IDENTITY, &off);
    assert_eq!(plain.data, same.data);
}

/// The shader's colour maths on one straight sRGB-encoded colour (0–1).
fn reference(c: [f32; 3], k: &ColorCorrection) -> [f32; 3] {
    let g = [c[0] * k.red, c[1] * k.green, c[2] * k.blue];
    let luma = 0.2126 * g[0] + 0.7152 * g[1] + 0.0722 * g[2];
    g.map(|v| {
        let v = luma + (v - luma) * k.saturation;
        let v = (v - 0.5) * k.contrast + 0.5 + k.brightness;
        let v = k.black_level + v * (1.0 - k.black_level);
        v.clamp(0.0, 1.0).powf(1.0 / k.gamma)
    })
}

#[test]
fn color_correction_matches_the_reference() {
    let base = [0.25f32, 0.5, 0.75];
    let layers = [LayerContent::color(
        Layer::Media,
        Color::rgb(base[0], base[1], base[2]),
    )];
    let cases = [
        (
            "brightness",
            ColorCorrection {
                brightness: 0.1,
                ..ColorCorrection::IDENTITY
            },
        ),
        (
            "contrast",
            ColorCorrection {
                contrast: 1.5,
                ..ColorCorrection::IDENTITY
            },
        ),
        (
            "gamma",
            ColorCorrection {
                gamma: 2.2,
                ..ColorCorrection::IDENTITY
            },
        ),
        (
            "saturation",
            ColorCorrection {
                saturation: 0.0,
                ..ColorCorrection::IDENTITY
            },
        ),
        (
            "gains",
            ColorCorrection {
                red: 1.5,
                green: 0.5,
                blue: 1.1,
                ..ColorCorrection::IDENTITY
            },
        ),
        (
            "black",
            ColorCorrection {
                black_level: 0.2,
                ..ColorCorrection::IDENTITY
            },
        ),
        (
            "all",
            ColorCorrection {
                brightness: -0.05,
                contrast: 1.2,
                gamma: 0.8,
                saturation: 1.4,
                red: 0.9,
                green: 1.05,
                blue: 1.2,
                black_level: 0.05,
            },
        ),
    ];
    for (name, k) in cases {
        let f = render_with(
            ScreenConfig::opaque(64, 64),
            &layers,
            &k,
            &CornerPin::default(),
        );
        let want = reference(base, &k).map(|v| (v * 255.0).round() as i32);
        let got = f.pixel(32, 32);
        for ch in 0..3 {
            assert!(
                (got[ch] as i32 - want[ch]).abs() <= 2,
                "{name}: channel {ch}: got {got:?}, want {want:?}"
            );
        }
        assert_eq!(got[3], 255, "{name}: opaque stays opaque");
    }
    let img = card();
    let k = ColorCorrection {
        gamma: 1.6,
        saturation: 0.3,
        red: 1.2,
        ..ColorCorrection::IDENTITY
    };
    save(
        &render_with(
            ScreenConfig::opaque(W, H),
            &[card_layer(&img)],
            &k,
            &CornerPin::default(),
        ),
        "color",
    );
}

/// The forward homography of the test (unit square → quad), for expectations.
fn quad_point(q: [[f32; 2]; 4], u: f64, v: f64) -> (f64, f64) {
    // Bilinear is exact at the corners and close enough near them for these probes.
    let lerp = |a: [f32; 2], b: [f32; 2], t: f64| {
        (
            a[0] as f64 + (b[0] - a[0]) as f64 * t,
            a[1] as f64 + (b[1] - a[1]) as f64 * t,
        )
    };
    let top = lerp(q[0], q[1], u);
    let bot = lerp(q[3], q[2], u);
    (top.0 + (bot.0 - top.0) * v, top.1 + (bot.1 - top.1) * v)
}

fn centroid(f: &RenderedFrame, want: [u8; 3]) -> Option<(f64, f64)> {
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0usize);
    for y in 0..f.height {
        for x in 0..f.width {
            let p = f.pixel(x, y);
            if (0..3).all(|c| (p[c] as i32 - want[c] as i32).abs() < 40) && p[3] > 200 {
                sx += x as f64 + 0.5;
                sy += y as f64 + 0.5;
                n += 1;
            }
        }
    }
    (n > 20).then(|| (sx / n as f64, sy / n as f64))
}

#[test]
fn corner_pin_puts_the_corners_where_asked() {
    let img = card();
    let layers = [card_layer(&img)];
    // Keystone: the top pulled in, the bottom right pushed out a little, bottom left up.
    let p = pin((-80.0, -30.0), (-60.0, -10.0), (0.0, 0.0), (-20.0, -40.0));
    let quad = p.quad(W as f32, H as f32);
    for (name, cfg) in [
        ("pin", ScreenConfig::opaque(W, H)),
        ("pin-alpha", ScreenConfig::transparent(W, H)),
    ] {
        let f = render_with(cfg, &layers, &ColorCorrection::IDENTITY, &p);
        save(&f, name);
        // Each corner square's centre (12 px in from its corner) lands near the corner of the
        // quad, 12 px of the picture inwards.
        let s = 12.0 / W as f64;
        let t = 12.0 / H as f64;
        for (colour, (u, v)) in [
            ([255, 0, 0], (s, t)),
            ([0, 255, 0], (1.0 - s, t)),
            ([0, 0, 255], (1.0 - s, 1.0 - t)),
            ([255, 255, 0], (s, 1.0 - t)),
        ] {
            let want = quad_point(quad, u, v);
            let got = centroid(&f, colour).unwrap_or_else(|| panic!("{name}: {colour:?} missing"));
            assert!(
                (got.0 - want.0).abs() < 4.0 && (got.1 - want.1).abs() < 4.0,
                "{name}: {colour:?} at {got:?}, want {want:?}"
            );
        }
        // Outside the quad: black (opaque) or transparent.
        let outside = f.pixel(20, 10);
        if name == "pin" {
            assert_eq!(outside, [0, 0, 0, 255]);
        } else {
            assert_eq!(outside, [0, 0, 0, 0]);
        }
        // Inside: opaque picture.
        assert_eq!(f.pixel(W / 2, H / 2)[3], 255);
    }
}

#[test]
fn alpha_survives_the_adjustments() {
    let layers = [LayerContent::color(
        Layer::Slide,
        Color::rgba(1.0, 1.0, 1.0, 0.5),
    )];
    let k = ColorCorrection {
        brightness: -0.2,
        gamma: 1.5,
        ..ColorCorrection::IDENTITY
    };
    let p = pin(
        (-50.0, -50.0),
        (-50.0, -50.0),
        (-50.0, -50.0),
        (-50.0, -50.0),
    );
    let f = render_with(ScreenConfig::transparent(W, H), &layers, &k, &p);
    let inside = f.pixel(W / 2, H / 2);
    assert!(
        (inside[3] as i32 - 128).abs() <= 1,
        "alpha kept: {inside:?}"
    );
    // Premultiplied: colour ≤ alpha, and corrected (darker than the uncorrected white).
    assert!(inside[0] <= inside[3] && inside[0] < 120, "{inside:?}");
    assert_eq!(f.pixel(10, 10), [0, 0, 0, 0], "outside the quad");
    // Fully transparent areas stay transparent whatever the brightness.
    let lift = ColorCorrection {
        brightness: 0.5,
        black_level: 0.3,
        ..ColorCorrection::IDENTITY
    };
    let none: [LayerContent<'_>; 0] = [];
    let f = render_with(
        ScreenConfig::transparent(64, 64),
        &none,
        &lift,
        &CornerPin::default(),
    );
    assert_eq!(f.pixel(32, 32), [0, 0, 0, 0]);
    let _ = Size::HD;
}
