//! Editing timers, messages, props and stage layouts (`/api/v1/timers`, `/messages`, `/props`,
//! `/stage-layouts`).
//!
//! The same rules as screens ([`crate::config`]): every change is **planned** against the
//! current data without side effects, `If-Match` revisions are content hashes, a mismatch is
//! `409` with the current entry, validation errors are `400` with a field, writes are atomic
//! and never replace a document from a newer schema. The Codex writer ([`crate::codex::writer`])
//! persists the plan, has the show actor swap in the new data
//! ([`ShowData::with_show_docs`](crate::data::ShowData::with_show_docs)) and drop what was
//! deleted from the show state (a running timer, a prop or message on screen), and sends
//! `documentChanged`.
//!
//! - **Create** (`POST`) takes a partial document: the ID, a name ("Timer 3"), a sort key
//!   (last) and defaults are filled in. A new timer without a kind counts down 5 minutes. A new
//!   prop takes `fromSlide: {presentation, slide}` to copy that slide's elements (fresh IDs,
//!   no actions) on its presentation's canvas size; otherwise it is blank, on the show's
//!   default canvas size.
//! - **Props in the Edit view:** a prop is edited like a one-slide presentation. Its
//!   **document** ([`prop_document`]) wraps it: a presentation with the prop's ID, name and
//!   canvas size, one section (the prop's ID) holding the prop's slide. Presentation edit ops
//!   apply to that wrapper ([`DocOp::EditProp`]); ops that add, delete, move or split slides
//!   and sections, arrangements and slide actions are refused (`400`).
//! - **Stage layouts** are edited the same way ([`stage_layout_document`],
//!   [`DocOp::EditStageLayout`]). A new layout takes `template` (`currentNext`,
//!   `lyricsClockTimer`, `notes`, `musician`; see [`crate::stage::Template`]) to start from a built-in
//!   template; otherwise it is blank, 1920 × 1080. Deleting a layout that a screen shows makes
//!   the screen fall back to its default (see [`crate::stage`]).

use crate::config::{
    ConfigError, body_object, check_name, check_revision, decode, not_found, put, revision,
};
use crate::data::CollectionContent;
#[cfg(test)]
use crate::data::ShowData;
use crate::store::DocWrite;
use model::{
    Document, DocumentKind, Message, MessageTokenKind, Presentation, PresentationId, Prop, PropId,
    Section, SectionId, Size, Slide, StageLayout, StageLayoutId, Timer, TimerKind,
    sort_key_between,
};
use protocol::{
    ApiError, DocumentChange, EditConflict, ErrorCode, MessageEntry, PresentationEdit,
    PresentationEditResult, PresentationOp, PropEntry, SlideRef, StageLayoutEntry, TimerEntry,
};
use serde_json::Value;

/// The kind of show document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocKind {
    /// Timers.
    Timer,
    /// Messages.
    Message,
    /// Props.
    Prop,
    /// Stage layouts.
    StageLayout,
}

impl DocKind {
    fn label(self) -> &'static str {
        match self {
            DocKind::Timer => "timer",
            DocKind::Message => "message",
            DocKind::Prop => "prop",
            DocKind::StageLayout => "stage layout",
        }
    }

    /// The document kind.
    pub fn kind(self) -> DocumentKind {
        match self {
            DocKind::Timer => DocumentKind::Timer,
            DocKind::Message => DocumentKind::Message,
            DocKind::Prop => DocumentKind::Prop,
            DocKind::StageLayout => DocumentKind::StageLayout,
        }
    }
}

/// A change.
#[derive(Clone, Debug)]
pub enum DocOp {
    /// `POST /<kind>s` (partial JSON).
    Create(DocKind, Value),
    /// `PUT /<kind>s/{id}` (full JSON).
    Update {
        /// Kind.
        kind: DocKind,
        /// Document.
        id: uuid::Uuid,
        /// Body.
        body: Value,
        /// `If-Match`.
        if_match: Option<String>,
    },
    /// `DELETE /<kind>s/{id}`.
    Delete {
        /// Kind.
        kind: DocKind,
        /// Document.
        id: uuid::Uuid,
        /// `If-Match`.
        if_match: Option<String>,
    },
    /// `POST /props/{id}/edits`.
    EditProp(PropId, PresentationEdit),
    /// `POST /stage-layouts/{id}/edits`.
    EditStageLayout(StageLayoutId, PresentationEdit),
}

/// What a change answers.
#[derive(Clone, Debug)]
pub enum DocResult {
    /// A created / updated timer.
    Timer(TimerEntry),
    /// A created / updated message.
    Message(MessageEntry),
    /// A created / updated prop.
    Prop(Box<PropEntry>),
    /// A prop or stage layout edit.
    PropEdit(PresentationEditResult),
    /// A created / updated stage layout.
    StageLayout(Box<StageLayoutEntry>),
    /// Deleted.
    Deleted,
}

/// A validated change.
#[derive(Debug)]
pub struct DocPlan {
    /// New timer list (when it changed).
    pub timers: Option<Vec<Timer>>,
    /// New prop list.
    pub props: Option<Vec<Prop>>,
    /// New message list.
    pub messages: Option<Vec<Message>>,
    /// New stage layout list.
    pub stage_layouts: Option<Vec<StageLayout>>,
    /// Writes.
    pub writes: Vec<DocWrite>,
    /// `documentChanged` events: kind, ID, change, new revision.
    pub changes: Vec<(DocumentKind, uuid::Uuid, DocumentChange, Option<String>)>,
    /// `ops` for prop edits.
    pub ops: Option<Vec<PresentationOp>>,
    /// The answer.
    pub result: DocResult,
}

fn entry_value<T: serde::Serialize>(doc: &T) -> Value {
    let mut v = serde_json::to_value(doc).unwrap_or_default();
    if let Value::Object(m) = &mut v {
        m.insert("revision".into(), Value::String(revision(doc)));
    }
    v
}

/// A timer with its revision.
pub fn timer_entry(t: &Timer) -> TimerEntry {
    TimerEntry {
        revision: revision(t),
        timer: t.clone(),
    }
}

/// A message with its revision.
pub fn message_entry(m: &Message) -> MessageEntry {
    MessageEntry {
        revision: revision(m),
        message: m.clone(),
    }
}

/// A prop with its revision.
pub fn prop_entry(p: &Prop) -> PropEntry {
    PropEntry {
        revision: revision(p),
        prop: p.clone(),
    }
}

/// A stage layout with its revision.
pub fn stage_layout_entry(l: &StageLayout) -> StageLayoutEntry {
    StageLayoutEntry {
        revision: revision(l),
        layout: l.clone(),
    }
}

fn next_name<T>(list: &[T], name: impl Fn(&T) -> &str, base: &str) -> String {
    (1..)
        .map(|n| format!("{base} {n}"))
        .find(|n| !list.iter().any(|x| name(x).eq_ignore_ascii_case(n)))
        .expect("a free name")
}

fn bad_field(field: &str, msg: impl Into<String>) -> ConfigError {
    ApiError::invalid(field, msg).into()
}

/// Longest timer duration / start value, seconds (99 hours).
pub const MAX_TIMER_SECONDS: f64 = 99.0 * 3600.0;

fn validate_timer(t: &Timer) -> Result<(), ConfigError> {
    check_name(&t.name, "timer")?;
    let ok = |v: f64| v.is_finite() && (0.0..=MAX_TIMER_SECONDS).contains(&v);
    match t.kind {
        TimerKind::Countdown { seconds } if !ok(seconds) => Err(bad_field(
            "kind.seconds",
            "a countdown runs 0 seconds to 99 hours",
        )),
        TimerKind::CountdownToTime {
            seconds_after_midnight,
            ..
        } if !(seconds_after_midnight.is_finite()
            && (0.0..crate::timers::DAY).contains(&seconds_after_midnight)) =>
        {
            Err(bad_field(
                "kind.secondsAfterMidnight",
                "the time of day must be 0 – 86399 seconds after midnight",
            ))
        }
        TimerKind::Elapsed { start, end } => {
            if start.is_some_and(|s| !ok(s)) {
                return Err(bad_field("kind.start", "start must be 0 to 99 hours"));
            }
            if let Some(e) = end
                && (!ok(e) || e < start.unwrap_or(0.0))
            {
                return Err(bad_field(
                    "kind.end",
                    "the end must be after the start (at most 99 hours)",
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_message(m: &Message) -> Result<(), ConfigError> {
    check_name(&m.name, "message")?;
    for (i, t) in m.tokens.iter().enumerate() {
        // Names may have inner spaces (`Child Number`): the importer keeps them as written and
        // the live replacement matches `${name}` literally. Oriel declares them the same way.
        let n = t.name.as_str();
        if n.is_empty() || n.trim() != n || n.contains(['{', '}', '$']) {
            return Err(bad_field(
                "tokens",
                format!(
                    "token names can't be empty, start or end with a space, or use {{, }} or $ ({:?})",
                    t.name
                ),
            ));
        }
        if m.tokens[..i].iter().any(|o| o.name == t.name) {
            return Err(bad_field(
                "tokens",
                format!("token {:?} is listed twice", t.name),
            ));
        }
        if matches!(t.kind, MessageTokenKind::Unknown) {
            return Err(bad_field("tokens", "unknown token kind"));
        }
    }
    Ok(())
}

fn validate_prop(p: &Prop) -> Result<(), ConfigError> {
    check_name(&p.name, "prop")?;
    crate::config::check_size(p.size.width, p.size.height, "size")?;
    Ok(())
}

fn validate_stage_layout(l: &StageLayout) -> Result<(), ConfigError> {
    check_name(&l.name, "stage layout")?;
    crate::config::check_size(l.size.width, l.size.height, "size")?;
    Ok(())
}

/// The ID of a create body (given or new); `409` when it exists.
fn new_id(
    m: &serde_json::Map<String, Value>,
    exists: impl Fn(uuid::Uuid) -> bool,
    what: &str,
) -> Result<uuid::Uuid, ConfigError> {
    let id = match m.get("id").and_then(Value::as_str) {
        Some(s) => s
            .parse::<uuid::Uuid>()
            .map_err(|_| ApiError::invalid("id", "id must be a UUID"))?,
        None => uuid::Uuid::new_v4(),
    };
    if exists(id) {
        return Err(
            ApiError::new(ErrorCode::Conflict, format!("{what} {id} already exists")).into(),
        );
    }
    Ok(id)
}

fn last_key<'a>(keys: impl Iterator<Item = &'a str>) -> String {
    sort_key_between(keys.max(), None)
}

/// A one-slide presentation wrapping a prop or stage layout (see the module docs).
fn one_slide_document(id: uuid::Uuid, name: &str, size: Size, slide: &Slide) -> Presentation {
    let mut doc = Presentation::new_blank(name);
    doc.id = PresentationId::from(id);
    doc.canvas_size = size;
    doc.sections = vec![Section {
        id: SectionId::from(id),
        name: name.to_string(),
        slides: vec![slide.clone()],
        ..Section::new(name)
    }];
    doc.modified_at = None;
    doc.created_at = None;
    doc
}

/// The presentation wrapper a prop is edited through (see the module docs).
pub fn prop_document(p: &Prop) -> Presentation {
    one_slide_document(p.id.0, &p.name, p.size, &p.slide)
}

/// The presentation wrapper a stage layout is edited through (as a prop).
pub fn stage_layout_document(l: &StageLayout) -> Presentation {
    one_slide_document(l.id.0, &l.name, l.size, &l.slide)
}

/// Applies presentation edit ops to a one-slide document; returns its new name, size and
/// slide, and the edit result.
fn edit_one_slide(
    what: &str,
    rev: String,
    mut doc: Presentation,
    edit: &PresentationEdit,
) -> Result<
    (
        String,
        Size,
        Option<Slide>,
        crate::edit::Applied<PresentationOp>,
    ),
    ConfigError,
> {
    if let Some((k, op)) = edit
        .ops
        .iter()
        .enumerate()
        .find(|(_, o)| !prop_op_allowed(o))
    {
        return Err(ApiError::new(
            ErrorCode::BadRequest,
            format!(
                "edit refused (op {k}, {}): a {what} has exactly one slide and no actions",
                op.name()
            ),
        )
        .into());
    }
    let applied =
        crate::edit::apply_presentation(&mut doc, &edit.ops).map_err(|f| ConfigError {
            error: ApiError::new(f.code, f.message),
            current: serde_json::to_value(EditConflict {
                revision: Some(rev),
                op_index: f.op_index,
                slide: f.slide,
                element: f.element,
                text: f.text,
            })
            .ok(),
        })?;
    let slide = doc.sections.first().and_then(|s| s.slides.first()).cloned();
    Ok((doc.name, doc.canvas_size, slide, applied))
}

fn prop_op_allowed(op: &PresentationOp) -> bool {
    !matches!(
        op,
        PresentationOp::AddSection { .. }
            | PresentationOp::DeleteSection { .. }
            | PresentationOp::MoveSection { .. }
            | PresentationOp::AddSlide { .. }
            | PresentationOp::DeleteSlide { .. }
            | PresentationOp::MoveSlide { .. }
            | PresentationOp::DuplicateSlide { .. }
            | PresentationOp::SplitSlide { .. }
            | PresentationOp::MergeSlides { .. }
            | PresentationOp::SetArrangements { .. }
            | PresentationOp::AddSlideAction { .. }
            | PresentationOp::RemoveSlideAction { .. }
            | PresentationOp::MoveSlideAction { .. }
            | PresentationOp::SetSlideAction { .. }
    )
}

/// Plans a change of the collection `data` (no side effects). A new blank prop gets
/// `default_canvas_size` (the show settings').
pub fn plan(
    data: &CollectionContent,
    default_canvas_size: model::Size,
    op: DocOp,
) -> Result<DocPlan, ConfigError> {
    let mut timers = (*data.timers).clone();
    let mut props = (*data.props).clone();
    let mut messages = (*data.messages).clone();
    let mut layouts = (*data.stage_layouts).clone();
    let mut writes = Vec::new();
    let mut changes = Vec::new();
    let mut ops = None;
    let (mut t_changed, mut p_changed, mut m_changed) = (false, false, false);
    let mut l_changed = false;
    let result = match op {
        DocOp::Create(kind, body) => {
            let mut m = body_object(body)?;
            m.remove("import");
            match kind {
                DocKind::Timer => {
                    let id = new_id(&m, |id| timers.iter().any(|t| t.id.0 == id), "timer")?;
                    if !m.contains_key("name") {
                        m.insert(
                            "name".into(),
                            next_name(&timers, |t| &t.name, "Timer").into(),
                        );
                    }
                    if !m.contains_key("sortKey") {
                        m.insert(
                            "sortKey".into(),
                            last_key(timers.iter().map(|t| t.sort_key.as_str())).into(),
                        );
                    }
                    if !m.contains_key("kind") {
                        m.insert(
                            "kind".into(),
                            serde_json::json!({"type": "countdown", "seconds": 300.0}),
                        );
                    }
                    let mut t: Timer = decode(m, id, "timer")?;
                    t.name = t.name.trim().to_string();
                    validate_timer(&t)?;
                    writes.push(put(&t)?);
                    changes.push((
                        DocumentKind::Timer,
                        id,
                        DocumentChange::Added,
                        Some(revision(&t)),
                    ));
                    timers.push(t.clone());
                    t_changed = true;
                    DocResult::Timer(timer_entry(&t))
                }
                DocKind::Message => {
                    let id = new_id(&m, |id| messages.iter().any(|x| x.id.0 == id), "message")?;
                    if !m.contains_key("name") {
                        m.insert(
                            "name".into(),
                            next_name(&messages, |x| &x.name, "Message").into(),
                        );
                    }
                    if !m.contains_key("sortKey") {
                        m.insert(
                            "sortKey".into(),
                            last_key(messages.iter().map(|x| x.sort_key.as_str())).into(),
                        );
                    }
                    let mut msg: Message = decode(m, id, "message")?;
                    msg.name = msg.name.trim().to_string();
                    validate_message(&msg)?;
                    writes.push(put(&msg)?);
                    changes.push((
                        DocumentKind::Message,
                        id,
                        DocumentChange::Added,
                        Some(revision(&msg)),
                    ));
                    messages.push(msg.clone());
                    m_changed = true;
                    DocResult::Message(message_entry(&msg))
                }
                DocKind::Prop => {
                    let id = new_id(&m, |id| props.iter().any(|x| x.id.0 == id), "prop")?;
                    let from: Option<SlideRef> = match m.remove("fromSlide") {
                        None | Some(Value::Null) => None,
                        Some(v) => Some(
                            serde_json::from_value(v)
                                .map_err(|e| bad_field("fromSlide", format!("fromSlide: {e}")))?,
                        ),
                    };
                    if !m.contains_key("name") {
                        m.insert("name".into(), next_name(&props, |x| &x.name, "Prop").into());
                    }
                    if !m.contains_key("sortKey") {
                        m.insert(
                            "sortKey".into(),
                            last_key(props.iter().map(|x| x.sort_key.as_str())).into(),
                        );
                    }
                    let mut size = default_canvas_size;
                    let slide = match from {
                        Some(r) => {
                            let p = data.presentation(r.presentation).ok_or_else(|| {
                                ConfigError::from(ApiError::new(
                                    ErrorCode::NotFound,
                                    format!("presentation {} not found", r.presentation.0),
                                ))
                            })?;
                            let s = p.slides().find(|s| s.id == r.slide).ok_or_else(|| {
                                ConfigError::from(ApiError::new(
                                    ErrorCode::NotFound,
                                    format!("slide {} not in presentation", r.slide.0),
                                ))
                            })?;
                            size = p.canvas_size;
                            let mut copy = s.duplicate();
                            copy.actions.clear();
                            copy.label = None;
                            copy.hotkey = None;
                            copy.notes = Default::default();
                            copy.transition = None;
                            copy.import = None;
                            copy
                        }
                        None => Slide::new(),
                    };
                    if !m.contains_key("slide") {
                        m.insert(
                            "slide".into(),
                            serde_json::to_value(&slide).unwrap_or_default(),
                        );
                    }
                    if !m.contains_key("size") {
                        m.insert(
                            "size".into(),
                            serde_json::to_value(size).unwrap_or_default(),
                        );
                    }
                    let mut p: Prop = decode(m, id, "prop")?;
                    p.name = p.name.trim().to_string();
                    validate_prop(&p)?;
                    writes.push(put(&p)?);
                    changes.push((
                        DocumentKind::Prop,
                        id,
                        DocumentChange::Added,
                        Some(revision(&p)),
                    ));
                    props.push(p.clone());
                    p_changed = true;
                    DocResult::Prop(Box::new(prop_entry(&p)))
                }
                DocKind::StageLayout => {
                    let id = new_id(
                        &m,
                        |id| layouts.iter().any(|x| x.id.0 == id),
                        "stage layout",
                    )?;
                    let template = match m.remove("template") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(t)) => Some(
                            crate::stage::Template::from_id(&t).ok_or_else(|| {
                                bad_field(
                                    "template",
                                    format!(
                                        "unknown template {t:?} (currentNext, lyricsClockTimer, notes, musician)"
                                    ),
                                )
                            })?,
                        ),
                        Some(_) => return Err(bad_field("template", "template must be a string")),
                    };
                    let base = template.map(|t| t.layout()).unwrap_or_else(|| StageLayout {
                        size: crate::stage::TEMPLATE_CANVAS,
                        ..StageLayout::default()
                    });
                    if !m.contains_key("name") {
                        let name = match template {
                            Some(t)
                                if !layouts
                                    .iter()
                                    .any(|x| x.name.eq_ignore_ascii_case(t.name())) =>
                            {
                                t.name().to_string()
                            }
                            _ => next_name(&layouts, |x| &x.name, "Layout"),
                        };
                        m.insert("name".into(), name.into());
                    }
                    if !m.contains_key("sortKey") {
                        m.insert(
                            "sortKey".into(),
                            last_key(layouts.iter().map(|x| x.sort_key.as_str())).into(),
                        );
                    }
                    if !m.contains_key("slide") {
                        m.insert(
                            "slide".into(),
                            serde_json::to_value(&base.slide).unwrap_or_default(),
                        );
                    }
                    if !m.contains_key("size") {
                        m.insert(
                            "size".into(),
                            serde_json::to_value(base.size).unwrap_or_default(),
                        );
                    }
                    let mut l: StageLayout = decode(m, id, "stage layout")?;
                    l.name = l.name.trim().to_string();
                    validate_stage_layout(&l)?;
                    writes.push(put(&l)?);
                    changes.push((
                        DocumentKind::StageLayout,
                        id,
                        DocumentChange::Added,
                        Some(revision(&l)),
                    ));
                    layouts.push(l.clone());
                    l_changed = true;
                    DocResult::StageLayout(Box::new(stage_layout_entry(&l)))
                }
            }
        }
        DocOp::Update {
            kind,
            id,
            body,
            if_match,
        } => {
            let what = format!("this {}", kind.label());
            match kind {
                DocKind::Timer => {
                    let i = timers.iter().position(|t| t.id.0 == id);
                    check_revision(i.map(|i| &timers[i]), entry_value, &if_match, &what)?;
                    let i = i.ok_or_else(|| not_found("timer", id))?;
                    let mut t: Timer = decode(body_object(body)?, id, "timer")?;
                    t.name = t.name.trim().to_string();
                    validate_timer(&t)?;
                    if t != timers[i] {
                        writes.push(put(&t)?);
                        changes.push((
                            DocumentKind::Timer,
                            id,
                            DocumentChange::Changed,
                            Some(revision(&t)),
                        ));
                        timers[i] = t.clone();
                        t_changed = true;
                    }
                    DocResult::Timer(timer_entry(&t))
                }
                DocKind::Message => {
                    let i = messages.iter().position(|t| t.id.0 == id);
                    check_revision(i.map(|i| &messages[i]), entry_value, &if_match, &what)?;
                    let i = i.ok_or_else(|| not_found("message", id))?;
                    let mut msg: Message = decode(body_object(body)?, id, "message")?;
                    msg.name = msg.name.trim().to_string();
                    validate_message(&msg)?;
                    if msg != messages[i] {
                        writes.push(put(&msg)?);
                        changes.push((
                            DocumentKind::Message,
                            id,
                            DocumentChange::Changed,
                            Some(revision(&msg)),
                        ));
                        messages[i] = msg.clone();
                        m_changed = true;
                    }
                    DocResult::Message(message_entry(&msg))
                }
                DocKind::Prop => {
                    let i = props.iter().position(|t| t.id.0 == id);
                    check_revision(i.map(|i| &props[i]), entry_value, &if_match, &what)?;
                    let i = i.ok_or_else(|| not_found("prop", id))?;
                    let mut m = body_object(body)?;
                    m.remove("fromSlide");
                    let mut p: Prop = decode(m, id, "prop")?;
                    p.name = p.name.trim().to_string();
                    validate_prop(&p)?;
                    if p != props[i] {
                        writes.push(put(&p)?);
                        changes.push((
                            DocumentKind::Prop,
                            id,
                            DocumentChange::Changed,
                            Some(revision(&p)),
                        ));
                        props[i] = p.clone();
                        p_changed = true;
                    }
                    DocResult::Prop(Box::new(prop_entry(&p)))
                }
                DocKind::StageLayout => {
                    let i = layouts.iter().position(|t| t.id.0 == id);
                    check_revision(i.map(|i| &layouts[i]), entry_value, &if_match, &what)?;
                    let i = i.ok_or_else(|| not_found("stage layout", id))?;
                    let mut m = body_object(body)?;
                    m.remove("template");
                    let mut l: StageLayout = decode(m, id, "stage layout")?;
                    l.name = l.name.trim().to_string();
                    validate_stage_layout(&l)?;
                    if l != layouts[i] {
                        writes.push(put(&l)?);
                        changes.push((
                            DocumentKind::StageLayout,
                            id,
                            DocumentChange::Changed,
                            Some(revision(&l)),
                        ));
                        layouts[i] = l.clone();
                        l_changed = true;
                    }
                    DocResult::StageLayout(Box::new(stage_layout_entry(&l)))
                }
            }
        }
        DocOp::Delete { kind, id, if_match } => {
            let what = format!("this {}", kind.label());
            match kind {
                DocKind::Timer => {
                    let cur = timers.iter().find(|t| t.id.0 == id);
                    check_revision(cur, entry_value, &if_match, &what)?;
                    cur.ok_or_else(|| not_found("timer", id))?;
                    writes.push(DocWrite::delete::<Timer>(id));
                    timers.retain(|t| t.id.0 != id);
                    t_changed = true;
                }
                DocKind::Message => {
                    let cur = messages.iter().find(|t| t.id.0 == id);
                    check_revision(cur, entry_value, &if_match, &what)?;
                    cur.ok_or_else(|| not_found("message", id))?;
                    writes.push(DocWrite::delete::<Message>(id));
                    messages.retain(|t| t.id.0 != id);
                    m_changed = true;
                }
                DocKind::Prop => {
                    let cur = props.iter().find(|t| t.id.0 == id);
                    check_revision(cur, entry_value, &if_match, &what)?;
                    cur.ok_or_else(|| not_found("prop", id))?;
                    writes.push(DocWrite::delete::<Prop>(id));
                    props.retain(|t| t.id.0 != id);
                    p_changed = true;
                }
                DocKind::StageLayout => {
                    let cur = layouts.iter().find(|t| t.id.0 == id);
                    check_revision(cur, entry_value, &if_match, &what)?;
                    cur.ok_or_else(|| not_found("stage layout", id))?;
                    writes.push(DocWrite::delete::<StageLayout>(id));
                    layouts.retain(|t| t.id.0 != id);
                    l_changed = true;
                }
            }
            changes.push((kind.kind(), id, DocumentChange::Removed, None));
            DocResult::Deleted
        }
        DocOp::EditProp(id, edit) => {
            let i = props
                .iter()
                .position(|p| p.id == id)
                .ok_or_else(|| not_found("prop", id.0))?;
            let cur = props[i].clone();
            let (name, size, slide, applied) =
                edit_one_slide("prop", revision(&cur), prop_document(&cur), &edit)?;
            let mut p = cur.clone();
            p.name = name;
            p.size = size;
            if let Some(s) = slide {
                p.slide = s;
            }
            validate_prop(&p)?;
            if p != cur {
                writes.push(put(&p)?);
                changes.push((
                    DocumentKind::Prop,
                    id.0,
                    DocumentChange::Changed,
                    Some(revision(&p)),
                ));
                props[i] = p.clone();
                p_changed = true;
                ops = Some(applied.applied.clone());
            }
            DocResult::PropEdit(PresentationEditResult {
                revision: revision(&p),
                applied: applied.applied,
                inverse: applied.inverse,
            })
        }
        DocOp::EditStageLayout(id, edit) => {
            let i = layouts
                .iter()
                .position(|p| p.id == id)
                .ok_or_else(|| not_found("stage layout", id.0))?;
            let cur = layouts[i].clone();
            let (name, size, slide, applied) = edit_one_slide(
                "stage layout",
                revision(&cur),
                stage_layout_document(&cur),
                &edit,
            )?;
            let mut l = cur.clone();
            l.name = name;
            l.size = size;
            if let Some(s) = slide {
                l.slide = s;
            }
            validate_stage_layout(&l)?;
            if l != cur {
                writes.push(put(&l)?);
                changes.push((
                    DocumentKind::StageLayout,
                    id.0,
                    DocumentChange::Changed,
                    Some(revision(&l)),
                ));
                layouts[i] = l.clone();
                l_changed = true;
                ops = Some(applied.applied.clone());
            }
            DocResult::PropEdit(PresentationEditResult {
                revision: revision(&l),
                applied: applied.applied,
                inverse: applied.inverse,
            })
        }
    };
    Ok(DocPlan {
        timers: t_changed.then_some(timers),
        props: p_changed.then_some(props),
        messages: m_changed.then_some(messages),
        stage_layouts: l_changed.then_some(layouts),
        writes,
        changes,
        ops,
        result,
    })
}

/// The kind's document as stored, by ID (for `GET /<kind>s/{id}`).
pub fn find<T: Document + Clone>(list: &[T], id: uuid::Uuid) -> Option<T> {
    list.iter().find(|d| d.id() == id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// [`super::plan`] against a show's active collection and settings.
    fn plan(d: &ShowData, op: DocOp) -> Result<DocPlan, ConfigError> {
        super::plan(&d.content, d.settings.default_canvas_size, op)
    }

    fn data() -> ShowData {
        let dir = std::env::temp_dir().join(format!("showdocs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("portable")).unwrap();
        let d = ShowData::load(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        d
    }

    #[test]
    fn create_update_delete_with_if_match() {
        let d = data();
        let p = plan(&d, DocOp::Create(DocKind::Timer, json!({}))).unwrap();
        let DocResult::Timer(e) = p.result else {
            panic!()
        };
        assert_eq!(e.timer.name, "Timer 1");
        assert_eq!(e.timer.kind, TimerKind::Countdown { seconds: 300.0 });
        let d = d.with_show_docs(p.timers, None, None, None);
        // Update with the right revision.
        let mut body = serde_json::to_value(&e.timer).unwrap();
        body["name"] = json!("Walk-in");
        let up = plan(
            &d,
            DocOp::Update {
                kind: DocKind::Timer,
                id: e.timer.id.0,
                body: body.clone(),
                if_match: Some(e.revision.clone()),
            },
        )
        .unwrap();
        let DocResult::Timer(e2) = up.result else {
            panic!()
        };
        assert_eq!(e2.timer.name, "Walk-in");
        let d2 = d.with_show_docs(up.timers, None, None, None);
        // A stale revision conflicts with the current entry.
        let err = plan(
            &d2,
            DocOp::Update {
                kind: DocKind::Timer,
                id: e.timer.id.0,
                body,
                if_match: Some(e.revision.clone()),
            },
        )
        .unwrap_err();
        assert_eq!(err.error.code, ErrorCode::Conflict);
        assert_eq!(err.current.as_ref().unwrap()["name"], json!("Walk-in"));
        assert_eq!(err.current.unwrap()["revision"], json!(e2.revision));
        // Validation.
        let err = plan(
            &d2,
            DocOp::Create(
                DocKind::Timer,
                json!({"kind": {"type": "countdown", "seconds": -1}}),
            ),
        )
        .unwrap_err();
        assert_eq!(err.error.field.as_deref(), Some("kind.seconds"));
        // Delete with a stale revision conflicts; with the current one it goes.
        let del = |rev: &str| {
            plan(
                &d2,
                DocOp::Delete {
                    kind: DocKind::Timer,
                    id: e.timer.id.0,
                    if_match: Some(rev.into()),
                },
            )
        };
        assert_eq!(
            del(&e.revision).unwrap_err().error.code,
            ErrorCode::Conflict
        );
        let ok = del(&e2.revision).unwrap();
        assert_eq!(ok.timers.unwrap().len(), 0);
        assert_eq!(ok.changes[0].2, DocumentChange::Removed);
    }

    #[test]
    fn messages_validate_tokens_and_props_copy_slides() {
        let d = data();
        let err = plan(
            &d,
            DocOp::Create(
                DocKind::Message,
                json!({"text": "${a}", "tokens": [{"name": "a"}, {"name": "a"}]}),
            ),
        )
        .unwrap_err();
        assert_eq!(err.error.field.as_deref(), Some("tokens"));
        let ok = plan(
            &d,
            DocOp::Create(
                DocKind::Message,
                json!({"text": "Parent of ${child}", "tokens": [{"name": "child"}]}),
            ),
        )
        .unwrap();
        let DocResult::Message(m) = ok.result else {
            panic!()
        };
        assert_eq!(m.message.name, "Message 1");
        // A prop from a missing slide is 404; a blank one gets the default canvas.
        let err = plan(
            &d,
            DocOp::Create(
                DocKind::Prop,
                json!({"fromSlide": {"presentation": uuid::Uuid::new_v4(), "slide": uuid::Uuid::new_v4()}}),
            ),
        )
        .unwrap_err();
        assert_eq!(err.error.code, ErrorCode::NotFound);
        let ok = plan(&d, DocOp::Create(DocKind::Prop, json!({"name": "Logo"}))).unwrap();
        let DocResult::Prop(p) = ok.result else {
            panic!()
        };
        assert_eq!(p.prop.size, model::Size::HD);
        // Prop edits go through the one-slide wrapper; slide ops are refused.
        let d = d.with_show_docs(None, ok.props, None, None);
        let slide = p.prop.slide.id;
        let el = model::Element::text(
            model::Rect::new(0.0, 0.0, 100.0, 50.0),
            model::TextBox::plain("Hi"),
        );
        let ok = plan(
            &d,
            DocOp::EditProp(
                p.prop.id,
                PresentationEdit {
                    ops: vec![
                        PresentationOp::AddElement {
                            slide,
                            element: el,
                            index: None,
                        },
                        PresentationOp::SetPresentation {
                            name: Some("Logo 2".into()),
                            song_info: None,
                            scripture: None,
                            transition: None,
                            canvas_size: None,
                            notes: None,
                            category: None,
                            language: None,
                        },
                    ],
                    client_id: None,
                    base_revision: None,
                },
            ),
        )
        .unwrap();
        let new = &ok.props.as_ref().unwrap()[0];
        assert_eq!(new.name, "Logo 2");
        assert_eq!(new.slide.elements.len(), 1);
        let DocResult::PropEdit(r) = ok.result else {
            panic!()
        };
        assert_eq!(r.inverse.len(), 2);
        let err = plan(
            &d,
            DocOp::EditProp(
                p.prop.id,
                PresentationEdit {
                    ops: vec![PresentationOp::DeleteSlide { slide }],
                    client_id: None,
                    base_revision: None,
                },
            ),
        )
        .unwrap_err();
        assert_eq!(err.error.code, ErrorCode::BadRequest);
    }
}
