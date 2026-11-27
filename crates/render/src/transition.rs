//! Geometry of the moving transitions (push, slide, wipe, zoom) of a [`crate::LayerMix`].
//!
//! A dissolve (and every kind that plays as one) blends the two sides' draws straight into the
//! mix store. A moving kind first draws each side into its own screen-sized texture (a
//! snapshot side already is one), then composites the two textures into the store, moved,
//! scaled or clipped as [`Motion::draws`] says:
//!
//! | kind | outgoing | incoming |
//! |---|---|---|
//! | `push` | moves out in `direction` | follows it in |
//! | `slide` | stays, clipped to what the incoming doesn't cover | slides in over it |
//! | `wipe` | clipped to what the edge hasn't passed | clipped to what it has |
//! | `zoom` | grows (in) or shrinks (out), fading | grows to (in) or shrinks onto (out) the screen, fading in |
//!
//! Push, slide and wipe never overlap the two sides, so a transparent slide never shows the
//! other through it; zoom cross-fades in linear light like a dissolve (outgoing "over",
//! incoming "add").

use crate::geom::{Affine, RectF};
use crate::gpu::{Draw, Pipe, Uniforms, paint};
use model::{Direction, TransitionKind, ZoomDirection};

/// How a moving transition places its two sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Motion {
    /// Both move by the unit vector (`dx`, `dy`).
    Push(f32, f32),
    /// The incoming side moves in along (`dx`, `dy`) over the outgoing one.
    Slide(f32, f32),
    /// A hard edge moving in this direction reveals the incoming side.
    Wipe(Direction),
    /// Zoom in (`true`) or out.
    Zoom(bool),
}

impl Motion {
    /// The motion of `kind`, or `None` for kinds drawn as a dissolve.
    pub(crate) fn of(kind: &TransitionKind) -> Option<Motion> {
        Some(match kind {
            TransitionKind::Push { direction } => {
                let (x, y) = unit(*direction);
                Motion::Push(x, y)
            }
            TransitionKind::Slide { direction } => {
                let (x, y) = unit(*direction);
                Motion::Slide(x, y)
            }
            TransitionKind::Wipe { direction } => Motion::Wipe(*direction),
            TransitionKind::Zoom { direction } => Motion::Zoom(*direction != ZoomDirection::Out),
            _ => return None,
        })
    }

    /// Draws compositing the side textures (`from`, `to`: `None` = empty) at progress `p` on
    /// a `full`-sized target.
    pub(crate) fn draws(
        self,
        full: RectF,
        p: f32,
        from: Option<&wgpu::TextureView>,
        to: Option<&wgpu::TextureView>,
    ) -> Vec<Draw> {
        let p = p.clamp(0.0, 1.0);
        let mut out = Vec::new();
        let mut put = |view: Option<&wgpu::TextureView>, xf: Affine, clip: RectF, o: f32, pipe| {
            if let Some(v) = view
                && !clip.is_empty()
                && o > 0.0
            {
                out.push(textured(v, full, xf, clip, o, pipe));
            }
        };
        let (w, h) = (full.w, full.h);
        match self {
            Motion::Push(dx, dy) => {
                put(
                    from,
                    Affine::translate(dx * w * p, dy * h * p),
                    full,
                    1.0,
                    Pipe::Over,
                );
                put(
                    to,
                    Affine::translate(dx * w * (p - 1.0), dy * h * (p - 1.0)),
                    full,
                    1.0,
                    Pipe::Over,
                );
            }
            Motion::Slide(dx, dy) => {
                let (ox, oy) = (dx * w * (p - 1.0), dy * h * (p - 1.0));
                let covered = full.translate(ox, oy);
                for r in subtract(full, covered) {
                    put(from, Affine::IDENTITY, r, 1.0, Pipe::Over);
                }
                put(to, Affine::translate(ox, oy), full, 1.0, Pipe::Over);
            }
            Motion::Wipe(d) => {
                let shown = revealed(full, d, p);
                for r in subtract(full, shown) {
                    put(from, Affine::IDENTITY, r, 1.0, Pipe::Over);
                }
                put(to, Affine::IDENTITY, shown, 1.0, Pipe::Over);
            }
            Motion::Zoom(zoom_in) => {
                let (s_from, s_to) = if zoom_in {
                    (1.0 + p, 0.5 + 0.5 * p)
                } else {
                    (1.0 - 0.5 * p, 2.0 - p)
                };
                put(from, about_centre(full, s_from), full, 1.0 - p, Pipe::Over);
                put(to, about_centre(full, s_to), full, p, Pipe::Add);
            }
        }
        out
    }
}

/// Unit motion vector of a direction (y down); `center` and unknown move left.
fn unit(d: Direction) -> (f32, f32) {
    match d {
        Direction::Right => (1.0, 0.0),
        Direction::Up => (0.0, -1.0),
        Direction::Down => (0.0, 1.0),
        Direction::UpLeft => (-1.0, -1.0),
        Direction::UpRight => (1.0, -1.0),
        Direction::DownLeft => (-1.0, 1.0),
        Direction::DownRight => (1.0, 1.0),
        Direction::Left | Direction::Center | Direction::Unknown => (-1.0, 0.0),
    }
}

/// What a wipe moving in `d` has revealed at `p`: the edge starts on the side it moves away
/// from; a diagonal grows a box from the opposite corner, `center` a box from the middle.
fn revealed(full: RectF, d: Direction, p: f32) -> RectF {
    let (w, h) = (full.w, full.h);
    let (rw, rh) = (w * p, h * p);
    let (x0, y0) = (full.x, full.y);
    let (far_x, far_y) = (x0 + w - rw, y0 + h - rh);
    match d {
        Direction::Left => RectF::new(far_x, y0, rw, h),
        Direction::Right => RectF::new(x0, y0, rw, h),
        Direction::Up => RectF::new(x0, far_y, w, rh),
        Direction::Down => RectF::new(x0, y0, w, rh),
        Direction::UpLeft => RectF::new(far_x, far_y, rw, rh),
        Direction::UpRight => RectF::new(x0, far_y, rw, rh),
        Direction::DownLeft => RectF::new(far_x, y0, rw, rh),
        Direction::DownRight => RectF::new(x0, y0, rw, rh),
        Direction::Center | Direction::Unknown => {
            RectF::new(x0 + (w - rw) / 2.0, y0 + (h - rh) / 2.0, rw, rh)
        }
    }
}

/// `full` minus `hole`, as up to four rectangles (top, bottom, left, right bands).
fn subtract(full: RectF, hole: RectF) -> Vec<RectF> {
    let hx0 = hole.x.max(full.x);
    let hy0 = hole.y.max(full.y);
    let hx1 = hole.right().min(full.right());
    let hy1 = hole.bottom().min(full.bottom());
    if hx1 <= hx0 || hy1 <= hy0 {
        return vec![full];
    }
    [
        RectF::new(full.x, full.y, full.w, hy0 - full.y),
        RectF::new(full.x, hy1, full.w, full.bottom() - hy1),
        RectF::new(full.x, hy0, hx0 - full.x, hy1 - hy0),
        RectF::new(hx1, hy0, full.right() - hx1, hy1 - hy0),
    ]
    .into_iter()
    .filter(|r| !r.is_empty())
    .collect()
}

/// Scale by `s` about the centre of `full`.
fn about_centre(full: RectF, s: f32) -> Affine {
    let (cx, cy) = (full.x + full.w / 2.0, full.y + full.h / 2.0);
    Affine::translate(cx, cy)
        .then_after(&Affine::scale(s, s))
        .then_after(&Affine::translate(-cx, -cy))
}

/// A full-target texture drawn through `xf`, only over `clip` (local frame), at `opacity`.
fn textured(
    view: &wgpu::TextureView,
    full: RectF,
    xf: Affine,
    clip: RectF,
    opacity: f32,
    pipe: Pipe,
) -> Draw {
    let u = Uniforms::new(&xf, clip.corners())
        .paint(paint::TEXTURE_PREMUL)
        .paint_rect(full.xywh())
        .opacity(opacity);
    Draw::new(pipe, u).with_paint(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: RectF = RectF::new(0.0, 0.0, 100.0, 50.0);

    fn area(rs: &[RectF]) -> f32 {
        rs.iter().map(|r| r.w * r.h).sum()
    }

    #[test]
    fn wipes_split_the_screen() {
        for d in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
            Direction::UpLeft,
            Direction::DownRight,
            Direction::Center,
        ] {
            for p in [0.0, 0.3, 0.5, 1.0] {
                let r = revealed(FULL, d, p);
                let rest = subtract(FULL, r);
                let shown = if r.is_empty() { 0.0 } else { r.w * r.h };
                assert!(
                    (shown + area(&rest) - 5000.0).abs() < 1e-3,
                    "{d:?} {p}: {shown} + {}",
                    area(&rest)
                );
            }
        }
        assert_eq!(
            revealed(FULL, Direction::Left, 0.25),
            RectF::new(75.0, 0.0, 25.0, 50.0)
        );
        assert_eq!(
            revealed(FULL, Direction::Center, 0.5),
            RectF::new(25.0, 12.5, 50.0, 25.0)
        );
    }

    #[test]
    fn kinds_without_motion_dissolve() {
        assert_eq!(Motion::of(&TransitionKind::Dissolve), None);
        assert_eq!(Motion::of(&TransitionKind::Cut), None);
        assert_eq!(
            Motion::of(&TransitionKind::Push {
                direction: Direction::Up
            }),
            Some(Motion::Push(0.0, -1.0))
        );
        assert_eq!(
            Motion::of(&TransitionKind::Zoom {
                direction: ZoomDirection::Out
            }),
            Some(Motion::Zoom(false))
        );
    }
}
