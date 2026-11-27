//! Shared low-level conversions used by every area: colours, geometry, hotkeys, links,
//! transitions, vendor effects, timestamps and writer versions.
//!
//! **Ownership:** shared, implemented by the scaffold. Signatures are frozen. The *slides* area
//! maintains the bodies (bug fixes only, announced to the other areas); every area may call
//! anything here. Nothing here touches the bundle or report except through the explicit `ctx`
//! parameter of [`transition`].

use model::{
    Color, EdgeInsets, EffectParam, HotKey, Link, ParamValue, Point, Rect, Size, Transition,
    TransitionKind, VendorEffect,
};

use crate::context::{ImportContext, ids};
use crate::pb::rv::data as pp;

/// A ProPresenter colour as-is (straight alpha, sRGB components `0..=1`).
pub fn color(c: &pp::Color) -> Color {
    Color::rgba(c.red, c.green, c.blue, c.alpha)
}

/// An optional colour; missing → `None`. Keeps alpha-0 colours (use [`visible_color`] where
/// ProPresenter's `(1,1,1,0)` means "no colour").
pub fn opt_color(c: Option<&pp::Color>) -> Option<Color> {
    c.map(color)
}

/// A colour where alpha 0 means "none" (slide labels, group colours, screen backgrounds).
pub fn visible_color(c: Option<&pp::Color>) -> Option<Color> {
    c.filter(|c| c.alpha > 0.0).map(color)
}

/// A point (missing → origin).
pub fn point(p: Option<&pp::graphics::Point>) -> Point {
    p.map(|p| Point::new(p.x, p.y)).unwrap_or_default()
}

/// A size (missing or non-positive → `None`).
pub fn size(s: Option<&pp::graphics::Size>) -> Option<Size> {
    s.filter(|s| s.width > 0.0 && s.height > 0.0)
        .map(|s| Size::new(s.width, s.height))
}

/// A rectangle in canvas px (missing parts → 0).
pub fn rect(r: Option<&pp::graphics::Rect>) -> Rect {
    let Some(r) = r else {
        return Rect::default();
    };
    let o = point(r.origin.as_ref());
    let (w, h) = r
        .size
        .as_ref()
        .map(|s| (s.width, s.height))
        .unwrap_or_default();
    Rect::new(o.x, o.y, w, h)
}

/// Edge insets in px (missing → zero).
pub fn edge_insets(e: Option<&pp::graphics::EdgeInsets>) -> EdgeInsets {
    e.map(|e| EdgeInsets {
        left: e.left,
        right: e.right,
        top: e.top,
        bottom: e.bottom,
    })
    .unwrap_or_default()
}

/// W3C `KeyboardEvent.code` for a ProPresenter key code (`KEY_CODE_ANSI_A` → `"KeyA"`).
/// Unknown / unset codes → `None`.
pub fn key_code(code: i32) -> Option<String> {
    let name = pp::KeyCode::try_from(code).ok()?.as_str_name();
    let k = name.strip_prefix("KEY_CODE_")?;
    if k == "UNKNOWN" {
        return None;
    }
    if let Some(rest) = k.strip_prefix("ANSI_KEYPAD_") {
        return Some(match rest {
            "CLEAR" => "NumLock".into(),
            "DECIMAL" => "NumpadDecimal".into(),
            "DIVIDE" => "NumpadDivide".into(),
            "ENTER" => "NumpadEnter".into(),
            "EQUALS" => "NumpadEqual".into(),
            "MINUS" => "NumpadSubtract".into(),
            "PLUS" => "NumpadAdd".into(),
            d => format!("Numpad{d}"),
        });
    }
    if let Some(rest) = k.strip_prefix("ANSI_") {
        return Some(match rest {
            c if c.len() == 1 && c.as_bytes()[0].is_ascii_uppercase() => format!("Key{c}"),
            d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => format!("Digit{d}"),
            "BACKSLASH" => "Backslash".into(),
            "COMMA" => "Comma".into(),
            "EQUAL" => "Equal".into(),
            "GRAVE" => "Backquote".into(),
            "LEFT_BRACKET" => "BracketLeft".into(),
            "RIGHT_BRACKET" => "BracketRight".into(),
            "MINUS" => "Minus".into(),
            "PERIOD" => "Period".into(),
            "QUOTE" => "Quote".into(),
            "SEMICOLON" => "Semicolon".into(),
            "SLASH" => "Slash".into(),
            other => pascal(other),
        });
    }
    Some(match k {
        f if f.starts_with('F') && f[1..].chars().all(|c| c.is_ascii_digit()) => f.to_string(),
        "RETURN" => "Enter".into(),
        "DELETE" => "Backspace".into(),
        "FORWARD_DELETE" => "Delete".into(),
        "DOWN_ARROW" => "ArrowDown".into(),
        "UP_ARROW" => "ArrowUp".into(),
        "LEFT_ARROW" => "ArrowLeft".into(),
        "RIGHT_ARROW" => "ArrowRight".into(),
        "COMMAND" => "MetaLeft".into(),
        "OPTION" => "AltLeft".into(),
        "RIGHT_OPTION" => "AltRight".into(),
        "CONTROL" => "ControlLeft".into(),
        "RIGHT_CONTROL" => "ControlRight".into(),
        "SHIFT" => "ShiftLeft".into(),
        "RIGHT_SHIFT" => "ShiftRight".into(),
        "VOLUME_UP" => "AudioVolumeUp".into(),
        "VOLUME_DOWN" => "AudioVolumeDown".into(),
        "MUTE" => "AudioVolumeMute".into(),
        "FUNCTION" => "Fn".into(),
        other => pascal(other),
    })
}

/// `PAGE_DOWN` → `PageDown`.
fn pascal(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase(),
                None => String::new(),
            }
        })
        .collect()
}

/// A hotkey (no modifiers: ProPresenter stores only the key). Unset → `None`.
pub fn hotkey(h: Option<&pp::HotKey>) -> Option<HotKey> {
    key_code(h?.code).map(HotKey::key)
}

/// A link from a `CollectionElementType` (`parameterUuid` + `parameterName`). `None` when the
/// UUID is missing. The link may dangle; the name is kept as a hint.
pub fn collection_link<T: From<uuid::Uuid>>(
    c: Option<&pp::CollectionElementType>,
) -> Option<Link<T>> {
    let c = c?;
    let id = ids::parse_uuid(c.parameter_uuid.as_ref())?;
    Some(if c.parameter_name.is_empty() {
        Link::to(T::from(id))
    } else {
        Link::named(T::from(id), c.parameter_name.clone())
    })
}

/// `"major.minor.patch"` of the app that wrote a document.
pub fn writer_version(app: Option<&pp::ApplicationInfo>) -> Option<String> {
    let v = app?.application_version.as_ref()?;
    if v.major_version == 0 && v.minor_version == 0 && v.patch_version == 0 {
        return None;
    }
    Some(format!(
        "{}.{}.{}",
        v.major_version, v.minor_version, v.patch_version
    ))
}

/// A protobuf timestamp as an RFC 3339 UTC string (`None` for unset / zero).
pub fn timestamp(t: Option<&pp::Timestamp>) -> Option<String> {
    let t = t?;
    if t.seconds == 0 && t.nanos == 0 {
        return None;
    }
    let days = t.seconds.div_euclid(86_400);
    let secs = t.seconds.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    ))
}

/// A vendor effect / transition with every parameter value (min / max / defaults are implied by
/// the vendor ID and dropped).
pub fn vendor_effect(e: &pp::Effect) -> VendorEffect {
    use pp::effect::effect_variable::Type;
    let params = e
        .variables
        .iter()
        .filter_map(|v| {
            let value = match v.r#type.as_ref()? {
                Type::Int(i) => ParamValue::Integer { value: i.value },
                Type::Float(f) => ParamValue::Number {
                    value: f64::from(f.value),
                },
                Type::Double(d) => ParamValue::Number { value: d.value },
                Type::Color(c) => ParamValue::Color {
                    value: c.color.as_ref().map(color).unwrap_or_default(),
                },
                Type::Direction(d) => ParamValue::Direction {
                    value: direction(d.direction),
                },
            };
            Some(EffectParam {
                name: v.name.clone(),
                value,
            })
        })
        .collect();
    VendorEffect {
        name: e.name.clone(),
        vendor_id: (!e.render_id.is_empty()).then(|| e.render_id.clone()),
        category: (!e.category.is_empty()).then(|| e.category.clone()),
        params,
    }
}

/// ProPresenter effect direction bit → model direction.
pub fn direction(d: i32) -> model::Direction {
    use model::Direction as D;
    match d {
        1 => D::UpLeft,
        2 => D::Up,
        4 => D::UpRight,
        8 => D::Left,
        16 => D::Center,
        32 => D::Right,
        64 => D::DownLeft,
        128 => D::Down,
        256 => D::DownRight,
        _ => D::Unknown,
    }
}

/// A transition. Effect names "Cut" / "Dissolve" map to the typed kinds, and so do the moving
/// ones the renderer draws: "Push", "Wipe", "Slide" (also "Cover", "Move In"), "Zoom" ("Zoom
/// In", "Zoom Out"), with the effect's first direction parameter (left when it has none).
/// Every other effect is kept as [`TransitionKind::Vendor`] (rendered as a dissolve until
/// supported). A transition without an effect is a dissolve when it has a duration, else a cut.
pub fn transition(ctx: &mut ImportContext, t: &pp::Transition) -> Transition {
    let _ = ctx; // reserved for warnings about odd transitions
    let kind = match t.effect.as_ref() {
        None if t.duration > 0.0 => TransitionKind::Dissolve,
        None => TransitionKind::Cut,
        Some(e) => {
            let direction = || {
                e.variables
                    .iter()
                    .find_map(|v| match v.r#type.as_ref()? {
                        pp::effect::effect_variable::Type::Direction(d) => {
                            Some(direction(d.direction))
                        }
                        _ => None,
                    })
                    .filter(|d| *d != model::Direction::Unknown)
                    .unwrap_or(model::Direction::Left)
            };
            match e.name.trim().to_ascii_lowercase().as_str() {
                "cut" => TransitionKind::Cut,
                "dissolve" => TransitionKind::Dissolve,
                "push" => TransitionKind::Push {
                    direction: direction(),
                },
                "wipe" => TransitionKind::Wipe {
                    direction: direction(),
                },
                "slide" | "cover" | "move in" => TransitionKind::Slide {
                    direction: direction(),
                },
                "zoom" | "zoom in" => TransitionKind::Zoom {
                    direction: model::ZoomDirection::In,
                },
                "zoom out" => TransitionKind::Zoom {
                    direction: model::ZoomDirection::Out,
                },
                _ => TransitionKind::Vendor(vendor_effect(e)),
            }
        }
    };
    Transition {
        duration: if matches!(kind, TransitionKind::Cut) {
            0.0
        } else {
            t.duration
        },
        kind,
        easing: model::Easing::Linear,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_codes() {
        assert_eq!(key_code(pp::KeyCode::AnsiA as i32).as_deref(), Some("KeyA"));
        assert_eq!(
            key_code(pp::KeyCode::Ansi1 as i32).as_deref(),
            Some("Digit1")
        );
        assert_eq!(key_code(pp::KeyCode::F5 as i32).as_deref(), Some("F5"));
        assert_eq!(
            key_code(pp::KeyCode::PageDown as i32).as_deref(),
            Some("PageDown")
        );
        assert_eq!(key_code(pp::KeyCode::Unknown as i32), None);
        assert_eq!(key_code(-5), None);
    }

    #[test]
    fn timestamps() {
        let t = pp::Timestamp {
            seconds: 1_760_866_200,
            nanos: 0,
        };
        assert_eq!(timestamp(Some(&t)).as_deref(), Some("2025-10-19T09:30:00Z"));
        assert_eq!(timestamp(Some(&pp::Timestamp::default())), None);
    }

    #[test]
    fn transitions() {
        let mut ctx = ImportContext::new(
            std::path::Path::new("/nonexistent"),
            crate::ImportOptions::default(),
        );
        let t = pp::Transition {
            duration: 0.3,
            effect: Some(pp::Effect {
                name: "Dissolve".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(transition(&mut ctx, &t), Transition::dissolve(0.3));
        let v = pp::Transition {
            duration: 1.0,
            effect: Some(pp::Effect {
                name: "Example Warp".into(),
                render_id: "ABC".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(matches!(
            transition(&mut ctx, &v).kind,
            TransitionKind::Vendor(VendorEffect { ref vendor_id, .. }) if vendor_id.as_deref() == Some("ABC")
        ));
        // Moving kinds the renderer draws are typed, with their direction.
        let named = |name: &str, dir: Option<i32>| pp::Transition {
            duration: 0.8,
            effect: Some(pp::Effect {
                name: name.into(),
                variables: dir
                    .map(|d| pp::effect::EffectVariable {
                        name: "Direction".into(),
                        r#type: Some(pp::effect::effect_variable::Type::Direction(
                            pp::effect::effect_variable::EffectDirection {
                                direction: d,
                                ..Default::default()
                            },
                        )),
                        ..Default::default()
                    })
                    .into_iter()
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            transition(&mut ctx, &named("Push", Some(2))),
            Transition::new(
                TransitionKind::Push {
                    direction: model::Direction::Up
                },
                0.8
            )
        );
        assert_eq!(
            transition(&mut ctx, &named("Wipe", None)).kind,
            TransitionKind::Wipe {
                direction: model::Direction::Left
            }
        );
        assert_eq!(
            transition(&mut ctx, &named("Slide", Some(32))).kind,
            TransitionKind::Slide {
                direction: model::Direction::Right
            }
        );
        assert_eq!(
            transition(&mut ctx, &named("Zoom Out", None)).kind,
            TransitionKind::Zoom {
                direction: model::ZoomDirection::Out
            }
        );
    }
}
