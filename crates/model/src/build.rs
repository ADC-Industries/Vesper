//! Object builds: elements of a slide that appear or disappear one step at a time.
//!
//! A slide's [`Slide::builds`](crate::Slide::builds) is its build **order**. It splits into
//! **steps** ([`steps`]): step 0 plays when the slide goes live (every `withSlide` build, and
//! the `withPrevious` / `afterPrevious` builds before the first `onNext`); each `onNext` build
//! opens the next step, which `next` plays, and the `withPrevious` / `afterPrevious` builds
//! after it join it. Within a step a build starts at:
//! - `onNext` / `withSlide`: the step's start plus its `delay`;
//! - `withPrevious`: the previous build's start plus its `delay`;
//! - `afterPrevious`: the previous build's end plus its `delay`.
//!
//! [`frame`] gives the slide as drawn at a moment (pure; the engine calls it every frame
//! while a build moves). See `docs/BUILDS-PLAN.md`.

use crate::common::{Rect, Size, is_default, is_zero};
use crate::effects::{Direction, Transition, TransitionKind, ZoomDirection};
use crate::ids::ElementId;
use crate::slide::Slide;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// One build of one element.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Build {
    /// The element it moves.
    pub element: ElementId,
    /// In (hidden until it builds) or out (shown until it builds).
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub mode: BuildMode,
    /// When it starts.
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub start: BuildStart,
    /// Seconds after its start point.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub delay: f64,
    /// How it moves (the shared transition kinds; `cut` = at once).
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub transition: Transition,
}

impl Build {
    /// A build of `element` on next, with `transition`.
    pub fn new(element: ElementId, mode: BuildMode, transition: Transition) -> Self {
        Build {
            element,
            mode,
            start: BuildStart::OnNext,
            delay: 0.0,
            transition,
        }
    }

    fn duration(&self) -> f64 {
        match self.transition.kind {
            TransitionKind::Cut => 0.0,
            _ => self.transition.duration.max(0.0),
        }
    }
}

/// Build in or out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum BuildMode {
    /// The element is hidden until it builds in.
    #[default]
    In,
    /// The element is shown until it builds out.
    Out,
    /// Unknown value from a newer schema (played as `in`).
    #[serde(other)]
    Unknown,
}

/// When a build starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum BuildStart {
    /// On the next `next`: opens a new step.
    #[default]
    OnNext,
    /// With the previous build (same step).
    WithPrevious,
    /// After the previous build ends (same step).
    AfterPrevious,
    /// When the slide goes live (step 0).
    WithSlide,
    /// Unknown value from a newer schema (played as `onNext`).
    #[serde(other)]
    Unknown,
}

/// A build placed in time: its step and its start within the step, seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timed {
    /// Index in `Slide::builds`.
    pub index: usize,
    /// Step it plays in (0 = with the slide).
    pub step: usize,
    /// Seconds after the step's start.
    pub start: f64,
    /// Seconds it takes (0 = a cut).
    pub duration: f64,
}

/// The builds of `slide` that name one of its elements, placed in steps and time.
pub fn steps(slide: &Slide) -> Vec<Timed> {
    let mut out: Vec<Timed> = Vec::new();
    let mut step = 0usize;
    // The previous build of each step (start, end).
    let mut prev: Vec<(f64, f64)> = vec![(0.0, 0.0)];
    for (index, b) in slide.builds.iter().enumerate() {
        if slide.element(b.element).is_none() {
            continue;
        }
        let delay = b.delay.max(0.0);
        let (s, start) = match b.start {
            BuildStart::WithSlide => (0, delay),
            BuildStart::OnNext | BuildStart::Unknown => {
                step += 1;
                prev.push((0.0, 0.0));
                (step, delay)
            }
            BuildStart::WithPrevious => (step, prev[step].0 + delay),
            BuildStart::AfterPrevious => (step, prev[step].1 + delay),
        };
        let duration = b.duration();
        prev[s] = (start, start + duration);
        out.push(Timed {
            index,
            step: s,
            start,
            duration,
        });
    }
    out
}

/// Number of `next` steps the slide's builds take (0 = no builds to step through).
pub fn step_count(slide: &Slide) -> usize {
    steps(slide).iter().map(|t| t.step).max().unwrap_or(0)
}

/// Seconds a step takes to finish (its last build's end).
pub fn step_length(slide: &Slide, step: usize) -> f64 {
    steps(slide)
        .iter()
        .filter(|t| t.step == step)
        .map(|t| t.start + t.duration)
        .fold(0.0, f64::max)
}

/// The slide (on a `canvas`) as drawn with `step` steps taken, `t0` seconds after step 0 started (the slide
/// went live) and `t` seconds after step `step` started (`None`: settled, every build up to
/// `step` finished). Elements are hidden, faded or moved per their builds; a slide without
/// builds comes back borrowed.
pub fn frame(
    slide: &Slide,
    canvas: Size,
    step: usize,
    t0: Option<f64>,
    t: Option<f64>,
) -> Cow<'_, Slide> {
    let timed = steps(slide);
    if timed.is_empty() {
        return Cow::Borrowed(slide);
    }
    let canvas = (canvas.width, canvas.height);
    let mut out = slide.clone();
    for tb in &timed {
        let b = &slide.builds[tb.index];
        // Linear progress of this build.
        let p = if tb.step > step {
            0.0
        } else {
            let now = if tb.step == step && tb.step != 0 {
                t
            } else if tb.step == 0 {
                if step == 0 { t0 } else { None }
            } else {
                None
            };
            match now {
                None => 1.0,
                Some(now) if tb.duration <= 0.0 => {
                    if now >= tb.start {
                        1.0
                    } else {
                        0.0
                    }
                }
                Some(now) => ((now - tb.start) / tb.duration).clamp(0.0, 1.0),
            }
        };
        let p = b.transition.easing.apply(p);
        let Some(e) = out.element_mut(b.element) else {
            continue;
        };
        // Visibility amount: 0 = gone, 1 = in place.
        let v = match b.mode {
            BuildMode::Out => 1.0 - p,
            _ => p,
        };
        apply(e, &b.transition.kind, b.mode, v, canvas);
    }
    Cow::Owned(out)
}

/// The slide settled at `step` (no motion): what the stage's current picture shows.
pub fn settled(slide: &Slide, canvas: Size, step: usize) -> Cow<'_, Slide> {
    frame(slide, canvas, step, None, None)
}

/// Puts `e` at visibility `v` (0 = gone, 1 = in place) for a build of `kind`.
fn apply(
    e: &mut crate::Element,
    kind: &TransitionKind,
    mode: BuildMode,
    v: f64,
    canvas: (f64, f64),
) {
    if v <= 0.0 {
        e.hidden = true;
        return;
    }
    if v >= 1.0 {
        return;
    }
    match kind {
        TransitionKind::Push { direction } | TransitionKind::Slide { direction } => {
            let r = e.bounds;
            // Where the element is when gone: past the canvas edge it moves toward (out), or
            // comes from (in: the opposite edge of the direction of motion).
            let (dx, dy) = unit(*direction);
            let sign = if mode == BuildMode::Out { 1.0 } else { -1.0 };
            let off_x = if dx * sign > 0.0 {
                canvas.0 - r.x
            } else if dx * sign < 0.0 {
                -(r.x + r.width)
            } else {
                0.0
            };
            let off_y = if dy * sign > 0.0 {
                canvas.1 - r.y
            } else if dy * sign < 0.0 {
                -(r.y + r.height)
            } else {
                0.0
            };
            e.bounds.x = r.x + off_x * (1.0 - v);
            e.bounds.y = r.y + off_y * (1.0 - v);
        }
        TransitionKind::Zoom { direction } => {
            let from = match direction {
                ZoomDirection::Out => 2.0,
                _ => 0.0,
            };
            let s = from + (1.0 - from) * v;
            e.bounds = scaled(e.bounds, s.max(0.01));
            e.opacity *= v;
        }
        _ => e.opacity *= v,
    }
}

fn scaled(r: Rect, s: f64) -> Rect {
    let (cx, cy) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
    Rect {
        x: cx - r.width * s / 2.0,
        y: cy - r.height * s / 2.0,
        width: r.width * s,
        height: r.height * s,
    }
}

/// The direction of motion as a unit step (x right, y down); `center` / unknown = left.
fn unit(d: Direction) -> (f64, f64) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Element;
    use crate::effects::Easing;
    use serde_json::json;

    fn slide(n: usize) -> Slide {
        let mut s = Slide::new();
        for i in 0..n {
            s.elements.push(Element::new(Rect {
                x: 100.0 * i as f64,
                y: 100.0,
                width: 100.0,
                height: 50.0,
            }));
        }
        s
    }

    const HD: Size = Size {
        width: 1920.0,
        height: 1080.0,
    };

    fn id(s: &Slide, i: usize) -> ElementId {
        s.elements[i].id
    }

    #[test]
    fn wire_form_is_compact() {
        let s = slide(1);
        let b = Build::new(id(&s, 0), BuildMode::In, Transition::dissolve(0.5));
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(
            v,
            json!({"element": id(&s, 0), "transition": {"kind": {"type": "dissolve"}, "duration": 0.5}})
        );
        let o: Build = serde_json::from_value(json!({
            "element": id(&s, 0), "mode": "out", "start": "afterPrevious", "delay": 1.5,
            "transition": {"kind": {"type": "push", "direction": "up"}, "duration": 1}
        }))
        .unwrap();
        assert_eq!(o.mode, BuildMode::Out);
        assert_eq!(o.start, BuildStart::AfterPrevious);
        // Values from a newer schema play as the defaults.
        let u: Build =
            serde_json::from_value(json!({"element": id(&s, 0), "mode": "spin", "start": "x"}))
                .unwrap();
        assert_eq!((u.mode, u.start), (BuildMode::Unknown, BuildStart::Unknown));
    }

    #[test]
    fn steps_group_and_time_the_builds() {
        let mut s = slide(5);
        let d = |x| Transition::dissolve(x);
        s.builds = vec![
            Build {
                start: BuildStart::WithSlide,
                delay: 0.5,
                ..Build::new(id(&s, 0), BuildMode::In, d(1.0))
            },
            Build::new(id(&s, 1), BuildMode::In, d(1.0)),
            Build {
                start: BuildStart::WithPrevious,
                delay: 0.25,
                ..Build::new(id(&s, 2), BuildMode::In, d(1.0))
            },
            Build {
                start: BuildStart::AfterPrevious,
                delay: 0.5,
                ..Build::new(id(&s, 3), BuildMode::In, d(2.0))
            },
            Build::new(id(&s, 4), BuildMode::Out, Transition::CUT),
            // Names an element the slide does not have: ignored.
            Build::new(ElementId::new(), BuildMode::In, d(1.0)),
        ];
        let t = steps(&s);
        let got: Vec<_> = t
            .iter()
            .map(|t| (t.index, t.step, t.start, t.duration))
            .collect();
        assert_eq!(
            got,
            vec![
                (0, 0, 0.5, 1.0),
                (1, 1, 0.0, 1.0),
                (2, 1, 0.25, 1.0),
                (3, 1, 1.75, 2.0),
                (4, 2, 0.0, 0.0),
            ]
        );
        assert_eq!(step_count(&s), 2);
        assert_eq!(step_length(&s, 1), 3.75);
    }

    #[test]
    fn frames_hide_fade_and_settle() {
        let mut s = slide(3);
        s.builds = vec![
            Build::new(id(&s, 0), BuildMode::In, Transition::dissolve(1.0)),
            Build::new(id(&s, 1), BuildMode::Out, Transition::dissolve(1.0)),
        ];
        // Step 0: the in-build's element is hidden, the out-build's shown, the third untouched.
        let f = frame(&s, HD, 0, Some(5.0), None);
        assert!(f.elements[0].hidden);
        assert!(!f.elements[1].hidden && f.elements[1].opacity == 1.0);
        assert!(!f.elements[2].hidden);
        // Step 1, half way: the first fades in.
        let f = frame(&s, HD, 1, Some(9.0), Some(0.5));
        assert!(!f.elements[0].hidden);
        assert!((f.elements[0].opacity - 0.5).abs() < 1e-9);
        assert!(!f.elements[1].hidden);
        // Step 2 started a quarter in: the first is in place, the second fading out.
        let f = frame(&s, HD, 2, None, Some(0.25));
        assert_eq!(f.elements[0].opacity, 1.0);
        assert!((f.elements[1].opacity - 0.75).abs() < 1e-9);
        // Settled at the end: the second is gone.
        let f = settled(&s, HD, 2);
        assert!(f.elements[1].hidden && !f.elements[0].hidden);
        // Before its delay a cut build has not happened; a slide without builds is borrowed.
        assert!(matches!(
            frame(&slide(1), HD, 0, Some(0.0), None),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn moving_builds_come_from_the_canvas_edge() {
        let mut s = slide(1);
        let t = Transition::new(
            TransitionKind::Push {
                direction: Direction::Left,
            },
            1.0,
        );
        s.builds = vec![Build::new(id(&s, 0), BuildMode::In, t.clone())];
        // Moving left, it comes in from the right edge (x = 1920) to x = 0.
        assert!(frame(&s, HD, 1, None, Some(0.0)).elements[0].hidden);
        let f = frame(&s, HD, 1, None, Some(0.5));
        assert!(
            (f.elements[0].bounds.x - 960.0).abs() < 1e-9,
            "{}",
            f.elements[0].bounds.x
        );
        assert_eq!(f.elements[0].opacity, 1.0);
        // Out to the left: past x = -width.
        s.builds = vec![Build::new(
            id(&s, 0),
            BuildMode::Out,
            t.with_easing(Easing::Linear),
        )];
        let f = frame(&s, HD, 1, None, Some(0.5));
        assert!((f.elements[0].bounds.x + 50.0).abs() < 1e-9);
        // A zoom in grows from the centre while it fades.
        s.builds = vec![Build::new(
            id(&s, 0),
            BuildMode::In,
            Transition::new(
                TransitionKind::Zoom {
                    direction: ZoomDirection::In,
                },
                1.0,
            ),
        )];
        let f = frame(&s, HD, 1, None, Some(0.5));
        let b = f.elements[0].bounds;
        assert!((b.width - 50.0).abs() < 1e-9 && (b.x - 25.0).abs() < 1e-9);
        assert!((f.elements[0].opacity - 0.5).abs() < 1e-9);
    }
}
