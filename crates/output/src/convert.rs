//! Pixel conversion from the render contract's format (RGBA8, sRGB-encoded, premultiplied
//! alpha, tightly packed) to what transports want.
//!
//! - Every routine writes into a caller-owned buffer, so sinks reuse buffers across frames.
//! - Loops walk source/destination as fixed-size pixel arrays (`as_chunks`) with
//!   integer fixed-point math, which keeps bounds checks out of the hot loop and lets the
//!   compiler vectorise the simple cases (channel swaps, alpha drop, UYVY).
//! - Un-premultiplying uses a 256-entry reciprocal table instead of a division per channel.
//!
//! ## YCbCr
//! [`PixelFormat::Uyvy`] / [`PixelFormat::Uyva`] use **BT.709 limited range** (Y 16–235,
//! Cb/Cr 16–240), the convention for HD video and for network video. The sRGB-encoded values
//! are treated as the gamma-encoded R'G'B' input (the usual practice; sRGB and BT.709 differ
//! only slightly in their transfer curve, which is not re-encoded here). Chroma is 4:2:2:
//! each horizontal pixel pair shares one Cb/Cr sample computed from the pair's average.
//! For UYVA the average is alpha-weighted, so a transparent neighbour does not pull an edge
//! pixel's chroma toward black.

use anyhow::{Result, ensure};

/// Output pixel formats supported by [`convert`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// RGBA8, straight alpha.
    Rgba,
    /// RGBA8 flattened over black, alpha byte set to 255 (ffmpeg `rgb0`-compatible).
    Rgbx,
    /// RGB8 flattened over black (3 bytes per pixel).
    Rgb,
    /// BGRA8, straight alpha.
    Bgra,
    /// BGRA8 flattened over black, alpha byte set to 255.
    Bgrx,
    /// BGRA8, premultiplied alpha (channel swap only).
    BgraPremultiplied,
    /// UYVY 4:2:2, BT.709 limited range, flattened over black. Needs an even width.
    Uyvy,
    /// UYVY 4:2:2 (straight colour) followed by a full-range 8-bit alpha plane
    /// (`width` bytes per row), the `UYVA` layout. Needs an even width.
    Uyva,
}

impl PixelFormat {
    /// Bytes a tightly packed frame of this format needs.
    pub fn buffer_len(self, width: u32, height: u32) -> usize {
        let px = width as usize * height as usize;
        match self {
            PixelFormat::Rgba
            | PixelFormat::Rgbx
            | PixelFormat::Bgra
            | PixelFormat::Bgrx
            | PixelFormat::BgraPremultiplied => px * 4,
            PixelFormat::Rgb => px * 3,
            PixelFormat::Uyvy => px * 2,
            PixelFormat::Uyva => px * 3,
        }
    }

    /// Bytes per row of the (first) plane.
    pub fn line_stride(self, width: u32) -> usize {
        let w = width as usize;
        match self {
            PixelFormat::Rgb => w * 3,
            PixelFormat::Uyvy | PixelFormat::Uyva => w * 2,
            _ => w * 4,
        }
    }

    /// True for the 4:2:2 formats, which need an even width.
    pub fn needs_even_width(self) -> bool {
        matches!(self, PixelFormat::Uyvy | PixelFormat::Uyva)
    }

    /// True when the format carries alpha.
    pub fn has_alpha(self) -> bool {
        matches!(
            self,
            PixelFormat::Rgba
                | PixelFormat::Bgra
                | PixelFormat::BgraPremultiplied
                | PixelFormat::Uyva
        )
    }
}

/// Converts a premultiplied RGBA8 frame into `format`, resizing `dst` as needed (it keeps its
/// capacity across calls, so a reused `Vec` does not reallocate).
pub fn convert(
    src: &[u8],
    width: u32,
    height: u32,
    format: PixelFormat,
    dst: &mut Vec<u8>,
) -> Result<()> {
    let px = width as usize * height as usize;
    ensure!(
        src.len() == px * 4,
        "source is {} bytes but {width}x{height} RGBA needs {}",
        src.len(),
        px * 4
    );
    ensure!(
        !format.needs_even_width() || width.is_multiple_of(2),
        "{format:?} needs an even width, got {width}"
    );
    dst.resize(format.buffer_len(width, height), 0);
    match format {
        PixelFormat::Rgba => premul_rgba_to_straight_rgba(src, dst),
        PixelFormat::Rgbx => premul_rgba_to_rgbx(src, dst),
        PixelFormat::Rgb => premul_rgba_to_rgb(src, dst),
        PixelFormat::Bgra => premul_rgba_to_straight_bgra(src, dst),
        PixelFormat::Bgrx => premul_rgba_to_bgrx(src, dst),
        PixelFormat::BgraPremultiplied => premul_rgba_to_premul_bgra(src, dst),
        PixelFormat::Uyvy => premul_rgba_to_uyvy(src, dst),
        PixelFormat::Uyva => {
            let (uyvy, alpha) = dst.split_at_mut(px * 2);
            premul_rgba_to_uyva(src, uyvy, alpha)
        }
    }
    Ok(())
}

// ---- tables and coefficients --------------------------------------------------------------

/// `UNPREMUL[a] = round(255 * 2^16 / a)`; `straight = (c * UNPREMUL[a] + 2^15) >> 16`.
/// Exact for `a == 255` (the table entry is exactly `2^16`).
static UNPREMUL: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut a = 1;
    while a < 256 {
        t[a] = ((255u32 << 16) + a as u32 / 2) / a as u32;
        a += 1;
    }
    t
};

/// Same as [`UNPREMUL`] for a pair's alpha sum `0..=510`, mapping a pair's colour sum to the
/// alpha-weighted average straight colour.
static UNPREMUL_PAIR: [u32; 511] = {
    let mut t = [0u32; 511];
    let mut s = 1;
    while s < 511 {
        t[s] = ((255u32 << 16) + s as u32 / 2) / s as u32;
        s += 1;
    }
    t
};

#[inline(always)]
fn unpremul(c: u8, a: u8, recip: u32) -> u8 {
    // Clamp invalid premultiplied input (c > a) so the result stays in range.
    let c = c.min(a) as u32;
    ((c * recip + 0x8000) >> 16) as u8
}

const fn fx(x: f64) -> i32 {
    (x * 65536.0 + if x >= 0.0 { 0.5 } else { -0.5 }) as i32
}

// BT.709 luma coefficients.
const KR: f64 = 0.2126;
const KB: f64 = 0.0722;
// Limited-range scale factors for 8-bit input.
const YS: f64 = 219.0 / 255.0;
const CS: f64 = 224.0 / 255.0;

// 16.16 fixed point. The middle coefficient of each row is derived so the row sums exactly
// (white maps to 235 and neutral greys to Cb = Cr = 128 without rounding drift).
const KYR: i32 = fx(YS * KR);
const KYB: i32 = fx(YS * KB);
const KYG: i32 = fx(YS) - KYR - KYB;
const KUR: i32 = fx(-CS * KR / (2.0 * (1.0 - KB)));
const KUB: i32 = fx(CS * 0.5);
const KUG: i32 = -KUR - KUB;
const KVR: i32 = fx(CS * 0.5);
const KVB: i32 = fx(-CS * KB / (2.0 * (1.0 - KR)));
const KVG: i32 = -KVR - KVB;

const Y_OFFSET: i32 = (16 << 16) + (1 << 15);
const C_OFFSET: i32 = (128 << 16) + (1 << 15);

#[inline(always)]
fn luma(r: i32, g: i32, b: i32) -> u8 {
    ((KYR * r + KYG * g + KYB * b + Y_OFFSET) >> 16) as u8
}

#[inline(always)]
fn chroma(r: i32, g: i32, b: i32) -> (u8, u8) {
    let u = (KUR * r + KUG * g + KUB * b + C_OFFSET) >> 16;
    let v = (KVR * r + KVG * g + KVB * b + C_OFFSET) >> 16;
    (u as u8, v as u8)
}

/// Chroma of a pixel pair from the pair's channel sums (`0..=510`); halves with rounding.
#[inline(always)]
fn chroma_pair_sum(rs: i32, gs: i32, bs: i32) -> (u8, u8) {
    let u = (KUR * rs + KUG * gs + KUB * bs + (C_OFFSET << 1)) >> 17;
    let v = (KVR * rs + KVG * gs + KVB * bs + (C_OFFSET << 1)) >> 17;
    (u as u8, v as u8)
}

fn check_len(src: &[u8], dst: &[u8], dst_bpp_num: usize, dst_bpp_den: usize) {
    assert_eq!(src.len() % 4, 0, "source length must be a multiple of 4");
    assert_eq!(
        dst.len(),
        src.len() / 4 * dst_bpp_num / dst_bpp_den,
        "destination buffer has the wrong length"
    );
}

/// Un-premultiplies one pixel, optionally swapping R and B. Opaque and fully transparent
/// pixels (the common cases) skip the table lookups.
#[inline(always)]
fn straight_px<const BGR: bool>(s: &[u8; 4]) -> [u8; 4] {
    let a = s[3];
    let [r, g, b] = if a == 255 {
        [s[0], s[1], s[2]]
    } else if a == 0 {
        return [0; 4];
    } else {
        let t = UNPREMUL[a as usize];
        [
            unpremul(s[0], a, t),
            unpremul(s[1], a, t),
            unpremul(s[2], a, t),
        ]
    };
    if BGR { [b, g, r, a] } else { [r, g, b, a] }
}

// ---- RGB(A) / BGR(A) ------------------------------------------------------------------------

/// Premultiplied RGBA → straight RGBA. `dst.len() == src.len()`.
pub fn premul_rgba_to_straight_rgba(src: &[u8], dst: &mut [u8]) {
    check_len(src, dst, 4, 1);
    for (s, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0.iter_mut())
    {
        *d = straight_px::<false>(s);
    }
}

/// Premultiplied RGBA → straight BGRA. `dst.len() == src.len()`.
pub fn premul_rgba_to_straight_bgra(src: &[u8], dst: &mut [u8]) {
    check_len(src, dst, 4, 1);
    for (s, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0.iter_mut())
    {
        *d = straight_px::<true>(s);
    }
}

/// Premultiplied RGBA → premultiplied BGRA (R/B swap). `dst.len() == src.len()`.
pub fn premul_rgba_to_premul_bgra(src: &[u8], dst: &mut [u8]) {
    check_len(src, dst, 4, 1);
    for (s, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0.iter_mut())
    {
        let x = u32::from_le_bytes(*s);
        let y = (x & 0xFF00_FF00) | ((x & 0xFF) << 16) | ((x >> 16) & 0xFF);
        *d = y.to_le_bytes();
    }
}

/// Premultiplied RGBA → RGBX flattened over black (alpha byte 255). `dst.len() == src.len()`.
pub fn premul_rgba_to_rgbx(src: &[u8], dst: &mut [u8]) {
    check_len(src, dst, 4, 1);
    for (s, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0.iter_mut())
    {
        let x = u32::from_le_bytes(*s);
        *d = (x | 0xFF00_0000).to_le_bytes();
    }
}

/// Premultiplied RGBA → the **fill** of an unshaped Fill/Key pair: straight (un-premultiplied)
/// colour, black where alpha is 0, alpha byte 255. `dst.len() == src.len()`. (The shaped fill
/// is [`premul_rgba_to_rgbx`]: premultiplied colour over black.)
pub fn premul_rgba_to_straight_rgbx(src: &[u8], dst: &mut [u8]) {
    check_len(src, dst, 4, 1);
    for (s, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0.iter_mut())
    {
        let [r, g, b, _] = straight_px::<false>(s);
        *d = [r, g, b, 255];
    }
}

/// Premultiplied RGBA → the **key** of a Fill/Key pair: alpha as grey (`a, a, a`), alpha byte
/// 255. `dst.len() == src.len()`.
pub fn premul_rgba_to_key(src: &[u8], dst: &mut [u8]) {
    check_len(src, dst, 4, 1);
    for (s, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0.iter_mut())
    {
        let a = s[3];
        *d = [a, a, a, 255];
    }
}

/// Premultiplied RGBA → BGRX flattened over black (alpha byte 255). `dst.len() == src.len()`.
pub fn premul_rgba_to_bgrx(src: &[u8], dst: &mut [u8]) {
    check_len(src, dst, 4, 1);
    for (s, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0.iter_mut())
    {
        let x = u32::from_le_bytes(*s);
        let y = (x & 0x0000_FF00) | ((x & 0xFF) << 16) | ((x >> 16) & 0xFF) | 0xFF00_0000;
        *d = y.to_le_bytes();
    }
}

/// Premultiplied RGBA → RGB flattened over black. `dst.len() == src.len() / 4 * 3`.
pub fn premul_rgba_to_rgb(src: &[u8], dst: &mut [u8]) {
    check_len(src, dst, 3, 1);
    for (s, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<3>().0.iter_mut())
    {
        d[0] = s[0];
        d[1] = s[1];
        d[2] = s[2];
    }
}

// ---- YCbCr 4:2:2 ----------------------------------------------------------------------------

/// Premultiplied RGBA → UYVY 4:2:2 BT.709 limited range, flattened over black.
///
/// Rows must have an even number of pixels (pairs never straddle rows when the width is
/// even). `dst.len() == src.len() / 2`.
pub fn premul_rgba_to_uyvy(src: &[u8], dst: &mut [u8]) {
    check_len(src, dst, 2, 1);
    assert_eq!(src.len() % 8, 0, "UYVY needs pixel pairs");
    for (s, d) in src
        .as_chunks::<8>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0.iter_mut())
    {
        *d = uyvy_pair(s);
    }
}

/// One UYVY macropixel from two pixels' colour bytes (alpha ignored).
#[inline(always)]
fn uyvy_pair(s: &[u8; 8]) -> [u8; 4] {
    let (r0, g0, b0) = (s[0] as i32, s[1] as i32, s[2] as i32);
    let (r1, g1, b1) = (s[4] as i32, s[5] as i32, s[6] as i32);
    let (u, v) = chroma_pair_sum(r0 + r1, g0 + g1, b0 + b1);
    [u, luma(r0, g0, b0), v, luma(r1, g1, b1)]
}

/// Premultiplied RGBA → UYVY (straight colour, BT.709 limited range) plus a separate
/// full-range alpha plane: the `UYVA` layout when `alpha` directly follows `uyvy` in memory.
///
/// `uyvy.len() == src.len() / 2`, `alpha.len() == src.len() / 4`.
pub fn premul_rgba_to_uyva(src: &[u8], uyvy: &mut [u8], alpha: &mut [u8]) {
    check_len(src, uyvy, 2, 1);
    check_len(src, alpha, 1, 1);
    assert_eq!(src.len() % 8, 0, "UYVA needs pixel pairs");
    for ((s, d), a) in src
        .as_chunks::<8>()
        .0
        .iter()
        .zip(uyvy.as_chunks_mut::<4>().0.iter_mut())
        .zip(alpha.as_chunks_mut::<2>().0.iter_mut())
    {
        let (a0, a1) = (s[3], s[7]);
        *a = [a0, a1];
        // Fast paths for the common cases: fully opaque or fully transparent pairs.
        if a0 & a1 == 255 {
            *d = uyvy_pair(s);
            continue;
        }
        if a0 | a1 == 0 {
            *d = [128, 16, 128, 16];
            continue;
        }
        let (t0, t1) = (UNPREMUL[a0 as usize], UNPREMUL[a1 as usize]);
        let y0 = luma(
            unpremul(s[0], a0, t0) as i32,
            unpremul(s[1], a0, t0) as i32,
            unpremul(s[2], a0, t0) as i32,
        );
        let y1 = luma(
            unpremul(s[4], a1, t1) as i32,
            unpremul(s[5], a1, t1) as i32,
            unpremul(s[6], a1, t1) as i32,
        );
        // Alpha-weighted average straight colour of the pair.
        let asum = a0 as u32 + a1 as u32;
        let tp = UNPREMUL_PAIR[asum as usize];
        let avg = |c0: u8, c1: u8| -> i32 {
            let cs = c0.min(a0) as u32 + c1.min(a1) as u32;
            ((cs * tp + 0x8000) >> 16) as i32
        };
        let (u, v) = chroma(avg(s[0], s[4]), avg(s[1], s[5]), avg(s[2], s[6]));
        d[0] = u;
        d[1] = y0;
        d[2] = v;
        d[3] = y1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_and_key_of_a_half_transparent_picture() {
        // Orange at half alpha (premultiplied 128, 64, 0, 128), opaque blue, transparent.
        let src = [128u8, 64, 0, 128, 0, 0, 255, 255, 0, 0, 0, 0];
        let mut fill = [0u8; 12];
        premul_rgba_to_straight_rgbx(&src, &mut fill);
        assert_eq!(fill, [255, 128, 0, 255, 0, 0, 255, 255, 0, 0, 0, 255]);
        let mut shaped = [0u8; 12];
        premul_rgba_to_rgbx(&src, &mut shaped);
        assert_eq!(shaped, [128, 64, 0, 255, 0, 0, 255, 255, 0, 0, 0, 255]);
        let mut key = [0u8; 12];
        premul_rgba_to_key(&src, &mut key);
        assert_eq!(key, [128, 128, 128, 255, 255, 255, 255, 255, 0, 0, 0, 255]);
    }
    use std::time::Instant;

    fn px(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
        [r, g, b, a]
    }

    fn premul(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
        let p = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        [p(r), p(g), p(b), a]
    }

    fn close(a: u8, b: u8, tol: u8) -> bool {
        a.abs_diff(b) <= tol
    }

    #[test]
    fn coefficient_rows_sum_exactly() {
        assert_eq!(KYR + KYG + KYB, fx(YS));
        const KG: f64 = 1.0 - KR - KB;
        assert!((KYG - fx(YS * KG)).abs() <= 1);
        assert_eq!(KUR + KUG + KUB, 0);
        assert_eq!(KVR + KVG + KVB, 0);
    }

    #[test]
    fn unpremultiply_is_exact_for_opaque_and_round_trips() {
        for c in 0..=255u8 {
            assert_eq!(unpremul(c, 255, UNPREMUL[255]), c);
        }
        // Premultiply then un-premultiply loses at most the quantisation of the alpha.
        for a in 1..=255u8 {
            for c in (0..=255u8).step_by(5) {
                let p = premul(c, 0, 0, a)[0];
                let back = unpremul(p, a, UNPREMUL[a as usize]);
                let tol = (255 / a as u32 / 2 + 1) as u8;
                assert!(close(back, c, tol), "a={a} c={c} p={p} back={back}");
            }
        }
        // Transparent pixels become black, invalid c > a is clamped.
        assert_eq!(unpremul(0, 0, UNPREMUL[0]), 0);
        assert_eq!(unpremul(200, 100, UNPREMUL[100]), 255);
    }

    #[test]
    fn rgba_and_bgra_variants() {
        let src: Vec<u8> = [
            premul(255, 128, 0, 128),
            px(10, 20, 30, 255),
            px(0, 0, 0, 0),
            premul(40, 80, 160, 64),
        ]
        .concat();
        let mut dst = Vec::new();

        convert(&src, 2, 2, PixelFormat::BgraPremultiplied, &mut dst).unwrap();
        assert_eq!(&dst[0..4], &[src[2], src[1], src[0], src[3]]);
        assert_eq!(&dst[4..8], &[30, 20, 10, 255]);

        convert(&src, 2, 2, PixelFormat::Bgrx, &mut dst).unwrap();
        assert_eq!(&dst[4..8], &[30, 20, 10, 255]);
        assert_eq!(&dst[8..12], &[0, 0, 0, 255]);
        assert_eq!(&dst[0..4], &[src[2], src[1], src[0], 255]);

        convert(&src, 2, 2, PixelFormat::Rgbx, &mut dst).unwrap();
        assert_eq!(&dst[0..4], &[src[0], src[1], src[2], 255]);

        convert(&src, 2, 2, PixelFormat::Rgb, &mut dst).unwrap();
        assert_eq!(dst.len(), 12);
        assert_eq!(&dst[3..6], &[10, 20, 30]);

        convert(&src, 2, 2, PixelFormat::Bgra, &mut dst).unwrap();
        assert!(close(dst[2], 255, 1) && close(dst[1], 128, 1) && dst[0] == 0 && dst[3] == 128);
        assert_eq!(&dst[4..8], &[30, 20, 10, 255]);
        assert_eq!(&dst[8..12], &[0, 0, 0, 0]);
        assert!(close(dst[14], 40, 2) && close(dst[13], 80, 2) && close(dst[12], 160, 2));

        convert(&src, 2, 2, PixelFormat::Rgba, &mut dst).unwrap();
        assert!(close(dst[0], 255, 1) && close(dst[1], 128, 1) && dst[2] == 0 && dst[3] == 128);
    }

    fn uyvy_of(rgb: [u8; 3]) -> [u8; 4] {
        let src = [
            px(rgb[0], rgb[1], rgb[2], 255),
            px(rgb[0], rgb[1], rgb[2], 255),
        ]
        .concat();
        let mut dst = [0u8; 4];
        premul_rgba_to_uyvy(&src, &mut dst);
        dst
    }

    #[test]
    fn uyvy_reference_colours_bt709_limited() {
        // (R,G,B) -> (Y, Cb, Cr) from the BT.709 equations, limited range.
        let cases: [([u8; 3], [u8; 3]); 8] = [
            ([0, 0, 0], [16, 128, 128]),
            ([255, 255, 255], [235, 128, 128]),
            ([128, 128, 128], [126, 128, 128]),
            ([255, 0, 0], [63, 102, 240]),
            ([0, 255, 0], [173, 42, 26]),
            ([0, 0, 255], [32, 240, 118]),
            ([255, 255, 0], [219, 16, 138]),
            ([0, 255, 255], [188, 154, 16]),
        ];
        for (rgb, [y, cb, cr]) in cases {
            let [u, y0, v, y1] = uyvy_of(rgb);
            assert!(close(y0, y, 1) && y0 == y1, "{rgb:?}: Y {y0} want {y}");
            assert!(close(u, cb, 1), "{rgb:?}: Cb {u} want {cb}");
            assert!(close(v, cr, 1), "{rgb:?}: Cr {v} want {cr}");
        }
        // Exact at the ends of the range.
        assert_eq!(uyvy_of([0, 0, 0]), [128, 16, 128, 16]);
        assert_eq!(uyvy_of([255, 255, 255]), [128, 235, 128, 235]);
    }

    #[test]
    fn uyvy_flattens_over_black_and_shares_chroma() {
        // Half-transparent white over black is mid grey; a transparent pixel is black.
        let src = [premul(255, 255, 255, 128), px(0, 0, 0, 0)].concat();
        let mut dst = [0u8; 4];
        premul_rgba_to_uyvy(&src, &mut dst);
        assert_eq!(dst[0], 128);
        assert_eq!(dst[2], 128);
        assert!(close(dst[1], 126, 1));
        assert_eq!(dst[3], 16);
        // Pair chroma = average: red + blue.
        let src = [px(255, 0, 0, 255), px(0, 0, 255, 255)].concat();
        premul_rgba_to_uyvy(&src, &mut dst);
        let (u, v) = chroma(128, 0, 128);
        assert!(close(dst[0], u, 1) && close(dst[2], v, 1));
    }

    #[test]
    fn uyva_alpha_plane_and_weighted_chroma() {
        let w = 4;
        let src: Vec<u8> = [
            premul(255, 0, 0, 255),
            px(0, 0, 0, 0), // transparent neighbour must not darken the red chroma
            premul(255, 255, 255, 64),
            premul(255, 255, 255, 192),
        ]
        .concat();
        let mut dst = Vec::new();
        convert(&src, w, 1, PixelFormat::Uyva, &mut dst).unwrap();
        assert_eq!(dst.len(), 4 * 2 + 4);
        let red = uyvy_of([255, 0, 0]);
        assert_eq!(dst[0], red[0], "Cb of red with a transparent neighbour");
        assert_eq!(dst[2], red[2], "Cr of red with a transparent neighbour");
        assert_eq!(dst[1], red[1]);
        assert_eq!(dst[3], 16, "transparent pixel luma is black");
        // Translucent white stays white in straight colour.
        assert!(close(dst[5], 235, 1) && close(dst[7], 235, 1));
        assert!(close(dst[4], 128, 1) && close(dst[6], 128, 1));
        assert_eq!(&dst[8..], &[255, 0, 64, 192]);
    }

    #[test]
    fn uyva_fast_paths_agree_with_general_path() {
        // Opaque content: UYVA colour equals UYVY, alpha plane all 255.
        let src: Vec<u8> = (0..64u32)
            .flat_map(|i| [(i * 37) as u8, (i * 91) as u8, (i * 13) as u8, 255])
            .collect();
        let mut uyvy = Vec::new();
        let mut uyva = Vec::new();
        convert(&src, 64, 1, PixelFormat::Uyvy, &mut uyvy).unwrap();
        convert(&src, 64, 1, PixelFormat::Uyva, &mut uyva).unwrap();
        assert_eq!(&uyva[..128], &uyvy[..]);
        assert!(uyva[128..].iter().all(|&a| a == 255));
        // Nearly opaque (general path) stays within rounding of the opaque result.
        let near: Vec<u8> = src
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                let pm = |c: u8| ((c as u32 * 254 + 127) / 255) as u8;
                [pm(p[0]), pm(p[1]), pm(p[2]), 254]
            })
            .collect();
        convert(&near, 64, 1, PixelFormat::Uyva, &mut uyva).unwrap();
        for (x, y) in uyva[..128].iter().zip(&uyvy) {
            assert!(close(*x, *y, 2), "{x} vs {y}");
        }
    }

    #[test]
    fn convert_rejects_bad_input() {
        let mut dst = Vec::new();
        assert!(convert(&[0; 12], 3, 1, PixelFormat::Uyvy, &mut dst).is_err());
        assert!(convert(&[0; 12], 3, 1, PixelFormat::Bgra, &mut dst).is_ok());
        assert!(convert(&[0; 11], 3, 1, PixelFormat::Bgra, &mut dst).is_err());
    }

    #[test]
    fn buffer_reuse_keeps_capacity() {
        let src = crate::test_util::pattern(64, 32, 1);
        let mut dst = Vec::new();
        convert(&src, 64, 32, PixelFormat::Bgra, &mut dst).unwrap();
        let cap = dst.capacity();
        let ptr = dst.as_ptr();
        convert(&src, 64, 32, PixelFormat::Uyvy, &mut dst).unwrap();
        assert_eq!(dst.len(), 64 * 32 * 2);
        assert_eq!((dst.capacity(), dst.as_ptr()), (cap, ptr));
    }

    /// Simple timing test on a 1080p frame. Prints per-format timings; the bound is loose so
    /// it holds for unoptimised test builds on slow CI (run
    /// `cargo run -p output --release --example convert_bench` for real numbers).
    #[test]
    fn timing_1080p() {
        let (w, h) = (1920, 1080);
        let src = crate::test_util::pattern(w, h, 7);
        let mut dst = Vec::new();
        // Warm up: fault in the destination pages once so the first format is not penalised.
        convert(&src, w, h, PixelFormat::Bgra, &mut dst).unwrap();
        for format in [
            PixelFormat::BgraPremultiplied,
            PixelFormat::Bgrx,
            PixelFormat::Bgra,
            PixelFormat::Uyvy,
            PixelFormat::Uyva,
        ] {
            let start = Instant::now();
            let n = 3;
            for _ in 0..n {
                convert(&src, w, h, format, &mut dst).unwrap();
            }
            let per = start.elapsed() / n;
            eprintln!("1080p {format:?}: {per:?} per frame");
            assert!(per.as_secs_f64() < 2.0, "{format:?} took {per:?}");
        }
    }
}
