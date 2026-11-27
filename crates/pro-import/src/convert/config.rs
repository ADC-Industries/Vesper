//! **Area: config.** The other `Configuration/*` documents → global groups, timers, stage
//! layouts, clear groups, props, messages, macros, show settings (CCLI); key mappings and the
//! calendar are reported only.
//!
//! ## Owns
//! - [`import_groups`] / [`global_group`]: `Configuration/Groups` → [`GlobalGroup`]s (ID =
//!   `applicationGroupIdentifier`, else `uuid`; name, colour, hotkey, list order sort keys);
//!   registers `ctx.lookups.global_groups` (presentations link sections through it).
//! - [`import_timers`] / [`timer`] / [`timer_kind`]: [`Timer`]s (countdown, countdown-to-time
//!   with AM/PM/24h, elapsed with optional end, `allowsOverrun`); registers `ctx.lookups.timers`
//!   (data links in themes / stage layouts resolve through it). The timers document's `clock`
//!   → unmapped count.
//! - [`import_stage`] / [`stage_layout`]: [`StageLayout`]s (ID = layout UUID, size from the
//!   slide, body via `slide::slide_from_pb`); registers `ctx.lookups.stage_layouts`.
//! - [`import_clear_groups`] / [`clear_group`] / [`clear_target`] / [`timeline_target`]:
//!   [`ClearGroup`]s keeping every (layer, destination) pair in order, timeline targets →
//!   `ClearLayer::Timelines`; icon / tint kept. `clear_target` is also used by the slides area for
//!   `ACTION_TYPE_CLEAR`.
//! - [`import_props`] / [`prop`]: each prop cue (`ACTION_TYPE_PROP_SLIDE`) → [`Prop`] (ID = cue
//!   UUID, slide body from `PropSlide.baseSlide`, transition = the prop slide's or the
//!   document's); prop collections → unmapped count; registers `ctx.lookups.props`.
//! - [`import_messages`] / [`message`]: [`Message`]s (text, tokens, template as a provisional
//!   [`TemplateRef`] fixed by `workspace::resolve_theme_links`); registers `ctx.lookups.messages`.
//! - [`import_macros`] / [`macro_doc`]: [`Macro`]s (actions via `slide::slide_actions`, colour,
//!   `triggerOnStartup`); macro collections → unmapped count; registers `ctx.lookups.macros`.
//! - [`import_ccli`] / [`ccli_settings`]: CCLI → `ShowSettings.ccli` (licence, enabled, display
//!   rule, token layout, inline template slide via `slide::slide_from_pb`).
//! - [`show_settings`]: get-or-create the singleton in `ctx.out.show_settings` (also used by
//!   `workspace.rs` for `startupLook`).
//! - [`report_key_mappings`] / [`report_calendar`]: `notImported` warnings when non-empty.
//!
//! ## Conventions used here
//! - **IDs** are source UUIDs; a missing / invalid one gets a UUIDv5 from the document path plus
//!   the list index. A UUID that repeats inside one document gets a derived ID (warned), so no
//!   two documents share an ID (global groups: the repeat is skipped, see [`import_groups`]).
//! - **Sort keys** follow the source list order (`ids::sort_keys`).
//! - **Unmodelled data:** documents without an `import` slot (props, CCLI) keep it in their
//!   slide's `import.unmapped`; pure UI state (icons, collections, preview flags) is only counted
//!   in `unmappedFeatures`.
//! - **Times** are seconds. A countdown-to-time target is stored as seconds after midnight on a
//!   24-hour clock: ProPresenter counts `timeOfDay` from 12:00 within the AM / PM period, so
//!   `300 + PM` is 12:05 PM = 43,500 s; the period is kept as the entry / display hint.
//!
//! ## May call
//! `common::*`, `slide::slide_from_pb`, `slide::slide_actions`, `slide::slide_from_cue`,
//! `element::timer_format`, `element::clock_format`, `ImportContext` helpers.

use std::collections::BTreeSet;

use model::{
    CcliDisplay, CcliSettings, CcliToken, ClearGroup, ClearGroupId, ClearLayer, ClearTarget,
    ContentDestination, GlobalGroup, GlobalGroupId, Link, Macro, MacroId, Message, MessageId,
    MessageToken, MessageTokenKind, Prop, PropId, ShowSettings, Size, SlideId, SortKey,
    StageLayout, StageLayoutId, TemplateRef, TemplateSlideId, ThemeId, TimePeriod, Timer, TimerId,
    TimerKind,
};
use serde_json::json;
use uuid::Uuid;

use crate::context::{ImportContext, NamedId, ids};
use crate::convert::{common, element, slide};
use crate::pb::rv::data as pp;
use crate::report::Category;

/// Seconds in 12 hours.
const HALF_DAY: f64 = 43_200.0;

/// `Configuration/Groups`.
///
/// A group whose identity (`applicationGroupIdentifier`, else `uuid`) repeats an earlier one is
/// skipped with a warning: sections link by that identity and could not tell them apart.
pub fn import_groups(ctx: &mut ImportContext, doc: &pp::ProGroupsDocument, rel_path: &str) {
    let keys = ids::sort_keys(doc.groups.len());
    let mut seen: BTreeSet<GlobalGroupId> = BTreeSet::new();
    for (i, (g, key)) in doc.groups.iter().zip(keys).enumerate() {
        let mut group = global_group(ctx, g, key);
        if ids::parse_uuid(g.application_group_identifier.as_ref()).is_none()
            && ids::parse_uuid(g.uuid.as_ref()).is_none()
        {
            group.id =
                GlobalGroupId::from_uuid(ids::derived("globalGroup", &[rel_path, &i.to_string()]));
        }
        if !seen.insert(group.id) {
            ctx.warn(
                Category::Other,
                format!(
                    "global group \"{}\" repeats the identifier of an earlier group and was skipped",
                    group.name
                ),
            );
            continue;
        }
        let named = NamedId {
            id: group.id,
            name: group.name.clone(),
        };
        // Sections link by `applicationGroupIdentifier`; register the plain UUID too.
        for u in [
            ids::parse_uuid(g.application_group_identifier.as_ref()),
            ids::parse_uuid(g.uuid.as_ref()),
            Some(group.id.0),
        ]
        .into_iter()
        .flatten()
        {
            ctx.lookups
                .global_groups
                .entry(u)
                .or_insert_with(|| named.clone());
        }
        ctx.out.global_groups.push(group);
    }
}

/// One global group.
pub fn global_group(ctx: &mut ImportContext, g: &pp::Group, sort_key: SortKey) -> GlobalGroup {
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = ids::parse_uuid(g.application_group_identifier.as_ref())
        .or_else(|| ids::parse_uuid(g.uuid.as_ref()))
        .unwrap_or_else(|| ids::derived("globalGroup", &[&path, &g.name]));
    let name = if g.name.trim().is_empty() {
        g.application_group_name.clone()
    } else {
        g.name.clone()
    };
    if !g.application_group_name.is_empty() && g.application_group_name != name {
        ctx.count_unmapped("group.applicationGroupName");
    }
    GlobalGroup {
        id: GlobalGroupId::from_uuid(id),
        name,
        color: common::visible_color(g.color.as_ref()),
        hotkey: common::hotkey(g.hot_key.as_ref()),
        sort_key,
        ..GlobalGroup::default()
    }
}

/// `Configuration/Timers`.
pub fn import_timers(ctx: &mut ImportContext, doc: &pp::TimersDocument, rel_path: &str) {
    ctx.set_writer(doc.application_info.as_ref());
    if doc.clock.is_some() {
        ctx.count_unmapped("timers.clock");
    }
    let keys = ids::sort_keys(doc.timers.len());
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    for (i, (t, key)) in doc.timers.iter().zip(keys).enumerate() {
        let mut tm = timer(ctx, t, key, i);
        if !seen.insert(tm.id.0) {
            ctx.warn(
                Category::Other,
                format!("timer \"{}\" repeats an ID; a new one was derived", tm.name),
            );
            tm.id = TimerId::from_uuid(ids::derived("timer", &[rel_path, &i.to_string()]));
            seen.insert(tm.id.0);
        }
        let source = ids::parse_uuid(t.uuid.as_ref()).unwrap_or(tm.id.0);
        ctx.lookups.timers.entry(source).or_insert_with(|| NamedId {
            id: tm.id,
            name: tm.name.clone(),
        });
        ctx.out.timers.push(tm);
    }
}

/// One timer.
pub fn timer(ctx: &mut ImportContext, t: &pp::Timer, sort_key: SortKey, index: usize) -> Timer {
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = TimerId::from_uuid(ids::source_or_derived(
        t.uuid.as_ref(),
        "timer",
        &[&path, &index.to_string()],
    ));
    let (kind, allow_overrun) = timer_kind(t.configuration.as_ref());
    Timer {
        id,
        name: t.name.clone(),
        sort_key,
        kind,
        allow_overrun,
        ..Timer::default()
    }
}

/// Timer configuration → (kind, allow overrun).
///
/// - countdown → `Countdown { seconds: duration }`;
/// - countdown-to-time → `CountdownToTime { secondsAfterMidnight, period }` (24-hour seconds,
///   see the module docs);
/// - elapsed → `Elapsed { start (unless 0), end (only with hasEndTime) }`;
/// - no configuration → a zero countdown. Non-finite / negative times become 0.
pub fn timer_kind(c: Option<&pp::timer::Configuration>) -> (TimerKind, bool) {
    use pp::timer::configuration::TimerType;
    use pp::timer::configuration::timer_type_countdown_to_time::TimePeriod as P;
    let Some(c) = c else {
        return (TimerKind::default(), false);
    };
    let sane = |v: f64| if v.is_finite() && v > 0.0 { v } else { 0.0 };
    let kind = match c.timer_type.as_ref() {
        None => TimerKind::default(),
        Some(TimerType::Countdown(cd)) => TimerKind::Countdown {
            seconds: sane(cd.duration),
        },
        Some(TimerType::CountdownToTime(ct)) => {
            let tod = sane(ct.time_of_day);
            let (secs, period) = match P::try_from(ct.period) {
                Ok(P::Am) => (tod % HALF_DAY, TimePeriod::Am),
                Ok(P::Pm) => (tod % HALF_DAY + HALF_DAY, TimePeriod::Pm),
                Ok(P::TimePeriod24) => (tod % (2.0 * HALF_DAY), TimePeriod::H24),
                Err(_) => (tod % (2.0 * HALF_DAY), TimePeriod::Unknown),
            };
            TimerKind::CountdownToTime {
                seconds_after_midnight: secs,
                period,
            }
        }
        Some(TimerType::ElapsedTime(e)) => TimerKind::Elapsed {
            start: (e.start_time.is_finite() && e.start_time != 0.0).then_some(e.start_time),
            end: (e.has_end_time && e.end_time.is_finite()).then_some(e.end_time),
        },
    };
    (kind, c.allows_overrun)
}

/// `Configuration/Stage`.
pub fn import_stage(ctx: &mut ImportContext, doc: &pp::stage::Document, rel_path: &str) {
    ctx.set_writer(doc.application_info.as_ref());
    let keys = ids::sort_keys(doc.layouts.len());
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    for (i, (l, key)) in doc.layouts.iter().zip(keys).enumerate() {
        let mut layout = stage_layout(ctx, l, key, i);
        if !seen.insert(layout.id.0) {
            ctx.warn(
                Category::Other,
                format!(
                    "stage layout \"{}\" repeats an ID; a new one was derived",
                    layout.name
                ),
            );
            layout.id =
                StageLayoutId::from_uuid(ids::derived("stageLayout", &[rel_path, &i.to_string()]));
            seen.insert(layout.id.0);
        }
        let source = ids::parse_uuid(l.uuid.as_ref()).unwrap_or(layout.id.0);
        ctx.lookups
            .stage_layouts
            .entry(source)
            .or_insert_with(|| NamedId {
                id: layout.id,
                name: layout.name.clone(),
            });
        ctx.out.stage_layouts.push(layout);
    }
}

/// One stage layout.
pub fn stage_layout(
    ctx: &mut ImportContext,
    l: &pp::stage::Layout,
    sort_key: SortKey,
    index: usize,
) -> StageLayout {
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = ids::source_or_derived(l.uuid.as_ref(), "stageLayout", &[&path, &index.to_string()]);
    let body = l.slide.clone().unwrap_or_default();
    let slide_id = ids::source_or_derived(body.uuid.as_ref(), "stageSlide", &[&id.to_string()]);
    let size = common::size(body.size.as_ref()).unwrap_or(Size::HD);
    let slide = slide::slide_from_pb(ctx, &body, SlideId::from_uuid(slide_id));
    StageLayout {
        id: StageLayoutId::from_uuid(id),
        name: l.name.clone(),
        sort_key,
        size,
        slide,
        import: Some(ctx.document_info()),
        ..StageLayout::default()
    }
}

/// `Configuration/ClearGroups`.
pub fn import_clear_groups(ctx: &mut ImportContext, doc: &pp::ClearGroupsDocument, rel_path: &str) {
    ctx.set_writer(doc.application_info.as_ref());
    let keys = ids::sort_keys(doc.groups.len());
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    for (i, (g, key)) in doc.groups.iter().zip(keys).enumerate() {
        let mut group = clear_group(ctx, g, key, i);
        if !seen.insert(group.id.0) {
            ctx.warn(
                Category::Other,
                format!(
                    "clear group \"{}\" repeats an ID; a new one was derived",
                    group.name
                ),
            );
            group.id =
                ClearGroupId::from_uuid(ids::derived("clearGroup", &[rel_path, &i.to_string()]));
            seen.insert(group.id.0);
        }
        let source = ids::parse_uuid(g.uuid.as_ref()).unwrap_or(group.id.0);
        ctx.lookups
            .clear_groups
            .entry(source)
            .or_insert_with(|| NamedId {
                id: group.id,
                name: group.name.clone(),
            });
        ctx.out.clear_groups.push(group);
    }
}

/// One clear group.
pub fn clear_group(
    ctx: &mut ImportContext,
    g: &pp::clear_groups_document::ClearGroup,
    sort_key: SortKey,
    index: usize,
) -> ClearGroup {
    use pp::clear_groups_document::clear_group::ImageType;
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = ids::source_or_derived(g.uuid.as_ref(), "clearGroup", &[&path, &index.to_string()]);
    let mut targets: Vec<ClearTarget> = g.layer_targets.iter().map(clear_target).collect();
    targets.extend(g.timeline_targets.iter().map(|&d| timeline_target(d)));

    // Icon: the built-in symbol name ("all", "one", "xClear", ...). A custom image is UI only.
    let icon = match ImageType::try_from(g.image_type) {
        Ok(ImageType::Custom) | Err(_) => None,
        Ok(t) => Some(icon_name(t.as_str_name())),
    };
    if !g.image_data.is_empty() {
        ctx.count_unmapped("clearGroup.imageData");
    }
    if g.is_hidden_in_preview {
        ctx.count_unmapped("clearGroup.isHiddenInPreview");
    }
    if g.clear_presentation_next_slide {
        ctx.count_unmapped("clearGroup.clearPresentationNextSlide");
    }
    ClearGroup {
        id: ClearGroupId::from_uuid(id),
        name: g.name.clone(),
        sort_key,
        targets,
        icon,
        tint: if g.is_icon_tinted {
            g.icon_tint_color.as_ref().map(common::color)
        } else {
            None
        },
        ..ClearGroup::default()
    }
}

/// `"ImageTypeXClear"` → `"xClear"`.
fn icon_name(source: &str) -> String {
    let s = source.strip_prefix("ImageType").unwrap_or(source);
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_lowercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

/// `Action.ContentDestination` → model destination (`GLOBAL` = no restriction).
fn destination(d: i32) -> Option<ContentDestination> {
    match pp::action::ContentDestination::try_from(d) {
        Ok(pp::action::ContentDestination::Announcements) => {
            Some(ContentDestination::Announcements)
        }
        Ok(pp::action::ContentDestination::Global) => None,
        Err(_) => Some(ContentDestination::Unknown),
    }
}

/// `Action.ClearType` → (layer, destination) pair.
///
/// Layers: ALL → all, AUDIO → audio, AUDIO_EFFECTS → audioEffects, BACKGROUND → media,
/// LIVE_VIDEO → videoInput, SLIDE → slide, PROP → props, MESSAGES → messages, LOGO → mask.
/// Destination: GLOBAL → `None` (any), ANNOUNCEMENTS → announcements.
pub fn clear_target(c: &pp::action::ClearType) -> ClearTarget {
    use pp::action::clear_type::ClearTargetLayer as L;
    let layer = match L::try_from(c.target_layer) {
        Ok(L::All) => ClearLayer::All,
        Ok(L::Audio) => ClearLayer::Audio,
        Ok(L::AudioEffects) => ClearLayer::AudioEffects,
        Ok(L::Background) => ClearLayer::Media,
        Ok(L::LiveVideo) => ClearLayer::VideoInput,
        Ok(L::Slide) => ClearLayer::Slide,
        Ok(L::Prop) => ClearLayer::Props,
        Ok(L::Messages) => ClearLayer::Messages,
        Ok(L::Logo) => ClearLayer::Mask,
        Err(_) => ClearLayer::Unknown,
    };
    ClearTarget {
        layer,
        destination: destination(c.content_destination),
    }
}

/// A clear group's timeline target (a content destination) → `ClearLayer::Timelines` pair.
pub fn timeline_target(content_destination: i32) -> ClearTarget {
    ClearTarget {
        layer: ClearLayer::Timelines,
        destination: destination(content_destination),
    }
}

/// `Configuration/Props`.
pub fn import_props(ctx: &mut ImportContext, doc: &pp::PropDocument, rel_path: &str) {
    ctx.set_writer(doc.application_info.as_ref());
    // Cues of the document, then cues that only live inside a collection.
    let mut cues: Vec<&pp::Cue> = doc.cues.iter().collect();
    let mut known: BTreeSet<Uuid> = doc
        .cues
        .iter()
        .filter_map(|c| ids::parse_uuid(c.uuid.as_ref()))
        .collect();
    for coll in &doc.prop_collections {
        ctx.count_unmapped("props.propCollections");
        if coll.single_prop_enabled {
            ctx.count_unmapped("props.propCollections.singlePropEnabled");
        }
        for c in &coll.cues {
            match ids::parse_uuid(c.uuid.as_ref()) {
                Some(u) if !known.insert(u) => {}
                _ => cues.push(c),
            }
        }
    }
    let keys = ids::sort_keys(cues.len());
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    for (i, (cue, key)) in cues.into_iter().zip(keys).enumerate() {
        let Some(mut p) = prop(ctx, cue, doc.transition.as_ref(), key) else {
            ctx.warn(
                Category::Other,
                format!(
                    "prop cue \"{}\" holds no prop slide and was skipped",
                    cue.name
                ),
            );
            continue;
        };
        if !seen.insert(p.id.0) {
            ctx.warn(
                Category::Other,
                format!("prop \"{}\" repeats an ID; a new one was derived", p.name),
            );
            p.id = PropId::from_uuid(ids::derived("prop", &[rel_path, &i.to_string()]));
            seen.insert(p.id.0);
        }
        let source = ids::parse_uuid(cue.uuid.as_ref()).unwrap_or(p.id.0);
        ctx.lookups.props.entry(source).or_insert_with(|| NamedId {
            id: p.id,
            name: p.name.clone(),
        });
        ctx.out.props.push(p);
    }
}

/// The `PropSlide` of a cue, with the index of its action.
fn prop_slide(cue: &pp::Cue) -> Option<(usize, &pp::PropSlide)> {
    cue.actions
        .iter()
        .enumerate()
        .find_map(|(i, a)| match &a.action_type_data {
            Some(pp::action::ActionTypeData::Slide(pp::action::SlideType {
                slide: Some(pp::action::slide_type::Slide::Prop(ps)),
            })) => Some((i, ps)),
            _ => None,
        })
}

/// One prop cue (`None` when the cue holds no prop slide).
///
/// The slide body comes from `PropSlide.baseSlide` (`slide::slide_from_pb`); the cue provides
/// `enabled` and the hotkey, its other actions become slide actions; the prop slide's auto-clear
/// settings go to the slide's `import.unmapped.autoClear`.
pub fn prop(
    ctx: &mut ImportContext,
    cue: &pp::Cue,
    default_transition: Option<&pp::Transition>,
    sort_key: SortKey,
) -> Option<Prop> {
    let (index, ps) = prop_slide(cue)?;
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = ids::source_or_derived(cue.uuid.as_ref(), "prop", &[&path, &cue.name]);
    let base = ps.base_slide.clone().unwrap_or_default();
    let slide_id = ids::source_or_derived(base.uuid.as_ref(), "propSlide", &[&id.to_string()]);
    let size = common::size(base.size.as_ref()).unwrap_or(Size::HD);
    let mut body = slide::slide_from_pb(ctx, &base, SlideId::from_uuid(slide_id));
    body.enabled = cue.is_enabled;
    body.hotkey = common::hotkey(cue.hot_key.as_ref());
    body.actions = slide::slide_actions(ctx, &cue.actions, Some(index));
    if ps.auto_clear_enabled || ps.auto_clear_duration != 0.0 {
        ctx.keep_unmapped(
            &mut body.import,
            "propSlide.autoClear",
            json!({ "enabled": ps.auto_clear_enabled, "duration": ps.auto_clear_duration }),
        );
    }
    let transition = ps
        .transition
        .as_ref()
        .or(default_transition)
        .map(|t| common::transition(ctx, t));
    Some(Prop {
        id: PropId::from_uuid(id),
        name: cue.name.clone(),
        sort_key,
        size,
        slide: body,
        transition,
        ..Prop::default()
    })
}

/// `Configuration/Messages`.
pub fn import_messages(ctx: &mut ImportContext, doc: &pp::MessageDocument, rel_path: &str) {
    ctx.set_writer(doc.application_info.as_ref());
    let keys = ids::sort_keys(doc.messages.len());
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    for (i, (m, key)) in doc.messages.iter().zip(keys).enumerate() {
        let mut msg = message(ctx, m, key, i);
        if !seen.insert(msg.id.0) {
            ctx.warn(
                Category::Other,
                format!(
                    "message \"{}\" repeats an ID; a new one was derived",
                    msg.name
                ),
            );
            msg.id = MessageId::from_uuid(ids::derived("message", &[rel_path, &i.to_string()]));
            seen.insert(msg.id.0);
        }
        let source = ids::parse_uuid(m.uuid.as_ref()).unwrap_or(msg.id.0);
        ctx.lookups
            .messages
            .entry(source)
            .or_insert_with(|| NamedId {
                id: msg.id,
                name: msg.name.clone(),
            });
        ctx.out.messages.push(msg);
    }
}

/// One message.
///
/// Tokens keep their names (`${name}` in the text). A token's stored value (`tokenValues`, matched
/// by token UUID, else by name) supplies a text token's default value and a timer / clock token's
/// display format. Clear behaviour (`clearType`, `timeToRemove`) and `visibleOnNetwork` have no
/// model slot and are counted as unmapped.
pub fn message(
    ctx: &mut ImportContext,
    m: &pp::Message,
    sort_key: SortKey,
    index: usize,
) -> Message {
    use pp::message::token::TokenType;
    use pp::message::token_value::TokenValueType;
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = ids::source_or_derived(m.uuid.as_ref(), "message", &[&path, &index.to_string()]);

    let mut tokens = Vec::new();
    for t in &m.tokens {
        let token_uuid = ids::parse_uuid(t.uuid.as_ref());
        let declared = match t.token_type.as_ref() {
            Some(TokenType::Text(x)) => x.name.clone(),
            Some(TokenType::Timer(x)) => x.name.clone(),
            Some(TokenType::Clock(_)) | None => String::new(),
        };
        let value = m.token_values.iter().find(|v| {
            (token_uuid.is_some() && ids::parse_uuid(v.token_id.as_ref()) == token_uuid)
                || (!declared.is_empty() && v.token_name == declared)
        });
        let name = if declared.is_empty() {
            value
                .map(|v| v.token_name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "clock".into())
        } else {
            declared
        };
        let value_type = value.and_then(|v| v.token_value_type.as_ref());
        let kind = match t.token_type.as_ref() {
            Some(TokenType::Text(_)) => MessageTokenKind::Text {
                default_value: match value_type {
                    Some(TokenValueType::Text(x)) => x.value.clone(),
                    _ => String::new(),
                },
            },
            Some(TokenType::Timer(x)) => {
                let mut format = model::TimerFormat::default();
                if let Some(TokenValueType::Timer(v)) = value_type {
                    if v.format.is_some() {
                        format = element::timer_format(v.format.as_ref(), "");
                    }
                    if v.configuration.is_some() {
                        // A per-message timer configuration; the model links timers only.
                        ctx.count_unmapped("message.tokenValues.timer.configuration");
                    }
                }
                MessageTokenKind::Timer {
                    timer: timer_link(ctx, x.timer_uuid.as_ref(), &x.name),
                    format,
                }
            }
            Some(TokenType::Clock(_)) => {
                let mut format = model::ClockFormat::default();
                if let Some(TokenValueType::Clock(v)) = value_type
                    && v.format.is_some()
                {
                    format = element::clock_format(v.format.as_ref(), "");
                }
                MessageTokenKind::Clock { format }
            }
            None => MessageTokenKind::Unsupported {
                source_type: "Message.Token".into(),
                payload: ctx.to_json(t),
            },
        };
        tokens.push(MessageToken { name, kind });
    }

    if m.clear_type != 0 || m.time_to_remove != 0.0 {
        ctx.count_unmapped("message.clearType");
    }
    if m.visible_on_network {
        ctx.count_unmapped("message.visibleOnNetwork");
    }
    let template = m.template.as_ref().and_then(|t| template_ref(ctx, t));
    Message {
        id: MessageId::from_uuid(id),
        name: m.title.clone(),
        sort_key,
        text: m.message_text.clone(),
        template,
        tokens,
        ..Message::default()
    }
}

/// A timer link from a UUID + name (`None` when unbound; dangling links keep the name hint).
fn timer_link(
    ctx: &mut ImportContext,
    uuid: Option<&pp::Uuid>,
    name: &str,
) -> Option<Link<TimerId>> {
    let u = ids::parse_uuid(uuid)?;
    if let Some(t) = ctx.lookups.timers.get(&u) {
        return Some(Link::named(t.id, t.name.clone()));
    }
    ctx.warn(
        Category::Other,
        format!("a message token links timer \"{name}\" ({u}), which does not exist"),
    );
    Some(if name.is_empty() {
        Link::to(TimerId::from_uuid(u))
    } else {
        Link::named(TimerId::from_uuid(u), name)
    })
}

/// A message template reference (provisional theme ID, fixed by the post-pass).
///
/// Theme documents have no UUID of their own, so the provisional theme ID is the source theme
/// UUID (or one derived from the theme name) with the theme name as hint; the post-pass resolves
/// the slide UUID through `ctx.lookups.template_slides`, else by theme name. `None` without a
/// slide UUID.
pub fn template_ref(
    ctx: &mut ImportContext,
    t: &pp::TemplateIdentification,
) -> Option<TemplateRef> {
    let Some(slide) = ids::parse_uuid(t.slide_uuid.as_ref()) else {
        if !t.name.is_empty() || ids::parse_uuid(t.uuid.as_ref()).is_some() {
            ctx.warn(
                Category::Other,
                format!(
                    "a message template of theme \"{}\" names no slide; dropped",
                    t.name
                ),
            );
            ctx.count_unmapped("message.template");
        }
        return None;
    };
    let theme_id = ids::parse_uuid(t.uuid.as_ref())
        .unwrap_or_else(|| ids::derived("messageTheme", &[&t.name]));
    let theme = if t.name.is_empty() {
        Link::to(ThemeId::from_uuid(theme_id))
    } else {
        Link::named(ThemeId::from_uuid(theme_id), t.name.clone())
    };
    Some(TemplateRef {
        theme,
        slide: TemplateSlideId::from_uuid(slide),
    })
}

/// `Configuration/Macros`.
pub fn import_macros(ctx: &mut ImportContext, doc: &pp::MacrosDocument, rel_path: &str) {
    ctx.set_writer(doc.application_info.as_ref());
    let mut macros: Vec<&pp::macros_document::Macro> = doc.macros.iter().collect();
    let mut known: BTreeSet<Uuid> = doc
        .macros
        .iter()
        .filter_map(|m| ids::parse_uuid(m.uuid.as_ref()))
        .collect();
    for coll in &doc.macro_collections {
        ctx.count_unmapped("macros.macroCollections");
        for m in &coll.macros {
            match ids::parse_uuid(m.uuid.as_ref()) {
                Some(u) if !known.insert(u) => {}
                _ => macros.push(m),
            }
        }
    }
    let keys = ids::sort_keys(macros.len());
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    for (i, (m, key)) in macros.into_iter().zip(keys).enumerate() {
        let mut mc = macro_doc(ctx, m, key, i);
        if !seen.insert(mc.id.0) {
            ctx.warn(
                Category::Other,
                format!("macro \"{}\" repeats an ID; a new one was derived", mc.name),
            );
            mc.id = MacroId::from_uuid(ids::derived("macro", &[rel_path, &i.to_string()]));
            seen.insert(mc.id.0);
        }
        let source = ids::parse_uuid(m.uuid.as_ref()).unwrap_or(mc.id.0);
        ctx.lookups.macros.entry(source).or_insert_with(|| NamedId {
            id: mc.id,
            name: mc.name.clone(),
        });
        ctx.out.macros.push(mc);
    }
}

/// One macro.
pub fn macro_doc(
    ctx: &mut ImportContext,
    m: &pp::macros_document::Macro,
    sort_key: SortKey,
    index: usize,
) -> Macro {
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = ids::source_or_derived(m.uuid.as_ref(), "macro", &[&path, &index.to_string()]);
    if m.image_type != 0 || !m.image_data.is_empty() {
        ctx.count_unmapped("macro.image");
    }
    let actions = slide::slide_actions(ctx, &m.actions, None);
    Macro {
        id: MacroId::from_uuid(id),
        name: m.name.clone(),
        sort_key,
        color: common::visible_color(m.color.as_ref()),
        actions,
        run_on_startup: m.trigger_on_startup,
        ..Macro::default()
    }
}

/// `Configuration/CCLI` → `ShowSettings.ccli`.
pub fn import_ccli(ctx: &mut ImportContext, doc: &pp::CcliDocument, rel_path: &str) {
    let _ = rel_path;
    ctx.set_writer(doc.application_info.as_ref());
    let settings = ccli_settings(ctx, doc);
    show_settings(ctx).ccli = settings;
}

/// CCLI document → settings. The template slide is inline; its name goes to
/// `import.originalName` and a canvas other than 1920×1080 to `import.unmapped.size` (the model
/// draws it at the show's default canvas size).
pub fn ccli_settings(ctx: &mut ImportContext, doc: &pp::CcliDocument) -> CcliSettings {
    use pp::ccli_document::DisplayType;
    use pp::copyright_layout::TokenType;
    let show_on = match DisplayType::try_from(doc.display_type) {
        Ok(DisplayType::FirstSlide) => CcliDisplay::FirstSlide,
        Ok(DisplayType::LastSlide) => CcliDisplay::LastSlide,
        Ok(DisplayType::FirstAndLastSlide) => CcliDisplay::FirstAndLast,
        Ok(DisplayType::AllSlides) => CcliDisplay::AllSlides,
        Err(_) => CcliDisplay::Unknown,
    };
    let layout = doc
        .layout
        .as_ref()
        .map(|l| {
            l.tokens
                .iter()
                .map(|t| match TokenType::try_from(t.token_type) {
                    Ok(TokenType::Text) => CcliToken::Text {
                        text: t.text.clone(),
                    },
                    Ok(TokenType::Artist) => CcliToken::Artist,
                    Ok(TokenType::Author) => CcliToken::Author,
                    Ok(TokenType::Publisher) => CcliToken::Publisher,
                    Ok(TokenType::Title) => CcliToken::Title,
                    Ok(TokenType::CopyrightYear) => CcliToken::CopyrightYear,
                    Ok(TokenType::LicenseNumber) => CcliToken::LicenseNumber,
                    Ok(TokenType::SongNumber) => CcliToken::SongNumber,
                    Err(_) => CcliToken::Unknown,
                })
                .collect()
        })
        .unwrap_or_default();
    let template = doc.template.as_ref().map(|t| {
        let base = t.base_slide.clone().unwrap_or_default();
        let path = ctx.current.path.clone().unwrap_or_default();
        let id = ids::source_or_derived(base.uuid.as_ref(), "ccliTemplate", &[&path]);
        let mut s = slide::slide_from_pb(ctx, &base, SlideId::from_uuid(id));
        let actions = slide::slide_actions(ctx, &t.actions, None);
        s.actions.extend(actions);
        if !t.name.is_empty() {
            s.import.get_or_insert_with(Default::default).original_name = Some(t.name.clone());
        }
        if let Some(size) = common::size(base.size.as_ref())
            && size != Size::HD
        {
            ctx.keep_unmapped(
                &mut s.import,
                "ccli.template.size",
                json!({ "width": size.width, "height": size.height }),
            );
        }
        s
    });
    let license = doc.ccli_license.trim();
    CcliSettings {
        license_number: (!license.is_empty()).then(|| license.to_string()),
        enabled: doc.enable_ccli_display,
        show_on,
        layout,
        template,
    }
}

/// The show settings singleton (created on first use).
pub fn show_settings(ctx: &mut ImportContext) -> &mut ShowSettings {
    ctx.out
        .show_settings
        .get_or_insert_with(ShowSettings::default)
}

/// `Configuration/KeyMappings`: reported (`notImported`) when it has mappings.
pub fn report_key_mappings(ctx: &mut ImportContext, doc: &pp::KeyMappingDocument, rel_path: &str) {
    let _ = rel_path;
    ctx.set_writer(doc.application_info.as_ref());
    let n = doc.keymappings.len() + doc.macos_keymappings.len() + doc.windows_keymappings.len();
    if n > 0 {
        ctx.count_unmapped("keyMappings");
        ctx.warn(
            Category::NotImported,
            format!("{n} keyboard / MIDI mapping(s) are not imported"),
        );
    }
}

/// `Configuration/Calendar`: reported (`notImported`) when it has events.
pub fn report_calendar(ctx: &mut ImportContext, doc: &pp::Calendar, rel_path: &str) {
    let _ = rel_path;
    if !doc.events.is_empty() {
        ctx.count_unmapped("calendar.events");
        ctx.warn(
            Category::NotImported,
            format!(
                "{} scheduled calendar event(s) are not imported{}",
                doc.events.len(),
                if doc.active {
                    ""
                } else {
                    " (calendar inactive)"
                }
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImportContext, ImportOptions};
    use std::path::Path;

    fn ctx(path: &str) -> ImportContext {
        let mut c = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        c.current.path = Some(path.into());
        c
    }

    fn uuid(s: &str) -> Option<pp::Uuid> {
        Some(pp::Uuid { string: s.into() })
    }

    const A: &str = "11111111-1111-4111-8111-111111111111";
    const B: &str = "22222222-2222-4222-8222-222222222222";
    const C: &str = "33333333-3333-4333-8333-333333333333";

    fn u(s: &str) -> Uuid {
        Uuid::parse_str(s).unwrap()
    }

    #[test]
    fn groups_use_application_identifier_and_skip_repeats() {
        let mut ctx = ctx("Configuration/Groups");
        let doc = pp::ProGroupsDocument {
            groups: vec![
                pp::Group {
                    uuid: uuid(A),
                    name: "Verse".into(),
                    color: Some(pp::Color {
                        red: 0.0,
                        green: 0.0,
                        blue: 1.0,
                        alpha: 1.0,
                    }),
                    hot_key: Some(pp::HotKey {
                        code: pp::KeyCode::AnsiV as i32,
                        ..Default::default()
                    }),
                    application_group_identifier: uuid(B),
                    application_group_name: "Verse".into(),
                },
                pp::Group {
                    uuid: uuid(B),
                    name: "Again".into(),
                    ..Default::default()
                },
                pp::Group {
                    name: "No Id".into(),
                    ..Default::default()
                },
            ],
        };
        import_groups(&mut ctx, &doc, "Configuration/Groups");
        assert_eq!(ctx.out.global_groups.len(), 2);
        let g = &ctx.out.global_groups[0];
        assert_eq!(g.id.0, u(B));
        assert_eq!(g.hotkey.as_ref().unwrap().code, "KeyV");
        assert_eq!(g.color, Some(model::Color::rgb(0.0, 0.0, 1.0)));
        assert!(ctx.lookups.global_groups.contains_key(&u(A)));
        assert!(ctx.lookups.global_groups.contains_key(&u(B)));
        assert_eq!(ctx.report.warnings.len(), 1);
        assert!(ctx.out.global_groups[0].sort_key < ctx.out.global_groups[1].sort_key);
        assert_eq!(ctx.out.global_groups[1].color, None);
    }

    #[test]
    fn timer_kinds() {
        use pp::timer::configuration::timer_type_countdown_to_time::TimePeriod as P;
        use pp::timer::configuration::*;
        let cfg = |t: TimerType, overrun: bool| pp::timer::Configuration {
            allows_overrun: overrun,
            timer_type: Some(t),
        };
        assert_eq!(
            timer_kind(Some(&cfg(
                TimerType::Countdown(TimerTypeCountdown { duration: 300.0 }),
                true
            ))),
            (TimerKind::Countdown { seconds: 300.0 }, true)
        );
        let to_time = |tod: f64, p: P| {
            timer_kind(Some(&cfg(
                TimerType::CountdownToTime(TimerTypeCountdownToTime {
                    time_of_day: tod,
                    period: p as i32,
                }),
                false,
            )))
            .0
        };
        assert_eq!(
            to_time(300.0, P::Pm),
            TimerKind::CountdownToTime {
                seconds_after_midnight: 43_500.0,
                period: TimePeriod::Pm
            }
        );
        assert_eq!(
            to_time(300.0, P::Am),
            TimerKind::CountdownToTime {
                seconds_after_midnight: 300.0,
                period: TimePeriod::Am
            }
        );
        assert_eq!(
            to_time(50_000.0, P::TimePeriod24),
            TimerKind::CountdownToTime {
                seconds_after_midnight: 50_000.0,
                period: TimePeriod::H24
            }
        );
        let elapsed = |start: f64, end: f64, has_end: bool| {
            timer_kind(Some(&cfg(
                TimerType::ElapsedTime(TimerTypeElapsedTime {
                    start_time: start,
                    end_time: end,
                    has_end_time: has_end,
                }),
                false,
            )))
            .0
        };
        assert_eq!(
            elapsed(0.0, 60.0, true),
            TimerKind::Elapsed {
                start: None,
                end: Some(60.0)
            }
        );
        assert_eq!(
            elapsed(10.0, 60.0, false),
            TimerKind::Elapsed {
                start: Some(10.0),
                end: None
            }
        );
        assert_eq!(
            timer_kind(None),
            (TimerKind::Countdown { seconds: 0.0 }, false)
        );
        assert_eq!(
            timer_kind(Some(&cfg(
                TimerType::Countdown(TimerTypeCountdown { duration: f64::NAN }),
                false
            )))
            .0,
            TimerKind::Countdown { seconds: 0.0 }
        );
    }

    #[test]
    fn timers_register_and_count_clock() {
        let mut ctx = ctx("Configuration/Timers");
        let doc = pp::TimersDocument {
            clock: Some(pp::Clock::default()),
            timers: vec![
                pp::Timer {
                    uuid: uuid(A),
                    name: "Walk-in".into(),
                    ..Default::default()
                },
                pp::Timer {
                    uuid: uuid(A),
                    name: "Copy".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        import_timers(&mut ctx, &doc, "Configuration/Timers");
        assert_eq!(ctx.out.timers.len(), 2);
        assert_ne!(ctx.out.timers[0].id, ctx.out.timers[1].id);
        assert_eq!(ctx.lookups.timers[&u(A)].name, "Walk-in");
        assert_eq!(ctx.report.unmapped_features["timers.clock"], 1);
    }

    #[test]
    fn clear_groups_keep_pairs_in_order() {
        use pp::action::ContentDestination as D;
        use pp::action::clear_type::ClearTargetLayer as L;
        let mut ctx = ctx("Configuration/ClearGroups");
        let ct = |l: L, d: D| pp::action::ClearType {
            target_layer: l as i32,
            content_destination: d as i32,
        };
        let doc = pp::ClearGroupsDocument {
            groups: vec![pp::clear_groups_document::ClearGroup {
                uuid: uuid(A),
                name: "Clear All".into(),
                layer_targets: vec![
                    ct(L::Audio, D::Global),
                    ct(L::AudioEffects, D::Global),
                    ct(L::All, D::Announcements),
                    ct(L::Background, D::Global),
                    ct(L::Logo, D::Global),
                    ct(L::LiveVideo, D::Global),
                ],
                image_type: pp::clear_groups_document::clear_group::ImageType::All as i32,
                icon_tint_color: Some(pp::Color {
                    red: 1.0,
                    green: 1.0,
                    blue: 1.0,
                    alpha: 1.0,
                }),
                timeline_targets: vec![D::Global as i32, D::Announcements as i32],
                ..Default::default()
            }],
            ..Default::default()
        };
        import_clear_groups(&mut ctx, &doc, "Configuration/ClearGroups");
        let g = &ctx.out.clear_groups[0];
        let t = |layer, destination| ClearTarget { layer, destination };
        assert_eq!(
            g.targets,
            vec![
                t(ClearLayer::Audio, None),
                t(ClearLayer::AudioEffects, None),
                t(ClearLayer::All, Some(ContentDestination::Announcements)),
                t(ClearLayer::Media, None),
                t(ClearLayer::Mask, None),
                t(ClearLayer::VideoInput, None),
                t(ClearLayer::Timelines, None),
                t(
                    ClearLayer::Timelines,
                    Some(ContentDestination::Announcements)
                ),
            ]
        );
        assert_eq!(g.icon.as_deref(), Some("all"));
        assert_eq!(g.tint, None, "tint only when the icon is tinted");
        assert!(ctx.lookups.clear_groups.contains_key(&u(A)));
        assert_eq!(icon_name("ImageTypeXClear"), "xClear");
        assert_eq!(
            clear_target(&pp::action::ClearType {
                target_layer: 99,
                content_destination: 7
            }),
            t(ClearLayer::Unknown, Some(ContentDestination::Unknown))
        );
    }

    #[test]
    fn messages_tokens_and_templates() {
        use pp::message::token::*;
        use pp::message::token_value::*;
        let mut ctx = ctx("Configuration/Messages");
        ctx.lookups.timers.insert(
            u(B),
            NamedId {
                id: TimerId::from_uuid(u(B)),
                name: "Countdown".into(),
            },
        );
        let slide_uuid = "55555555-5555-4555-8555-555555555555";
        let doc = pp::MessageDocument {
            messages: vec![pp::Message {
                uuid: uuid(A),
                title: "Nursery".into(),
                message_text: "Parent of ${child}, ${t}".into(),
                tokens: vec![
                    pp::message::Token {
                        uuid: uuid(C),
                        token_type: Some(TokenType::Text(TokenTypeText {
                            name: "child".into(),
                        })),
                    },
                    pp::message::Token {
                        uuid: uuid("44444444-4444-4444-8444-444444444444"),
                        token_type: Some(TokenType::Timer(TokenTypeTimer {
                            name: "t".into(),
                            timer_uuid: uuid(B),
                        })),
                    },
                    pp::message::Token {
                        uuid: None,
                        token_type: Some(TokenType::Clock(TokenTypeClock {})),
                    },
                ],
                token_values: vec![pp::message::TokenValue {
                    token_id: uuid(C),
                    token_name: "child".into(),
                    token_value_type: Some(TokenValueType::Text(TokenValueText {
                        value: "A12".into(),
                    })),
                }],
                template: Some(pp::TemplateIdentification {
                    name: "Banner".into(),
                    slide_uuid: uuid(slide_uuid),
                    ..Default::default()
                }),
                visible_on_network: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        import_messages(&mut ctx, &doc, "Configuration/Messages");
        let m = &ctx.out.messages[0];
        assert_eq!(m.name, "Nursery");
        assert_eq!(m.text, "Parent of ${child}, ${t}");
        assert_eq!(m.tokens.len(), 3);
        assert_eq!(
            m.tokens[0].kind,
            MessageTokenKind::Text {
                default_value: "A12".into()
            }
        );
        assert_eq!(
            m.tokens[1].kind,
            MessageTokenKind::Timer {
                timer: Some(Link::named(TimerId::from_uuid(u(B)), "Countdown")),
                format: Default::default()
            }
        );
        assert_eq!(m.tokens[2].name, "clock");
        let t = m.template.as_ref().unwrap();
        assert_eq!(t.theme.name.as_deref(), Some("Banner"));
        assert_eq!(ctx.report.unmapped_features["message.visibleOnNetwork"], 1);
        assert!(ctx.lookups.messages.contains_key(&u(A)));

        // The post-pass resolves the template slide to the kept theme (by name + slide).
        let slide = TemplateSlideId::from_uuid(u(slide_uuid));
        let theme = ids::theme_id("Themes/Banner");
        ctx.lookups.themes.insert(
            "Themes/Banner".into(),
            crate::context::ThemeEntry {
                id: theme,
                name: "Banner".into(),
                folder: "Themes/Banner".into(),
                slides: vec![slide],
            },
        );
        crate::convert::workspace::resolve_theme_links(&mut ctx);
        let t = ctx.out.messages[0].template.as_ref().unwrap();
        assert_eq!(t.theme, Link::named(theme, "Banner"));
        assert_eq!(t.slide, slide);
        assert!(ctx.report.warnings.is_empty());
    }

    #[test]
    fn dangling_message_timer_keeps_name() {
        let mut ctx = ctx("Configuration/Messages");
        let link = timer_link(&mut ctx, uuid(A).as_ref(), "Gone");
        assert_eq!(link, Some(Link::named(TimerId::from_uuid(u(A)), "Gone")));
        assert_eq!(ctx.report.warnings.len(), 1);
        assert_eq!(timer_link(&mut ctx, None, "x"), None);
        assert!(
            template_ref(&mut ctx, &pp::TemplateIdentification::default()).is_none(),
            "an empty template identification is no template"
        );
    }

    #[test]
    fn empty_documents_produce_nothing() {
        let mut ctx = ctx("Configuration/Macros");
        import_macros(
            &mut ctx,
            &pp::MacrosDocument {
                macro_collections: vec![Default::default()],
                ..Default::default()
            },
            "Configuration/Macros",
        );
        import_messages(&mut ctx, &Default::default(), "Configuration/Messages");
        import_props(&mut ctx, &Default::default(), "Configuration/Props");
        report_key_mappings(&mut ctx, &Default::default(), "Configuration/KeyMappings");
        report_calendar(
            &mut ctx,
            &pp::Calendar {
                active: true,
                events: vec![],
            },
            "Configuration/Calendar",
        );
        assert!(ctx.out.macros.is_empty());
        assert!(ctx.out.messages.is_empty());
        assert!(ctx.out.props.is_empty());
        assert!(ctx.report.warnings.is_empty());
        assert_eq!(ctx.report.unmapped_features["macros.macroCollections"], 1);
    }

    #[test]
    fn key_mappings_and_calendar_are_reported() {
        let mut ctx = ctx("Configuration/KeyMappings");
        let km = pp::KeyMappingDocument {
            keymappings: vec![Default::default(), Default::default()],
            ..Default::default()
        };
        report_key_mappings(&mut ctx, &km, "Configuration/KeyMappings");
        report_calendar(
            &mut ctx,
            &pp::Calendar {
                active: false,
                events: vec![Default::default()],
            },
            "Configuration/Calendar",
        );
        let w: Vec<_> = ctx
            .report
            .warnings
            .iter()
            .filter(|w| w.category == Category::NotImported)
            .collect();
        assert_eq!(w.len(), 2);
        assert!(w[0].detail.starts_with("2 "));
        assert!(w[1].detail.contains("inactive"));
    }

    #[test]
    fn ccli_settings_without_template() {
        use pp::copyright_layout::{Token, TokenType};
        let mut ctx = ctx("Configuration/CCLI");
        let doc = pp::CcliDocument {
            enable_ccli_display: true,
            ccli_license: " 123456 ".into(),
            display_type: pp::ccli_document::DisplayType::FirstAndLastSlide as i32,
            layout: Some(pp::CopyrightLayout {
                tokens: vec![
                    Token {
                        token_type: TokenType::Title as i32,
                        text: String::new(),
                    },
                    Token {
                        token_type: TokenType::Text as i32,
                        text: " / ".into(),
                    },
                    Token {
                        token_type: TokenType::LicenseNumber as i32,
                        text: String::new(),
                    },
                ],
            }),
            ..Default::default()
        };
        import_ccli(&mut ctx, &doc, "Configuration/CCLI");
        let c = &ctx.out.show_settings.as_ref().unwrap().ccli;
        assert_eq!(c.license_number.as_deref(), Some("123456"));
        assert!(c.enabled);
        assert_eq!(c.show_on, CcliDisplay::FirstAndLast);
        assert_eq!(
            c.layout,
            vec![
                CcliToken::Title,
                CcliToken::Text { text: " / ".into() },
                CcliToken::LicenseNumber
            ]
        );
        assert!(c.template.is_none());
    }

    #[test]
    fn prop_cue_without_prop_slide_is_skipped() {
        let mut ctx = ctx("Configuration/Props");
        let doc = pp::PropDocument {
            cues: vec![pp::Cue {
                uuid: uuid(A),
                name: "Not a prop".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        import_props(&mut ctx, &doc, "Configuration/Props");
        assert!(ctx.out.props.is_empty());
        assert_eq!(ctx.report.warnings.len(), 1);
    }

    // ---- The tests below go through the slides area (`slide::slide_from_pb` /
    // `slide::slide_actions`) and only assert the fields this area sets. ----

    #[test]
    fn prop_from_cue() {
        let mut ctx = ctx("Configuration/Props");
        let cue = pp::Cue {
            uuid: uuid(A),
            name: "Logo".into(),
            is_enabled: true,
            actions: vec![pp::Action {
                uuid: uuid(B),
                is_enabled: true,
                r#type: pp::action::ActionType::PropSlide as i32,
                action_type_data: Some(pp::action::ActionTypeData::Slide(pp::action::SlideType {
                    slide: Some(pp::action::slide_type::Slide::Prop(pp::PropSlide {
                        base_slide: Some(pp::Slide {
                            uuid: uuid(C),
                            size: Some(pp::graphics::Size {
                                width: 1280.0,
                                height: 720.0,
                            }),
                            ..Default::default()
                        }),
                        auto_clear_enabled: true,
                        auto_clear_duration: 5.0,
                        ..Default::default()
                    })),
                })),
                ..Default::default()
            }],
            ..Default::default()
        };
        let doc = pp::PropDocument {
            cues: vec![cue.clone()],
            transition: Some(pp::Transition {
                duration: 0.0,
                effect: Some(pp::Effect {
                    name: "Cut".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            // A collection repeating the document's cue: imported once.
            prop_collections: vec![pp::prop_document::PropCollection {
                cues: vec![cue],
                ..Default::default()
            }],
            ..Default::default()
        };
        import_props(&mut ctx, &doc, "Configuration/Props");
        assert_eq!(ctx.out.props.len(), 1);
        let p = &ctx.out.props[0];
        assert_eq!(p.id.0, u(A));
        assert_eq!(p.name, "Logo");
        assert_eq!(p.size, Size::new(1280.0, 720.0));
        assert_eq!(p.transition, Some(model::Transition::CUT));
        assert_eq!(p.slide.id.0, u(C));
        assert!(p.slide.enabled);
        assert_eq!(
            p.slide.import.as_ref().unwrap().unmapped["autoClear"]["duration"],
            5.0
        );
        assert_eq!(ctx.report.unmapped_features["props.propCollections"], 1);
        assert!(ctx.lookups.props.contains_key(&u(A)));
    }

    #[test]
    fn stage_layout_ids_and_size() {
        let mut ctx = ctx("Configuration/Stage");
        let doc = pp::stage::Document {
            layouts: vec![pp::stage::Layout {
                uuid: uuid(A),
                name: "Confidence".into(),
                slide: Some(pp::Slide {
                    size: Some(pp::graphics::Size {
                        width: 1280.0,
                        height: 720.0,
                    }),
                    ..Default::default()
                }),
            }],
            ..Default::default()
        };
        import_stage(&mut ctx, &doc, "Configuration/Stage");
        let l = &ctx.out.stage_layouts[0];
        assert_eq!(l.id.0, u(A));
        assert_eq!(l.size, Size::new(1280.0, 720.0));
        assert_eq!(
            l.import.as_ref().unwrap().document_path.as_deref(),
            Some("Configuration/Stage")
        );
        assert!(ctx.lookups.stage_layouts.contains_key(&u(A)));
    }

    #[test]
    fn ccli_template_is_inline() {
        let mut ctx = ctx("Configuration/CCLI");
        let doc = pp::CcliDocument {
            template: Some(pp::template::Slide {
                base_slide: Some(pp::Slide {
                    uuid: uuid(A),
                    ..Default::default()
                }),
                name: "Copyright Line".into(),
                actions: vec![],
            }),
            ..Default::default()
        };
        import_ccli(&mut ctx, &doc, "Configuration/CCLI");
        let s = ctx
            .out
            .show_settings
            .as_ref()
            .unwrap()
            .ccli
            .template
            .as_ref()
            .unwrap();
        assert_eq!(s.id.0, u(A));
        assert_eq!(
            s.import.as_ref().unwrap().original_name.as_deref(),
            Some("Copyright Line")
        );
    }
}
