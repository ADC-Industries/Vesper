//! Logical video inputs ("Camera 1", `portable/videoInput`) and this machine's bindings of
//! them (`machine/inputBinding`): create, rename, bind, delete (`/video-inputs`, see
//! `docs/LIVE-SOURCES-PLAN.md` stage 4).
//!
//! Like the screen configuration ([`crate::config`]) every change is planned here against the
//! current [`ShowData`] without side effects (decode, defaults, `If-Match`, validation); the
//! show actor persists the writes atomically, swaps the new documents in and lets a live input
//! follow its binding. The revision covers the input and its binding together.

use crate::config::{body_object, decode, not_found, put};
use crate::data::ShowData;
use crate::store::DocWrite;
use model::{DocumentKind, InputBinding, InputSource, VideoInput, VideoInputId, sort_key_between};
use protocol::{ApiError, DocumentChange, ErrorCode, VideoInputEntry};
use serde_json::Value;
pub use vesper_core::change::{ConfigError, check_revision, revision};

/// Longest input name, characters.
pub const MAX_NAME: usize = 64;
/// Longest bound source name or address, characters.
pub const MAX_SOURCE: usize = 512;

/// A change of the inputs.
#[derive(Clone, Debug)]
pub enum InputOp {
    /// `POST /video-inputs` (partial JSON, optional `source`).
    Create(Value),
    /// `PUT /video-inputs/{id}` (the whole input, `source` absent / `null` = unbound).
    Update {
        /// The input.
        id: VideoInputId,
        /// Body.
        body: Value,
        /// `If-Match` revision.
        if_match: Option<String>,
    },
    /// `DELETE /video-inputs/{id}` (the input and its binding).
    Delete {
        /// The input.
        id: VideoInputId,
        /// `If-Match` revision.
        if_match: Option<String>,
    },
}

/// What a successful change returns.
#[derive(Clone, Debug)]
pub enum InputResult {
    /// The created / updated input.
    Input(Box<VideoInputEntry>),
    /// The input was deleted.
    Deleted,
}

/// A validated change, ready to persist and apply.
#[derive(Debug)]
pub struct InputPlan {
    /// Inputs after the change.
    pub video_inputs: Vec<VideoInput>,
    /// Bindings after the change.
    pub input_bindings: Vec<InputBinding>,
    /// Files to write / delete, in order.
    pub writes: Vec<DocWrite>,
    /// `documentChanged` events to send once applied.
    pub changes: Vec<(DocumentKind, uuid::Uuid, DocumentChange)>,
    /// The response.
    pub result: InputResult,
}

/// This box's binding of `input` (`None`: unbound).
pub fn binding_of(data: &ShowData, input: VideoInputId) -> Option<&InputSource> {
    data.input_bindings
        .iter()
        .find(|b| b.input == input)
        .map(|b| &b.source)
        .filter(|s| **s != InputSource::Unknown)
}

/// An input with its binding and revision.
pub fn entry(input: &VideoInput, source: Option<&InputSource>) -> VideoInputEntry {
    VideoInputEntry {
        revision: revision(&(input, source)),
        input: input.clone(),
        source: source.cloned(),
    }
}

/// Every input of this box, in order, with its binding.
pub fn entries(data: &ShowData) -> Vec<VideoInputEntry> {
    data.video_inputs
        .iter()
        .map(|v| entry(v, binding_of(data, v.id)))
        .collect()
}

/// A free "Camera N" name.
fn next_name(data: &ShowData) -> String {
    (1..)
        .map(|n| format!("Camera {n}"))
        .find(|n| {
            !data
                .video_inputs
                .iter()
                .any(|v| v.name.trim().eq_ignore_ascii_case(n))
        })
        .expect("a free name")
}

/// Takes `source` out of a body: `None` when absent or `null` (unbound), checked.
fn take_source(m: &mut serde_json::Map<String, Value>) -> Result<Option<InputSource>, ApiError> {
    let Some(v) = m.remove("source").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let s: InputSource = serde_json::from_value(v)
        .map_err(|e| ApiError::invalid("source", format!("not a source: {e}")))?;
    let name = match &s {
        InputSource::Idn { source_name } => source_name,
        InputSource::Omt { name } | InputSource::Device { name } => name,
        InputSource::Unknown => {
            return Err(ApiError::invalid(
                "source",
                "unknown source type (use `idn` or `omt`, or null to unbind)",
            ));
        }
    };
    let t = name.trim();
    if t.is_empty() || t.chars().count() > MAX_SOURCE {
        return Err(ApiError::invalid(
            "source",
            format!("give a source name or address (1–{MAX_SOURCE} characters)"),
        ));
    }
    Ok(Some(match s {
        InputSource::Idn { .. } => InputSource::Idn {
            source_name: t.to_string(),
        },
        InputSource::Omt { .. } => InputSource::Omt { name: t.into() },
        InputSource::Device { .. } => InputSource::Device { name: t.into() },
        InputSource::Unknown => unreachable!(),
    }))
}

fn check_name(v: &mut VideoInput) -> Result<(), ApiError> {
    v.name = v.name.trim().to_string();
    if v.name.is_empty() || v.name.chars().count() > MAX_NAME {
        return Err(ApiError::invalid(
            "name",
            format!("give the input a name (1–{MAX_NAME} characters)"),
        ));
    }
    Ok(())
}

/// The binding writes for `input` going from `old` to `new`.
fn bind(
    input: VideoInputId,
    old: Option<&InputSource>,
    new: Option<InputSource>,
    bindings: &mut Vec<InputBinding>,
    writes: &mut Vec<DocWrite>,
    changes: &mut Vec<(DocumentKind, uuid::Uuid, DocumentChange)>,
) -> Result<(), ConfigError> {
    if old == new.as_ref() {
        return Ok(());
    }
    let stored = bindings.iter().any(|b| b.input == input);
    bindings.retain(|b| b.input != input);
    match new {
        Some(source) => {
            let b = InputBinding {
                input,
                source,
                ..InputBinding::default()
            };
            writes.push(put(&b)?);
            let change = if stored {
                DocumentChange::Changed
            } else {
                DocumentChange::Added
            };
            changes.push((DocumentKind::InputBinding, input.0, change));
            bindings.push(b);
        }
        None if stored => {
            writes.push(DocWrite::delete::<InputBinding>(input.0));
            changes.push((DocumentKind::InputBinding, input.0, DocumentChange::Removed));
        }
        None => {}
    }
    Ok(())
}

/// Plans a change (no side effects).
pub fn plan(data: &ShowData, op: InputOp) -> Result<InputPlan, ConfigError> {
    let mut video_inputs = data.video_inputs.clone();
    let mut input_bindings = data.input_bindings.clone();
    let mut writes = Vec::new();
    let mut changes = Vec::new();
    let result = match op {
        InputOp::Create(body) => {
            let mut m = body_object(body)?;
            let id = match m.get("id").and_then(Value::as_str) {
                Some(s) => s
                    .parse::<uuid::Uuid>()
                    .map_err(|_| ApiError::invalid("id", "id must be a UUID"))?,
                None => uuid::Uuid::new_v4(),
            };
            if data.video_inputs.iter().any(|v| v.id.0 == id) {
                return Err(ApiError::new(
                    ErrorCode::Conflict,
                    format!("video input {id} already exists"),
                )
                .into());
            }
            if m.get("name")
                .and_then(Value::as_str)
                .is_none_or(|n| n.trim().is_empty())
            {
                m.insert("name".into(), Value::String(next_name(data)));
            }
            if !m.contains_key("sortKey") {
                let last = data.video_inputs.iter().map(|v| v.sort_key.as_str()).max();
                m.insert(
                    "sortKey".into(),
                    Value::String(sort_key_between(last, None)),
                );
            }
            m.remove("import");
            let source = take_source(&mut m)?;
            let mut v: VideoInput = decode(m, id, "video input")?;
            check_name(&mut v)?;
            writes.push(put(&v)?);
            changes.push((DocumentKind::VideoInput, v.id.0, DocumentChange::Added));
            video_inputs.push(v.clone());
            bind(
                v.id,
                None,
                source.clone(),
                &mut input_bindings,
                &mut writes,
                &mut changes,
            )?;
            InputResult::Input(Box::new(entry(&v, source.as_ref())))
        }
        InputOp::Update { id, body, if_match } => {
            let old = data.video_inputs.iter().find(|v| v.id == id);
            let old_source = binding_of(data, id);
            check_revision(
                old.map(|v| (v, old_source)).as_ref(),
                |(v, s)| serde_json::to_value(entry(v, *s)).unwrap_or_default(),
                &if_match,
                "this video input",
            )?;
            let old = old.ok_or_else(|| not_found("video input", id.0))?;
            let mut m = body_object(body)?;
            let source = take_source(&mut m)?;
            let mut v: VideoInput = decode(m, id.0, "video input")?;
            if v.sort_key.is_empty() {
                v.sort_key = old.sort_key.clone();
            }
            check_name(&mut v)?;
            if v != *old {
                writes.push(put(&v)?);
                changes.push((DocumentKind::VideoInput, v.id.0, DocumentChange::Changed));
            }
            if let Some(slot) = video_inputs.iter_mut().find(|x| x.id == id) {
                *slot = v.clone();
            }
            bind(
                id,
                old_source,
                source.clone(),
                &mut input_bindings,
                &mut writes,
                &mut changes,
            )?;
            InputResult::Input(Box::new(entry(&v, source.as_ref())))
        }
        InputOp::Delete { id, if_match } => {
            let old = data.video_inputs.iter().find(|v| v.id == id);
            let old_source = binding_of(data, id);
            check_revision(
                old.map(|v| (v, old_source)).as_ref(),
                |(v, s)| serde_json::to_value(entry(v, *s)).unwrap_or_default(),
                &if_match,
                "this video input",
            )?;
            old.ok_or_else(|| not_found("video input", id.0))?;
            // The binding goes first: an interrupted delete leaves an unbound input, never
            // a binding of nothing.
            if data.input_bindings.iter().any(|b| b.input == id) {
                writes.push(DocWrite::delete::<InputBinding>(id.0));
                changes.push((DocumentKind::InputBinding, id.0, DocumentChange::Removed));
            }
            input_bindings.retain(|b| b.input != id);
            writes.push(DocWrite::delete::<VideoInput>(id.0));
            changes.push((DocumentKind::VideoInput, id.0, DocumentChange::Removed));
            video_inputs.retain(|v| v.id != id);
            InputResult::Deleted
        }
    };
    Ok(InputPlan {
        video_inputs,
        input_bindings,
        writes,
        changes,
        result,
    })
}
