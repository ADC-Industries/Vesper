//! Small geometry and colour helpers (f32, pixel space).

use model::{Anchor, Color, QuarterTurn, ScaleMode};

/// A rectangle `x, y, w, h` in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    /// Left.
    pub x: f32,
    /// Top.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl RectF {
    /// Rectangle from position and size.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        RectF { x, y, w, h }
    }
    /// Right edge.
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    /// Bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    /// True when it has no area.
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
    /// Smallest rectangle containing both (an empty rectangle is ignored).
    pub fn union(&self, o: &RectF) -> RectF {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        RectF::new(x0, y0, x1 - x0, y1 - y0)
    }
    /// Grown by `d` on every side.
    pub fn outset(&self, d: f32) -> RectF {
        RectF::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }
    /// Moved by `(dx, dy)`.
    pub fn translate(&self, dx: f32, dy: f32) -> RectF {
        RectF::new(self.x + dx, self.y + dy, self.w, self.h)
    }
    /// As `[x0, y0, x1, y1]`.
    pub fn corners(&self) -> [f32; 4] {
        [self.x, self.y, self.right(), self.bottom()]
    }
    /// As `[x, y, w, h]`.
    pub fn xywh(&self) -> [f32; 4] {
        [self.x, self.y, self.w, self.h]
    }
}

/// A 2D affine transform `x' = a x + b y + c`, `y' = d x + e y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        e: 1.0,
        f: 0.0,
    };

    pub fn translate(x: f32, y: f32) -> Affine {
        Affine {
            c: x,
            f: y,
            ..Affine::IDENTITY
        }
    }

    pub fn scale(sx: f32, sy: f32) -> Affine {
        Affine {
            a: sx,
            e: sy,
            ..Affine::IDENTITY
        }
    }

    /// Clockwise rotation (y down) by `deg` degrees about the origin.
    pub fn rotate(deg: f32) -> Affine {
        let (s, c) = deg.to_radians().sin_cos();
        Affine {
            a: c,
            b: -s,
            c: 0.0,
            d: s,
            e: c,
            f: 0.0,
        }
    }

    /// `self ∘ o`: apply `o` first, then `self`.
    pub fn then_after(&self, o: &Affine) -> Affine {
        Affine {
            a: self.a * o.a + self.b * o.d,
            b: self.a * o.b + self.b * o.e,
            c: self.a * o.c + self.b * o.f + self.c,
            d: self.d * o.a + self.e * o.d,
            e: self.d * o.b + self.e * o.e,
            f: self.d * o.c + self.e * o.f + self.f,
        }
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.b * y + self.c,
            self.d * x + self.e * y + self.f,
        )
    }
}

/// sRGB transfer function, encoded → linear.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Model colour (straight sRGB) → linear premultiplied, with an extra alpha multiplier.
pub fn premul_linear(c: Color, alpha: f32) -> [f32; 4] {
    let a = (c.a * alpha).clamp(0.0, 1.0);
    [
        srgb_to_linear(c.r.clamp(0.0, 1.0)) * a,
        srgb_to_linear(c.g.clamp(0.0, 1.0)) * a,
        srgb_to_linear(c.b.clamp(0.0, 1.0)) * a,
        a,
    ]
}

/// Where an image of `img_w × img_h` pixels lands inside `target` for a scale mode and anchor.
/// `native_scale` converts image pixels to target pixels for [`ScaleMode::None`].
pub fn place_media(
    target: RectF,
    img_w: f32,
    img_h: f32,
    mode: ScaleMode,
    anchor: Anchor,
    native_scale: f32,
) -> RectF {
    if img_w <= 0.0 || img_h <= 0.0 {
        return RectF::default();
    }
    let (w, h) = match mode {
        ScaleMode::Stretch => return target,
        ScaleMode::Fill => {
            let s = (target.w / img_w).max(target.h / img_h);
            (img_w * s, img_h * s)
        }
        ScaleMode::None => (img_w * native_scale, img_h * native_scale),
        ScaleMode::Fit | ScaleMode::Unknown => {
            let s = (target.w / img_w).min(target.h / img_h);
            (img_w * s, img_h * s)
        }
    };
    let (fx, fy) = anchor_factors(anchor);
    RectF::new(
        target.x + (target.w - w) * fx,
        target.y + (target.h - h) * fy,
        w,
        h,
    )
}

/// 0 / 0.5 / 1 placement factors of an anchor.
pub fn anchor_factors(anchor: Anchor) -> (f32, f32) {
    match anchor {
        Anchor::TopLeft => (0.0, 0.0),
        Anchor::Top => (0.5, 0.0),
        Anchor::TopRight => (1.0, 0.0),
        Anchor::Right => (1.0, 0.5),
        Anchor::BottomRight => (1.0, 1.0),
        Anchor::Bottom => (0.5, 1.0),
        Anchor::BottomLeft => (0.0, 1.0),
        Anchor::Left => (0.0, 0.5),
        Anchor::Center | Anchor::Unknown => (0.5, 0.5),
    }
}

/// The uv matrix (about the centre) for a quarter turn and flips, mapping *displayed* uv to
/// *source* uv. Returned as `[m00, m01, m10, m11]` (rows).
pub fn uv_matrix(turn: QuarterTurn, flip_h: bool, flip_v: bool) -> [f32; 4] {
    // displayed (u, v) → after un-flipping → after un-rotating = source uv
    let fx = if flip_h { -1.0 } else { 1.0 };
    let fy = if flip_v { -1.0 } else { 1.0 };
    // rotation of the *source* clockwise by k quarter turns: displayed = R(source).
    // source = R^-1(displayed).
    let r: [f32; 4] = match turn {
        QuarterTurn::Cw90 => [0.0, 1.0, -1.0, 0.0],
        QuarterTurn::Cw180 => [-1.0, 0.0, 0.0, -1.0],
        QuarterTurn::Cw270 => [0.0, -1.0, 1.0, 0.0],
        _ => [1.0, 0.0, 0.0, 1.0],
    };
    [r[0] * fx, r[1] * fy, r[2] * fx, r[3] * fy]
}

/// True when a quarter turn swaps width and height.
pub fn swaps_axes(turn: QuarterTurn) -> bool {
    matches!(turn, QuarterTurn::Cw90 | QuarterTurn::Cw270)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_fill_stretch_none() {
        let t = RectF::new(0.0, 0.0, 200.0, 100.0);
        let fit = place_media(t, 100.0, 100.0, ScaleMode::Fit, Anchor::Center, 1.0);
        assert_eq!(fit, RectF::new(50.0, 0.0, 100.0, 100.0));
        let fill = place_media(t, 100.0, 100.0, ScaleMode::Fill, Anchor::Center, 1.0);
        assert_eq!(fill, RectF::new(0.0, -50.0, 200.0, 200.0));
        let st = place_media(t, 100.0, 100.0, ScaleMode::Stretch, Anchor::Center, 1.0);
        assert_eq!(st, t);
        let none = place_media(t, 40.0, 20.0, ScaleMode::None, Anchor::TopLeft, 0.5);
        assert_eq!(none, RectF::new(0.0, 0.0, 20.0, 10.0));
    }

    #[test]
    fn rotate_is_clockwise_y_down() {
        let (x, y) = Affine::rotate(90.0).apply(1.0, 0.0);
        assert!(x.abs() < 1e-6 && (y - 1.0).abs() < 1e-6, "{x} {y}");
    }
}
