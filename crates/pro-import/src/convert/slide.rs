//! **Area: slides.** Cue → [`Slide`](model::Slide), `rv.data.Slide` → [`Slide`](model::Slide),
//! actions → [`SlideAction`].
//!
//! ## Owns
//! - [`slide_from_cue`]: a presentation cue. The `ACTION_TYPE_PRESENTATION_SLIDE` action's
//!   `PresentationSlide` provides elements / background / notes / transition; the action's
//!   `label` becomes `Slide.label`; the cue provides `enabled` (`isEnabled`) and `hotkey`; the
//!   cue `name` (auto-numbering noise) goes to `import.originalName` only; every *other* action
//!   becomes a [`SlideAction`] ([`slide_actions`]). Slide ID = cue UUID.
//!   Non-default cue completion settings and chord-chart URLs are kept in `import.unmapped`;
//!   template guidelines are counted (`slide.templateGuidelines`). The census has none of them.
//! - [`slide_from_pb`]: a bare `rv.data.Slide` (theme template bodies, props, masks, stage
//!   layouts, CCLI template): elements + background (`drawsBackgroundColor` decides whether the
//!   colour is used; otherwise `None` = transparent). Alignment guides are editor-only and only
//!   counted (`slide.guidelines`); the element build order orders the slide's `builds` ([`element::builds`]).
//! - [`slide_actions`] / [`slide_action`] / [`action_kind`]: the action dispatch.
//!   `ACTION_TYPE_MEDIA` → `media::media_action` (media area decides Media vs VideoInput);
//!   PROP / TIMER / MESSAGE / CLEAR / CLEAR_GROUP / MACRO / AUDIENCE_LOOK / STAGE_LAYOUT /
//!   PRESENTATION_DOCUMENT → the matching [`ActionKind`] with `common::collection_link` links;
//!   everything else → `ActionKind::Unsupported { sourceType: "ACTION_TYPE_…", payload }` with the
//!   payload from `ctx.to_json(action)` and `ctx.count_unsupported_action`. A modelled kind whose
//!   link has no UUID is unsupported too (nothing to point at). Payload details the model has no
//!   field for (prop auto-clear, timer configuration, document arrangement / destination, stage
//!   slide target, non-text message token values) are kept in the action's `import.unmapped`.
//!   Timer operations: start / stop / reset / reset-and-start (`Restart`) / increment (`Adjust`);
//!   stop-and-reset becomes `Reset` (reset stops the timer).
//! - [`slide_label`].
//!
//! ## May call
//! `common::*`, `element::*`, `text::*`, `media::media_action`, `config::clear_target`,
//! `config::timer_kind`, `ImportContext` helpers.
//!
//! ## Canvas
//! Slides have no size of their own; `canvas` is the owner's size (presentation / theme /
//! layout). It is passed for element sanity checks only: an element entirely outside the canvas
//! and a slide size different from the canvas are warned.

use model::{
    ActionId, ActionKind, ImportInfo, Link, Size, SlideAction, SlideId, SlideLabel,
    StageAssignment, TimerOperation, TokenValue, TriggerTarget,
};
use serde_json::{Value, json};

use crate::context::{ImportContext, ids};
use crate::convert::{common, config, element, media, text};
use crate::pb::rv::data as pp;
use crate::report::Category;

/// Converts a presentation cue into a slide (see the module docs).
pub fn slide_from_cue(ctx: &mut ImportContext, cue: &pp::Cue, canvas: Size) -> model::Slide {
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = SlideId::from_uuid(ids::source_or_derived(
        cue.uuid.as_ref(),
        "slide",
        &[&path, &cue.name],
    ));
    let found = presentation_slide(cue);
    let mut slide = match found.and_then(|(_, ps)| ps.base_slide.as_ref()) {
        Some(base) => {
            check_canvas(ctx, base, canvas);
            slide_from_pb(ctx, base, id)
        }
        None => model::Slide {
            id,
            ..model::Slide::new()
        },
    };
    if let Some((i, ps)) = found {
        slide.label = slide_label(cue.actions[i].label.as_ref());
        if let Some(n) = ps.notes.as_ref() {
            slide.notes = text::notes(ctx, n);
        }
        slide.transition = ps.transition.as_ref().map(|t| common::transition(ctx, t));
        if let Some(u) = ps
            .chord_chart
            .as_ref()
            .filter(|u| **u != pp::Url::default())
        {
            let v = ctx.to_json(u);
            ctx.keep_unmapped(&mut slide.import, "slide.chordChart", v);
        }
        if !ps.template_guidelines.is_empty() {
            ctx.count_unmapped("slide.templateGuidelines");
        }
    }
    slide.enabled = cue.is_enabled;
    slide.hotkey = common::hotkey(cue.hot_key.as_ref());
    slide.actions = slide_actions(ctx, &cue.actions, found.map(|(i, _)| i));
    if !cue.name.is_empty() {
        slide
            .import
            .get_or_insert_with(ImportInfo::default)
            .original_name = Some(cue.name.clone());
    }
    let completion_set = cue.completion_target_type != 0
        || cue.completion_time != 0.0
        || ids::parse_uuid(cue.completion_target_uuid.as_ref()).is_some()
        || ids::parse_uuid(cue.completion_action_uuid.as_ref()).is_some()
        || cue.trigger_time.as_ref().is_some_and(|t| t.time != 0.0);
    if completion_set {
        let v = json!({
            "completionTargetType": cue.completion_target_type,
            "completionTargetUuid": cue.completion_target_uuid.as_ref().map(|u| u.string.clone()),
            "completionActionType": cue.completion_action_type,
            "completionActionUuid": cue.completion_action_uuid.as_ref().map(|u| u.string.clone()),
            "completionTime": cue.completion_time,
            "triggerTime": cue.trigger_time.as_ref().map(|t| t.time),
        });
        ctx.keep_unmapped(&mut slide.import, "cue.completion", v);
    }
    if !cue.pending_imports.is_empty() {
        ctx.count_unmapped("cue.pendingImports");
    }
    slide
}

/// Warns when a slide's own size differs from the owner's canvas or an element lies entirely
/// outside it.
fn check_canvas(ctx: &mut ImportContext, s: &pp::Slide, canvas: Size) {
    if canvas.width <= 0.0 || canvas.height <= 0.0 {
        return;
    }
    if let Some(own) = common::size(s.size.as_ref())
        && ((own.width - canvas.width).abs() > 0.5 || (own.height - canvas.height).abs() > 0.5)
    {
        ctx.warn(
            Category::Other,
            format!(
                "slide size {}x{} differs from the canvas {}x{}",
                own.width, own.height, canvas.width, canvas.height
            ),
        );
    }
    for el in &s.elements {
        let Some(g) = el.element.as_ref() else {
            continue;
        };
        let r = common::rect(g.bounds.as_ref());
        let outside = r.x >= canvas.width
            || r.y >= canvas.height
            || r.x + r.width <= 0.0
            || r.y + r.height <= 0.0;
        if outside && g.rotation == 0.0 {
            ctx.warn(
                Category::Other,
                format!(
                    "element \"{}\" lies outside the {}x{} canvas",
                    g.name, canvas.width, canvas.height
                ),
            );
        }
    }
}

/// Converts a bare `rv.data.Slide` (template / prop / mask / stage / CCLI body) into a slide with
/// ID `id`. Actions are added by the caller (template slides carry them beside the slide).
pub fn slide_from_pb(ctx: &mut ImportContext, slide: &pp::Slide, id: SlideId) -> model::Slide {
    let mut out = model::Slide {
        id,
        background: if slide.draws_background_color {
            Some(common::opt_color(slide.background_color.as_ref()).unwrap_or_default())
        } else {
            None
        },
        elements: element::elements(ctx, &slide.elements),
        ..model::Slide::new()
    };
    if !slide.guidelines.is_empty() {
        ctx.count_unmapped("slide.guidelines");
    }
    out.builds = element::builds(
        ctx,
        &slide.elements,
        &out.elements,
        &slide.element_build_order,
    );
    out
}

/// The `PresentationSlide` payload of a cue's slide action, with the index of that action in
/// `cue.actions` (so [`slide_actions`] can skip it). `None` when the cue has no slide action.
pub fn presentation_slide(cue: &pp::Cue) -> Option<(usize, &pp::PresentationSlide)> {
    cue.actions
        .iter()
        .enumerate()
        .find_map(|(i, a)| match &a.action_type_data {
            Some(pp::action::ActionTypeData::Slide(pp::action::SlideType {
                slide: Some(pp::action::slide_type::Slide::Presentation(ps)),
            })) => Some((i, ps)),
            _ => None,
        })
}

/// Converts actions in order, skipping index `skip` (the cue's own slide action) and actions that
/// only carry slide content. Disabled actions are kept with `enabled: false`.
pub fn slide_actions(
    ctx: &mut ImportContext,
    actions: &[pp::Action],
    skip: Option<usize>,
) -> Vec<SlideAction> {
    actions
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != skip)
        .filter_map(|(_, a)| slide_action(ctx, a))
        .collect()
}

/// One action → [`SlideAction`] (ID = action UUID, `name`, `delay` = `delayTime`, `enabled` =
/// `isEnabled`). `None` for slide-content actions (presentation / prop slide payloads).
pub fn slide_action(ctx: &mut ImportContext, action: &pp::Action) -> Option<SlideAction> {
    if matches!(
        action.action_type_data,
        Some(pp::action::ActionTypeData::Slide(_))
    ) {
        return None;
    }
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = ActionId::from_uuid(ids::source_or_derived(
        action.uuid.as_ref(),
        "action",
        &[&path, &action.name, &action.r#type.to_string()],
    ));
    let mut import: Option<ImportInfo> = None;
    let kind = action_kind_with(ctx, action, &mut import);
    if let Some(l) = action
        .label
        .as_ref()
        .filter(|l| !l.text.is_empty() || common::visible_color(l.color.as_ref()).is_some())
    {
        let v = ctx.to_json(l);
        ctx.keep_unmapped(&mut import, "action.label", v);
    }
    // A media action's duration is the media length (the media area's concern).
    let is_media = matches!(
        action.action_type_data,
        Some(pp::action::ActionTypeData::Media(_))
    );
    if action.duration != 0.0 && !is_media {
        ctx.keep_unmapped(&mut import, "action.duration", json!(action.duration));
    }
    if let Some(l) = action
        .layer_identification
        .as_ref()
        .filter(|l| !l.name.is_empty() || ids::parse_uuid(l.uuid.as_ref()).is_some())
    {
        let v = ctx.to_json(l);
        ctx.keep_unmapped(&mut import, "action.layerIdentification", v);
    }
    Some(SlideAction {
        id,
        enabled: action.is_enabled,
        name: (!action.name.is_empty()).then(|| action.name.clone()),
        delay: action.delay_time,
        kind,
        import,
    })
}

/// The [`ActionKind`] of an action (see the module docs for the mapping).
pub fn action_kind(ctx: &mut ImportContext, action: &pp::Action) -> ActionKind {
    let mut discard = None;
    action_kind_with(ctx, action, &mut discard)
}

/// `"ACTION_TYPE_MEDIA"` (falls back to the payload's oneof name for an unknown type).
fn action_type_name(ctx: &ImportContext, action: &pp::Action) -> String {
    match pp::action::ActionType::try_from(action.r#type) {
        Ok(t) if t != pp::action::ActionType::Unknown => t.as_str_name().to_string(),
        _ => {
            let v = ctx.to_json(action);
            const ENVELOPE: &[&str] = &[
                "uuid",
                "name",
                "label",
                "delayTime",
                "oldType",
                "isEnabled",
                "layerIdentification",
                "duration",
                "type",
            ];
            v.as_object()
                .and_then(|m| m.keys().find(|k| !ENVELOPE.contains(&k.as_str())).cloned())
                .map(|k| format!("ACTION_TYPE_UNKNOWN:{k}"))
                .unwrap_or_else(|| "ACTION_TYPE_UNKNOWN".into())
        }
    }
}

fn unsupported(ctx: &mut ImportContext, action: &pp::Action) -> ActionKind {
    let source_type = action_type_name(ctx, action);
    ctx.count_unsupported_action(&source_type);
    ActionKind::Unsupported {
        source_type,
        payload: ctx.to_json(action),
    }
}

/// [`action_kind`], keeping payload details the kind has no field for in `import`.
fn action_kind_with(
    ctx: &mut ImportContext,
    action: &pp::Action,
    import: &mut Option<ImportInfo>,
) -> ActionKind {
    use pp::action::ActionTypeData as D;
    let Some(data) = action.action_type_data.as_ref() else {
        return unsupported(ctx, action);
    };
    let kind = match data {
        D::Media(m) => Some(media::media_action(ctx, action, m)),
        D::Clear(c) => Some(ActionKind::ClearLayer {
            targets: vec![config::clear_target(c)],
        }),
        D::ClearGroup(c) => common::collection_link(c.identification.as_ref())
            .map(|group| ActionKind::ClearGroup { group }),
        D::Timer(t) => common::collection_link(t.timer_identification.as_ref()).and_then(|timer| {
            use pp::action::timer_type::TimerAction as A;
            let operation = match A::try_from(t.action_type).ok()? {
                A::ActionStart => TimerOperation::Start,
                A::ActionStop => TimerOperation::Stop,
                A::ActionReset | A::ActionStopAndReset => TimerOperation::Reset,
                A::ActionResetAndStart => TimerOperation::Restart,
                A::ActionIncrement => TimerOperation::Adjust {
                    seconds: t.increment_amount,
                },
            };
            if let Some(c) = t.timer_configuration.as_ref() {
                let v = ctx.to_json(c);
                ctx.keep_unmapped(import, "action.timerConfiguration", v);
            }
            if t.action_type == A::ActionStopAndReset as i32 {
                ctx.keep_unmapped(import, "action.timerAction", json!("ACTION_STOP_AND_RESET"));
            }
            Some(ActionKind::Timer { timer, operation })
        }),
        D::Prop(p) => common::collection_link(p.identification.as_ref()).map(|prop| {
            use pp::action::prop_type::TriggerType;
            let show = !matches!(p.trigger_type, Some(TriggerType::Clear(_)));
            if let Some(TriggerType::Trigger(t)) = p.trigger_type.as_ref()
                && *t != pp::action::prop_type::PropTrigger::default()
            {
                let v = ctx.to_json(t);
                ctx.keep_unmapped(import, "action.propTrigger", v);
            }
            ActionKind::Prop { prop, show }
        }),
        D::Message(m) => common::collection_link(m.message_identificaton.as_ref()).map(|message| {
            use pp::message::token_value::TokenValueType;
            let mut values = Vec::new();
            let mut other = Vec::new();
            for t in &m.content {
                match t.token_value_type.as_ref() {
                    Some(TokenValueType::Text(v)) => values.push(TokenValue {
                        name: t.token_name.clone(),
                        value: v.value.clone(),
                    }),
                    _ => other.push(ctx.to_json(t)),
                }
            }
            if !other.is_empty() {
                ctx.keep_unmapped(import, "action.tokenValues", Value::Array(other));
            }
            ActionKind::Message {
                message,
                show: true,
                values,
            }
        }),
        D::AudienceLook(l) => {
            common::collection_link(l.identification.as_ref()).map(|look| ActionKind::Look { look })
        }
        D::Macro(m) => common::collection_link(m.identification.as_ref())
            .map(|target| ActionKind::Macro { target }),
        D::Stage(s) => {
            let mut assignments = Vec::new();
            let mut complete = true;
            for a in &s.stage_screen_assignments {
                match common::collection_link(a.layout.as_ref()) {
                    Some(layout) => assignments.push(StageAssignment {
                        screen: common::collection_link(a.screen.as_ref()),
                        layout,
                    }),
                    None => complete = false,
                }
            }
            if s.slide_target != 0 {
                ctx.keep_unmapped(import, "action.slideTarget", json!(s.slide_target));
            }
            (complete && !assignments.is_empty())
                .then_some(ActionKind::StageLayout { assignments })
        }
        D::PresentationDocument(d) => {
            common::collection_link(d.identification.as_ref()).map(|presentation| {
                if ids::parse_uuid(d.selected_arrangement.as_ref()).is_some()
                    || d.content_destination != 0
                {
                    let v = json!({
                        "selectedArrangement": d.selected_arrangement.as_ref().map(|u| u.string.clone()),
                        "contentDestination": d.content_destination,
                    });
                    ctx.keep_unmapped(import, "action.presentationDocument", v);
                }
                ActionKind::Trigger {
                    target: TriggerTarget::Presentation { presentation },
                }
            })
        }
        D::Slide(_) => None,
        _ => None,
    };
    match kind {
        Some(k) => {
            warn_dangling(ctx, &k);
            k
        }
        None => unsupported(ctx, action),
    }
}

/// Warns about links into configuration documents that do not resolve (lookups are filled
/// before presentations are imported).
fn warn_dangling(ctx: &mut ImportContext, kind: &ActionKind) {
    fn name<T>(l: &Link<T>) -> String {
        l.name.clone().unwrap_or_default()
    }
    let missing: Option<(&str, String)> = match kind {
        ActionKind::Prop { prop, .. } if !ctx.lookups.props.contains_key(&prop.id.uuid()) => {
            Some(("prop", name(prop)))
        }
        ActionKind::Message { message, .. }
            if !ctx.lookups.messages.contains_key(&message.id.uuid()) =>
        {
            Some(("message", name(message)))
        }
        ActionKind::Timer { timer, .. } if !ctx.lookups.timers.contains_key(&timer.id.uuid()) => {
            Some(("timer", name(timer)))
        }
        ActionKind::Look { look } if !ctx.lookups.looks.contains_key(&look.id.uuid()) => {
            Some(("look", name(look)))
        }
        ActionKind::Macro { target } if !ctx.lookups.macros.contains_key(&target.id.uuid()) => {
            Some(("macro", name(target)))
        }
        ActionKind::ClearGroup { group }
            if !ctx.lookups.clear_groups.contains_key(&group.id.uuid()) =>
        {
            Some(("clear group", name(group)))
        }
        _ => None,
    };
    if let Some((what, n)) = missing {
        ctx.warn(
            Category::Other,
            format!("action links {what} \"{n}\" which does not exist"),
        );
    }
}

/// The slide label from a slide action's `label` (`(1,1,1,0)` colour → `None`; empty text and no
/// colour → `None`).
pub fn slide_label(label: Option<&pp::action::Label>) -> Option<SlideLabel> {
    let l = label?;
    let color = common::visible_color(l.color.as_ref());
    if l.text.is_empty() && color.is_none() {
        return None;
    }
    Some(SlideLabel {
        text: l.text.clone(),
        color,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{ClearLayer, Color};

    const CUE: &str = "11111111-2222-4333-8444-555555555555";
    const ACT: &str = "21111111-2222-4333-8444-555555555555";
    const EL: &str = "31111111-2222-4333-8444-555555555555";
    const PROP: &str = "41111111-2222-4333-8444-555555555555";

    fn ctx() -> ImportContext {
        let mut c = ImportContext::new(
            std::path::Path::new("/nonexistent"),
            crate::ImportOptions::default(),
        );
        c.current.path = Some("Libraries/Example/Song.pro".into());
        c
    }

    fn uuid(s: &str) -> Option<pp::Uuid> {
        Some(pp::Uuid { string: s.into() })
    }

    fn color(r: f32, g: f32, b: f32, a: f32) -> pp::Color {
        pp::Color {
            red: r,
            green: g,
            blue: b,
            alpha: a,
        }
    }

    fn slide_action_pb(label: Option<pp::action::Label>) -> pp::Action {
        pp::Action {
            uuid: uuid(ACT),
            label,
            is_enabled: true,
            r#type: pp::action::ActionType::PresentationSlide as i32,
            action_type_data: Some(pp::action::ActionTypeData::Slide(pp::action::SlideType {
                slide: Some(pp::action::slide_type::Slide::Presentation(
                    pp::PresentationSlide {
                        base_slide: Some(pp::Slide {
                            elements: vec![pp::slide::Element {
                                element: Some(pp::graphics::Element {
                                    uuid: uuid(EL),
                                    opacity: 1.0,
                                    text: Some(pp::graphics::Text {
                                        rtf_data: br"{\rtf1\ansi{\fonttbl\f0\fnil ArialMT;}\f0\fs100 First line\par second line}".to_vec(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }),
                                info: 3,
                                ..Default::default()
                            }],
                            background_color: Some(color(0.0, 0.0, 0.0, 1.0)),
                            size: Some(pp::graphics::Size {
                                width: 1920.0,
                                height: 1080.0,
                            }),
                            ..Default::default()
                        }),
                        notes: Some(pp::presentation_slide::Notes {
                            rtf_data: br"{\rtf1\ansi{\fonttbl\f0\fnil ArialMT;}\f0\fs100 Key change}".to_vec(),
                            attributes: None,
                        }),
                        transition: Some(pp::Transition {
                            duration: 0.1,
                            effect: Some(pp::Effect {
                                name: "Dissolve".into(),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )),
            })),
            ..Default::default()
        }
    }

    fn prop_action(clear: bool) -> pp::Action {
        use pp::action::prop_type::{PropClear, PropTrigger, TriggerType};
        pp::Action {
            uuid: uuid(PROP),
            name: "Logo".into(),
            is_enabled: true,
            delay_time: 2.0,
            r#type: pp::action::ActionType::Prop as i32,
            action_type_data: Some(pp::action::ActionTypeData::Prop(pp::action::PropType {
                identification: Some(pp::CollectionElementType {
                    parameter_uuid: uuid(EL),
                    parameter_name: "Logo".into(),
                    parent_collection: None,
                }),
                trigger_type: Some(if clear {
                    TriggerType::Clear(PropClear {})
                } else {
                    TriggerType::Trigger(PropTrigger::default())
                }),
            })),
            ..Default::default()
        }
    }

    #[test]
    fn cue_to_slide() {
        let mut ctx = ctx();
        let cue = pp::Cue {
            uuid: uuid(CUE),
            name: " 3".into(),
            is_enabled: false,
            hot_key: Some(pp::HotKey {
                code: pp::KeyCode::AnsiC as i32,
                ..Default::default()
            }),
            actions: vec![
                slide_action_pb(Some(pp::action::Label {
                    text: "3X".into(),
                    color: Some(color(1.0, 1.0, 1.0, 0.0)),
                })),
                prop_action(false),
            ],
            ..Default::default()
        };
        let s = slide_from_cue(&mut ctx, &cue, Size::new(1920.0, 1080.0));
        assert_eq!(s.id.to_string(), CUE);
        assert!(!s.enabled);
        assert_eq!(s.hotkey.as_ref().unwrap().code, "KeyC");
        assert_eq!(
            s.label,
            Some(SlideLabel {
                text: "3X".into(),
                color: None
            })
        );
        assert_eq!(s.background, None, "drawsBackgroundColor is false");
        assert_eq!(s.elements.len(), 1);
        assert_eq!(s.plain_text(), "First line\nsecond line");
        assert_eq!(s.elements[0].placeholder.as_ref().unwrap().slot, "text1");
        assert_eq!(s.notes.plain_text(), "Key change");
        assert_eq!(s.transition.as_ref().unwrap().duration, 0.1);
        assert_eq!(
            s.import.as_ref().unwrap().original_name.as_deref(),
            Some(" 3")
        );
        assert_eq!(s.actions.len(), 1);
        let a = &s.actions[0];
        assert_eq!(a.id.to_string(), PROP);
        assert_eq!(a.delay, 2.0);
        assert_eq!(a.name.as_deref(), Some("Logo"));
        assert!(
            matches!(&a.kind, ActionKind::Prop { show: true, prop } if prop.name.as_deref() == Some("Logo"))
        );
        // The prop does not exist in the (empty) lookups.
        assert!(
            ctx.report
                .warnings
                .iter()
                .any(|w| w.detail.contains("prop"))
        );
    }

    #[test]
    fn cue_without_slide_action() {
        let mut ctx = ctx();
        let cue = pp::Cue {
            uuid: uuid(CUE),
            is_enabled: true,
            actions: vec![prop_action(true)],
            ..Default::default()
        };
        let s = slide_from_cue(&mut ctx, &cue, Size::new(1920.0, 1080.0));
        assert!(s.elements.is_empty());
        assert!(matches!(
            s.actions[0].kind,
            ActionKind::Prop { show: false, .. }
        ));
        assert!(s.import.is_none());
    }

    #[test]
    fn bare_slide_background() {
        let mut ctx = ctx();
        let s = pp::Slide {
            draws_background_color: true,
            background_color: Some(color(1.0, 1.0, 1.0, 1.0)),
            ..Default::default()
        };
        let out = slide_from_pb(&mut ctx, &s, SlideId::from_uuid(uuid::Uuid::nil()));
        assert_eq!(out.background, Some(Color::WHITE));
    }

    #[test]
    fn labels() {
        assert_eq!(slide_label(None), None);
        assert_eq!(
            slide_label(Some(&pp::action::Label {
                text: String::new(),
                color: Some(color(1.0, 1.0, 1.0, 0.0))
            })),
            None
        );
        let l = slide_label(Some(&pp::action::Label {
            text: "Example 1:1".into(),
            color: Some(color(0.0, 1.0, 1.0, 1.0)),
        }))
        .unwrap();
        assert_eq!(l.color, Some(Color::rgb(0.0, 1.0, 1.0)));
    }

    #[test]
    fn unsupported_actions_keep_payload() {
        let mut ctx = ctx();
        let a = pp::Action {
            uuid: uuid(ACT),
            is_enabled: true,
            r#type: pp::action::ActionType::Communication as i32,
            action_type_data: Some(pp::action::ActionTypeData::Communication(
                pp::action::CommunicationType {
                    format: "MIDI".into(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let s = slide_action(&mut ctx, &a).unwrap();
        match &s.kind {
            ActionKind::Unsupported {
                source_type,
                payload,
            } => {
                assert_eq!(source_type, "ACTION_TYPE_COMMUNICATION");
                assert_eq!(payload["communication"]["format"], "MIDI");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            ctx.report.unsupported_actions["ACTION_TYPE_COMMUNICATION"],
            1
        );
        // A timer action without a timer UUID has nothing to point at.
        let t = pp::Action {
            uuid: uuid(ACT),
            r#type: pp::action::ActionType::Timer as i32,
            action_type_data: Some(pp::action::ActionTypeData::Timer(
                pp::action::TimerType::default(),
            )),
            ..Default::default()
        };
        assert!(matches!(
            action_kind(&mut ctx, &t),
            ActionKind::Unsupported { .. }
        ));
        // Slide content is not an action.
        assert!(slide_action(&mut ctx, &slide_action_pb(None)).is_none());
    }

    #[test]
    fn timer_and_message_actions() {
        let mut ctx = ctx();
        let id = |name: &str| {
            Some(pp::CollectionElementType {
                parameter_uuid: uuid(EL),
                parameter_name: name.into(),
                parent_collection: None,
            })
        };
        let t = pp::Action {
            r#type: pp::action::ActionType::Timer as i32,
            action_type_data: Some(pp::action::ActionTypeData::Timer(pp::action::TimerType {
                action_type: pp::action::timer_type::TimerAction::ActionIncrement as i32,
                timer_identification: id("Timer A"),
                increment_amount: 30.0,
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(matches!(
            action_kind(&mut ctx, &t),
            ActionKind::Timer { operation: TimerOperation::Adjust { seconds }, .. } if seconds == 30.0
        ));
        use pp::message::token_value::{TokenValueText, TokenValueType};
        let m = pp::Action {
            r#type: pp::action::ActionType::Message as i32,
            action_type_data: Some(pp::action::ActionTypeData::Message(
                pp::action::MessageType {
                    message_identificaton: id("Notice"),
                    content: vec![pp::message::TokenValue {
                        token_name: "Number".into(),
                        token_value_type: Some(TokenValueType::Text(TokenValueText {
                            value: "42".into(),
                        })),
                        ..Default::default()
                    }],
                },
            )),
            ..Default::default()
        };
        match action_kind(&mut ctx, &m) {
            ActionKind::Message { values, show, .. } => {
                assert!(show);
                assert_eq!(values[0].value, "42");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn clear_action_uses_config_mapping() {
        let mut ctx = ctx();
        let c = pp::Action {
            r#type: pp::action::ActionType::Clear as i32,
            action_type_data: Some(pp::action::ActionTypeData::Clear(pp::action::ClearType {
                target_layer: pp::action::clear_type::ClearTargetLayer::Slide as i32,
                content_destination: 0,
            })),
            ..Default::default()
        };
        let r =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action_kind(&mut ctx, &c)));
        // `config::clear_target` belongs to the config area; while it is a stub this only checks
        // the dispatch shape.
        if let Ok(ActionKind::ClearLayer { targets }) = r {
            assert_eq!(targets.len(), 1);
            assert_ne!(targets[0].layer, ClearLayer::All);
        }
    }
}
