//! Layer transitions (dissolves in linear light, mid-transition retriggers; push, slide, wipe
//! and zoom probed at 0, 50 and 100 %) and video frames (YUV planes → RGB on the GPU). Any
//! adapter, including lavapipe.

use model::{Color, Direction, Layer, MediaDisplay, ScaleMode, TransitionKind, ZoomDirection};
use render::{
    LayerContent, LayerMix, LayerSource, MixInput, RenderedFrame, Renderer, RendererOptions,
    ScreenConfig, VideoFrameRef, VideoMatrix, VideoPlanes,
};
use std::sync::{Mutex, MutexGuard, OnceLock};

fn shared() -> MutexGuard<'static, Renderer> {
    static R: OnceLock<Mutex<Renderer>> = OnceLock::new();
    R.get_or_init(|| {
        let r = Renderer::new(RendererOptions {
            system_fonts: false,
            ..RendererOptions::default()
        })
        .expect("renderer");
        Mutex::new(r)
    })
    .lock()
    .unwrap_or_else(|e| e.into_inner())
}

const W: u32 = 64;
const H: u32 = 36;

fn srgb_encode(lin: f64) -> f64 {
    if lin <= 0.0031308 {
        lin * 12.92 * 255.0
    } else {
        (1.055 * lin.powf(1.0 / 2.4) - 0.055) * 255.0
    }
}

fn color(r: f32, g: f32, b: f32) -> LayerSource<'static> {
    LayerSource::Color(Color::rgb(r, g, b))
}

fn mix<'a>(from: MixInput<'a>, to: MixInput<'a>, progress: f32, store: u64) -> LayerContent<'a> {
    LayerContent::mix(Layer::Slide, LayerMix::dissolve(from, to, progress, store))
}

fn moving<'a>(
    kind: TransitionKind,
    from: MixInput<'a>,
    to: MixInput<'a>,
    progress: f32,
    store: u64,
) -> LayerContent<'a> {
    LayerContent::mix(
        Layer::Slide,
        LayerMix {
            kind,
            ..LayerMix::dissolve(from, to, progress, store)
        },
    )
}

/// Left half red, right half green (on transparent: a slide whose halves differ).
fn halves() -> LayerSource<'static> {
    let half = |x: u32, c: Color| {
        (
            model::Rect {
                x: x as f64,
                y: 0.0,
                width: (W / 2) as f64,
                height: H as f64,
            },
            c,
        )
    };
    LayerSource::Rects(vec![
        half(0, Color::rgb(1.0, 0.0, 0.0)),
        half(W / 2, Color::rgb(0.0, 1.0, 0.0)),
    ])
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const BLACK: [u8; 3] = [0, 0, 0];

/// Colours at x = 1/4 and 3/4 of the width, mid-height.
fn quarters(f: &RenderedFrame) -> ([u8; 3], [u8; 3]) {
    let q = |x: u32| {
        let p = f.pixel(x, H / 2);
        [p[0], p[1], p[2]]
    };
    (q(W / 4), q(3 * W / 4))
}

#[test]
fn push_moves_both_sides() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    let kind = TransitionKind::Push {
        direction: Direction::Left,
    };
    let at = |r: &mut Renderer, p: f32| {
        quarters(&render(
            r,
            cfg,
            &[moving(
                kind.clone(),
                MixInput::Content(halves(), 1.0),
                MixInput::Content(color(0.0, 0.0, 1.0), 1.0),
                p,
                30,
            )],
        ))
    };
    assert_eq!(at(&mut r, 0.0), (RED, GREEN));
    // Half way: the old right half is now on the left, the new content on the right.
    assert_eq!(at(&mut r, 0.5), (GREEN, BLUE));
    assert_eq!(at(&mut r, 1.0), (BLUE, BLUE));
}

#[test]
fn slide_covers_the_old_content_which_stays() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    let kind = TransitionKind::Slide {
        direction: Direction::Left,
    };
    // The incoming side is a transparent slide with a blue left half: what it doesn't draw
    // shows nothing of the old one under it.
    let blue_left = LayerSource::Rects(vec![(
        model::Rect {
            x: 0.0,
            y: 0.0,
            width: (W / 2) as f64,
            height: H as f64,
        },
        Color::rgb(0.0, 0.0, 1.0),
    )]);
    let at = |r: &mut Renderer, p: f32| {
        quarters(&render(
            r,
            cfg,
            &[moving(
                kind.clone(),
                MixInput::Content(halves(), 1.0),
                MixInput::Content(blue_left.clone(), 1.0),
                p,
                31,
            )],
        ))
    };
    assert_eq!(at(&mut r, 0.0), (RED, GREEN));
    // Half way: the old left half stays; the new one's blue left half is on the right.
    assert_eq!(at(&mut r, 0.5), (RED, BLUE));
    assert_eq!(at(&mut r, 1.0), (BLUE, BLACK));
}

#[test]
fn wipe_reveals_without_showing_through() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    let at = |r: &mut Renderer, d: Direction, p: f32, to: MixInput<'static>| {
        quarters(&render(
            r,
            cfg,
            &[moving(
                TransitionKind::Wipe { direction: d },
                MixInput::Content(halves(), 1.0),
                to,
                p,
                32,
            )],
        ))
    };
    let blue = || MixInput::Content(color(0.0, 0.0, 1.0), 1.0);
    assert_eq!(at(&mut r, Direction::Right, 0.0, blue()), (RED, GREEN));
    // An edge moving right has revealed the left half.
    assert_eq!(at(&mut r, Direction::Right, 0.5, blue()), (BLUE, GREEN));
    assert_eq!(at(&mut r, Direction::Left, 0.5, blue()), (RED, BLUE));
    assert_eq!(at(&mut r, Direction::Right, 1.0, blue()), (BLUE, BLUE));
    // Wiping to a cleared layer: the revealed part is empty, not the old content.
    assert_eq!(
        at(&mut r, Direction::Right, 0.5, MixInput::Empty),
        (BLACK, GREEN)
    );
}

#[test]
fn zoom_scales_and_cross_fades() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    let frame_at = |r: &mut Renderer, d: ZoomDirection, p: f32| {
        render(
            r,
            cfg,
            &[moving(
                TransitionKind::Zoom { direction: d },
                MixInput::Content(color(1.0, 1.0, 1.0), 1.0),
                MixInput::Content(color(0.0, 0.0, 1.0), 1.0),
                p,
                33,
            )],
        )
    };
    let f = frame_at(&mut r, ZoomDirection::In, 0.0);
    assert_eq!(f.pixel(1, 1)[..3], [255, 255, 255]);
    // Half way in: the new content (at 75 %) covers the centre, not the corner.
    let f = frame_at(&mut r, ZoomDirection::In, 0.5);
    let corner = f.pixel(1, 1);
    let centre = f.pixel(W / 2, H / 2);
    let half = srgb_encode(0.5);
    assert!(
        (corner[0] as f64 - half).abs() <= 1.5 && (corner[2] as f64 - half).abs() <= 1.5,
        "{corner:?}"
    );
    assert!(
        (centre[0] as f64 - half).abs() <= 1.5 && centre[2] == 255,
        "{centre:?}"
    );
    let f = frame_at(&mut r, ZoomDirection::In, 1.0);
    assert_eq!(f.pixel(1, 1)[..3], BLUE);
    // Zooming out, the new content starts larger than the screen and the old one shrinks
    // away: the corner shows only the new one, half faded in.
    let f = frame_at(&mut r, ZoomDirection::Out, 0.5);
    let corner = f.pixel(1, 1);
    assert!(
        corner[0] <= 1 && (corner[2] as f64 - half).abs() <= 1.5,
        "{corner:?}"
    );
}

#[test]
fn moving_transition_retriggered_starts_from_the_blend() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    let push = TransitionKind::Push {
        direction: Direction::Up,
    };
    let before = render(
        &mut r,
        cfg,
        &[moving(
            push.clone(),
            MixInput::Content(halves(), 1.0),
            MixInput::Content(color(0.0, 0.0, 1.0), 1.0),
            0.4,
            40,
        )],
    );
    let after = render(
        &mut r,
        cfg,
        &[moving(
            TransitionKind::Wipe {
                direction: Direction::Down,
            },
            MixInput::Snapshot(40),
            MixInput::Content(color(1.0, 1.0, 1.0), 1.0),
            0.0,
            41,
        )],
    );
    for (a, b) in before.data.iter().zip(&after.data) {
        assert!((*a as i32 - *b as i32).abs() <= 1, "pop: {a} vs {b}");
    }
}

fn render(r: &mut Renderer, cfg: ScreenConfig, layers: &[LayerContent<'_>]) -> RenderedFrame {
    r.render_layers(cfg, layers).expect("render")
}

#[test]
fn dissolve_midpoint_is_half_in_linear_light() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    // White → black over black: at 50 % the layer is 0.5 linear = sRGB 188, not 128.
    for (p, want_lin) in [
        (0.0f32, 1.0f64),
        (0.25, 0.75),
        (0.5, 0.5),
        (0.75, 0.25),
        (1.0, 0.0),
    ] {
        let f = render(
            &mut r,
            cfg,
            &[mix(
                MixInput::Content(color(1.0, 1.0, 1.0), 1.0),
                MixInput::Content(color(0.0, 0.0, 0.0), 1.0),
                p,
                1,
            )],
        );
        let want = srgb_encode(want_lin);
        let got = f.pixel(W / 2, H / 2)[0] as f64;
        assert!(
            (got - want).abs() <= 1.5,
            "progress {p}: {got} vs {want:.1}"
        );
    }
    // Red → blue: each channel follows its own linear ramp.
    let f = render(
        &mut r,
        cfg,
        &[mix(
            MixInput::Content(color(1.0, 0.0, 0.0), 1.0),
            MixInput::Content(color(0.0, 0.0, 1.0), 1.0),
            0.5,
            2,
        )],
    );
    let p = f.pixel(1, 1);
    assert!(
        (p[0] as f64 - 188.0).abs() <= 1.5 && (p[2] as f64 - 188.0).abs() <= 1.5,
        "{p:?}"
    );
    assert!(p[1] <= 1);
}

#[test]
fn dissolve_to_and_from_clear_keeps_what_is_below() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    let below = LayerContent::color(Layer::Media, Color::rgb(0.0, 0.0, 1.0));
    // Slide layer (white) dissolving out over a blue background: 25 % in.
    let f = render(
        &mut r,
        cfg,
        &[
            below.clone(),
            mix(
                MixInput::Content(color(1.0, 1.0, 1.0), 1.0),
                MixInput::Empty,
                0.25,
                3,
            ),
        ],
    );
    let p = f.pixel(3, 3);
    // 0.75 white over blue: r = g = 0.75, b = 1.
    assert!((p[0] as f64 - srgb_encode(0.75)).abs() <= 1.5, "{p:?}");
    assert_eq!(p[2], 255);
    // And in from clear.
    let f = render(
        &mut r,
        cfg,
        &[
            below,
            mix(
                MixInput::Empty,
                MixInput::Content(color(1.0, 1.0, 1.0), 1.0),
                0.25,
                4,
            ),
        ],
    );
    let p = f.pixel(3, 3);
    assert!((p[0] as f64 - srgb_encode(0.25)).abs() <= 1.5, "{p:?}");
}

#[test]
fn retrigger_mid_transition_has_no_pop() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    // A → B is 60 % done when C is triggered: the new transition starts from the blend.
    let before = render(
        &mut r,
        cfg,
        &[mix(
            MixInput::Content(color(1.0, 0.0, 0.0), 1.0),
            MixInput::Content(color(0.0, 1.0, 0.0), 1.0),
            0.6,
            10,
        )],
    );
    let after = render(
        &mut r,
        cfg,
        &[mix(
            MixInput::Snapshot(10),
            MixInput::Content(color(0.0, 0.0, 1.0), 1.0),
            0.0,
            11,
        )],
    );
    for (a, b) in before.data.iter().zip(&after.data) {
        assert!((*a as i32 - *b as i32).abs() <= 1, "pop: {a} vs {b}");
    }
    // ... and it continues towards C from there.
    let later = render(
        &mut r,
        cfg,
        &[mix(
            MixInput::Snapshot(10),
            MixInput::Content(color(0.0, 0.0, 1.0), 1.0),
            0.5,
            11,
        )],
    );
    let p = later.pixel(1, 1);
    assert!((p[0] as f64 - srgb_encode(0.4 * 0.5)).abs() <= 1.5, "{p:?}");
    assert!((p[1] as f64 - srgb_encode(0.6 * 0.5)).abs() <= 1.5, "{p:?}");
    assert!((p[2] as f64 - srgb_encode(0.5)).abs() <= 1.5, "{p:?}");
}

/// A synthetic NV12 frame: left half Y=`a`, right half Y=`b`, neutral chroma.
fn nv12(w: u32, h: u32, a: u8, b: u8) -> (Vec<u8>, Vec<u8>) {
    let mut y = vec![0u8; (w * h) as usize];
    for row in 0..h {
        for x in 0..w {
            y[(row * w + x) as usize] = if x < w / 2 { a } else { b };
        }
    }
    let uv = vec![128u8; (w.div_ceil(2) * 2 * h.div_ceil(2)) as usize];
    (y, uv)
}

fn frame<'a>(
    stream: u64,
    seq: u64,
    w: u32,
    h: u32,
    y: &'a [u8],
    uv: &'a [u8],
) -> VideoFrameRef<'a> {
    VideoFrameRef {
        stream,
        seq,
        width: w,
        height: h,
        planes: VideoPlanes::Nv12,
        y,
        y_stride: w as usize,
        uv,
        uv_stride: w.div_ceil(2) as usize * 2,
        matrix: VideoMatrix::Bt709,
        full_range: false,
        rotation: 0,
    }
}

#[test]
fn video_frame_converts_and_scales() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    // 128×72 source scaled to the 64×36 screen: limited-range black and white halves.
    let (y, uv) = nv12(128, 72, 16, 235);
    let display = MediaDisplay {
        scale: ScaleMode::Fill,
        ..Default::default()
    };
    let f = render(
        &mut r,
        cfg,
        &[LayerContent::video(
            frame(77, 1, 128, 72, &y, &uv),
            display.clone(),
        )],
    );
    assert_eq!(f.pixel(4, 18)[..3], [0, 0, 0]);
    assert_eq!(f.pixel(W - 4, 18)[..3], [255, 255, 255]);
    // A new frame of the same stream replaces the picture; the same seq is not re-uploaded.
    let (y2, uv2) = nv12(128, 72, 126, 126);
    let f = render(
        &mut r,
        cfg,
        &[LayerContent::video(
            frame(77, 2, 128, 72, &y2, &uv2),
            display.clone(),
        )],
    );
    let mid = f.pixel(4, 18)[0];
    // (126 − 16) / 219 = 0.502 R'G'B' → passes through as code 128.
    assert!((mid as i32 - 128).abs() <= 1, "{mid}");
    let f = render(
        &mut r,
        cfg,
        &[LayerContent::video(frame(77, 2, 128, 72, &y, &uv), display)],
    );
    assert!(
        (f.pixel(4, 18)[0] as i32 - 128).abs() <= 1,
        "same seq must not re-upload"
    );
}

#[test]
fn video_frame_rotation_turns_the_picture_upright() {
    let mut r = shared();
    // A portrait screen; the 128×72 picture (black left, white right) stored sideways.
    let cfg = ScreenConfig::opaque(36, 64);
    let (y, uv) = nv12(128, 72, 16, 235);
    let display = MediaDisplay {
        scale: ScaleMode::Stretch,
        ..Default::default()
    };
    for (rotation, top, bottom) in [(90u16, 255u8, 0u8), (270, 0, 255)] {
        let mut fr = frame(78 + rotation as u64, 1, 128, 72, &y, &uv);
        fr.rotation = rotation;
        let f = render(&mut r, cfg, &[LayerContent::video(fr, display.clone())]);
        // Counter-clockwise 90: the right half goes to the top; 270: to the bottom.
        assert_eq!(f.pixel(18, 4)[..3], [top; 3], "rotation {rotation}");
        assert_eq!(f.pixel(18, 60)[..3], [bottom; 3], "rotation {rotation}");
    }
}

#[test]
fn video_element_plays_inside_its_box() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    // A 32 × 18 element at (16, 9) of a 64 × 36 slide with a blue background; its media fill
    // stretches the frame (black left half, white right half) over its box.
    let mut e = model::Element::media(
        model::Rect::new(16.0, 9.0, 32.0, 18.0),
        model::MediaAssetId::new(),
    );
    if let model::FillKind::Media { display, .. } = &mut e.fill.kind {
        display.scale = ScaleMode::Stretch;
    }
    let slide = model::Slide {
        background: Some(Color::rgb(0.0, 0.0, 1.0)),
        elements: vec![e.clone()],
        ..model::Slide::new()
    };
    let (y, uv) = nv12(128, 72, 16, 235);
    let source = |videos| LayerSource::Slide {
        slide: &slide,
        canvas: model::Size::new(W as f64, H as f64),
        media: None,
        time: Some(1.0),
        videos,
    };
    let f = render(
        &mut r,
        cfg,
        &[LayerContent {
            layer: Layer::Slide,
            source: source(vec![(e.id, frame(90, 1, 128, 72, &y, &uv))]),
            opacity: 1.0,
        }],
    );
    assert_eq!(f.pixel(20, 18)[..3], [0, 0, 0], "left half of the frame");
    assert_eq!(
        f.pixel(44, 18)[..3],
        [255, 255, 255],
        "right half of the frame"
    );
    assert_eq!(f.pixel(8, 18)[..3], [0, 0, 255], "outside the box");
    assert_eq!(f.pixel(32, 4)[..3], [0, 0, 255], "above the box");
    // Without a frame the element's fill draws nothing.
    let f = render(
        &mut r,
        cfg,
        &[LayerContent {
            layer: Layer::Slide,
            source: source(Vec::new()),
            opacity: 1.0,
        }],
    );
    assert_eq!(f.pixel(20, 18)[..3], [0, 0, 255]);
}

/// D03-2: a slide with a video element (several overlapping draws) dissolves in, or is drawn
/// at a layer opacity, as one picture: the frame covers the background, nothing adds up.
#[test]
fn a_slide_with_moving_parts_dissolves_as_one_picture() {
    let mut r = shared();
    let cfg = ScreenConfig::opaque(W, H);
    let mut e = model::Element::media(
        model::Rect::new(16.0, 9.0, 32.0, 18.0),
        model::MediaAssetId::new(),
    );
    if let model::FillKind::Media { display, .. } = &mut e.fill.kind {
        display.scale = ScaleMode::Stretch;
    }
    let slide = model::Slide {
        background: Some(Color::rgb(0.0, 0.0, 1.0)),
        elements: vec![e.clone()],
        ..model::Slide::new()
    };
    let (y, uv) = nv12(128, 72, 16, 235);
    let source = || LayerSource::Slide {
        slide: &slide,
        canvas: model::Size::new(W as f64, H as f64),
        media: None,
        time: Some(1.0),
        videos: vec![(e.id, frame(90, 1, 128, 72, &y, &uv))],
    };
    let half = srgb_encode(0.5).round() as i32;
    let near = |p: [u8; 4], want: [i32; 3], what: &str| {
        for (c, w) in p[..3].iter().zip(want) {
            assert!((*c as i32 - w).abs() <= 2, "{what}: {p:?}, want {want:?}");
        }
    };
    // Halfway through a dissolve from nothing: half the slide over black.
    let f = render(
        &mut r,
        cfg,
        &[mix(
            MixInput::Empty,
            MixInput::Content(source(), 1.0),
            0.5,
            7,
        )],
    );
    near(f.pixel(44, 18), [half, half, half], "white frame half in");
    near(f.pixel(8, 18), [0, 0, half], "background half in");
    // The same at a layer opacity of one half.
    let f = render(
        &mut r,
        cfg,
        &[LayerContent {
            layer: Layer::Slide,
            source: source(),
            opacity: 0.5,
        }],
    );
    near(
        f.pixel(44, 18),
        [half, half, half],
        "white frame at half opacity",
    );
    near(f.pixel(20, 18), [0, 0, 0], "black frame at half opacity");
}
