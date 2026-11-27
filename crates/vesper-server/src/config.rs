//! Editing the screen configuration: screens (`portable/screen`) and output bindings
//! (`machine/outputBinding`), plus the looks a screen deletion touches.
//!
//! Every edit is **planned** here against the current [`ShowData`] without side effects:
//! the request is decoded, defaults are filled in, the `If-Match` revision is checked and the
//! result is validated ([`plan`]). The show actor then persists the plan's writes
//! ([`crate::store::apply`]: atomic, never over a newer schema), swaps in the new data and
//! reconfigures the engine live ([`crate::engine::apply_config`]). A validation or conflict
//! error changes nothing; an output that fails to start after a successful save keeps its
//! configuration and reports `unavailable` in the show state.
//!
//! **Revisions** are content hashes of the stored document ([`revision`]): stable across
//! restarts, and any change by anyone (another operator, a reload of a hand-edited file)
//! changes them.
//!
//! **New screens and looks:** a look lists only the screens it restyles; a screen it does not
//! list draws every layer ([`model::Look::for_screen`]). A new screen therefore needs no look
//! changes and shows everything in every look. Deleting a screen removes its entry from every
//! look that lists it and deletes its output bindings.

use crate::data::ShowData;
use crate::outputs::MAX_VIDEO_DELAY;
use crate::store::DocWrite;
use model::{
    AlphaMode, Document, DocumentKind, Look, LookId, Mask, MaskId, MaskMode, OutputAlpha,
    OutputBinding, OutputId, OutputTarget, OutputTransport, Screen, ScreenId, ScreenKind,
    sort_key_between,
};
use protocol::{
    ApiError, DocumentChange, ErrorCode, LookEntry, MaskDeleted, MaskEntry, OutputEntry,
    OutputTransportInfo, ScreenDeleted, ScreenEntry,
};
use serde_json::Value;
pub use vesper_core::change::{ConfigError, check_revision, parse_if_match, revision};

/// Smallest / largest screen side, px.
pub const MIN_SIDE: f64 = 64.0;
/// Largest screen side, px (8K).
pub const MAX_SIDE: f64 = 8192.0;
/// Largest screen area, px (8K UHD).
pub const MAX_AREA: f64 = 7680.0 * 4320.0;
/// Frame rate bounds, fps.
pub const MIN_FPS: f64 = 1.0;
/// Frame rate bounds, fps.
pub const MAX_FPS: f64 = 120.0;
/// Longest screen / output name, characters.
pub const MAX_NAME: usize = 64;

/// A configuration change.
#[derive(Clone, Debug)]
pub enum ConfigOp {
    /// `POST /screens` (partial screen JSON).
    CreateScreen(Value),
    /// `PUT /screens/{id}` (full screen JSON).
    UpdateScreen {
        /// The screen.
        id: ScreenId,
        /// Body.
        body: Value,
        /// `If-Match` revision.
        if_match: Option<String>,
    },
    /// `PUT /screens/{id}/color` or `/screens/{id}/corner-pin`: one of the screen's picture
    /// adjustments, applied live (no `If-Match`: the last change wins, like a fader).
    AdjustScreen {
        /// The screen.
        id: ScreenId,
        /// The new colour correction (JSON), if it changes.
        color: Option<Value>,
        /// The new corner pin (JSON), if it changes.
        corner_pin: Option<Value>,
    },
    /// `DELETE /screens/{id}`.
    DeleteScreen {
        /// The screen.
        id: ScreenId,
        /// `If-Match` revision.
        if_match: Option<String>,
    },
    /// `POST /outputs` (partial binding JSON).
    CreateOutput(Value),
    /// `PUT /outputs/{id}`.
    UpdateOutput {
        /// The binding.
        id: OutputId,
        /// Body.
        body: Value,
        /// `If-Match` revision.
        if_match: Option<String>,
    },
    /// `DELETE /outputs/{id}`.
    DeleteOutput {
        /// The binding.
        id: OutputId,
        /// `If-Match` revision.
        if_match: Option<String>,
    },
    /// `POST /looks` (partial look JSON).
    CreateLook(Value),
    /// `PUT /looks/{id}` (full look JSON).
    UpdateLook {
        /// The look.
        id: LookId,
        /// Body.
        body: Value,
        /// `If-Match` revision.
        if_match: Option<String>,
    },
    /// `POST /looks/{id}/duplicate`.
    DuplicateLook {
        /// The look to copy.
        id: LookId,
        /// Name of the copy (default "<name> copy").
        name: Option<String>,
    },
    /// `DELETE /looks/{id}` (the show actor refuses the live look first).
    DeleteLook {
        /// The look.
        id: LookId,
        /// `If-Match` revision.
        if_match: Option<String>,
    },
    /// `POST /masks` (partial mask JSON).
    CreateMask(Value),
    /// `PUT /masks/{id}` (full mask JSON).
    UpdateMask {
        /// The mask.
        id: MaskId,
        /// Body.
        body: Value,
        /// `If-Match` revision.
        if_match: Option<String>,
    },
    /// `POST /masks/{id}/duplicate`.
    DuplicateMask {
        /// The mask to copy.
        id: MaskId,
        /// Name of the copy (default "<name> copy").
        name: Option<String>,
    },
    /// `DELETE /masks/{id}` (`detach`: also when looks use it, which then forget it).
    DeleteMask {
        /// The mask.
        id: MaskId,
        /// `If-Match` revision.
        if_match: Option<String>,
        /// Remove it from the looks that use it (else such a delete is refused).
        detach: bool,
    },
}

/// What a successful change returns.
#[derive(Clone, Debug)]
pub enum ConfigResult {
    /// The created / updated screen.
    Screen(ScreenEntry),
    /// The created / updated output binding.
    Output(OutputEntry),
    /// A screen was deleted, with what went with it.
    ScreenDeleted(ScreenDeleted),
    /// An output binding was deleted.
    OutputDeleted,
    /// The created / updated / duplicated look.
    Look(LookEntry),
    /// A look was deleted.
    LookDeleted,
    /// The created / updated / duplicated mask.
    Mask(Box<MaskEntry>),
    /// A mask was deleted, with the looks that forgot it.
    MaskDeleted(MaskDeleted),
}

/// A validated change, ready to persist and apply.
#[derive(Debug)]
pub struct Plan {
    /// Screens after the change.
    pub screens: Vec<Screen>,
    /// Looks after the change.
    pub looks: Vec<Look>,
    /// Masks after the change.
    pub masks: Vec<Mask>,
    /// Output bindings after the change.
    pub outputs: Vec<OutputBinding>,
    /// Files to write / delete, in order.
    pub writes: Vec<DocWrite>,
    /// `documentChanged` events to send once applied.
    pub changes: Vec<(DocumentKind, uuid::Uuid, DocumentChange)>,
    /// The response.
    pub result: ConfigResult,
}

/// A screen with its revision.
pub fn screen_entry(s: &Screen) -> ScreenEntry {
    ScreenEntry {
        revision: revision(s),
        screen: s.clone(),
    }
}

/// An output binding with its revision.
pub fn output_entry(o: &OutputBinding) -> OutputEntry {
    OutputEntry {
        revision: revision(o),
        output: o.clone(),
    }
}

pub(crate) fn not_found(what: &str, id: uuid::Uuid) -> ConfigError {
    ApiError::new(ErrorCode::NotFound, format!("{what} {id} not found")).into()
}

pub(crate) fn body_object(body: Value) -> Result<serde_json::Map<String, Value>, ConfigError> {
    match body {
        Value::Object(m) => Ok(m),
        _ => Err(ApiError::new(ErrorCode::BadRequest, "the body must be a JSON object").into()),
    }
}

/// Decodes a document from a request body: `revision` is ignored, `id` must match (or is
/// filled in), `schemaVersion` is set to the current one. A body from an older client (an
/// older `schemaVersion`, or none) is migrated first, like a stored document, so a schema-1
/// `ndi` transport arrives as `idn`.
pub(crate) fn decode<T: Document>(
    mut m: serde_json::Map<String, Value>,
    id: uuid::Uuid,
    what: &str,
) -> Result<T, ConfigError> {
    m.remove("revision");
    match m.get("id") {
        Some(Value::String(s)) if s.parse::<uuid::Uuid>().ok() != Some(id) => {
            return Err(ApiError::invalid(
                "id",
                format!("the {what} ID in the body does not match the URL"),
            )
            .into());
        }
        Some(Value::String(_)) => {}
        Some(_) => return Err(ApiError::invalid("id", "id must be a UUID string").into()),
        None => {
            m.insert("id".into(), Value::String(id.to_string()));
        }
    }
    let mut v = Value::Object(m);
    // A newer or malformed `schemaVersion` is simply replaced below, as before.
    let _ = model::migrate(T::KIND, &mut v);
    v["schemaVersion"] = Value::from(model::SCHEMA_VERSION);
    serde_json::from_value::<T>(v).map_err(|e| {
        let msg = e.to_string();
        // serde's messages name the field in backticks: point the UI at it when possible.
        let field = msg
            .split('`')
            .nth(1)
            .filter(|f| f.chars().all(|c| c.is_ascii_alphanumeric()))
            .map(str::to_owned);
        ConfigError::from(ApiError {
            code: ErrorCode::BadRequest,
            message: format!("invalid {what}: {msg}"),
            field,
            live: None,
        })
    })
}

pub(crate) fn check_name(name: &str, what: &str) -> Result<(), ApiError> {
    let t = name.trim();
    if t.is_empty() {
        return Err(ApiError::invalid("name", format!("a {what} needs a name")));
    }
    if t.chars().count() > MAX_NAME {
        return Err(ApiError::invalid(
            "name",
            format!("names are at most {MAX_NAME} characters"),
        ));
    }
    if t.chars().any(char::is_control) {
        return Err(ApiError::invalid(
            "name",
            "names cannot contain control characters",
        ));
    }
    Ok(())
}

/// Checks a size (screens and resolution overrides).
pub fn check_size(w: f64, h: f64, field: &str) -> Result<(), ApiError> {
    if !w.is_finite() || !h.is_finite() || w.fract() != 0.0 || h.fract() != 0.0 {
        return Err(ApiError::invalid(
            field,
            "width and height must be whole pixels",
        ));
    }
    if w < MIN_SIDE || h < MIN_SIDE || w > MAX_SIDE || h > MAX_SIDE || w * h > MAX_AREA {
        return Err(ApiError::invalid(
            field,
            format!(
                "{w} × {h} is out of range: each side {MIN_SIDE}–{MAX_SIDE} px, at most \
                 7680 × 4320 pixels in all"
            ),
        ));
    }
    if !(w as u64).is_multiple_of(2) || !(h as u64).is_multiple_of(2) {
        return Err(ApiError::invalid(
            field,
            format!("width and height must be even numbers (got {w} × {h})"),
        ));
    }
    Ok(())
}

/// Checks a frame rate.
pub fn check_rate(fr: model::FrameRate, field: &str) -> Result<(), ApiError> {
    let fps = fr.as_f64();
    if fr.num == 0 || fr.den == 0 || !(MIN_FPS..=MAX_FPS).contains(&fps) {
        return Err(ApiError::invalid(
            field,
            format!(
                "frame rate {}/{} is out of range ({MIN_FPS}–{MAX_FPS} fps)",
                fr.num, fr.den
            ),
        ));
    }
    Ok(())
}

/// Validates a screen against the others (`old`: its stored version, for an update).
pub fn validate_screen(
    s: &mut Screen,
    data: &ShowData,
    old: Option<&Screen>,
) -> Result<(), ApiError> {
    check_name(&s.name, "screen")?;
    // A signage channel's screen keeps its role (its channel makes and removes it).
    if let Some(o) = old
        && matches!(o.kind, ScreenKind::Signage { .. })
            != matches!(s.kind, ScreenKind::Signage { .. })
    {
        return Err(ApiError::new(
            ErrorCode::Conflict,
            "a signage channel's screen keeps its role: change the channel in Site ▸ Signage",
        ));
    }
    let renamed = old.is_none_or(|o| o.name.trim() != s.name.trim());
    if renamed
        && let Some(other) = data
            .screens
            .iter()
            .find(|o| o.id != s.id && o.name.trim().eq_ignore_ascii_case(s.name.trim()))
    {
        return Err(ApiError::invalid(
            "name",
            format!("another screen is already called “{}”", other.name),
        ));
    }
    if matches!(s.kind, ScreenKind::Unknown) {
        return Err(ApiError::invalid(
            "kind",
            "the role must be audience or stage",
        ));
    }
    check_size(s.size.width, s.size.height, "size")?;
    check_rate(s.frame_rate, "frameRate")?;
    if s.alpha == AlphaMode::Unknown {
        return Err(ApiError::invalid(
            "alpha",
            "alpha must be opaque, straight or premultiplied",
        ));
    }
    let c = s.background;
    if [c.r, c.g, c.b, c.a]
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(ApiError::invalid(
            "background",
            "colour components must be between 0 and 1",
        ));
    }
    check_color(&s.color)?;
    check_corner_pin(&s.corner_pin, s.size)?;
    check_arrangement(&s.arrangement, s.size)?;
    // The half-tile rule (F08-3) is newer than some screens (saved before it, or imported): it
    // goes with an edit of the arrangement or the size, not with every edit of the screen
    // (a colour or keystone adjustment, a rename).
    if old.is_none_or(|o| o.arrangement != s.arrangement || o.size != s.size) {
        check_blend_overlap(&s.arrangement, s.size)?;
    }
    // A mask kept as it was that no longer fits (the mask deleted, or the screen no longer a
    // stage screen) goes with this edit instead of refusing every edit of the screen.
    if let Some(m) = s.mask
        && old.is_some_and(|o| o.mask == Some(m))
        && (!matches!(s.kind, ScreenKind::Stage { .. }) || !data.masks.iter().any(|x| x.id == m))
    {
        s.mask = None;
    }
    if let Some(m) = s.mask {
        if !matches!(s.kind, ScreenKind::Stage { .. }) {
            return Err(ApiError::invalid(
                "mask",
                "only a stage screen has its own mask (an audience screen's comes from the look)",
            ));
        }
        if !data.masks.iter().any(|x| x.id == m) {
            return Err(ApiError::invalid("mask", "no such mask"));
        }
    }
    Ok(())
}

/// Smallest tile or region an output may show, px.
pub const MIN_TILE: f64 = 16.0;

/// Checks a screen's arrangement: grid 1–8 × 1–8 (1 × 1 unless Grouped / Edge Blend), bezel
/// 0–2000 px, overlap ≥ 0 and smaller than a tile, blend gamma 1–3 and curve 1–4, and every
/// tile at least [`MIN_TILE`] px. (At most half a tile with three or more tiles in a line:
/// [`check_blend_overlap`], for a new or changed arrangement.)
pub fn check_arrangement(a: &model::ScreenArrangement, size: model::Size) -> Result<(), ApiError> {
    use model::ArrangementKind as K;
    let bad = |field: &str, msg: &str| Err(ApiError::invalid(format!("arrangement.{field}"), msg));
    match a.kind {
        K::Unknown => {
            return bad(
                "kind",
                "the arrangement must be single, mirror, grouped or edgeBlend",
            );
        }
        K::Single | K::Mirror if a.columns != 1 || a.rows != 1 => {
            return bad("columns", "only Grouped and Edge Blend screens have tiles");
        }
        _ => {}
    }
    for (field, v) in [("columns", a.columns), ("rows", a.rows)] {
        if !(1..=8).contains(&v) {
            return bad(field, "1 to 8 tiles");
        }
    }
    let finite = [
        a.bezel.x,
        a.bezel.y,
        a.overlap.x,
        a.overlap.y,
        a.blend.gamma,
        a.blend.curve,
    ]
    .iter()
    .all(|v| v.is_finite());
    if !finite {
        return bad("kind", "numbers must be finite");
    }
    if [a.bezel.x, a.bezel.y]
        .iter()
        .any(|v| !(0.0..=2000.0).contains(v))
    {
        return bad("bezel", "the bezel is 0 to 2000 px");
    }
    if [a.overlap.x, a.overlap.y].iter().any(|v| *v < 0.0) {
        return bad("overlap", "the overlap cannot be negative");
    }
    if !(1.0..=3.0).contains(&a.blend.gamma) {
        return bad("blend.gamma", "the blend gamma is 1 to 3");
    }
    if !(1.0..=4.0).contains(&a.blend.curve) {
        return bad("blend.curve", "the blend curve is 1 to 4");
    }
    if a.is_tiled() {
        let (tw, th) = a.tile_size(size);
        if tw < MIN_TILE || th < MIN_TILE {
            return bad(
                "columns",
                "the tiles don't fit the screen: make the screen larger or the bezel smaller",
            );
        }
        if a.kind == K::EdgeBlend && (a.overlap.x >= tw || a.overlap.y >= th) {
            return bad("overlap", "the overlap must be smaller than a tile");
        }
    }
    Ok(())
}

/// With three or more Edge Blend tiles across (or down), an overlap over half a tile lets three
/// projectors light the same strip, which the blend can't even out (F08-3).
pub fn check_blend_overlap(
    a: &model::ScreenArrangement,
    size: model::Size,
) -> Result<(), ApiError> {
    if a.kind != model::ArrangementKind::EdgeBlend || !a.is_tiled() {
        return Ok(());
    }
    let (tw, th) = a.tile_size(size);
    if (a.columns >= 3 && a.overlap.x > tw / 2.0) || (a.rows >= 3 && a.overlap.y > th / 2.0) {
        return Err(ApiError::invalid(
            "arrangement.overlap",
            "with three or more tiles in a row or column, the overlap is at most half a tile",
        ));
    }
    Ok(())
}

/// Checks a colour correction: every setting finite and in its range.
pub fn check_color(c: &model::ColorCorrection) -> Result<(), ApiError> {
    for (name, v, range) in c.fields() {
        if !v.is_finite() || !range.contains(&v) {
            return Err(ApiError::invalid(
                format!("color.{name}"),
                format!(
                    "{name} must be between {} and {} (got {v})",
                    range.start(),
                    range.end()
                ),
            ));
        }
    }
    Ok(())
}

/// Checks a corner pin on a screen of `size`: offsets finite and at most one screen width /
/// height, and (when it is on) corners that make a convex quadrilateral in their order (no
/// crossed or folded picture).
pub fn check_corner_pin(p: &model::CornerPin, size: model::Size) -> Result<(), ApiError> {
    let (w, h) = (size.width as f32, size.height as f32);
    let names = ["topLeft", "topRight", "bottomRight", "bottomLeft"];
    for (o, name) in p.corners().iter().zip(names) {
        if !o.x.is_finite() || !o.y.is_finite() || o.x.abs() > w || o.y.abs() > h {
            return Err(ApiError::invalid(
                format!("cornerPin.{name}"),
                format!("corner offsets are at most the screen's size ({w} × {h} px)"),
            ));
        }
    }
    if p.is_active() {
        let q = p.quad(w, h);
        let cross = |i: usize| {
            let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
            (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
        };
        // Clockwise on screen (y down) means every turn is positive.
        if (0..4).any(|i| cross(i) <= 0.0) {
            return Err(ApiError::invalid(
                "cornerPin",
                "the corners must make a convex shape (no crossed or folded corners)",
            ));
        }
    }
    Ok(())
}

/// The network name an output advertises, for duplicate checks: `(transport label, name)`.
/// IDN sources must be unique among the IDN outputs of this server: receivers list them as
/// `"HOST (name)"`, so two with one name would collide. OMT names likewise among OMT outputs.
fn network_name(b: &OutputBinding) -> Option<(&'static str, &str)> {
    match &b.transport {
        OutputTransport::Idn { source_name, .. } => Some(("IDN", source_name.as_str())),
        OutputTransport::Omt { name } => Some(("OMT", name.as_str())),
        _ => None,
    }
}

/// Fills in defaults that depend on the transport (an empty name becomes the source name)
/// and trims names.
fn normalize_output(b: &mut OutputBinding) {
    match &mut b.transport {
        OutputTransport::Idn {
            source_name,
            groups,
        } => {
            *source_name = source_name.trim().to_string();
            for g in groups.iter_mut() {
                *g = g.trim().to_string();
            }
            groups.retain(|g| !g.is_empty());
        }
        OutputTransport::Omt { name } => *name = name.trim().to_string(),
        _ => {}
    }
    b.name = b.name.trim().to_string();
    if b.name.is_empty()
        && let Some((_, n)) = network_name(b)
    {
        b.name = n.to_string();
    }
    // A display output is named after its display ("Example Monitor", else "HDMI-A-1").
    if b.name.is_empty()
        && let OutputTransport::Display { selector, .. } = &b.transport
    {
        b.name = selector
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| crate::displays::selector_label(selector))
            .trim()
            .to_string();
    }
}

/// Validates an output binding against the others (`old`: its stored version; `displays`:
/// the display list, to tell which display a selector names).
pub fn validate_output(
    b: &OutputBinding,
    data: &ShowData,
    old: Option<&OutputBinding>,
    displays: &[crate::displays::Display],
) -> Result<(), ApiError> {
    if !data.screens.iter().any(|s| s.id == b.screen) {
        return Err(ApiError::invalid("screen", "choose an existing screen"));
    }
    let unchanged_transport = old.is_some_and(|o| {
        std::mem::discriminant(&o.transport) == std::mem::discriminant(&b.transport)
    });
    match &b.transport {
        OutputTransport::Idn {
            source_name,
            groups,
        } => {
            if source_name.is_empty() {
                return Err(ApiError::invalid(
                    "transport.sourceName",
                    "an IDN output needs a source name",
                ));
            }
            if source_name.chars().count() > MAX_NAME || source_name.chars().any(char::is_control) {
                return Err(ApiError::invalid(
                    "transport.sourceName",
                    format!(
                        "source names are at most {MAX_NAME} characters, without control characters"
                    ),
                ));
            }
            if groups.iter().any(|g| g.contains(',')) {
                return Err(ApiError::invalid(
                    "transport.groups",
                    "group names cannot contain commas",
                ));
            }
        }
        OutputTransport::Omt { name } => {
            if name.is_empty() {
                return Err(ApiError::invalid(
                    "transport.name",
                    "an OMT output needs a source name",
                ));
            }
        }
        OutputTransport::Display { selector, mode, .. } => {
            if selector.connector.as_deref().unwrap_or("").is_empty()
                && selector.name.as_deref().unwrap_or("").is_empty()
                && selector.index.is_none()
            {
                return Err(ApiError::invalid("transport.selector", "choose a display"));
            }
            if let Some(m) = mode
                && (m.width == 0
                    || m.height == 0
                    || m.width > 16384
                    || m.height > 16384
                    || m.refresh_mhz == 0)
            {
                return Err(ApiError::invalid("transport.mode", "invalid display mode"));
            }
            let same = |o: &OutputBinding| match &o.transport {
                OutputTransport::Display { selector: s, .. } => {
                    crate::displays::same_display(displays, s, selector)
                }
                _ => false,
            };
            // An edit that keeps the display cannot make a new clash (E06-3: an output saved
            // before card-prefixed IDs stays editable whatever the other outputs use).
            let same_display_as_before = old.is_some_and(|o| {
                matches!(&o.transport, OutputTransport::Display { selector: s, .. } if s == selector)
            });
            if !same_display_as_before
                && let Some(other) = data.outputs.iter().find(|o| o.id != b.id && same(o))
            {
                let on = data
                    .screens
                    .iter()
                    .find(|s| s.id == other.screen)
                    .map(|s| format!(" of screen “{}”", s.name))
                    .unwrap_or_default();
                return Err(ApiError::invalid(
                    "transport.selector",
                    format!(
                        "another output{on} already uses display {}",
                        crate::displays::selector_label(selector)
                    ),
                ));
            }
        }
        OutputTransport::Sdi { .. } if !unchanged_transport => {
            return Err(ApiError::invalid(
                "transport",
                "SDI outputs are not supported yet",
            ));
        }
        OutputTransport::Unknown => {
            return Err(ApiError::invalid("transport", "choose a transport"));
        }
        _ => {}
    }
    check_name(&b.name, "output")?;
    if let Some((kind, name)) = network_name(b)
        && let Some(other) = data.outputs.iter().find(|o| {
            o.id != b.id
                && network_name(o).is_some_and(|(k, n)| k == kind && n.eq_ignore_ascii_case(name))
        })
    {
        let on = data
            .screens
            .iter()
            .find(|s| s.id == other.screen)
            .map(|s| format!(" on screen “{}”", s.name))
            .unwrap_or_default();
        let field = if kind == "IDN" {
            "transport.sourceName"
        } else {
            "transport.name"
        };
        let other_name = network_name(other).map_or(name, |(_, n)| n);
        return Err(ApiError::invalid(
            field,
            format!(
                "another output{on} already sends {kind} source “{other_name}”; names \
                 must be unique on this server"
            ),
        ));
    }
    if b.video_delay > MAX_VIDEO_DELAY {
        return Err(ApiError::invalid(
            "videoDelay",
            format!("video delay is 0–{MAX_VIDEO_DELAY} frames"),
        ));
    }
    if let Some(r) = b.resolution_override {
        check_size(r.width as f64, r.height as f64, "resolutionOverride")?;
    }
    if let Some(fr) = b.frame_rate_override {
        check_rate(fr, "frameRateOverride")?;
    }
    if matches!(b.alpha_override, Some(OutputAlpha::Unknown)) {
        return Err(ApiError::invalid(
            "alphaOverride",
            "alpha override must be opaque, straight or premultiplied",
        ));
    }
    if b.target == OutputTarget::Unknown {
        return Err(ApiError::invalid(
            "target",
            "the output target must be full, fill or key",
        ));
    }
    let screen = data.screens.iter().find(|s| s.id == b.screen);
    if let (Some(r), Some(s)) = (b.region, screen)
        && old.is_none_or(|o| o.region != b.region)
    {
        let inside = r.x >= 0.0
            && r.y >= 0.0
            && r.x + r.width <= s.size.width + 0.5
            && r.y + r.height <= s.size.height + 0.5;
        if !inside || !r.width.is_finite() || !r.height.is_finite() {
            return Err(ApiError::invalid(
                "region",
                "the region must lie inside the screen",
            ));
        }
        if r.width < MIN_TILE || r.height < MIN_TILE {
            return Err(ApiError::invalid(
                "region",
                format!("the region must be at least {MIN_TILE} px"),
            ));
        }
    }
    if let (Some(t), Some(s)) = (b.tile, screen)
        && old.is_none_or(|o| o.tile != b.tile)
    {
        let a = &s.arrangement;
        if !a.is_tiled() {
            return Err(ApiError::invalid(
                "tile",
                format!("screen “{}” isn't Grouped or Edge Blend", s.name),
            ));
        }
        if t >= a.cells() {
            return Err(ApiError::invalid(
                "tile",
                format!(
                    "screen “{}” has {} tiles ({}×{})",
                    s.name,
                    a.cells(),
                    a.columns,
                    a.rows
                ),
            ));
        }
    }
    Ok(())
}

/// A look with its revision.
pub fn look_entry(l: &Look) -> LookEntry {
    LookEntry {
        revision: revision(l),
        look: l.clone(),
    }
}

/// Validates and normalises a look: its name; per-screen entries only for existing audience
/// screens, at most one each; entries that change nothing are dropped; theme overrides name an
/// existing theme and template slide; the transition is a cut or has a sane duration.
pub fn validate_look(l: &mut Look, data: &ShowData, old: Option<&Look>) -> Result<(), ApiError> {
    l.name = l.name.trim().to_string();
    check_name(&l.name, "look")?;
    // (A copy is checked as a new look: its source keeps the name.)
    let renamed = old.is_none_or(|o| o.id != l.id || o.name.trim() != l.name);
    if renamed
        && let Some(other) = data
            .looks
            .iter()
            .find(|o| o.id != l.id && o.name.trim().eq_ignore_ascii_case(&l.name))
    {
        return Err(ApiError::invalid(
            "name",
            format!("another look is already called “{}”", other.name),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for (i, sl) in l.screens.iter().enumerate() {
        let field = format!("screens.{i}");
        let unchanged = old.is_some_and(|o| o.screens.iter().any(|x| x == sl));
        match data.screens.iter().find(|s| s.id == sl.screen) {
            Some(s) if !matches!(s.kind, ScreenKind::Audience) && !unchanged => {
                return Err(ApiError::invalid(
                    field,
                    format!(
                        "“{}” is a stage screen; looks only change audience screens",
                        s.name
                    ),
                ));
            }
            None if !unchanged => {
                return Err(ApiError::invalid(field, "choose an existing screen"));
            }
            _ => {}
        }
        if !seen.insert(sl.screen) {
            return Err(ApiError::invalid(field, "a screen is listed twice"));
        }
        if let Some(m) = sl.mask
            && !unchanged
            && !data.masks.iter().any(|x| x.id == m)
        {
            return Err(ApiError::invalid(
                format!("{field}.mask"),
                "choose an existing mask",
            ));
        }
        if let Some(t) = &sl.template_override
            && !unchanged
        {
            let theme = data.content.themes.get(&t.theme.id);
            match theme {
                None => {
                    return Err(ApiError::invalid(
                        format!("{field}.templateOverride"),
                        "choose an existing theme",
                    ));
                }
                Some(th) if !th.slides.iter().any(|ts| ts.id == t.slide) => {
                    return Err(ApiError::invalid(
                        format!("{field}.templateOverride"),
                        format!("theme “{}” has no such template slide", th.name),
                    ));
                }
                _ => {}
            }
        }
    }
    l.screens.retain(|sl| {
        sl.layers != model::LayerVisibility::ALL
            || sl.template_override.is_some()
            || sl.mask.is_some()
            || sl.language != model::LanguageDisplay::Auto
    });
    if let Some(t) = &l.transition {
        if !t.duration.is_finite() || !(0.0..=MAX_LOOK_TRANSITION).contains(&t.duration) {
            return Err(ApiError::invalid(
                "transition",
                format!("transitions last 0–{MAX_LOOK_TRANSITION} s"),
            ));
        }
        if matches!(t.kind, model::TransitionKind::Unknown) {
            return Err(ApiError::invalid("transition", "unknown transition"));
        }
    }
    Ok(())
}

/// A mask with its revision.
pub fn mask_entry(m: &Mask) -> MaskEntry {
    MaskEntry {
        revision: revision(m),
        mask: m.clone(),
    }
}

/// Largest mask feather, canvas pixels.
pub const MAX_FEATHER: f64 = 200.0;

/// Validates and normalises a mask: name (unique), canvas size, mode, colour and feather.
pub fn validate_mask(m: &mut Mask, data: &ShowData, old: Option<&Mask>) -> Result<(), ApiError> {
    m.name = m.name.trim().to_string();
    check_name(&m.name, "mask")?;
    // (A copy is checked as a new mask: its source keeps the name.)
    let renamed = old.is_none_or(|o| o.id != m.id || o.name.trim() != m.name);
    if renamed
        && let Some(other) = data
            .masks
            .iter()
            .find(|o| o.id != m.id && o.name.trim().eq_ignore_ascii_case(&m.name))
    {
        return Err(ApiError::invalid(
            "name",
            format!("another mask is already called “{}”", other.name),
        ));
    }
    check_size(m.size.width, m.size.height, "size")?;
    if m.mode == MaskMode::Unknown {
        return Err(ApiError::invalid("mode", "mode must be overlay or keyhole"));
    }
    let c = m.color;
    if [c.r, c.g, c.b, c.a]
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(ApiError::invalid(
            "color",
            "colour components must be between 0 and 1",
        ));
    }
    if !m.feather.is_finite() || !(0.0..=MAX_FEATHER).contains(&m.feather) {
        return Err(ApiError::invalid(
            "feather",
            format!("feather is 0–{MAX_FEATHER} px"),
        ));
    }
    Ok(())
}

/// A free "<base> N" mask name, or `base` itself when free and `bare` is allowed.
fn free_mask_name(data: &ShowData, base: &str, bare: bool) -> String {
    let taken = |n: &str| {
        data.masks
            .iter()
            .any(|m| m.name.trim().eq_ignore_ascii_case(n))
    };
    if bare && !taken(base) {
        return base.to_string();
    }
    (if bare { 2 } else { 1 }..)
        .map(|n| format!("{base} {n}"))
        .find(|n| !taken(n))
        .expect("a free name")
}

/// Longest look transition, seconds.
pub const MAX_LOOK_TRANSITION: f64 = 10.0;

/// A free "<base> N" look name (`Look 1`, `Lyrics copy 2`), or `base` itself when free and
/// `bare` is allowed.
fn free_look_name(data: &ShowData, base: &str, bare: bool) -> String {
    let taken = |n: &str| {
        data.looks
            .iter()
            .any(|l| l.name.trim().eq_ignore_ascii_case(n))
    };
    if bare && !taken(base) {
        return base.to_string();
    }
    (if bare { 2 } else { 1 }..)
        .map(|n| format!("{base} {n}"))
        .find(|n| !taken(n))
        .expect("a free name")
}

/// A free "Screen N" name.
fn next_screen_name(data: &ShowData) -> String {
    (1..)
        .map(|n| format!("Screen {n}"))
        .find(|n| !data.screens.iter().any(|s| s.name.eq_ignore_ascii_case(n)))
        .expect("a free name")
}

pub(crate) fn put<T: Document>(doc: &T) -> Result<DocWrite, ConfigError> {
    DocWrite::put(doc).map_err(|e| ApiError::new(ErrorCode::Internal, format!("{e:#}")).into())
}

/// Plans a change (no side effects).
pub fn plan(
    data: &ShowData,
    displays: &[crate::displays::Display],
    op: ConfigOp,
) -> Result<Plan, ConfigError> {
    let mut screens = data.screens.clone();
    let mut looks = data.looks.clone();
    let mut masks = data.masks.clone();
    let mut outputs = data.outputs.clone();
    let mut writes = Vec::new();
    let mut changes = Vec::new();
    let result = match op {
        ConfigOp::CreateScreen(body) => {
            let mut m = body_object(body)?;
            let id = match m.get("id").and_then(Value::as_str) {
                Some(s) => s
                    .parse::<uuid::Uuid>()
                    .map_err(|_| ApiError::invalid("id", "id must be a UUID"))?,
                None => uuid::Uuid::new_v4(),
            };
            if data.screens.iter().any(|s| s.id.0 == id) {
                return Err(ApiError::new(
                    ErrorCode::Conflict,
                    format!("screen {id} already exists"),
                )
                .into());
            }
            if !m.contains_key("name") {
                m.insert("name".into(), Value::String(next_screen_name(data)));
            }
            if !m.contains_key("sortKey") {
                let last = data.screens.iter().map(|s| s.sort_key.as_str()).max();
                m.insert(
                    "sortKey".into(),
                    Value::String(sort_key_between(last, None)),
                );
            }
            m.remove("import");
            let mut s: Screen = decode(m, id, "screen")?;
            s.name = s.name.trim().to_string();
            validate_screen(&mut s, data, None)?;
            writes.push(put(&s)?);
            changes.push((DocumentKind::Screen, s.id.0, DocumentChange::Added));
            screens.push(s.clone());
            ConfigResult::Screen(screen_entry(&s))
        }
        ConfigOp::UpdateScreen { id, body, if_match } => {
            let old = data.screens.iter().find(|s| s.id == id);
            check_revision(
                old,
                |s| serde_json::to_value(screen_entry(s)).unwrap_or_default(),
                &if_match,
                "this screen",
            )?;
            let old = old.ok_or_else(|| not_found("screen", id.0))?;
            let mut s: Screen = decode(body_object(body)?, id.0, "screen")?;
            s.name = s.name.trim().to_string();
            validate_screen(&mut s, data, Some(old))?;
            if s != *old {
                writes.push(put(&s)?);
                changes.push((DocumentKind::Screen, s.id.0, DocumentChange::Changed));
            }
            if let Some(slot) = screens.iter_mut().find(|x| x.id == id) {
                *slot = s.clone();
            }
            ConfigResult::Screen(screen_entry(&s))
        }
        ConfigOp::AdjustScreen {
            id,
            color,
            corner_pin,
        } => {
            let old = data
                .screens
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(|| not_found("screen", id.0))?;
            let mut s = old.clone();
            let bad = |field: &str, e: serde_json::Error| {
                ConfigError::from(ApiError::invalid(field, format!("invalid {field}: {e}")))
            };
            if let Some(v) = color {
                s.color = serde_json::from_value(v).map_err(|e| bad("color", e))?;
            }
            if let Some(v) = corner_pin {
                s.corner_pin = serde_json::from_value(v).map_err(|e| bad("cornerPin", e))?;
            }
            validate_screen(&mut s, data, Some(old))?;
            if s != *old {
                writes.push(put(&s)?);
                changes.push((DocumentKind::Screen, s.id.0, DocumentChange::Changed));
            }
            if let Some(slot) = screens.iter_mut().find(|x| x.id == id) {
                *slot = s.clone();
            }
            ConfigResult::Screen(screen_entry(&s))
        }
        ConfigOp::DeleteScreen { id, if_match } => {
            let old = data.screens.iter().find(|s| s.id == id);
            check_revision(
                old,
                |s| serde_json::to_value(screen_entry(s)).unwrap_or_default(),
                &if_match,
                "this screen",
            )?;
            old.ok_or_else(|| not_found("screen", id.0))?;
            let mut deleted = ScreenDeleted::default();
            for l in looks.iter_mut() {
                if l.screens.iter().any(|s| s.screen == id) {
                    l.screens.retain(|s| s.screen != id);
                    writes.push(put(l)?);
                    changes.push((DocumentKind::Look, l.id.0, DocumentChange::Changed));
                    deleted.looks.push(l.id);
                }
            }
            for o in outputs.iter().filter(|o| o.screen == id) {
                writes.push(DocWrite::delete::<OutputBinding>(o.id.0));
                changes.push((DocumentKind::OutputBinding, o.id.0, DocumentChange::Removed));
                deleted.outputs.push(o.id);
            }
            outputs.retain(|o| o.screen != id);
            // The screen goes last: an interrupted delete never leaves orphaned bindings.
            writes.push(DocWrite::delete::<Screen>(id.0));
            changes.push((DocumentKind::Screen, id.0, DocumentChange::Removed));
            screens.retain(|s| s.id != id);
            ConfigResult::ScreenDeleted(deleted)
        }
        ConfigOp::CreateOutput(body) => {
            let m = body_object(body)?;
            let id = match m.get("id").and_then(Value::as_str) {
                Some(s) => s
                    .parse::<uuid::Uuid>()
                    .map_err(|_| ApiError::invalid("id", "id must be a UUID"))?,
                None => uuid::Uuid::new_v4(),
            };
            if data.outputs.iter().any(|o| o.id.0 == id) {
                return Err(ApiError::new(
                    ErrorCode::Conflict,
                    format!("output {id} already exists"),
                )
                .into());
            }
            if !m.contains_key("screen") {
                return Err(
                    ApiError::invalid("screen", "choose the screen this output sends").into(),
                );
            }
            if !m.contains_key("transport") {
                return Err(ApiError::invalid("transport", "choose a transport").into());
            }
            let mut b: OutputBinding = decode(m, id, "output")?;
            normalize_output(&mut b);
            validate_output(&b, data, None, displays)?;
            writes.push(put(&b)?);
            changes.push((DocumentKind::OutputBinding, b.id.0, DocumentChange::Added));
            outputs.push(b.clone());
            ConfigResult::Output(output_entry(&b))
        }
        ConfigOp::UpdateOutput { id, body, if_match } => {
            let old = data.outputs.iter().find(|o| o.id == id);
            check_revision(
                old,
                |o| serde_json::to_value(output_entry(o)).unwrap_or_default(),
                &if_match,
                "this output",
            )?;
            let old = old.ok_or_else(|| not_found("output", id.0))?;
            let mut b: OutputBinding = decode(body_object(body)?, id.0, "output")?;
            normalize_output(&mut b);
            validate_output(&b, data, Some(old), displays)?;
            if b != *old {
                writes.push(put(&b)?);
                changes.push((DocumentKind::OutputBinding, b.id.0, DocumentChange::Changed));
            }
            if let Some(slot) = outputs.iter_mut().find(|x| x.id == id) {
                *slot = b.clone();
            }
            ConfigResult::Output(output_entry(&b))
        }
        ConfigOp::DeleteOutput { id, if_match } => {
            let old = data.outputs.iter().find(|o| o.id == id);
            check_revision(
                old,
                |o| serde_json::to_value(output_entry(o)).unwrap_or_default(),
                &if_match,
                "this output",
            )?;
            old.ok_or_else(|| not_found("output", id.0))?;
            writes.push(DocWrite::delete::<OutputBinding>(id.0));
            changes.push((DocumentKind::OutputBinding, id.0, DocumentChange::Removed));
            outputs.retain(|o| o.id != id);
            ConfigResult::OutputDeleted
        }
        ConfigOp::CreateLook(body) => {
            let mut m = body_object(body)?;
            let id = match m.get("id").and_then(Value::as_str) {
                Some(s) => s
                    .parse::<uuid::Uuid>()
                    .map_err(|_| ApiError::invalid("id", "id must be a UUID"))?,
                None => uuid::Uuid::new_v4(),
            };
            if data.looks.iter().any(|l| l.id.0 == id) {
                return Err(ApiError::new(
                    ErrorCode::Conflict,
                    format!("look {id} already exists"),
                )
                .into());
            }
            if m.get("name")
                .and_then(Value::as_str)
                .is_none_or(|n| n.trim().is_empty())
            {
                m.insert(
                    "name".into(),
                    Value::String(free_look_name(data, "Look", false)),
                );
            }
            if !m.contains_key("sortKey") {
                let last = data.looks.iter().map(|l| l.sort_key.as_str()).max();
                m.insert(
                    "sortKey".into(),
                    Value::String(sort_key_between(last, None)),
                );
            }
            m.remove("import");
            let mut l: Look = decode(m, id, "look")?;
            validate_look(&mut l, data, None)?;
            writes.push(put(&l)?);
            changes.push((DocumentKind::Look, l.id.0, DocumentChange::Added));
            looks.push(l.clone());
            ConfigResult::Look(look_entry(&l))
        }
        ConfigOp::UpdateLook { id, body, if_match } => {
            let old = data.looks.iter().find(|l| l.id == id);
            check_revision(
                old,
                |l| serde_json::to_value(look_entry(l)).unwrap_or_default(),
                &if_match,
                "this look",
            )?;
            let old = old.ok_or_else(|| not_found("look", id.0))?;
            let mut l: Look = decode(body_object(body)?, id.0, "look")?;
            // Provenance is the server's: a client can't set or drop it.
            l.import = old.import.clone();
            if l.sort_key.is_empty() {
                l.sort_key = old.sort_key.clone();
            }
            validate_look(&mut l, data, Some(old))?;
            if l != *old {
                writes.push(put(&l)?);
                changes.push((DocumentKind::Look, l.id.0, DocumentChange::Changed));
            }
            if let Some(slot) = looks.iter_mut().find(|x| x.id == id) {
                *slot = l.clone();
            }
            ConfigResult::Look(look_entry(&l))
        }
        ConfigOp::DuplicateLook { id, name } => {
            let old = data
                .looks
                .iter()
                .find(|l| l.id == id)
                .ok_or_else(|| not_found("look", id.0))?;
            let mut l = old.clone();
            l.id = LookId::new();
            l.import = None;
            l.name = match name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
                Some(n) => n,
                None => free_look_name(data, &format!("{} copy", old.name.trim()), true),
            };
            let next = data
                .looks
                .iter()
                .map(|x| x.sort_key.as_str())
                .filter(|k| *k > old.sort_key.as_str())
                .min();
            l.sort_key = sort_key_between(Some(&old.sort_key), next);
            validate_look(&mut l, data, Some(old))?;
            writes.push(put(&l)?);
            changes.push((DocumentKind::Look, l.id.0, DocumentChange::Added));
            looks.push(l.clone());
            ConfigResult::Look(look_entry(&l))
        }
        ConfigOp::DeleteLook { id, if_match } => {
            let old = data.looks.iter().find(|l| l.id == id);
            check_revision(
                old,
                |l| serde_json::to_value(look_entry(l)).unwrap_or_default(),
                &if_match,
                "this look",
            )?;
            old.ok_or_else(|| not_found("look", id.0))?;
            writes.push(DocWrite::delete::<Look>(id.0));
            changes.push((DocumentKind::Look, id.0, DocumentChange::Removed));
            looks.retain(|l| l.id != id);
            ConfigResult::LookDeleted
        }
        ConfigOp::CreateMask(body) => {
            let mut m = body_object(body)?;
            let id = match m.get("id").and_then(Value::as_str) {
                Some(s) => s
                    .parse::<uuid::Uuid>()
                    .map_err(|_| ApiError::invalid("id", "id must be a UUID"))?,
                None => uuid::Uuid::new_v4(),
            };
            if data.masks.iter().any(|x| x.id.0 == id) {
                return Err(ApiError::new(
                    ErrorCode::Conflict,
                    format!("mask {id} already exists"),
                )
                .into());
            }
            if m.get("name")
                .and_then(Value::as_str)
                .is_none_or(|n| n.trim().is_empty())
            {
                m.insert(
                    "name".into(),
                    Value::String(free_mask_name(data, "Mask", false)),
                );
            }
            if !m.contains_key("sortKey") {
                let last = data.masks.iter().map(|x| x.sort_key.as_str()).max();
                m.insert(
                    "sortKey".into(),
                    Value::String(sort_key_between(last, None)),
                );
            }
            if !m.contains_key("slide") {
                m.insert(
                    "slide".into(),
                    serde_json::to_value(model::Slide::new()).unwrap_or_default(),
                );
            }
            m.remove("import");
            let mut x: Mask = decode(m, id, "mask")?;
            validate_mask(&mut x, data, None)?;
            writes.push(put(&x)?);
            changes.push((DocumentKind::Mask, x.id.0, DocumentChange::Added));
            masks.push(x.clone());
            ConfigResult::Mask(Box::new(mask_entry(&x)))
        }
        ConfigOp::UpdateMask { id, body, if_match } => {
            let old = data.masks.iter().find(|x| x.id == id);
            check_revision(
                old,
                |x| serde_json::to_value(mask_entry(x)).unwrap_or_default(),
                &if_match,
                "this mask",
            )?;
            let old = old.ok_or_else(|| not_found("mask", id.0))?;
            let mut x: Mask = decode(body_object(body)?, id.0, "mask")?;
            if x.sort_key.is_empty() {
                x.sort_key = old.sort_key.clone();
            }
            validate_mask(&mut x, data, Some(old))?;
            if x != *old {
                writes.push(put(&x)?);
                changes.push((DocumentKind::Mask, x.id.0, DocumentChange::Changed));
            }
            if let Some(slot) = masks.iter_mut().find(|m| m.id == id) {
                *slot = x.clone();
            }
            ConfigResult::Mask(Box::new(mask_entry(&x)))
        }
        ConfigOp::DuplicateMask { id, name } => {
            let old = data
                .masks
                .iter()
                .find(|x| x.id == id)
                .ok_or_else(|| not_found("mask", id.0))?;
            let mut x = old.clone();
            x.id = MaskId::new();
            x.slide.id = model::SlideId::new();
            x.name = match name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
                Some(n) => n,
                None => free_mask_name(data, &format!("{} copy", old.name.trim()), true),
            };
            let next = data
                .masks
                .iter()
                .map(|m| m.sort_key.as_str())
                .filter(|k| *k > old.sort_key.as_str())
                .min();
            x.sort_key = sort_key_between(Some(&old.sort_key), next);
            validate_mask(&mut x, data, Some(old))?;
            writes.push(put(&x)?);
            changes.push((DocumentKind::Mask, x.id.0, DocumentChange::Added));
            masks.push(x.clone());
            ConfigResult::Mask(Box::new(mask_entry(&x)))
        }
        ConfigOp::DeleteMask {
            id,
            if_match,
            detach,
        } => {
            let old = data.masks.iter().find(|x| x.id == id);
            check_revision(
                old,
                |x| serde_json::to_value(mask_entry(x)).unwrap_or_default(),
                &if_match,
                "this mask",
            )?;
            old.ok_or_else(|| not_found("mask", id.0))?;
            let users: Vec<LookId> = looks
                .iter()
                .filter(|l| l.screens.iter().any(|s| s.mask == Some(id)))
                .map(|l| l.id)
                .collect();
            let stage_users: Vec<ScreenId> = screens
                .iter()
                .filter(|s| s.mask == Some(id))
                .map(|s| s.id)
                .collect();
            if (!users.is_empty() || !stage_users.is_empty()) && !detach {
                return Err(ConfigError {
                    error: ApiError::new(
                        ErrorCode::Conflict,
                        format!(
                            "{} look(s) and {} stage screen(s) use this mask; delete with detach \
                             to remove it from them",
                            users.len(),
                            stage_users.len()
                        ),
                    ),
                    current: Some(serde_json::json!({ "looks": users, "screens": stage_users })),
                });
            }
            for s in screens.iter_mut().filter(|s| stage_users.contains(&s.id)) {
                s.mask = None;
                writes.push(put(s)?);
                changes.push((DocumentKind::Screen, s.id.0, DocumentChange::Changed));
            }
            for l in looks.iter_mut().filter(|l| users.contains(&l.id)) {
                for s in l.screens.iter_mut() {
                    if s.mask == Some(id) {
                        s.mask = None;
                    }
                }
                l.screens.retain(|s| {
                    s.layers != model::LayerVisibility::ALL
                        || s.template_override.is_some()
                        || s.mask.is_some()
                        || s.language != model::LanguageDisplay::Auto
                });
                writes.push(put(l)?);
                changes.push((DocumentKind::Look, l.id.0, DocumentChange::Changed));
            }
            // The mask goes last: an interrupted delete never leaves looks naming a lost mask.
            writes.push(DocWrite::delete::<Mask>(id.0));
            changes.push((DocumentKind::Mask, id.0, DocumentChange::Removed));
            masks.retain(|m| m.id != id);
            ConfigResult::MaskDeleted(MaskDeleted {
                looks: users,
                screens: stage_users,
            })
        }
    };
    Ok(Plan {
        screens,
        looks,
        masks,
        outputs,
        writes,
        changes,
        result,
    })
}

/// The trademark notice shown with the IDN backend.
pub const TRADEMARK: &str = "NDI® is a registered trademark of Vizrt NDI AB; this project is not affiliated with or \
     endorsed by Vizrt.";

/// What each output transport can do on this server (`GET /api/v1/output-transports`);
/// `display_backend` is [`crate::displays::Displays::backend`], `display_problem` its
/// [`crate::displays::Displays::problem`].
pub fn transports(
    display_backend: &str,
    display_problem: Option<&str>,
) -> Vec<OutputTransportInfo> {
    let version = output::idn::LIBRARY_VERSION;
    let idn = OutputTransportInfo {
        kind: "idn".into(),
        label: output::idn::LABEL.into(),
        supported: true,
        available: true,
        version: Some(version.into()),
        status: format!("built in (IDN Open Protocol {version})"),
        detail: Some(format!(
            "An open implementation compatible with NDI® (IDN Open Protocol {version}), \
             compiled into this server: no runtime to install. Its sources appear in the \
             source list of compatible receivers. {TRADEMARK}"
        )),
    };
    let (available, status, detail) = match (display_backend, display_problem) {
        ("kms", _) => (
            true,
            "direct (KMS)".to_string(),
            "Full screen on the monitors and projectors attached to this box, driven directly \
             (no desktop needed). The box must not run a desktop session on the same screens."
                .to_string(),
        ),
        ("window", None) => (
            true,
            "windows on this desktop".to_string(),
            "A borderless window covering the chosen monitor (or a plain window, for \
             rehearsal) on this computer's desktop. The server must run in the signed-in \
             user's session (on Windows: the installer's \"Show screens on this PC's monitors\")."
                .to_string(),
        ),
        ("window", Some(p)) => (
            false,
            "no desktop session".to_string(),
            format!(
                "Display outputs show windows on this computer's desktop, but the server cannot \
                 open any: {p}. Run it in the signed-in user's session."
            ),
        ),
        ("fake", _) => (
            true,
            "simulated".to_string(),
            "Simulated displays (tests).".to_string(),
        ),
        _ if cfg!(any(windows, target_os = "linux")) => (
            false,
            "turned off on this server".to_string(),
            "Display outputs are turned off on this server (--displays none).".to_string(),
        ),
        _ => (
            false,
            "not available on this platform".to_string(),
            "Full screen outputs on attached monitors are not available on this platform."
                .to_string(),
        ),
    };
    vec![
        idn,
        OutputTransportInfo {
            kind: "omt".into(),
            label: "OMT".into(),
            supported: true,
            available: true,
            version: Some(output::omt::LIBRARY_VERSION.into()),
            status: format!("built in (idn-omt {})", output::omt::LIBRARY_VERSION),
            detail: Some(
                "Open Media Transport (open, royalty-free; vMix, OBS), compiled into this \
                 server with its VMX codec: no runtime to install. Its sources appear as \
                 \"HOST (name)\" in OMT receivers. Alpha and program audio are sent."
                    .into(),
            ),
        },
        OutputTransportInfo {
            kind: "display".into(),
            label: "Display".into(),
            supported: true,
            available,
            version: None,
            status,
            detail: Some(detail),
        },
        OutputTransportInfo {
            kind: "sdi".into(),
            label: "SDI".into(),
            supported: false,
            available: false,
            version: None,
            status: "not yet supported".into(),
            detail: Some("SDI cards (DeckLink) are planned for a later version.".into()),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H06-1: a screen kept from before the half-tile rule (or imported) can still be adjusted
    /// and renamed; the rule goes with a change of its arrangement or size.
    #[test]
    fn the_half_tile_rule_goes_with_an_arrangement_edit_only() {
        let dir = std::env::temp_dir().join(format!("config-h06-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("portable")).unwrap();
        let data = ShowData::load(&dir).unwrap();
        let old = Screen {
            name: "Wide".into(),
            size: model::Size {
                width: 3840.0,
                height: 1080.0,
            },
            arrangement: model::ScreenArrangement {
                kind: model::ArrangementKind::EdgeBlend,
                columns: 3,
                overlap: model::TileGap { x: 1200.0, y: 0.0 },
                ..Default::default()
            },
            ..Default::default()
        };
        // A colour adjustment and a rename keep the arrangement: taken.
        let mut s = old.clone();
        s.color.brightness = 0.1;
        s.name = "Wide 2".into();
        validate_screen(&mut s, &data, Some(&old)).expect("an edit not of the arrangement");
        // A new screen, or the arrangement or size changed: refused.
        let mut s = old.clone();
        assert!(validate_screen(&mut s, &data, None).is_err());
        let mut s = old.clone();
        s.arrangement.overlap.x = 1100.0;
        assert!(validate_screen(&mut s, &data, Some(&old)).is_err());
        let mut s = old.clone();
        s.size.width = 3800.0;
        assert!(validate_screen(&mut s, &data, Some(&old)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
