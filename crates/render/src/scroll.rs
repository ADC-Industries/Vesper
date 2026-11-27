//! Scrolling text ([`model::TextScroll`]).
//!
//! A slide with scrolling text is drawn in two parts. The slide itself, with the scrolling
//! elements' text taken out (their fill, stroke and shadow stay), is the usual cached slide
//! layer. Each scrolling element's text is laid out once as a **strip** — unwrapped for
//! `left` / `right` (the box's height, its vertical alignment kept), wrapped to the box width
//! for `up` / `down` — and rendered into a cached texture. Every frame then draws the strip's
//! copies ([`copies`]) clipped to the element's box. Scrolling text is drawn above the slide's
//! other elements, ignores the element's rotation and flips, and is cut off past the GPU's
//! largest texture (16384 px at screen scale).
//!
//! Time is the seconds the slide has been live ([`crate::LayerSource::Slide`]'s `time`);
//! without one (thumbnails, the editor) the text is drawn still, where it is laid out.

use crate::geom::RectF;
use model::{ElementKind, FitMode, ScrollDirection, ScrollStart, Slide, TextScroll};

/// Canvas px the strip is laid out in along its axis (longer text is cut off).
pub(crate) const LAYOUT_LENGTH: f32 = 50_000.0;
/// Canvas px of room kept around the strip's ink (italic overhang, outlines, shadows).
pub(crate) const PAD: f32 = 24.0;

/// The element scrolls (a visible text element with scrolling on and some text).
pub(crate) fn scrolls(e: &model::Element) -> bool {
    e.scroll.enabled
        && !e.hidden
        && e.scroll.speed.is_finite()
        && e.bounds.width > 0.0
        && e.bounds.height > 0.0
        && e.text_box().is_some_and(|t| !t.is_blank())
}

/// The slide has scrolling text.
pub(crate) fn has_scrolling(slide: &Slide) -> bool {
    slide.elements.iter().any(scrolls)
}

/// The slide without the scrolling elements' text (they keep their fill, stroke and shadow).
pub(crate) fn base_slide(slide: &Slide) -> Slide {
    let mut s = slide.clone();
    for e in s.elements.iter_mut().filter(|e| scrolls(e)) {
        if let ElementKind::Text { shape, .. } = &e.kind {
            e.kind = ElementKind::Shape {
                shape: shape.clone(),
            };
        }
    }
    s
}

/// The element that draws `e`'s strip: only its text, unfitted, unrotated, at `bounds`.
pub(crate) fn strip_element(e: &model::Element, bounds: model::Rect) -> model::Element {
    let mut s = e.clone();
    s.bounds = bounds;
    s.rotation = 0.0;
    s.flip = Default::default();
    s.fill.enabled = false;
    s.stroke.enabled = false;
    s.shadow.enabled = false;
    s.feather.enabled = false;
    s.scroll = TextScroll::default();
    s.visibility.clear();
    if let Some(t) = s.text_box_mut() {
        t.fit = FitMode::None;
    }
    s
}

/// Where a strip sits and how it moves, in canvas px along its axis, relative to the box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Strip {
    /// Moves along x (left / right).
    pub horizontal: bool,
    /// Length of the strip along its axis (ink plus padding).
    pub length: f32,
    /// Where the strip's start is placed in the layout box ([`LAYOUT_LENGTH`] long).
    pub layout_start: f32,
    /// Where the strip's start sits in the element's box when drawn still.
    pub laid_out: f32,
}

/// Positions (strip start relative to the box start, canvas px, along the axis) of the copies
/// of a `strip` that overlap a box `len` long, `time` seconds after the slide went live.
pub(crate) fn copies(scroll: &TextScroll, strip: &Strip, len: f32, time: Option<f64>) -> Vec<f32> {
    let l = strip.length;
    let Some(t) = time else {
        return vec![strip.laid_out];
    };
    let backwards = matches!(
        scroll.direction,
        ScrollDirection::Right | ScrollDirection::Down
    );
    let fits = l <= len + 2.0 * PAD;
    let offscreen = match scroll.start {
        ScrollStart::Offscreen => true,
        ScrollStart::Onscreen => false,
        ScrollStart::Auto | ScrollStart::Unknown => !fits,
    };
    let x0 = match (offscreen, backwards) {
        (true, false) => len,
        (true, true) => -l,
        (false, _) => strip.laid_out,
    };
    // Distances are worked out in f64 and, once the loop is periodic, reduced by its period,
    // so a fast or long-running ticker keeps sub-pixel positions and the copy count stays small.
    let mut travelled = scroll.speed.max(0.0) * t.max(0.0);
    if !travelled.is_finite() {
        return Vec::new();
    }
    // The gap is ink to ink: strips carry `PAD` on both ends.
    let period = (f64::from(l) - 2.0 * f64::from(PAD) + scroll.gap.max(0.0)).max(1.0);
    let (len64, l64) = (f64::from(len), f64::from(l));
    if scroll.repeat {
        // Past `settled` the first copy has left and every period looks the same (copy k at
        // `travelled + period` is where copy k − 1 was at `travelled`).
        let settled = len64 + l64 + 2.0 * period;
        if travelled > settled {
            travelled = settled + (travelled - settled).rem_euclid(period);
        }
    }
    // Copy k starts `k · period` behind the first: p_k = x0 ∓ travelled ± k · period.
    let at = |k: i64| {
        let d = k as f64 * period - travelled;
        (if backwards {
            f64::from(x0) - d
        } else {
            f64::from(x0) + d
        }) as f32
    };
    let visible = |p: f32| p < len && p + l > 0.0;
    let (first, last) = if scroll.repeat {
        // The newest copy that has started (it may still be off screen).
        let ahead = if offscreen { 0.0 } else { l64.max(len64) };
        let last = ((travelled + ahead) / period).ceil() as i64 + 1;
        let first = (((travelled - len64 - l64) / period).floor() as i64 - 1).max(0);
        (first, last)
    } else {
        (0, 0)
    };
    // Bounded: at most a few thousand candidates (a 1 px period over a long box), 64 drawn.
    (first..=last.min(first + 4096))
        .map(at)
        .filter(|&p| visible(p))
        .take(64)
        .collect()
}

/// The part of `a` inside `b`.
pub(crate) fn intersect(a: RectF, b: RectF) -> RectF {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = a.right().min(b.right());
    let y1 = a.bottom().min(b.bottom());
    RectF::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(length: f32, laid_out: f32) -> Strip {
        Strip {
            horizontal: true,
            length,
            layout_start: 0.0,
            laid_out,
        }
    }

    fn scroll(direction: ScrollDirection, start: ScrollStart, repeat: bool) -> TextScroll {
        TextScroll {
            enabled: true,
            direction,
            speed: 100.0,
            start,
            repeat,
            gap: 50.0,
        }
    }

    #[test]
    fn offscreen_left_enters_from_the_right_and_loops() {
        let sc = scroll(ScrollDirection::Left, ScrollStart::Offscreen, true);
        let st = strip(300.0, 0.0);
        // At 0 s it is just outside the 1000 px box.
        assert!(copies(&sc, &st, 1000.0, Some(0.0)).is_empty());
        assert_eq!(copies(&sc, &st, 1000.0, Some(1.0)), vec![900.0]);
        // 4 s: the first copy at 600, the next a period (300 − 2 · 24 + 50) behind it.
        assert_eq!(copies(&sc, &st, 1000.0, Some(4.0)), vec![600.0, 902.0]);
        // Much later the loop still fills the box, seamlessly: copies a period apart.
        let c = copies(&sc, &st, 1000.0, Some(1000.0));
        assert!(c.len() >= 3, "{c:?}");
        for w in c.windows(2) {
            assert!((w[1] - w[0] - 302.0).abs() < 1e-3, "{c:?}");
        }
        assert!(c[0] < 302.0 && c[0] > -300.0, "{c:?}");
    }

    #[test]
    fn once_scrolls_through_and_is_gone() {
        let sc = scroll(ScrollDirection::Right, ScrollStart::Offscreen, false);
        let st = strip(300.0, 0.0);
        assert_eq!(copies(&sc, &st, 1000.0, Some(2.0)), vec![-100.0]);
        assert!(copies(&sc, &st, 1000.0, Some(13.1)).is_empty());
    }

    #[test]
    fn auto_starts_onscreen_when_it_fits() {
        let sc = scroll(ScrollDirection::Up, ScrollStart::Auto, true);
        // (the loop's next copy follows one period behind).
        assert_eq!(
            copies(&sc, &strip(300.0, 120.0), 1000.0, Some(0.0)),
            vec![120.0, 422.0, 724.0]
        );
        // Longer than the box: starts below it.
        assert!(copies(&sc, &strip(3000.0, -20.0), 1000.0, Some(0.0)).is_empty());
        // Still pictures show it where it is laid out.
        assert_eq!(
            copies(&sc, &strip(3000.0, -20.0), 1000.0, None),
            vec![-20.0]
        );
    }

    /// D03-1: huge speeds or very long-running loops must neither hang nor lose precision.
    #[test]
    fn huge_travel_stays_bounded_and_periodic() {
        let st = strip(68.0, 0.0); // period 20 + 0 gap = 20 px
        let mut sc = scroll(ScrollDirection::Left, ScrollStart::Offscreen, true);
        sc.gap = 0.0;
        for speed in [1e30, 1e300, f64::MAX] {
            sc.speed = speed;
            let c = copies(&sc, &st, 1000.0, Some(1.0));
            assert!(c.len() <= 64, "{c:?}");
        }
        // Speed 2000 for 60 hours (2^24 periods of 20 px went by after about 47 hours).
        sc.speed = 2000.0;
        let t = 60.0 * 3600.0;
        let a = copies(&sc, &st, 1000.0, Some(t));
        let b = copies(&sc, &st, 1000.0, Some(t + 0.001));
        assert!(!a.is_empty(), "the loop still fills the box");
        // Exactly 2 px further left after 1 ms, not quantised to whole pixels.
        assert!(
            (a[0] - b[0] - 2.0).abs() < 1e-3 || (a[0] - b[0] + 18.0).abs() < 1e-3,
            "{a:?} {b:?}"
        );
        for w in a.windows(2) {
            assert!((w[1] - w[0] - 20.0).abs() < 1e-3, "{a:?}");
        }
        // Once only: long gone, no loop at all.
        sc.repeat = false;
        sc.speed = 1e300;
        assert!(copies(&sc, &st, 1000.0, Some(5.0)).is_empty());
    }

    #[test]
    fn onscreen_start_moves_from_where_it_is() {
        let sc = scroll(ScrollDirection::Left, ScrollStart::Onscreen, true);
        let c = copies(&sc, &strip(300.0, 350.0), 1000.0, Some(1.0));
        assert_eq!(c[0], 250.0);
        // The next copy follows a period behind.
        assert!(c.contains(&552.0), "{c:?}");
    }
}
