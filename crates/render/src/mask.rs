//! Screen masks ([`model::Mask`], docs/LOOKS-MASKS-PLAN.md): finishing a mask's rendered
//! picture so it can be drawn over the screen as one more image, after all its layers.
//!
//! The server renders the mask's slide on a transparent target at the screen's size, then
//! [`finish_mask`] applies the mask's own feather (a blur of the whole mask, in screen pixels)
//! and, for a keyhole, turns the picture into its complement: the mask's colour wherever the
//! shapes are not, fading out where they are. Drawn over the screen with ordinary "over"
//! blending, an opaque keyhole colour gives exactly `picture·a + colour·(1−a)`.
//!
//! A keyhole whose colour is not opaque (transparent: cut the picture out to alpha) also needs
//! [`keyhole_erase`]: drawn first with "destination-out" ([`crate::LayerSource::Erase`]), it
//! scales what is under it so that the complement drawn over it gives
//! `picture·a + colour·colour.a·(1−a)` for every colour alpha (with a transparent colour, only
//! `picture·a`: outside the shapes the screen becomes transparent).
//!
//! CPU work, once per mask, screen size and change (the server caches the result).

use image::RgbaImage;
use model::Color;

/// Finishes a mask picture in place (straight-alpha sRGB, as rendered).
///
/// - `feather`: blur radius in pixels of this picture (≈ 2σ of a gaussian; 0 = none).
/// - `keyhole`: replace the picture with `color` outside the shapes (alpha `1 − a`, times the
///   colour's own alpha).
pub fn finish_mask(img: &mut RgbaImage, keyhole: bool, color: Color, feather: f32) {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 {
        return;
    }
    let sigma = (feather / 2.0).max(0.0);
    if keyhole {
        let a = mask_alpha(img, feather);
        let rgb = [color.r, color.g, color.b].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        let ca = color.a.clamp(0.0, 1.0);
        for (p, a) in img.pixels_mut().zip(a) {
            let out = ((1.0 - a.clamp(0.0, 1.0)) * ca * 255.0).round() as u8;
            p.0 = [rgb[0], rgb[1], rgb[2], out];
        }
        return;
    }
    if sigma < 0.5 {
        return;
    }
    // Overlay: blur premultiplied, so transparent pixels don't darken the edges.
    let mut buf: Vec<f32> = Vec::with_capacity(w * h * 4);
    for p in img.pixels() {
        let a = p.0[3] as f32 / 255.0;
        buf.extend([p.0[0] as f32 * a, p.0[1] as f32 * a, p.0[2] as f32 * a, a]);
    }
    blur(&mut buf, w, h, 4, sigma);
    for (p, c) in img.pixels_mut().zip(buf.as_chunks::<4>().0) {
        let a = c[3].clamp(0.0, 1.0);
        let un = |v: f32| {
            if a > 1e-4 {
                (v / a).clamp(0.0, 255.0).round() as u8
            } else {
                0
            }
        };
        p.0 = [un(c[0]), un(c[1]), un(c[2]), (a * 255.0).round() as u8];
    }
}

/// The mask's coverage (its picture's alpha, feathered), one value per pixel.
fn mask_alpha(img: &RgbaImage, feather: f32) -> Vec<f32> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let sigma = (feather / 2.0).max(0.0);
    let mut a: Vec<f32> = img.pixels().map(|p| p.0[3] as f32 / 255.0).collect();
    if sigma >= 0.5 && w > 0 && h > 0 {
        blur(&mut a, w, h, 1, sigma);
    }
    a
}

/// For a keyhole whose colour is not opaque, the picture to erase the screen with before its
/// complement is drawn (see the module docs): alpha `1 − a / (1 − (1 − a)·colour.a)` from the
/// unfinished mask picture `img` (call it before [`finish_mask`]). `None` for an opaque colour,
/// which needs no erase.
pub fn keyhole_erase(img: &RgbaImage, color: Color, feather: f32) -> Option<RgbaImage> {
    let ca = color.a.clamp(0.0, 1.0);
    if ca >= 1.0 - 1e-4 {
        return None;
    }
    let a = mask_alpha(img, feather);
    let mut out = RgbaImage::new(img.width(), img.height());
    for (p, a) in out.pixels_mut().zip(a) {
        let a = a.clamp(0.0, 1.0);
        let keep = a / (1.0 - (1.0 - a) * ca).max(1e-6);
        p.0 = [
            0,
            0,
            0,
            ((1.0 - keep.clamp(0.0, 1.0)) * 255.0).round() as u8,
        ];
    }
    Some(out)
}

/// Three box blurs per direction ≈ a gaussian of `sigma` px, over `ch` interleaved channels.
fn blur(buf: &mut [f32], w: usize, h: usize, ch: usize, sigma: f32) {
    // Box width for three passes: σ² = 3·(n² − 1)/12.
    let n = ((4.0 * sigma * sigma + 1.0).sqrt().round() as usize).max(1) | 1;
    let r = n / 2;
    if r == 0 {
        return;
    }
    let mut line = vec![0f32; w.max(h)];
    for _ in 0..3 {
        for y in 0..h {
            for c in 0..ch {
                box_line(buf, y * w * ch + c, ch, w, r, &mut line);
            }
        }
        for x in 0..w {
            for c in 0..ch {
                box_line(buf, x * ch + c, w * ch, h, r, &mut line);
            }
        }
    }
}

/// One box blur of radius `r` along `len` samples starting at `start`, `stride` apart; edges
/// are extended.
fn box_line(buf: &mut [f32], start: usize, stride: usize, len: usize, r: usize, tmp: &mut [f32]) {
    let at = |i: isize| -> f32 {
        let i = i.clamp(0, len as isize - 1) as usize;
        buf[start + i * stride]
    };
    let n = (2 * r + 1) as f32;
    let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
    for (i, t) in tmp.iter_mut().enumerate().take(len) {
        *t = sum / n;
        let i = i as isize;
        sum += at(i + r as isize + 1) - at(i - r as isize);
    }
    for (i, t) in tmp.iter().enumerate().take(len) {
        buf[start + i * stride] = *t;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 40×20 transparent picture with an opaque white 20×20 square on the left half.
    fn half() -> RgbaImage {
        RgbaImage::from_fn(40, 20, |x, _| {
            if x < 20 {
                image::Rgba([255, 255, 255, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        })
    }

    #[test]
    fn a_transparent_keyhole_erases_outside_the_shapes() {
        let img = half();
        assert!(
            keyhole_erase(&img, Color::BLACK, 0.0).is_none(),
            "opaque: no erase"
        );
        let e = keyhole_erase(&img, Color::TRANSPARENT, 0.0).unwrap();
        assert_eq!(e.get_pixel(5, 10).0[3], 0, "inside: kept");
        assert_eq!(e.get_pixel(35, 10).0[3], 255, "outside: erased");
        // Half see-through colour: outside keeps half of the picture under the colour's half.
        let e = keyhole_erase(&img, Color::rgba(1.0, 0.0, 0.0, 0.5), 0.0).unwrap();
        assert_eq!(
            e.get_pixel(35, 10).0[3],
            255,
            "outside: picture·0 under the colour"
        );
        assert_eq!(e.get_pixel(5, 10).0[3], 0);
    }

    #[test]
    fn overlay_without_feather_is_unchanged() {
        let mut img = half();
        finish_mask(&mut img, false, Color::BLACK, 0.0);
        assert_eq!(img, half());
    }

    #[test]
    fn keyhole_covers_outside_the_shapes() {
        let mut img = half();
        finish_mask(&mut img, true, Color::rgb(1.0, 0.0, 0.0), 0.0);
        assert_eq!(
            img.get_pixel(5, 10).0,
            [255, 0, 0, 0],
            "inside: see-through"
        );
        assert_eq!(
            img.get_pixel(35, 10).0,
            [255, 0, 0, 255],
            "outside: the colour"
        );
    }

    #[test]
    fn feather_softens_the_edge_and_keeps_colours() {
        let mut img = half();
        finish_mask(&mut img, false, Color::BLACK, 8.0);
        let a: Vec<u8> = (10..30).map(|x| img.get_pixel(x, 10).0[3]).collect();
        assert!(a.windows(2).all(|w| w[1] <= w[0]), "{a:?}");
        assert!(
            a.iter().any(|v| (60..200).contains(v)),
            "a soft edge: {a:?}"
        );
        assert_eq!(img.get_pixel(2, 10).0, [255, 255, 255, 255]);
        // Premultiplied blur: the half-transparent edge is still white, not grey.
        let edge = img.get_pixel(20, 10).0;
        assert!(edge[0] > 250 && edge[3] > 40 && edge[3] < 220, "{edge:?}");
        // A keyhole's edge fades the other way.
        let mut k = half();
        finish_mask(&mut k, true, Color::BLACK, 8.0);
        assert!(k.get_pixel(19, 10).0[3] > 20 && k.get_pixel(19, 10).0[3] < 235);
        assert_eq!(k.get_pixel(39, 10).0[3], 255);
    }
}
