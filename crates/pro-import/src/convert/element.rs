//! **Area: slides.** Slide elements: geometry, shape, decorations, placeholders, data links and
//! visibility conditions.
//!
//! ## Owns
//! - [`elements`] / [`element`]: `rv.data.Slide.Element` (wrapper with `info`, builds, data
//!   links, text scroller) around a `Graphics.Element`. Element ID = element UUID (UUIDv5 of the
//!   document path and the element bytes when it has none).
//!   - bounds / rotation / opacity / flip / hidden / locked / aspectLocked;
//!   - `path` → [`shape`] (parametric when the shape type says so and its parameters are present,
//!     else a custom `Shape::Path` from the Bézier points: `q0` = incoming handle, `q1` =
//!     outgoing handle, unit coordinates);
//!   - [`fill`] (colour, gradient, media → `media::media_fill`, background effect, cut-out),
//!     [`stroke`], [`shadow`] (polar, verbatim), [`feather`]. Disabled decorations keep their
//!     settings (they round-trip; the model omits them when equal to its defaults);
//!   - `text` → `text::text_box`; `mask` (`textLineMask`) → `text::line_mask` on the text box.
//!     ProPresenter draws the line boxes with the element fill, so for an enabled mask the fill
//!     colour (first stop of a gradient) moves into `LineMask.fill` and the element fill is
//!     disabled (its settings stay; `import.unmapped.lineMaskFill` records the move; a mask over
//!     a disabled fill draws nothing and is turned off, `import.unmapped.lineMaskEnabled`). An element
//!     whose text is completely empty, that is not a text placeholder and shows an enabled media
//!     fill (PowerPoint slide pictures) becomes a plain shape (counted as `element.emptyText`):
//!     its empty text box usually only holds ProPresenter's default typing attributes (ArialMT
//!     50, black; `default_typing`); other attributes (some theme pictures) are kept verbatim
//!     as `import.unmapped.emptyTextAttributes`;
//!   - `info` bits ([`apply_info`]): 1 → `themed`, 2 → `Placeholder { slot: element name or
//!     "text<N>", accepts: Text }` ([`SlotNamer`]), 4 → `ticker`; any other bit → raw `info` in
//!     `import.unmapped`;
//!   - `dataLinks` → [`data_links`] (one `DataLink`, any number of `VisibilityCondition`s; extra
//!     data links → `import.unmapped.dataLinks`, unmodelled kinds → `DataLink::Unsupported` /
//!     `VisibilityCondition::Unsupported`);
//!   - `buildIn` / `buildOut` → the slide's `builds` ([`builds`], ordered by the slide's
//!     `elementBuildOrder`);
//!   - `childBuilds` / `textScroller` (when not ProPresenter's default
//!     scroller, or on a ticker) / `revealType` → `import.unmapped` via
//!     `ctx.keep_unmapped("element.<field>", ctx.to_json(..))`.
//! - [`graphics_element`]: a bare `Graphics.Element` (mask shapes).
//! - [`timer_format`], [`clock_format`], [`timer_link`]: shared with the config area (messages).
//!
//! ## May call
//! `common::*`, `text::*`, `fonts::*`, `media::media_fill`, `ImportContext` helpers.

use model::{
    BackgroundEffect, ClockFormat, ColorTrigger, CountdownSource, DataLink, DateTimeStyle, Element,
    ElementId, ElementKind, Feather, FeatherStyle, Fill, FillKind, Flip, Gradient, GradientKind,
    GradientStop, Link, MediaCondition, PathPoint, Placeholder, PlaylistItemWhich, SectionWhich,
    Shadow, Shape, ShapePath, SlideCountKind, SlideTextSource, SlideWhich, Stroke, StrokeStyle,
    TimeUnitStyle, TimerFormat, TimerId, TimerState, VideoInputId, VisibilityCondition,
};
use prost::Message as _;
use serde_json::Value;

use crate::context::{ImportContext, ids};
use crate::convert::{common, media, text};
use crate::pb::rv::data as pp;
use crate::report::Category;

use pp::slide::element::data_link as dl;

/// Derives placeholder slot names within one slide: the element name when it has one (lowercased,
/// unique within the slide), else `"text1"`, `"text2"`, … by placeholder order.
#[derive(Clone, Debug, Default)]
pub struct SlotNamer {
    /// Placeholders named so far (for the `text<N>` counter).
    pub count: usize,
    /// Slot names already used in this slide.
    pub used: Vec<String>,
}

impl SlotNamer {
    /// A fresh namer for one slide.
    pub fn new() -> Self {
        SlotNamer::default()
    }

    /// The slot name for the next placeholder element named `element_name`.
    ///
    /// Named elements use their lowercased, trimmed name (`"Verse"` → `"verse"`); unnamed ones
    /// take `"text<N>"` with N counting every placeholder of the slide so far, so slot names
    /// follow placeholder order. A clash appends `"2"`, `"3"`, ….
    pub fn next_slot(&mut self, element_name: &str) -> String {
        self.count += 1;
        let name = element_name.trim().to_lowercase();
        let base = if name.is_empty() {
            format!("text{}", self.count)
        } else {
            name
        };
        let mut slot = base.clone();
        let mut n = 2;
        while self.used.contains(&slot) {
            slot = format!("{base}{n}");
            n += 1;
        }
        self.used.push(slot.clone());
        slot
    }
}

/// Converts a slide's elements in z-order (first = bottom) with one [`SlotNamer`].
/// The object builds of a slide's elements (`els` as stored, `out` their converted elements,
/// back to front): each element's `buildIn` / `buildOut` (start, delay, transition), in the
/// slide's `elementBuildOrder` (build UUIDs); builds it does not list follow in element order.
pub fn builds(
    ctx: &mut ImportContext,
    els: &[pp::slide::Element],
    out: &[Element],
    order: &[pp::Uuid],
) -> Vec<model::Build> {
    use pp::slide::element::build::Start;
    let mut found: Vec<(String, model::Build)> = Vec::new();
    for (i, el) in els.iter().enumerate() {
        let Some(e) = out.len().checked_sub(i + 1).and_then(|k| out.get(k)) else {
            continue;
        };
        for (mode, b) in [
            (model::BuildMode::In, el.build_in.as_ref()),
            (model::BuildMode::Out, el.build_out.as_ref()),
        ] {
            let Some(b) = b.filter(|b| **b != pp::slide::element::Build::default()) else {
                continue;
            };
            let start = match Start::try_from(b.start) {
                Ok(Start::OnClick) => model::BuildStart::OnNext,
                Ok(Start::WithPrevious) => model::BuildStart::WithPrevious,
                Ok(Start::AfterPrevious) => model::BuildStart::AfterPrevious,
                Ok(Start::WithSlide) => model::BuildStart::WithSlide,
                Err(_) => model::BuildStart::OnNext,
            };
            let transition = match b.transition.as_ref() {
                Some(t) => common::transition(ctx, t),
                None => model::Transition::CUT,
            };
            let uuid = b
                .uuid
                .as_ref()
                .map(|u| u.string.clone())
                .unwrap_or_default();
            found.push((
                uuid,
                model::Build {
                    element: e.id,
                    mode,
                    start,
                    delay: b.delay_time.max(0.0),
                    transition,
                },
            ));
        }
    }
    let mut builds = Vec::with_capacity(found.len());
    for u in order {
        if let Some(k) = found
            .iter()
            .position(|(id, _)| !id.is_empty() && id.eq_ignore_ascii_case(&u.string))
        {
            builds.push(found.remove(k).1);
        }
    }
    builds.extend(found.into_iter().map(|(_, b)| b));
    builds
}

pub fn elements(ctx: &mut ImportContext, els: &[pp::slide::Element]) -> Vec<Element> {
    let mut slots = SlotNamer::new();
    // ProPresenter stores elements front to back (first = topmost; its stock themes list the
    // "Background" picture last), while the model is back to front (first = bottom). Slot names
    // are still assigned in ProPresenter's order so "text1" stays the topmost text box.
    let mut out: Vec<Element> = els.iter().map(|e| element(ctx, e, &mut slots)).collect();
    out.reverse();
    out
}

/// A stable 64-bit FNV-1a hash (IDs for elements without a UUID).
fn fnv1a(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn element_id(ctx: &ImportContext, uuid: Option<&pp::Uuid>, bytes: &[u8]) -> ElementId {
    let path = ctx.current.path.clone().unwrap_or_default();
    ElementId::from_uuid(ids::source_or_derived(
        uuid,
        "element",
        &[&path, &fnv1a(bytes)],
    ))
}

/// True when a text scroller is ProPresenter's untouched default (only `repeatDistance` varies
/// with the element size).
fn default_scroller(s: &pp::slide::element::TextScroller) -> bool {
    let base = !s.should_scroll
        && s.scrolling_direction == 0
        && !s.starts_off_screen
        && s.fade_left == 0.0
        && s.fade_right == 0.0;
    base && ((s.should_repeat && (s.scroll_rate - 0.5).abs() < 1e-9)
        || *s == pp::slide::element::TextScroller::default())
}

/// One `Slide.Element` (see the module docs).
pub fn element(ctx: &mut ImportContext, el: &pp::slide::Element, slots: &mut SlotNamer) -> Element {
    let mut e = match el.element.as_ref() {
        Some(g) => graphics_inner(ctx, g, el.info & 2 != 0),
        None => {
            ctx.warn(Category::Other, "slide element without graphics element");
            Element {
                id: element_id(ctx, None, &el.encode_to_vec()),
                ..Element::default()
            }
        }
    };
    apply_info(ctx, el.info, &mut e, slots);

    // Data links: the first non-visibility link is the element's; visibility links become
    // conditions; further links are kept verbatim.
    let mut used: Vec<pp::slide::element::DataLink> = Vec::new();
    let mut extra: Vec<Value> = Vec::new();
    let mut have_link = false;
    for l in &el.data_links {
        let is_visibility = matches!(l.property_type, Some(dl::PropertyType::VisibilityLink(_)));
        if is_visibility || !have_link {
            have_link |= !is_visibility;
            used.push(l.clone());
        } else {
            extra.push(ctx.to_json(l));
        }
    }
    let (link, conditions) = data_links(ctx, &used);
    e.data_link = link;
    e.visibility = conditions;
    if !extra.is_empty() {
        ctx.keep_unmapped(&mut e.import, "element.dataLinks", Value::Array(extra));
    }

    // Builds (mapped by [`builds`]), reveal, scroller.
    if !el.child_builds.is_empty() {
        let v = el.child_builds.iter().map(|b| ctx.to_json(b)).collect();
        ctx.keep_unmapped(&mut e.import, "element.childBuilds", Value::Array(v));
    }
    if el.reveal_type != 0 || el.reveal_from_index != 0 {
        let v = serde_json::json!({
            "revealType": pp::slide::element::TextRevealType::try_from(el.reveal_type)
                .map(|t| t.as_str_name().to_string())
                .unwrap_or_else(|_| el.reveal_type.to_string()),
            "revealFromIndex": el.reveal_from_index,
        });
        ctx.keep_unmapped(&mut e.import, "element.revealType", v);
    }
    if let Some(s) = el
        .text_scroller
        .as_ref()
        .filter(|s| e.ticker || !default_scroller(s))
    {
        e.scroll = text_scroll(s);
        let v = ctx.to_json(s);
        ctx.keep_unmapped(&mut e.import, "element.textScroller", v);
    }
    e
}

/// A text scroller as [`model::TextScroll`]: the rate `0..=1` is taken as 240 canvas px/s at
/// full speed (ProPresenter's default 0.5 → 120 px/s); "starts off screen" → `offscreen`, else
/// `auto`. Fades and the repeat distance stay in `import.unmapped` only.
fn text_scroll(s: &pp::slide::element::TextScroller) -> model::TextScroll {
    use pp::slide::element::text_scroller::Direction as D;
    model::TextScroll {
        enabled: s.should_scroll,
        direction: match D::try_from(s.scrolling_direction) {
            Ok(D::Right) => model::ScrollDirection::Right,
            Ok(D::Up) => model::ScrollDirection::Up,
            Ok(D::Down) => model::ScrollDirection::Down,
            _ => model::ScrollDirection::Left,
        },
        speed: (s.scroll_rate.clamp(0.0, 1.0) * 240.0).max(10.0),
        start: if s.starts_off_screen {
            model::ScrollStart::Offscreen
        } else {
            model::ScrollStart::Auto
        },
        repeat: s.should_repeat,
        ..model::TextScroll::default()
    }
}

/// A bare `Graphics.Element` (no wrapper: no info bits, data links or builds).
pub fn graphics_element(ctx: &mut ImportContext, g: &pp::graphics::Element) -> Element {
    graphics_inner(ctx, g, false)
}

/// [`graphics_element`]; `text_slot` = the wrapper marks it as a text placeholder (which keeps
/// an empty text box on a picture element).
fn graphics_inner(ctx: &mut ImportContext, g: &pp::graphics::Element, text_slot: bool) -> Element {
    let id = match ids::parse_uuid(g.uuid.as_ref()) {
        Some(u) => ElementId::from_uuid(u),
        None => element_id(ctx, None, &g.encode_to_vec()),
    };
    let shape = shape(ctx, g.path.as_ref());
    let mut e = Element {
        id,
        name: g.name.clone(),
        bounds: common::rect(g.bounds.as_ref()),
        rotation: g.rotation,
        opacity: g.opacity.clamp(0.0, 1.0),
        flip: flip(g.flip_mode),
        hidden: g.hidden,
        locked: g.locked,
        aspect_locked: g.aspect_ratio_locked,
        kind: ElementKind::Shape {
            shape: shape.clone(),
        },
        fill: fill(ctx, g.fill.as_ref()),
        stroke: stroke(g.stroke.as_ref()),
        shadow: shadow(g.shadow.as_ref()),
        feather: feather(g.feather.as_ref()),
        ..Element::default()
    };
    let mask = g
        .mask
        .as_ref()
        .map(|pp::graphics::element::Mask::TextLineMask(m)| m);
    if let Some(t) = g.text.as_ref() {
        let ti = text::text_box(ctx, t);
        let empty_text = ti.text.paragraphs.iter().all(|p| p.runs.is_empty());
        let picture = g.fill.as_ref().is_some_and(|f| {
            f.enable && matches!(f.fill_type, Some(pp::graphics::fill::FillType::Media(_)))
        });
        if empty_text && picture && !text_slot {
            // Kept as a shape (see the module docs).
            ctx.count_unmapped("element.emptyText");
            if let Some(a) = t.attributes.as_ref().filter(|a| !default_typing(a)) {
                let v = ctx.to_json(a);
                ctx.keep_unmapped(&mut e.import, "element.emptyTextAttributes", v);
            }
            e.kind = ElementKind::Shape { shape };
            for (feature, value) in ti.unmapped {
                ctx.keep_unmapped(&mut e.import, &feature, value);
            }
        } else {
            let mut tb = ti.text;
            for (feature, value) in ti.unmapped {
                ctx.keep_unmapped(&mut e.import, &feature, value);
            }
            if let Some(m) = mask {
                tb.line_mask = text::line_mask(m);
                if tb.line_mask.enabled {
                    move_fill_to_line_mask(ctx, &mut e, &mut tb);
                }
            }
            e.kind = ElementKind::Text { shape, text: tb };
        }
    }
    if let Some(m) = mask.filter(|m| m.enabled)
        && !matches!(e.kind, ElementKind::Text { .. })
    {
        let v = ctx.to_json(m);
        ctx.keep_unmapped(&mut e.import, "element.textLineMask", v);
    }
    e
}

/// Whether text `attributes` are ProPresenter's default typing attributes for a new text box
/// (ArialMT 50, opaque black solid fill); dropped with an empty picture text box.
fn default_typing(a: &pp::graphics::text::Attributes) -> bool {
    use pp::graphics::text::attributes::Fill as F;
    let font = a
        .font
        .as_ref()
        .is_none_or(|f| f.name == "ArialMT" && (f.size - 50.0).abs() < 1e-6);
    let fill = match a.fill.as_ref() {
        None => true,
        Some(F::TextSolidFill(c)) => {
            c.red == 0.0 && c.green == 0.0 && c.blue == 0.0 && (c.alpha - 1.0).abs() < 1e-6
        }
        Some(_) => false,
    };
    font && fill
}

/// Moves the element fill into the text line mask (ProPresenter draws the mask boxes with it).
fn move_fill_to_line_mask(ctx: &mut ImportContext, e: &mut Element, tb: &mut model::TextBox) {
    if !e.fill.enabled {
        // No fill, no boxes.
        tb.line_mask.enabled = false;
        ctx.keep_unmapped(&mut e.import, "element.lineMaskEnabled", Value::Bool(true));
        return;
    }
    let color = match &e.fill.kind {
        FillKind::Color { color } => Some(*color),
        FillKind::Gradient { gradient } => {
            ctx.count_unmapped("element.lineMaskGradient");
            gradient.stops.first().map(|s| s.color)
        }
        _ => None,
    };
    match color {
        Some(c) => {
            tb.line_mask.fill = c;
            e.fill.enabled = false;
            ctx.keep_unmapped(&mut e.import, "element.lineMaskFill", Value::Bool(true));
        }
        None => {
            ctx.count_unmapped("element.lineMaskMediaFill");
        }
    }
}

/// Applies `Slide.Element.info` bits to `element` (themed / placeholder / ticker; unknown bits →
/// `import.unmapped.info`).
pub fn apply_info(
    ctx: &mut ImportContext,
    info: u32,
    element: &mut Element,
    slots: &mut SlotNamer,
) {
    const TEMPLATE: u32 = 1;
    const TEXT: u32 = 2;
    const TICKER: u32 = 4;
    element.themed = info & TEMPLATE != 0;
    element.ticker = info & TICKER != 0;
    if info & TEXT != 0 {
        element.placeholder = Some(Placeholder::text(slots.next_slot(&element.name)));
    }
    if info & !(TEMPLATE | TEXT | TICKER) != 0 {
        ctx.keep_unmapped(&mut element.import, "element.info", Value::from(info));
    }
}

/// Element outline from `Graphics.Path` (`None` → rectangle).
pub fn shape(ctx: &mut ImportContext, path: Option<&pp::graphics::Path>) -> Shape {
    use pp::graphics::path::shape::{AdditionalData as A, Type};
    let Some(p) = path else {
        return Shape::Rectangle;
    };
    let ty = p
        .shape
        .as_ref()
        .and_then(|s| Type::try_from(s.r#type).ok())
        .unwrap_or(Type::Unknown);
    let data = p.shape.as_ref().and_then(|s| s.additional_data.as_ref());
    let parametric = match (ty, data) {
        (Type::Rectangle, _) => Some(Shape::Rectangle),
        (Type::Ellipse, _) => Some(Shape::Ellipse),
        (Type::IsoscelesTriangle, _) => Some(Shape::IsoscelesTriangle),
        (Type::RightTriangle, _) => Some(Shape::RightTriangle),
        (Type::Rhombus, _) => Some(Shape::Rhombus),
        (Type::RoundedRectangle, Some(A::RoundedRectangle(r))) => Some(Shape::RoundedRectangle {
            roundness: r.roundness,
        }),
        (Type::Star, Some(A::Star(s))) if s.number_points >= 2 => Some(Shape::Star {
            points: s.number_points,
            inner_radius: s.inner_radius,
        }),
        (Type::Polygon, Some(A::Polygon(s))) if s.number_sides >= 3 => Some(Shape::Polygon {
            sides: s.number_sides,
        }),
        (Type::RightArrow, Some(A::Arrow(a))) => Some(Shape::RightArrow {
            corner: common::point(a.corner.as_ref()),
        }),
        (Type::DoubleArrow, Some(A::Arrow(a))) => Some(Shape::DoubleArrow {
            corner: common::point(a.corner.as_ref()),
        }),
        _ => None,
    };
    if let Some(s) = parametric {
        return s;
    }
    if p.points.is_empty() {
        if !matches!(ty, Type::Unknown | Type::Custom) {
            ctx.warn(
                Category::Other,
                format!("{} shape without parameters or points", ty.as_str_name()),
            );
        }
        return Shape::Rectangle;
    }
    let points: Vec<PathPoint> = p
        .points
        .iter()
        .map(|b| PathPoint {
            point: common::point(b.point.as_ref()),
            handle_in: common::point(b.q0.as_ref()),
            handle_out: common::point(b.q1.as_ref()),
            curved: b.curved,
        })
        .collect();
    if ty == Type::Unknown && is_unit_rect(&points) {
        return Shape::Rectangle;
    }
    Shape::Path {
        path: ShapePath {
            closed: p.closed,
            points,
        },
    }
}

/// Four straight corners of the unit square.
fn is_unit_rect(points: &[PathPoint]) -> bool {
    if points.len() != 4 || points.iter().any(|p| p.curved) {
        return false;
    }
    let mut corners: Vec<(i64, i64)> = points
        .iter()
        .map(|p| {
            (
                (p.point.x * 1e6).round() as i64,
                (p.point.y * 1e6).round() as i64,
            )
        })
        .collect();
    corners.sort_unstable();
    corners
        == [
            (0, 0),
            (0, 1_000_000),
            (1_000_000, 0),
            (1_000_000, 1_000_000),
        ]
}

/// Element fill. Media fills go through `media::media_fill` with `ctx.current.asset_scope`.
pub fn fill(ctx: &mut ImportContext, f: Option<&pp::graphics::Fill>) -> Fill {
    use pp::graphics::fill::FillType;
    let Some(f) = f else {
        return Fill::default();
    };
    let kind = match f.fill_type.as_ref() {
        None => FillKind::default(),
        Some(FillType::Color(c)) => FillKind::Color {
            color: common::color(c),
        },
        Some(FillType::Gradient(g)) => FillKind::Gradient {
            gradient: gradient(g),
        },
        Some(FillType::Media(m)) => {
            let scope = ctx.current.asset_scope.clone();
            media::media_fill(ctx, m, &scope)
        }
        Some(FillType::BackgroundEffect(e)) => match background_effect(e) {
            Some(effect) => FillKind::BackgroundEffect { effect },
            None => FillKind::Unsupported {
                source_type: "propresenter.fill.backgroundEffect".into(),
                payload: ctx.to_json(e),
            },
        },
    };
    Fill {
        enabled: f.enable,
        kind,
    }
}

/// A gradient (angle, length, stops with `blendPoint` → `midpoint`).
///
/// A zero `length` or `blendPoint` is the protobuf default of an unset field and maps to the
/// model default (full length, linear midpoint). Stops are ordered by position.
pub fn gradient(g: &pp::graphics::Gradient) -> Gradient {
    let mut stops: Vec<GradientStop> = g
        .stops
        .iter()
        .map(|s| GradientStop {
            color: s.color.as_ref().map(common::color).unwrap_or_default(),
            position: s.position,
            midpoint: if s.blend_point > 0.0 {
                s.blend_point
            } else {
                0.5
            },
        })
        .collect();
    stops.sort_by(|a, b| a.position.total_cmp(&b.position));
    Gradient {
        kind: match g.r#type {
            0 => GradientKind::Linear,
            1 => GradientKind::Radial,
            2 => GradientKind::Angle,
            _ => GradientKind::Unknown,
        },
        angle: g.angle,
        length: if g.length > 0.0 { g.length } else { 1.0 },
        stops,
    }
}

/// A background effect (blur / invert); `None` when the oneof is unset.
pub fn background_effect(e: &pp::graphics::BackgroundEffect) -> Option<BackgroundEffect> {
    use pp::graphics::background_effect::EffectType;
    match e.effect_type.as_ref()? {
        EffectType::BackgroundBlur(b) => Some(BackgroundEffect::Blur {
            amount: b.blur_amount,
            saturation: b.saturation,
        }),
        EffectType::BackgroundInvert(_) => Some(BackgroundEffect::Invert),
    }
}

/// Element stroke (missing → model default).
pub fn stroke(s: Option<&pp::graphics::Stroke>) -> Stroke {
    let Some(s) = s else {
        return Stroke::default();
    };
    Stroke {
        enabled: s.enable,
        width: s.width,
        color: s
            .color
            .as_ref()
            .map(common::color)
            .unwrap_or(Stroke::default().color),
        style: match s.style {
            0 => StrokeStyle::Solid,
            1 => StrokeStyle::SquareDash,
            2 => StrokeStyle::ShortDash,
            3 => StrokeStyle::LongDash,
            _ => StrokeStyle::Unknown,
        },
        pattern: s.pattern.clone(),
    }
}

/// Element or text shadow, polar and verbatim (missing → model default).
pub fn shadow(s: Option<&pp::graphics::Shadow>) -> Shadow {
    let Some(s) = s else {
        return Shadow::default();
    };
    Shadow {
        enabled: s.enable,
        angle: s.angle,
        offset: s.offset,
        radius: s.radius,
        color: s
            .color
            .as_ref()
            .map(common::color)
            .unwrap_or(model::Color::BLACK),
        opacity: s.opacity,
    }
}

/// Element feather (missing → model default).
pub fn feather(f: Option<&pp::graphics::Feather>) -> Feather {
    let Some(f) = f else {
        return Feather::default();
    };
    Feather {
        enabled: f.enable,
        style: match f.style {
            0 => FeatherStyle::Inside,
            1 => FeatherStyle::Center,
            2 => FeatherStyle::Outside,
            _ => FeatherStyle::Unknown,
        },
        radius: f.radius,
    }
}

/// `FlipMode` → [`Flip`].
pub fn flip(mode: i32) -> Flip {
    Flip {
        horizontal: mode == 2 || mode == 3,
        vertical: mode == 1 || mode == 3,
    }
}

/// The oneof field name of a data link (`"rssFeed"`), from its JSON dump.
fn link_kind(ctx: &ImportContext, l: &pp::slide::element::DataLink) -> (String, Value) {
    let v = ctx.to_json(l);
    let kind = v
        .as_object()
        .and_then(|m| m.keys().next().cloned())
        .unwrap_or_else(|| "unknown".into());
    (kind, v)
}

fn slide_which(v: i32) -> SlideWhich {
    match v {
        0 => SlideWhich::Current,
        1 => SlideWhich::Next,
        _ => SlideWhich::Unknown,
    }
}

fn color_triggers(t: &[dl::ColorTrigger]) -> Vec<ColorTrigger> {
    t.iter()
        .map(|c| ColorTrigger {
            remaining: c.time,
            color: c.color.as_ref().map(common::color).unwrap_or_default(),
        })
        .collect()
}

/// Data links of one element → (the element's data link, its visibility conditions). Timer links
/// resolve through [`timer_link`]; element-visibility links reference other elements by UUID
/// (`ElementId::from_uuid`).
///
/// Visibility links with the `ALL` criterion add each condition; `ANY` / `NONE` with a single
/// condition become that condition (inverted for `NONE` when the model can express it); other
/// combinations become one `VisibilityCondition::Unsupported`. When several non-visibility links
/// are given, the first one wins (the element converter keeps the rest in `import.unmapped`).
pub fn data_links(
    ctx: &mut ImportContext,
    links: &[pp::slide::element::DataLink],
) -> (Option<DataLink>, Vec<VisibilityCondition>) {
    use dl::PropertyType as P;
    let mut link: Option<DataLink> = None;
    let mut conditions = Vec::new();
    for l in links {
        let Some(p) = l.property_type.as_ref() else {
            continue;
        };
        if let P::VisibilityLink(v) = p {
            conditions.extend(visibility_link(ctx, l, v));
            continue;
        }
        if link.is_some() {
            continue;
        }
        let converted = match p {
            P::TimerText(t) => Some(DataLink::Timer {
                timer: timer_link(ctx, t.timer_uuid.as_ref(), &t.timer_name),
                format: timer_format(t.timer_format.as_ref(), &t.timer_format_string),
                color_triggers: color_triggers(&t.color_triggers),
                show: Default::default(),
            }),
            P::ClockText(c) => Some(DataLink::Clock {
                format: clock_format(c.format.as_ref(), &c.clock_format_string),
            }),
            P::SlideText(s) if s.element_text_transform == 0 => match s.source_option {
                0 => Some(DataLink::SlideText {
                    which: slide_which(s.source_slide),
                    source: SlideTextSource::AllText,
                    keep_formatting: false,
                }),
                1 if !s.preserve_notes_format => Some(DataLink::SlideNotes {
                    which: slide_which(s.source_slide),
                }),
                2 => Some(DataLink::SlideText {
                    which: slide_which(s.source_slide),
                    source: SlideTextSource::ElementNamed {
                        name: s.name_to_match.clone(),
                    },
                    keep_formatting: false,
                }),
                _ => None,
            },
            P::SlideImage(s) => Some(DataLink::SlideImage {
                which: slide_which(s.source_slide),
            }),
            P::SlideLabelText(s) => Some(DataLink::SlideLabel {
                which: slide_which(s.slide_label_source),
            }),
            P::GroupName(g) => Some(DataLink::SectionName {
                which: match g.group_source {
                    0 => SectionWhich::CurrentSlide,
                    1 => SectionWhich::NextSlide,
                    2 => SectionWhich::NextSection,
                    _ => SectionWhich::Unknown,
                },
            }),
            P::SlideCount(c) => Some(DataLink::SlideCount {
                count: match c.slide_count_source_type {
                    0 => SlideCountKind::Current,
                    1 => SlideCountKind::Remaining,
                    2 => SlideCountKind::Total,
                    _ => SlideCountKind::Unknown,
                },
            }),
            P::PlaylistItem(i) if !i.show_arrangement => Some(DataLink::PlaylistItemName {
                which: match i.playlist_item_source {
                    0 => PlaylistItemWhich::Current,
                    1 => PlaylistItemWhich::Next,
                    2 => PlaylistItemWhich::CurrentHeader,
                    3 => PlaylistItemWhich::NextHeader,
                    4 => PlaylistItemWhich::Playlist,
                    _ => PlaylistItemWhich::Unknown,
                },
            }),
            P::StageMessage(m) => Some(DataLink::Message {
                flash: m.should_flash,
                flash_color: m.flash_color.as_ref().map(common::color),
            }),
            P::VideoCountdown(v) if v.color_triggers.is_empty() => Some(DataLink::VideoCountdown {
                format: timer_format(v.timer_format.as_ref(), &v.timer_format_string),
                source: match v.video_countdown_source {
                    0 => CountdownSource::Presentation,
                    1 => CountdownSource::Announcements,
                    _ => CountdownSource::Unknown,
                },
                ignore_looping: v.ignore_looping_videos,
            }),
            P::AudioCountdown(v) if v.color_triggers.is_empty() => Some(DataLink::AudioCountdown {
                format: timer_format(v.timer_format.as_ref(), &v.timer_format_string),
                ignore_looping: v.ignore_looping_audio,
            }),
            P::CcliText(_) => Some(DataLink::CcliText),
            P::Presentation(x) => Some(DataLink::PresentationName {
                source: match x.presentation_source {
                    0 => CountdownSource::Presentation,
                    1 => CountdownSource::Announcements,
                    _ => CountdownSource::Unknown,
                },
            }),
            P::PresentationNotes(_) => Some(DataLink::PresentationNotes),
            _ => None,
        };
        link = Some(converted.unwrap_or_else(|| {
            let (source_type, payload) = link_kind(ctx, l);
            ctx.count_unmapped(&format!("dataLink.{source_type}"));
            DataLink::Unsupported {
                source_type,
                payload,
            }
        }));
    }
    (link, conditions)
}

/// One visibility link → conditions (see [`data_links`]).
fn visibility_link(
    ctx: &mut ImportContext,
    l: &pp::slide::element::DataLink,
    v: &dl::VisibilityLink,
) -> Vec<VisibilityCondition> {
    let unsupported = |ctx: &mut ImportContext| {
        ctx.count_unmapped("dataLink.visibilityLink");
        VisibilityCondition::Unsupported {
            source_type: "visibilityLink".into(),
            payload: ctx.to_json(l),
        }
    };
    let negate = match v.visibility_criterion {
        0 => false,                            // ALL
        1 if v.conditions.len() == 1 => false, // ANY of one
        2 if v.conditions.len() == 1 => true,  // NONE of one
        1 | 2 if v.conditions.is_empty() => return Vec::new(),
        _ => return vec![unsupported(ctx)],
    };
    let mut out = Vec::new();
    for c in &v.conditions {
        match condition(ctx, c, negate) {
            Some(c) => out.push(c),
            None => {
                return vec![unsupported(ctx)];
            }
        }
    }
    out
}

/// One visibility condition, optionally negated; `None` when the model cannot express it.
fn condition(
    ctx: &mut ImportContext,
    c: &dl::visibility_link::Condition,
    negate: bool,
) -> Option<VisibilityCondition> {
    use dl::visibility_link::condition::ConditionType as C;
    match c.condition_type.as_ref()? {
        C::ElementVisibility(e) => {
            let id = ids::parse_uuid(e.other_element_uuid.as_ref())?;
            let has_text = (e.visibility_criterion == 0) != negate;
            Some(VisibilityCondition::ElementHasText {
                element: ElementId::from_uuid(id),
                has_text,
            })
        }
        C::TimerVisibility(t) => {
            let state = match (t.visibility_criterion, negate) {
                (0, false) | (1, true) => TimerState::HasTimeRemaining,
                (1, false) | (0, true) => TimerState::HasExpired,
                (2, false) | (3, true) => TimerState::IsRunning,
                (3, false) | (2, true) => TimerState::NotRunning,
                _ => return None,
            };
            Some(VisibilityCondition::Timer {
                timer: timer_link(ctx, t.timer_uuid.as_ref(), &t.timer_name),
                state,
            })
        }
        C::VideoInputVisibility(v) => {
            let input = match ctx.lookups.video_input_by_index(v.video_input_index) {
                Some(n) => Link::named(n.id, n.name.clone()),
                None => {
                    ctx.warn(
                        Category::Other,
                        format!(
                            "visibility condition names video input {} which does not exist",
                            v.video_input_index
                        ),
                    );
                    Link::to(VideoInputId::from_uuid(ids::derived(
                        "videoInputIndex",
                        &[&v.video_input_index.to_string()],
                    )))
                }
            };
            let active = match v.visibility_criterion {
                0 => true,
                1 => false,
                _ => return None,
            } != negate;
            Some(VisibilityCondition::VideoInput { input, active })
        }
        C::VideoCountdownVisibility(v) => Some(VisibilityCondition::VideoCountdown {
            state: media_condition(v.visibility_criterion, negate)?,
        }),
        C::AudioCountdownVisibility(v) => Some(VisibilityCondition::AudioCountdown {
            state: media_condition(v.visibility_criterion, negate)?,
        }),
        _ => None,
    }
}

/// A video / audio countdown criterion (has time remaining, has expired, is running, not
/// running, looping, not looping), optionally negated.
fn media_condition(criterion: i32, negate: bool) -> Option<MediaCondition> {
    use MediaCondition as M;
    let pairs = [
        (M::HasTimeRemaining, M::HasExpired),
        (M::IsRunning, M::NotRunning),
        (M::Looping, M::NotLooping),
    ];
    let (yes, no) = pairs.get(usize::try_from(criterion).ok()? / 2)?;
    Some(if (criterion % 2 == 1) != negate {
        *no
    } else {
        *yes
    })
}

fn unit_style(v: i32) -> TimeUnitStyle {
    match v {
        0 => TimeUnitStyle::Hidden,
        1 => TimeUnitStyle::Short,
        2 => TimeUnitStyle::Long,
        3 => TimeUnitStyle::ShortOmitZero,
        4 => TimeUnitStyle::LongOmitZero,
        _ => TimeUnitStyle::Unknown,
    }
}

/// A timer display format plus its `${timer}` template string (empty → `None`).
pub fn timer_format(format: Option<&pp::timer::Format>, template: &str) -> TimerFormat {
    let template = (!template.is_empty()).then(|| template.to_string());
    match format {
        None => TimerFormat {
            template,
            ..TimerFormat::default()
        },
        Some(f) => TimerFormat {
            template,
            hours: unit_style(f.hour),
            minutes: unit_style(f.minute),
            seconds: unit_style(f.second),
            milliseconds: unit_style(f.millisecond),
        },
    }
}

fn date_style(v: i32) -> DateTimeStyle {
    match v {
        0 => DateTimeStyle::None,
        1 => DateTimeStyle::Short,
        2 => DateTimeStyle::Medium,
        3 => DateTimeStyle::Long,
        4 => DateTimeStyle::Full,
        _ => DateTimeStyle::Unknown,
    }
}

/// A clock format plus its `${clock}` template string (empty → `None`).
pub fn clock_format(format: Option<&pp::clock::Format>, template: &str) -> ClockFormat {
    let template = (!template.is_empty()).then(|| template.to_string());
    match format {
        None => ClockFormat {
            template,
            ..ClockFormat::default()
        },
        Some(f) => ClockFormat {
            template,
            time: date_style(f.time_format),
            date: date_style(f.date_type),
            use_24_hour: f.military_time_enabled.then_some(true),
        },
    }
}

/// A timer link: `Link { id, name }` when the UUID is present (resolved or dangling; dangling
/// links are warned), `None` when unbound (the model's "most recently started timer").
pub fn timer_link(
    ctx: &mut ImportContext,
    uuid: Option<&pp::Uuid>,
    name: &str,
) -> Option<Link<TimerId>> {
    let u = ids::parse_uuid(uuid)?;
    let id = TimerId::from_uuid(u);
    match ctx.lookups.timers.get(&u) {
        Some(t) => Some(Link::named(
            t.id,
            if name.is_empty() {
                t.name.clone()
            } else {
                name.to_string()
            },
        )),
        None => {
            ctx.warn(
                Category::Other,
                format!("timer link to \"{name}\" ({u}) does not resolve"),
            );
            Some(if name.is_empty() {
                Link::to(id)
            } else {
                Link::named(id, name)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::NamedId;

    fn ctx() -> ImportContext {
        ImportContext::new(
            std::path::Path::new("/nonexistent"),
            crate::ImportOptions::default(),
        )
    }

    const U1: &str = "5B2E36A8-2A7C-4E0A-9F1C-0D6C9E7F1A23";
    const U2: &str = "6C3F47B9-3B8D-4F1B-8A2D-1E7DAF802B34";

    fn uuid(s: &str) -> Option<pp::Uuid> {
        Some(pp::Uuid { string: s.into() })
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Option<pp::graphics::Rect> {
        Some(pp::graphics::Rect {
            origin: Some(pp::graphics::Point { x, y }),
            size: Some(pp::graphics::Size {
                width: w,
                height: h,
            }),
        })
    }

    fn color(r: f32, g: f32, b: f32, a: f32) -> pp::Color {
        pp::Color {
            red: r,
            green: g,
            blue: b,
            alpha: a,
        }
    }

    fn graphics(name: &str) -> pp::graphics::Element {
        pp::graphics::Element {
            uuid: uuid(U1),
            name: name.into(),
            bounds: rect(55.0, 55.0, 1800.0, 960.0),
            opacity: 1.0,
            fill: Some(pp::graphics::Fill {
                enable: false,
                fill_type: Some(pp::graphics::fill::FillType::Color(color(
                    30.0 / 255.0,
                    144.0 / 255.0,
                    1.0,
                    1.0,
                ))),
            }),
            stroke: Some(pp::graphics::Stroke {
                width: 3.0,
                color: Some(color(1.0, 1.0, 1.0, 1.0)),
                ..Default::default()
            }),
            shadow: Some(pp::graphics::Shadow {
                angle: 315.0,
                offset: 5.0,
                radius: 5.0,
                color: Some(color(0.0, 0.0, 0.0, 1.0)),
                opacity: 0.75,
                ..Default::default()
            }),
            feather: Some(pp::graphics::Feather {
                radius: 0.05,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn slot_names() {
        let mut s = SlotNamer::new();
        assert_eq!(s.next_slot("Verse"), "verse");
        assert_eq!(s.next_slot(""), "text2");
        assert_eq!(s.next_slot(" verse "), "verse2");
        assert_eq!(s.next_slot(""), "text4");
        let mut s = SlotNamer::new();
        assert_eq!(s.next_slot(""), "text1");
        assert_eq!(s.next_slot(""), "text2");
    }

    #[test]
    fn default_decorations_serialize_to_nothing() {
        let mut ctx = ctx();
        let el = pp::slide::Element {
            element: Some(graphics("")),
            text_scroller: Some(pp::slide::element::TextScroller {
                scroll_rate: 0.5,
                should_repeat: true,
                repeat_distance: 0.0556,
                ..Default::default()
            }),
            ..Default::default()
        };
        let e = element(&mut ctx, &el, &mut SlotNamer::new());
        assert_eq!(e.id.to_string(), U1.to_lowercase());
        let json = serde_json::to_value(&e).unwrap();
        let keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["bounds", "id"], "{json}");
        assert!(e.import.is_none());
    }

    #[test]
    fn ticker_scroller_becomes_scrolling_text() {
        let mut ctx = ctx();
        let el = pp::slide::Element {
            element: Some(graphics("Ticker")),
            info: 4,
            text_scroller: Some(pp::slide::element::TextScroller {
                should_scroll: true,
                scroll_rate: 0.25,
                should_repeat: false,
                scrolling_direction: pp::slide::element::text_scroller::Direction::Up as i32,
                starts_off_screen: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let e = element(&mut ctx, &el, &mut SlotNamer::new());
        assert!(e.ticker);
        assert_eq!(
            e.scroll,
            model::TextScroll {
                enabled: true,
                direction: model::ScrollDirection::Up,
                speed: 60.0,
                start: model::ScrollStart::Offscreen,
                repeat: false,
                ..model::TextScroll::default()
            }
        );
        // The source data stays as provenance.
        assert!(e.import.is_some());
    }

    #[test]
    fn verses_template_element() {
        let mut ctx = ctx();
        let mut g = graphics("Background");
        g.locked = true;
        g.fill.as_mut().unwrap().enable = true;
        g.fill.as_mut().unwrap().fill_type = Some(pp::graphics::fill::FillType::Color(color(
            0.098, 0.098, 0.098, 1.0,
        )));
        let el = pp::slide::Element {
            element: Some(g),
            info: 1,
            ..Default::default()
        };
        let mut slots = SlotNamer::new();
        let e = element(&mut ctx, &el, &mut slots);
        assert!(e.themed && e.locked && e.placeholder.is_none());
        assert!(e.fill.enabled);
        assert!(matches!(e.fill.kind, FillKind::Color { .. }));
        let mut g2 = graphics("Verse");
        g2.uuid = uuid(U2);
        g2.text = Some(pp::graphics::Text::default());
        let el2 = pp::slide::Element {
            element: Some(g2),
            info: 3 | 8,
            ..Default::default()
        };
        let e2 = element(&mut ctx, &el2, &mut slots);
        assert!(e2.themed);
        assert_eq!(e2.placeholder, Some(Placeholder::text("verse")));
        assert!(matches!(e2.kind, ElementKind::Text { .. }));
        assert_eq!(e2.import.as_ref().unwrap().unmapped["info"], 11);
    }

    #[test]
    fn enabled_decorations_round_trip_verbatim() {
        let mut ctx = ctx();
        let mut g = graphics("");
        g.stroke = Some(pp::graphics::Stroke {
            enable: true,
            width: 6.0,
            color: Some(color(1.0, 0.0, 0.0, 1.0)),
            style: 2,
            ..Default::default()
        });
        g.shadow.as_mut().unwrap().enable = true;
        g.shadow.as_mut().unwrap().offset = 9.0;
        g.feather = Some(pp::graphics::Feather {
            enable: true,
            style: 2,
            radius: 0.3,
        });
        g.flip_mode = 3;
        g.rotation = 270.0;
        let e = graphics_element(&mut ctx, &g);
        assert!(e.stroke.enabled && e.stroke.width == 6.0);
        assert_eq!(e.stroke.style, StrokeStyle::ShortDash);
        assert!(e.shadow.enabled && e.shadow.offset == 9.0 && e.shadow.angle == 315.0);
        assert_eq!(e.feather.style, FeatherStyle::Outside);
        assert!(e.flip.horizontal && e.flip.vertical);
        assert_eq!(e.rotation, 270.0);
    }

    #[test]
    fn shapes() {
        use pp::graphics::path::{BezierPoint, Shape as S, shape::AdditionalData as A};
        let mut ctx = ctx();
        let pt = |x: f64, y: f64| Some(pp::graphics::Point { x, y });
        let path = |ty: i32, data: Option<A>, points: Vec<BezierPoint>| pp::graphics::Path {
            closed: true,
            points,
            shape: Some(S {
                r#type: ty,
                additional_data: data,
            }),
        };
        let corner = |x: f64, y: f64| BezierPoint {
            point: pt(x, y),
            q0: pt(x, y),
            q1: pt(x, y),
            curved: false,
        };
        let rect_pts = vec![
            corner(0.0, 0.0),
            corner(1.0, 0.0),
            corner(1.0, 1.0),
            corner(0.0, 1.0),
        ];
        assert_eq!(shape(&mut ctx, None), Shape::Rectangle);
        assert_eq!(
            shape(&mut ctx, Some(&path(1, None, rect_pts.clone()))),
            Shape::Rectangle
        );
        assert_eq!(
            shape(&mut ctx, Some(&path(0, None, rect_pts.clone()))),
            Shape::Rectangle
        );
        assert_eq!(
            shape(
                &mut ctx,
                Some(&path(
                    11,
                    Some(A::RoundedRectangle(
                        pp::graphics::path::shape::RoundedRectangle { roundness: 0.2 }
                    )),
                    vec![]
                ))
            ),
            Shape::RoundedRectangle { roundness: 0.2 }
        );
        assert_eq!(
            shape(
                &mut ctx,
                Some(&path(
                    7,
                    Some(A::Polygon(pp::graphics::path::shape::Polygon {
                        number_sides: 8
                    })),
                    vec![]
                ))
            ),
            Shape::Polygon { sides: 8 }
        );
        let custom = shape(
            &mut ctx,
            Some(&path(
                8,
                None,
                vec![
                    BezierPoint {
                        point: pt(0.5, 0.0),
                        q0: pt(0.2, 0.0),
                        q1: pt(0.8, 0.0),
                        curved: true,
                    },
                    corner(1.0, 1.0),
                    corner(0.0, 1.0),
                ],
            )),
        );
        match custom {
            Shape::Path { path } => {
                assert!(path.closed);
                assert_eq!(path.points.len(), 3);
                assert_eq!(path.points[0].handle_in.x, 0.2);
                assert_eq!(path.points[0].handle_out.x, 0.8);
                assert!(path.points[0].curved);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn gradients_and_effects() {
        let g = gradient(&pp::graphics::Gradient {
            r#type: 1,
            angle: 315.0,
            length: 0.0,
            stops: vec![
                pp::graphics::gradient::ColorStop {
                    color: Some(color(1.0, 0.0, 0.0, 1.0)),
                    position: 1.0,
                    blend_point: 0.0,
                },
                pp::graphics::gradient::ColorStop {
                    color: Some(color(0.0, 0.0, 1.0, 1.0)),
                    position: 0.0,
                    blend_point: 0.3,
                },
            ],
        });
        assert_eq!(g.kind, GradientKind::Radial);
        assert_eq!(g.length, 1.0);
        assert_eq!(g.stops[0].position, 0.0);
        assert_eq!(g.stops[0].midpoint, 0.3);
        assert_eq!(g.stops[1].midpoint, 0.5);
        use pp::graphics::background_effect::{BackgroundEffectBlur, EffectType};
        let b = background_effect(&pp::graphics::BackgroundEffect {
            effect_type: Some(EffectType::BackgroundBlur(BackgroundEffectBlur {
                saturation: 1.2,
                blur_amount: 0.4,
            })),
        });
        assert_eq!(
            b,
            Some(BackgroundEffect::Blur {
                amount: 0.4,
                saturation: 1.2
            })
        );
        assert_eq!(
            background_effect(&pp::graphics::BackgroundEffect::default()),
            None
        );
    }

    #[test]
    fn empty_picture_text_keeps_non_default_typing() {
        use pp::graphics::text::attributes::Fill as F;
        let picture = |font: &str, size: f64, fill: pp::Color| {
            let mut g = graphics("");
            g.fill = Some(pp::graphics::Fill {
                enable: true,
                fill_type: Some(pp::graphics::fill::FillType::Media(pp::Media::default())),
            });
            g.text = Some(pp::graphics::Text {
                attributes: Some(pp::graphics::text::Attributes {
                    font: Some(pp::Font {
                        name: font.into(),
                        size,
                        ..Default::default()
                    }),
                    fill: Some(F::TextSolidFill(fill)),
                    ..Default::default()
                }),
                ..Default::default()
            });
            g
        };
        let mut ctx = ctx();
        // ProPresenter's default typing attributes: collapsed to a shape, nothing kept.
        let e = graphics_element(
            &mut ctx,
            &picture("ArialMT", 50.0, color(0.0, 0.0, 0.0, 1.0)),
        );
        assert!(matches!(e.kind, ElementKind::Shape { .. }));
        assert!(
            e.import
                .as_ref()
                .is_none_or(|i| i.unmapped.get("emptyTextAttributes").is_none())
        );
        // A themed style: still a shape, but the attributes survive in import.unmapped.
        let e = graphics_element(
            &mut ctx,
            &picture("HelveticaNeue", 42.0, color(1.0, 1.0, 1.0, 1.0)),
        );
        assert!(matches!(e.kind, ElementKind::Shape { .. }));
        let kept = &e.import.as_ref().unwrap().unmapped["emptyTextAttributes"];
        assert_eq!(kept["font"]["name"], "HelveticaNeue");
        assert_eq!(kept["font"]["size"], 42.0);
        // A text placeholder keeps its empty box regardless.
        let el = pp::slide::Element {
            element: Some(picture("HelveticaNeue", 42.0, color(1.0, 1.0, 1.0, 1.0))),
            info: 2,
            ..Default::default()
        };
        let e = element(&mut ctx, &el, &mut SlotNamer::new());
        assert!(matches!(e.kind, ElementKind::Text { .. }));
    }

    #[test]
    fn line_mask_takes_the_element_fill() {
        let mut ctx = ctx();
        let mut g = graphics("");
        g.fill.as_mut().unwrap().enable = true;
        g.text = Some(pp::graphics::Text::default());
        g.mask = Some(pp::graphics::element::Mask::TextLineMask(
            pp::graphics::text::LineFillMask {
                enabled: true,
                mask_style: 2,
                width_offset: 29.0,
                height_offset: 4.0,
                ..Default::default()
            },
        ));
        let e = graphics_element(&mut ctx, &g);
        let tb = e.text_box().unwrap();
        assert!(tb.line_mask.enabled);
        assert_eq!(tb.line_mask.fill, model::DEFAULT_FILL_COLOR);
        assert!(!e.fill.enabled);
        assert_eq!(e.import.unwrap().unmapped["lineMaskFill"], true);
    }

    #[test]
    fn data_links_and_visibility() {
        use dl::visibility_link::{Condition, condition};
        let mut ctx = ctx();
        let timer = uuid::Uuid::parse_str(U2).unwrap();
        ctx.lookups.timers.insert(
            timer,
            NamedId {
                id: TimerId::from_uuid(timer),
                name: "Countdown".into(),
            },
        );
        let links = vec![
            pp::slide::element::DataLink {
                property_type: Some(dl::PropertyType::TimerText(dl::TimerText {
                    timer_uuid: None,
                    timer_name: String::new(),
                    timer_format: Some(pp::timer::Format {
                        hour: 3,
                        minute: 2,
                        second: 2,
                        ..Default::default()
                    }),
                    timer_format_string: "${timer}".into(),
                    color_triggers: vec![dl::ColorTrigger {
                        time: 10.0,
                        color: Some(color(1.0, 0.0, 0.0, 1.0)),
                    }],
                })),
            },
            pp::slide::element::DataLink {
                property_type: Some(dl::PropertyType::VisibilityLink(dl::VisibilityLink {
                    visibility_criterion: 0,
                    conditions: vec![
                        Condition {
                            condition_type: Some(condition::ConditionType::ElementVisibility(
                                condition::ElementVisibility {
                                    other_element_uuid: uuid(U1),
                                    other_element_name: "Item 1".into(),
                                    visibility_criterion: 0,
                                },
                            )),
                        },
                        Condition {
                            condition_type: Some(condition::ConditionType::TimerVisibility(
                                condition::TimerVisibility {
                                    timer_uuid: uuid(U2),
                                    timer_name: String::new(),
                                    visibility_criterion: 1,
                                },
                            )),
                        },
                    ],
                })),
            },
            pp::slide::element::DataLink {
                property_type: Some(dl::PropertyType::RssFeed(dl::RssFeed::default())),
            },
        ];
        let (link, conds) = data_links(&mut ctx, &links);
        match link.unwrap() {
            DataLink::Timer {
                timer,
                format,
                color_triggers,
                ..
            } => {
                assert_eq!(timer, None, "unbound link = most recently started timer");
                assert_eq!(
                    format,
                    TimerFormat {
                        template: Some("${timer}".into()),
                        ..TimerFormat::default()
                    }
                );
                assert_eq!(color_triggers.len(), 1);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(conds.len(), 2);
        assert!(matches!(
            conds[0],
            VisibilityCondition::ElementHasText { has_text: true, .. }
        ));
        match &conds[1] {
            VisibilityCondition::Timer { timer, state } => {
                assert_eq!(timer.as_ref().unwrap().name.as_deref(), Some("Countdown"));
                assert_eq!(*state, TimerState::HasExpired);
            }
            other => panic!("{other:?}"),
        }

        // The RSS link alone becomes Unsupported with its oneof name.
        let (link, _) = data_links(&mut ctx, &links[2..]);
        assert!(matches!(
            link,
            Some(DataLink::Unsupported { ref source_type, .. }) if source_type == "rssFeed"
        ));

        // A second link on the element is kept in unmapped.
        let el = pp::slide::Element {
            element: Some(graphics("Timer")),
            data_links: vec![links[0].clone(), links[2].clone()],
            ..Default::default()
        };
        let e = element(&mut ctx, &el, &mut SlotNamer::new());
        assert!(matches!(e.data_link, Some(DataLink::Timer { .. })));
        assert!(e.import.unwrap().unmapped["dataLinks"].is_array());
    }

    #[test]
    fn negated_and_unsupported_visibility() {
        use dl::visibility_link::{Condition, condition};
        let mut ctx = ctx();
        let cond = |crit: i32| Condition {
            condition_type: Some(condition::ConditionType::ElementVisibility(
                condition::ElementVisibility {
                    other_element_uuid: uuid(U1),
                    other_element_name: String::new(),
                    visibility_criterion: crit,
                },
            )),
        };
        let link = |crit: i32, conditions: Vec<Condition>| pp::slide::element::DataLink {
            property_type: Some(dl::PropertyType::VisibilityLink(dl::VisibilityLink {
                visibility_criterion: crit,
                conditions,
            })),
        };
        let (_, c) = data_links(&mut ctx, &[link(2, vec![cond(0)])]);
        assert!(matches!(
            c[0],
            VisibilityCondition::ElementHasText {
                has_text: false,
                ..
            }
        ));
        let (_, c) = data_links(&mut ctx, &[link(1, vec![cond(0), cond(1)])]);
        assert!(matches!(c[0], VisibilityCondition::Unsupported { .. }));
    }

    #[test]
    fn presentation_links_and_media_visibility() {
        use dl::visibility_link::{Condition, condition};
        let mut ctx = ctx();
        let one = |p: dl::PropertyType| {
            vec![pp::slide::element::DataLink {
                property_type: Some(p),
            }]
        };
        let (l, _) = data_links(
            &mut ctx,
            &one(dl::PropertyType::Presentation(dl::Presentation {
                presentation_source: 1,
            })),
        );
        assert_eq!(
            l,
            Some(DataLink::PresentationName {
                source: CountdownSource::Announcements
            })
        );
        let (l, _) = data_links(
            &mut ctx,
            &one(dl::PropertyType::PresentationNotes(
                dl::PresentationNotes::default(),
            )),
        );
        assert_eq!(l, Some(DataLink::PresentationNotes));
        let media = |video: bool, crit: i32, negate: i32| {
            let c = if video {
                condition::ConditionType::VideoCountdownVisibility(
                    condition::VideoCountdownVisibility {
                        visibility_criterion: crit,
                    },
                )
            } else {
                condition::ConditionType::AudioCountdownVisibility(
                    condition::AudioCountdownVisibility {
                        visibility_criterion: crit,
                    },
                )
            };
            one(dl::PropertyType::VisibilityLink(dl::VisibilityLink {
                visibility_criterion: negate,
                conditions: vec![Condition {
                    condition_type: Some(c),
                }],
            }))
        };
        use MediaCondition as M;
        for (crit, want) in [
            (0, M::HasTimeRemaining),
            (1, M::HasExpired),
            (2, M::IsRunning),
            (3, M::NotRunning),
            (4, M::Looping),
            (5, M::NotLooping),
        ] {
            let (_, c) = data_links(&mut ctx, &media(true, crit, 0));
            assert_eq!(c, [VisibilityCondition::VideoCountdown { state: want }]);
        }
        // NONE of "is running" = not running; an unknown criterion stays unsupported.
        let (_, c) = data_links(&mut ctx, &media(false, 2, 2));
        assert_eq!(
            c,
            [VisibilityCondition::AudioCountdown {
                state: M::NotRunning
            }]
        );
        let (_, c) = data_links(&mut ctx, &media(false, 9, 0));
        assert!(matches!(c[0], VisibilityCondition::Unsupported { .. }));
    }

    #[test]
    fn clock_and_timer_formats() {
        let c = clock_format(
            Some(&pp::clock::Format {
                date_type: 3,
                time_format: 1,
                military_time_enabled: true,
            }),
            "",
        );
        assert_eq!(c.date, DateTimeStyle::Long);
        assert_eq!(c.time, DateTimeStyle::Short);
        assert_eq!(c.use_24_hour, Some(true));
        assert_eq!(c.template, None);
        assert_eq!(timer_format(None, ""), TimerFormat::default());
    }

    #[test]
    fn dangling_timer_link_keeps_name() {
        let mut ctx = ctx();
        let l = timer_link(&mut ctx, uuid(U1).as_ref(), "Timer A").unwrap();
        assert_eq!(l.name.as_deref(), Some("Timer A"));
        assert_eq!(ctx.report.warnings.len(), 1);
        assert_eq!(timer_link(&mut ctx, None, "x"), None);
    }

    #[test]
    fn elements_are_reordered_back_to_front() {
        let mut ctx = ctx();
        let mut back = graphics("Background");
        back.uuid = uuid(U2);
        let els = [
            pp::slide::Element {
                element: Some(graphics("Title")),
                ..Default::default()
            },
            pp::slide::Element {
                element: Some(back),
                ..Default::default()
            },
        ];
        let out = elements(&mut ctx, &els);
        let names: Vec<_> = out.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Background", "Title"]);
    }

    #[test]
    fn builds_follow_the_slide_build_order() {
        use pp::slide::element::{Build, build::Start};
        let mut ctx = ctx();
        const B1: &str = "11111111-2A7C-4E0A-9F1C-0D6C9E7F1A23";
        const B2: &str = "22222222-2A7C-4E0A-9F1C-0D6C9E7F1A23";
        let mut g2 = graphics("Second");
        g2.uuid = uuid(U2);
        let els = vec![
            pp::slide::Element {
                element: Some(graphics("First")),
                build_in: Some(Build {
                    uuid: uuid(B1),
                    element_uuid: uuid(U1),
                    start: Start::OnClick as i32,
                    delay_time: 0.0,
                    transition: Some(pp::Transition {
                        duration: 0.75,
                        ..Default::default()
                    }),
                }),
                ..Default::default()
            },
            pp::slide::Element {
                element: Some(g2),
                build_out: Some(Build {
                    uuid: uuid(B2),
                    element_uuid: uuid(U2),
                    start: Start::AfterPrevious as i32,
                    delay_time: 1.5,
                    transition: None,
                }),
                ..Default::default()
            },
        ];
        let out = elements(&mut ctx, &els);
        // The slide's order lists the out build first.
        let order = vec![uuid(B2).unwrap(), uuid(B1).unwrap()];
        let b = builds(&mut ctx, &els, &out, &order);
        assert_eq!(b.len(), 2);
        let second = out.iter().find(|e| e.name == "Second").unwrap().id;
        let first = out.iter().find(|e| e.name == "First").unwrap().id;
        assert_eq!(b[0].element, second);
        assert_eq!(b[0].mode, model::BuildMode::Out);
        assert_eq!(b[0].start, model::BuildStart::AfterPrevious);
        assert_eq!(b[0].delay, 1.5);
        assert_eq!(b[0].transition, model::Transition::CUT);
        assert_eq!(b[1].element, first);
        assert_eq!(b[1].start, model::BuildStart::OnNext);
        assert_eq!(b[1].transition, model::Transition::dissolve(0.75));
        // Not left in the unmapped data.
        assert!(out.iter().all(|e| {
            e.import
                .as_ref()
                .is_none_or(|i| i.unmapped.get("buildIn").is_none())
        }));
        // Without an order they follow in element order.
        let b = builds(&mut ctx, &els, &out, &[]);
        assert_eq!(b[0].element, first);
    }
}
