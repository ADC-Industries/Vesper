//! Per-screen picture adjustments: colour correction and corner pin.
//!
//! Both are applied by the screen's **output pass** (the draw that turns the linear
//! composite into the RGBA8 output frame), so they cost no extra pass and no extra texture:
//! the pass's fragment shader (`fs_adjust`) samples the composite through the inverse corner
//! pin homography (bilinear, with an anti-aliased edge; outside the quad: black on opaque
//! screens, transparent on screens with alpha), flattens over the background as usual,
//! encodes to sRGB, applies the colour correction to the straight encoded colour and
//! premultiplies. A screen without adjustments keeps the plain output pass (`fs_final`), so
//! its frames are bit for bit what they were.

use model::{ColorCorrection, CornerPin};

/// Flag: the corner pin homography is applied (`modes.y`).
pub(crate) const FLAG_PIN: u32 = 1;
/// Flag: the colour correction is applied (`modes.y`).
pub(crate) const FLAG_COLOR: u32 = 2;

/// A screen's adjustments in the form the shader takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Adjust {
    /// [`FLAG_PIN`] | [`FLAG_COLOR`].
    pub flags: u32,
    /// Rows of the homography from output pixels to the picture's unit square (`stop_c[0..3]`).
    pub inverse: [[f32; 4]; 3],
    /// (red, green, blue, saturation), (brightness, contrast, gamma, black level)
    /// (`stop_c[3..5]`).
    pub grade: [[f32; 4]; 2],
}

impl Adjust {
    /// The adjustments for a `width` × `height` screen; `None` when they change nothing.
    /// A corner pin whose quad has no usable homography (degenerate) is left out.
    pub fn new(color: &ColorCorrection, pin: &CornerPin, width: u32, height: u32) -> Option<Self> {
        let mut flags = 0;
        let mut inverse = [[0.0; 4]; 3];
        if pin.is_active()
            && let Some(m) = unit_to_quad(pin.quad(width as f32, height as f32)).and_then(invert)
        {
            flags |= FLAG_PIN;
            for (row, r) in inverse.iter_mut().zip(m) {
                *row = [r[0] as f32, r[1] as f32, r[2] as f32, 0.0];
            }
        }
        let c = color;
        if !c.is_identity() {
            flags |= FLAG_COLOR;
        }
        (flags != 0).then_some(Adjust {
            flags,
            inverse,
            grade: [
                [c.red, c.green, c.blue, c.saturation],
                [c.brightness, c.contrast, c.gamma.max(0.01), c.black_level],
            ],
        })
    }
}

type M3 = [[f64; 3]; 3];

/// The homography from the unit square (0,0), (1,0), (1,1), (0,1) to `quad` (clockwise from
/// the top left), as a 3×3 matrix on (u, v, 1). Heckbert's closed form.
pub(crate) fn unit_to_quad(quad: [[f32; 2]; 4]) -> Option<M3> {
    let p = quad.map(|[x, y]| (x as f64, y as f64));
    let ((x0, y0), (x1, y1), (x2, y2), (x3, y3)) = (p[0], p[1], p[2], p[3]);
    let (sx, sy) = (x0 - x1 + x2 - x3, y0 - y1 + y2 - y3);
    if sx.abs() < 1e-9 && sy.abs() < 1e-9 {
        return Some([
            [x1 - x0, x3 - x0, x0],
            [y1 - y0, y3 - y0, y0],
            [0.0, 0.0, 1.0],
        ]);
    }
    let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
    let den = dx1 * dy2 - dx2 * dy1;
    if den.abs() < 1e-9 {
        return None;
    }
    let g = (sx * dy2 - dx2 * sy) / den;
    let h = (dx1 * sy - sx * dy1) / den;
    Some([
        [x1 - x0 + g * x1, x3 - x0 + h * x3, x0],
        [y1 - y0 + g * y1, y3 - y0 + h * y3, y0],
        [g, h, 1.0],
    ])
}

/// The inverse of `m` (`None` when singular).
pub(crate) fn invert(m: M3) -> Option<M3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let k = 1.0 / det;
    Some([
        [
            (e * i - f * h) * k,
            (c * h - b * i) * k,
            (b * f - c * e) * k,
        ],
        [
            (f * g - d * i) * k,
            (a * i - c * g) * k,
            (c * d - a * f) * k,
        ],
        [
            (d * h - e * g) * k,
            (b * g - a * h) * k,
            (a * e - b * d) * k,
        ],
    ])
}

/// `m` applied to the point (x, y).
#[cfg(test)]
fn apply(m: &M3, x: f64, y: f64) -> (f64, f64) {
    let w = m[2][0] * x + m[2][1] * y + m[2][2];
    (
        (m[0][0] * x + m[0][1] * y + m[0][2]) / w,
        (m[1][0] * x + m[1][1] * y + m[1][2]) / w,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::CornerOffset;

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    #[test]
    fn homography_maps_corners_and_back() {
        let quad = [[100.0, 50.0], [1800.0, 0.0], [1920.0, 1080.0], [0.0, 900.0]];
        let m = unit_to_quad(quad).unwrap();
        let inv = invert(m).unwrap();
        for (uv, q) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
            .iter()
            .zip(quad)
        {
            let p = apply(&m, uv.0, uv.1);
            assert!(near(p, (q[0] as f64, q[1] as f64)), "{p:?} vs {q:?}");
            assert!(near(apply(&inv, p.0, p.1), *uv));
        }
        // The centre of the unit square goes to where the quad's diagonals cross.
        let c = apply(&m, 0.5, 0.5);
        let back = apply(&inv, c.0, c.1);
        assert!(near(back, (0.5, 0.5)));
    }

    #[test]
    fn identity_settings_need_no_pass() {
        let pin = CornerPin {
            enabled: false,
            top_left: CornerOffset { x: 40.0, y: 0.0 },
            ..CornerPin::default()
        };
        assert!(Adjust::new(&ColorCorrection::IDENTITY, &pin, 1920, 1080).is_none());
        let on = CornerPin {
            enabled: true,
            ..CornerPin::default()
        };
        assert!(Adjust::new(&ColorCorrection::IDENTITY, &on, 1920, 1080).is_none());
        let a = Adjust::new(
            &ColorCorrection::IDENTITY,
            &CornerPin {
                enabled: true,
                ..pin
            },
            1920,
            1080,
        )
        .unwrap();
        assert_eq!(a.flags, FLAG_PIN);
        let c = ColorCorrection {
            gamma: 1.2,
            ..ColorCorrection::IDENTITY
        };
        assert_eq!(Adjust::new(&c, &pin, 1920, 1080).unwrap().flags, FLAG_COLOR);
    }

    #[test]
    fn degenerate_quads_are_left_out() {
        // All four corners on one line.
        let pin = CornerPin {
            enabled: true,
            top_left: CornerOffset { x: 0.0, y: -540.0 },
            top_right: CornerOffset { x: 0.0, y: -540.0 },
            bottom_right: CornerOffset { x: 0.0, y: -540.0 },
            bottom_left: CornerOffset { x: 0.0, y: -540.0 },
        };
        assert!(Adjust::new(&ColorCorrection::IDENTITY, &pin, 1920, 1080).is_none());
    }
}
