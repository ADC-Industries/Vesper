//! Colour correctness end to end: SMPTE colour bars (limited range, BT.709 HD and BT.601 SD)
//! encoded losslessly, decoded by a player, converted YUV → RGB on the GPU by the renderer
//! and read back. The 75 % bars must come out at code 191 / 0, and every bar centre must
//! match the reference conversion of its decoded YCbCr values.

mod common;

use common::*;
use media::{Matrix, PlaneFormat, Player, PlayerOptions, VideoFrame};
use render::{
    LayerContent, Renderer, RendererOptions, ScreenConfig, VideoFrameRef, VideoMatrix, VideoPlanes,
};
use std::time::Duration;

fn frame_ref(f: &VideoFrame) -> VideoFrameRef<'_> {
    let (y, ys) = f.plane(0);
    let (uv, uvs) = f.plane(1);
    VideoFrameRef {
        stream: 1,
        seq: f.seq,
        width: f.width,
        height: f.height,
        planes: match f.format {
            PlaneFormat::Nv12 => VideoPlanes::Nv12,
            PlaneFormat::P010 => VideoPlanes::P010,
        },
        y,
        y_stride: ys,
        uv,
        uv_stride: uvs,
        matrix: match f.matrix {
            Matrix::Bt601 => VideoMatrix::Bt601,
            Matrix::Bt709 => VideoMatrix::Bt709,
            Matrix::Bt2020 => VideoMatrix::Bt2020,
        },
        full_range: f.full_range,
        rotation: f.rotation,
    }
}

/// Reference: limited-range 8-bit YCbCr → R'G'B' code values.
fn reference(m: Matrix, y: f64, cb: f64, cr: f64) -> [f64; 3] {
    let (kr, kb) = m.kr_kb();
    let kg = 1.0 - kr - kb;
    let yn = (y - 16.0) / 219.0;
    let (pb, pr) = ((cb - 128.0) / 224.0, (cr - 128.0) / 224.0);
    let r = yn + 2.0 * (1.0 - kr) * pr;
    let b = yn + 2.0 * (1.0 - kb) * pb;
    let g = (yn - kr * r - kb * b) / kg;
    [r, g, b].map(|c| c.clamp(0.0, 1.0) * 255.0)
}

fn chroma(f: &VideoFrame, x: u32, y: u32) -> (f64, f64) {
    let (p, s) = f.plane(1);
    let i = (y / 2) as usize * s + (x / 2) as usize * 2;
    (p[i] as f64, p[i + 1] as f64)
}

fn check_bars(source: &str, tag: &str, size: &str, want_matrix: Matrix) {
    let dir = TempDir::new("bars");
    let path = bars_video(dir.path(), "bars.mp4", tag, size);
    let p = Player::open(&path, PlayerOptions::default()).unwrap();
    let f = p
        .wait_frame_for(0.0, Duration::from_secs(20))
        .expect("frame");
    assert_eq!(f.matrix, want_matrix, "{source}: {f:?}");
    assert!(!f.full_range);
    assert_eq!(f.format, PlaneFormat::Nv12);
    let (w, h) = (f.width, f.height);
    let mut r = Renderer::new(RendererOptions {
        system_fonts: false,
        ..Default::default()
    })
    .unwrap();
    let out = r
        .render_layers(
            ScreenConfig::opaque(w, h),
            &[LayerContent::video(frame_ref(&f), Default::default())],
        )
        .unwrap();
    // Bar centres along a row in the upper part: where luma and chroma are flat around x.
    let row = h / 4;
    let mut checked = 0;
    let mut found = Vec::new();
    let mut wrong_matrix_misses = 0;
    let other = if want_matrix == Matrix::Bt709 {
        Matrix::Bt601
    } else {
        Matrix::Bt709
    };
    for x in (8..w - 8).step_by(2) {
        let flat = (x - 6..=x + 6).all(|xx| {
            f.luma(xx, row) == f.luma(x, row) && chroma(&f, xx, row) == chroma(&f, x, row)
        });
        if !flat {
            continue;
        }
        let (cb, cr) = chroma(&f, x, row);
        let want = reference(want_matrix, f.luma(x, row) as f64, cb, cr);
        let px = out.pixel(x, row);
        for c in 0..3 {
            assert!(
                (px[c] as f64 - want[c]).abs() <= 2.0,
                "{source} x={x}: got {px:?}, want {want:?}"
            );
        }
        let wrong = reference(other, f.luma(x, row) as f64, cb, cr);
        if (0..3).any(|c| (px[c] as f64 - wrong[c]).abs() > 4.0) {
            wrong_matrix_misses += 1;
        }
        found.push([px[0], px[1], px[2]]);
        checked += 1;
    }
    assert!(checked > 50, "{source}: only {checked} flat samples");
    // The 75 % bars: white, yellow, cyan, green, magenta, red, blue.
    for bar in [
        [191, 191, 191],
        [191, 191, 0],
        [0, 191, 191],
        [0, 191, 0],
        [191, 0, 191],
        [191, 0, 0],
        [0, 0, 191],
    ] {
        assert!(
            found
                .iter()
                .any(|p| (0..3).all(|c| (p[c] as i32 - bar[c]).abs() <= 3)),
            "{source}: bar {bar:?} not found in {found:?}"
        );
    }
    // The matrix matters: decoding with the other one would miss most coloured bars.
    assert!(
        wrong_matrix_misses > checked / 3,
        "{source}: test not sensitive to the matrix"
    );
}

#[test]
fn smpte_bars_bt709_limited() {
    check_bars("smptebars (BT.709)", "bt709", "1280x720", Matrix::Bt709);
}

#[test]
fn smpte_bars_bt601_limited() {
    check_bars("smptebars (BT.601)", "smpte170m", "720x480", Matrix::Bt601);
}
