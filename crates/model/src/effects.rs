//! Visual effects (on media) and transitions (between slides, media, looks, props, messages).
//!
//! Effects and transitions we implement get typed variants. Vendor effects imported from
//! ProPresenter (e.g. the "Color Warp" transition used on 6 songs) are kept losslessly as a
//! [`VendorEffect`] (name, vendor render ID, category, every parameter value): the renderer
//! substitutes a fallback (**dissolve** for transitions, **no-op** for effects) until it
//! supports them, and nothing is lost on re-save or later re-export. Source data that does not
//! even fit a vendor effect goes into `Unsupported { sourceType, payload }`.
//!
//! ## Transition resolution
//! A layer change uses the most specific transition that is set:
//! `slide.transition` → `presentation.transition` → the show's per-layer default
//! ([`crate::show::LayerTransitions`]). Media triggers, video-input triggers, props, messages
//! and looks carry their own `transition` override, which replaces the slide/presentation step
//! for their layer. See [`Transition::resolve`]. Durations are seconds.

use crate::common::{Color, is_true, yes};
use serde::{Deserialize, Serialize};

/// One effect in a media effect stack (applied in order).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    /// Disabled effects stay in the stack, so toggling restores their settings.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// What the effect does.
    pub kind: EffectKind,
}

impl Effect {
    /// An enabled effect.
    pub fn new(kind: EffectKind) -> Self {
        Effect {
            enabled: true,
            kind,
        }
    }
}

/// The effect type and its parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EffectKind {
    /// Multiply alpha by `amount` `0..=1` ("Alpha Adjust", 18 media fills in the census).
    AlphaAdjust {
        /// Alpha multiplier.
        amount: f64,
    },
    /// Brightness / contrast / saturation / hue ("Adjust Color").
    ColorAdjust {
        /// Brightness offset, `-1..=1`. 0 = unchanged.
        #[serde(default, skip_serializing_if = "crate::common::is_zero")]
        brightness: f64,
        /// Contrast multiplier, `0..=2`. 1 = unchanged.
        #[serde(
            default = "crate::common::one",
            skip_serializing_if = "crate::common::is_one"
        )]
        #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
        contrast: f64,
        /// Saturation multiplier, `0..=2`. 1 = unchanged.
        #[serde(
            default = "crate::common::one",
            skip_serializing_if = "crate::common::is_one"
        )]
        #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
        saturation: f64,
        /// Hue rotation in degrees, `-180..=180`. 0 = unchanged.
        #[serde(default, skip_serializing_if = "crate::common::is_zero")]
        hue: f64,
    },
    /// Duotone: maps luminance onto a gradient between two colours ("Two Color").
    TwoColor {
        /// Colour for dark regions.
        dark: Color,
        /// Colour for bright regions.
        bright: Color,
    },
    /// Gaussian blur.
    Blur {
        /// Blur radius, canvas px.
        radius: f64,
    },
    /// A vendor effect this build does not implement; rendered as a no-op.
    Vendor(VendorEffect),
    /// Imported data that is not even a vendor effect, kept verbatim.
    Unsupported {
        /// Source type name (e.g. `"propresenter.effect.<renderId>"`).
        source_type: String,
        /// Verbatim source JSON.
        #[serde(default, skip_serializing_if = "crate::common::is_null")]
        #[cfg_attr(feature = "ts", ts(type = "unknown", optional = nullable))]
        payload: serde_json::Value,
    },
    /// Unknown tag from a newer schema (lossy; see `crate::migrate`).
    #[serde(other)]
    Unknown,
}

/// An effect or transition defined by another product, preserved verbatim.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct VendorEffect {
    /// Human-readable name ("Color Warp").
    #[serde(skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub name: String,
    /// Vendor's stable identifier (ProPresenter `renderId`, a UUID string).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub vendor_id: Option<String>,
    /// Vendor category ("Dissolves", "Color", "Wipes").
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub category: Option<String>,
    /// Parameter values (min/max metadata is implied by `vendorId` and not kept).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub params: Vec<EffectParam>,
}

/// One named parameter value of a vendor effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EffectParam {
    /// Parameter name as the vendor spells it ("Zoom", "Direction").
    pub name: String,
    /// The value.
    pub value: ParamValue,
}

/// A vendor effect parameter value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ParamValue {
    /// Floating point value.
    Number {
        /// The value.
        value: f64,
    },
    /// Integer value (32-bit, as ProPresenter stores it; maps to a TypeScript `number`).
    Integer {
        /// The value.
        value: i32,
    },
    /// Colour value.
    Color {
        /// The value.
        value: Color,
    },
    /// Direction value.
    Direction {
        /// The value.
        value: Direction,
    },
    /// Text value.
    Text {
        /// The value.
        value: String,
    },
    /// Unknown value type from a newer schema.
    #[serde(other)]
    Unknown,
}

/// A compass direction for wipes, pushes and vendor parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    /// Toward / from the left.
    #[default]
    Left,
    /// Toward / from the right.
    Right,
    /// Toward / from the top.
    Up,
    /// Toward / from the bottom.
    Down,
    /// Toward / from the top-left.
    UpLeft,
    /// Toward / from the top-right.
    UpRight,
    /// Toward / from the bottom-left.
    DownLeft,
    /// Toward / from the bottom-right.
    DownRight,
    /// From / toward the centre.
    Center,
    /// Unknown value.
    #[serde(other)]
    Unknown,
}

/// A transition: what it looks like and how long it takes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct Transition {
    /// The visual style.
    #[serde(skip_serializing_if = "crate::common::is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub kind: TransitionKind,
    /// Duration in seconds (ignored for `cut`).
    #[serde(skip_serializing_if = "crate::common::is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub duration: f64,
    /// How progress follows time (`linear`, the default, is how every stored transition
    /// played before easing existed).
    #[serde(skip_serializing_if = "crate::common::is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub easing: Easing,
}

impl Transition {
    /// An instant cut.
    pub const CUT: Transition = Transition {
        kind: TransitionKind::Cut,
        duration: 0.0,
        easing: Easing::Linear,
    };

    /// A dissolve (cross-fade) of `seconds`.
    pub fn dissolve(seconds: f64) -> Self {
        Transition {
            kind: TransitionKind::Dissolve,
            duration: seconds,
            easing: Easing::Linear,
        }
    }

    /// A transition of `kind` over `seconds`, linear.
    pub fn new(kind: TransitionKind, seconds: f64) -> Self {
        Transition {
            kind,
            duration: seconds,
            easing: Easing::Linear,
        }
    }

    /// The same transition with `easing`.
    pub fn with_easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    /// The most specific transition that is set: `slide` → `presentation` → `layer_default`
    /// (see the module docs). Pass a trigger's / prop's / look's own override as `slide`.
    pub fn resolve<'a>(
        slide: Option<&'a Transition>,
        presentation: Option<&'a Transition>,
        layer_default: &'a Transition,
    ) -> &'a Transition {
        slide.or(presentation).unwrap_or(layer_default)
    }

    /// The transition the renderer actually plays: vendor, unsupported and unknown kinds
    /// become a dissolve of the same duration (and easing) until supported.
    pub fn renderable(&self) -> Transition {
        match self.kind {
            TransitionKind::Vendor(_)
            | TransitionKind::Unsupported { .. }
            | TransitionKind::Unknown => {
                Transition::dissolve(self.duration).with_easing(self.easing)
            }
            _ => self.clone(),
        }
    }
}

/// How a transition's progress follows time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Easing {
    /// Constant speed.
    #[default]
    Linear,
    /// Starts slowly (cubic).
    EaseIn,
    /// Ends slowly (cubic).
    EaseOut,
    /// Starts and ends slowly (cubic).
    EaseInOut,
    /// Unknown value from a newer schema (played as linear).
    #[serde(other)]
    Unknown,
}

impl Easing {
    /// Eased progress for linear progress `t` (clamped to `0..=1`).
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear | Easing::Unknown => t,
            Easing::EaseIn => t * t * t,
            Easing::EaseOut => 1.0 - (1.0 - t).powi(3),
            Easing::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

/// Which way a zoom transition goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ZoomDirection {
    /// The new content grows in from the centre; the old grows past the viewer.
    #[default]
    In,
    /// The new content shrinks onto the screen; the old shrinks away.
    Out,
    /// Unknown value from a newer schema (played as `in`).
    #[serde(other)]
    Unknown,
}

/// Transition styles.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TransitionKind {
    /// Instant switch.
    #[default]
    Cut,
    /// Cross-fade between old and new content (96% of real usage).
    Dissolve,
    /// Fade the old content out to a colour, then the new content in.
    FadeThroughColor {
        /// The intermediate colour (black for "fade through black").
        color: Color,
    },
    /// New content pushes the old out, both moving in `direction`.
    Push {
        /// Direction of motion.
        direction: Direction,
    },
    /// A moving edge reveals the new content (a diagonal reveals from a corner, `center`
    /// opens a box from the middle).
    Wipe {
        /// Direction of the edge's motion.
        direction: Direction,
    },
    /// New content slides in over the old, which stays where it is.
    Slide {
        /// Direction of the incoming content's motion.
        direction: Direction,
    },
    /// New content zooms in (or out) about the centre while the two cross-fade.
    Zoom {
        /// In or out.
        #[serde(default, skip_serializing_if = "crate::common::is_default")]
        #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
        direction: ZoomDirection,
    },
    /// A vendor transition we do not implement (rendered as a dissolve), kept verbatim.
    Vendor(VendorEffect),
    /// Imported data that is not even a vendor transition (rendered as a dissolve).
    Unsupported {
        /// Source type name.
        source_type: String,
        /// Verbatim source JSON.
        #[serde(default, skip_serializing_if = "crate::common::is_null")]
        #[cfg_attr(feature = "ts", ts(type = "unknown", optional = nullable))]
        payload: serde_json::Value,
    },
    /// Unknown tag from a newer schema (rendered as a dissolve).
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_kinds_and_easing_serialize() {
        let t = Transition::new(
            TransitionKind::Slide {
                direction: Direction::Up,
            },
            0.8,
        )
        .with_easing(Easing::EaseInOut);
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(
            v,
            json!({"kind": {"type": "slide", "direction": "up"}, "duration": 0.8, "easing": "easeInOut"})
        );
        assert_eq!(serde_json::from_value::<Transition>(v).unwrap(), t);
        // A zoom in is just its type; linear easing is omitted.
        let z = Transition::new(
            TransitionKind::Zoom {
                direction: ZoomDirection::In,
            },
            1.0,
        );
        assert_eq!(
            serde_json::to_value(&z).unwrap(),
            json!({"kind": {"type": "zoom"}, "duration": 1.0})
        );
        // An easing from a newer schema plays as linear.
        let u: Transition =
            serde_json::from_value(json!({"kind": {"type": "dissolve"}, "easing": "bounce"}))
                .unwrap();
        assert_eq!(u.easing, Easing::Unknown);
        assert_eq!(u.easing.apply(0.3), 0.3);
    }

    #[test]
    fn easing_curves() {
        for e in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
        ] {
            assert_eq!(e.apply(0.0), 0.0, "{e:?}");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-12, "{e:?}");
            assert!(e.apply(-1.0) == 0.0 && (e.apply(2.0) - 1.0).abs() < 1e-12);
        }
        assert!((Easing::EaseInOut.apply(0.5) - 0.5).abs() < 1e-12);
        assert!(Easing::EaseIn.apply(0.25) < 0.25);
        assert!(Easing::EaseOut.apply(0.25) > 0.25);
    }

    #[test]
    fn vendor_kinds_keep_their_easing_as_a_dissolve() {
        let v = Transition::new(TransitionKind::Vendor(VendorEffect::default()), 2.0)
            .with_easing(Easing::EaseOut);
        assert_eq!(
            v.renderable(),
            Transition::dissolve(2.0).with_easing(Easing::EaseOut)
        );
    }
}
