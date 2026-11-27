//! **Area: config.** `Configuration/Workspace` (`rv.data.ProPresenterWorkspace`) → screens,
//! output-binding suggestions, looks, masks, video / audio inputs (+ binding suggestions) and the
//! audio device suggestion; plus the theme-link post-pass.
//!
//! ## Owns
//! - [`import_workspace`]: everything below, in list order with `ids::sort_keys`; registers
//!   `ctx.lookups.{screens, looks, masks, video_inputs, audio_inputs}`. Video inputs MUST be
//!   registered here (in list order) because presentations and playlists resolve
//!   `liveVideoIndex` through `ctx.lookups.video_inputs` later. `liveAudienceLook` → the saved
//!   look it copies (`originalLookUuid`) becomes `ShowSettings.startupLook` (a live look without
//!   a saved original is imported as its own look). `selectedLibraryName`, `recordSettings`,
//!   cloud entries → `notImported` warnings / unmapped counts.
//! - [`screen`]: portable [`Screen`] (ID = ProScreen UUID, name, `kind` Audience / Stage with
//!   the layout from `stageLayoutMappings`, `size` = canvas from `bounds`, `frameRate` from the
//!   output display mode (rational: 29.97 → 30000/1001), `alpha` from `alphaSettings.mode`,
//!   `background` when `backgroundColorEnabled`). Arrangement / corner pin / colour adjustments
//!   that are non-default → `import.unmapped`.
//! - [`output_bindings`]: machine-local suggestions, one per sub-screen with an
//!   `outputDisplay`: TYPE_NDI → `Idn { sourceName: deviceName }` (IDN Open Protocol, compatible
//!   with NDI®), screen → `Display { selector }`,
//!   cards → `Sdi`, else `Unknown`; frame rate / resolution overrides only when they differ from
//!   the screen; `region` from `subscreenUnitRect` when not the full canvas; `embedAudio` from
//!   `digitalAudioSetup.enableSdiNdiDevice`. ID `ids::output_id(screen, index)`.
//! - [`look`] / [`screen_look`]: layer flags → [`LayerVisibility`] (absent flag = hidden in
//!   ProPresenter: build the struct explicitly from every flag; `propsEnabled ||
//!   propsLayerEnabled` → props), mask, template override with a *provisional* theme link
//!   (`ids::theme_id` of the URL's folder + the template slide UUID) fixed by
//!   [`resolve_theme_links`]. `transitionDuration` → `Transition::dissolve` when > 0.
//! - [`mask`]: [`Mask`] (ID = base slide UUID, else derived), body via `slide::slide_from_pb`.
//! - [`video_input`] / [`input_binding`]: portable [`VideoInput`] (name = `userDescription`,
//!   colour, alpha, audio routing → `AudioSettings`, `AUDIO_DEVICE_TYPE_NONE` → `muted`,
//!   thumbnail via `media::asset_for_url`); binding suggestion from the device name
//!   ([`input_source`]: `"HOST (name)"` → IDN).
//! - [`audio_input`] / [`audio_input_binding`], [`audio_device_config`], [`audio_routing`].
//! - [`resolve_theme_links`]: post-pass after themes. For every look override, the template
//!   slide UUID is looked up in `ctx.lookups.template_slides` (kept theme after de-duplication);
//!   the theme link gets the kept theme's ID and name; unresolved ones stay dangling with a
//!   warning. Same for message templates (`config.rs` messages, owned by this area too).
//!
//! ## Conventions used here
//! - **Order:** screens → masks → video inputs → audio inputs → looks (looks resolve masks and
//!   screens through the lookups filled just before) → audio device → reported leftovers.
//! - **IDs:** source UUIDs; a missing / invalid / repeated UUID gets a UUIDv5 derived from the
//!   document path plus the list index (repeats are warned), so every document ID is unique.
//! - **Audio levels** are dB as ProPresenter stores them (`masterChannel.level`, channel
//!   `level`); routing maps are `channelIndex` (source) → `mappedIndices` (destinations).
//! - **Machine-local data** (network source names, capture device names, audio interfaces) never goes
//!   into a portable document: it becomes a suggested binding in `machine/`. Output-display
//!   leftovers with no binding slot (`renderId`, `modeIndex`, Blackmagic options on non-card
//!   outputs) are machine identity of the source computer and are dropped on purpose.
//!
//! ## May call
//! `common::*`, `slide::slide_from_pb`, `media::asset_for_url`, `ImportContext` helpers.

use std::collections::BTreeSet;

use model::{
    AlphaMode, ArrangementKind, AudioDeviceConfig, AudioInput, AudioInputBinding, AudioInputId,
    AudioInputMode, AudioSettings, BlendCurve, ChannelRoute, DisplaySelector, FrameRate,
    ImportInfo, InputBinding, InputSource, LayerVisibility, Link, Look, LookId, Mask, MaskId,
    MediaKind, OutputAlpha, OutputBinding, OutputTarget, OutputTransport, Rect, Resolution, Screen,
    ScreenArrangement, ScreenId, ScreenKind, ScreenLook, Size, SlideId, SortKey, SourceAlpha,
    StageLayoutId, TemplateOverride, TemplateSlideId, ThemeId, TileGap, Transition, VideoInput,
    VideoInputId,
};
use serde_json::Value;
use uuid::Uuid;

use crate::context::{ImportContext, NamedId, ids};
use crate::convert::{common, media, slide};
use crate::pb::rv::data as pp;
use crate::report::Category;

/// Imports the workspace (see the module docs).
pub fn import_workspace(ctx: &mut ImportContext, ws: &pp::ProPresenterWorkspace, rel_path: &str) {
    let embed_audio = ws
        .digital_audio_setup
        .as_ref()
        .is_some_and(|d| d.enable_sdi_ndi_device);

    // ---- screens + output binding suggestions ----
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    let keys = ids::sort_keys(ws.pro_screens.len());
    for (i, (s, key)) in ws.pro_screens.iter().zip(keys).enumerate() {
        let mut sc = screen(ctx, s, key, &ws.stage_layout_mappings);
        let source = ids::parse_uuid(s.uuid.as_ref());
        if !seen.insert(sc.id.0) {
            ctx.warn(
                Category::Other,
                format!(
                    "screen \"{}\" repeats an ID; a new one was derived",
                    sc.name
                ),
            );
            sc.id = ScreenId::from_uuid(ids::derived("screen", &[rel_path, &i.to_string()]));
            seen.insert(sc.id.0);
        }
        ctx.lookups
            .screens
            .entry(source.unwrap_or(sc.id.0))
            .or_insert_with(|| NamedId {
                id: sc.id,
                name: sc.name.clone(),
            });
        let bindings = output_bindings(ctx, s, sc.id, embed_audio);
        ctx.out.output_bindings.extend(bindings);
        ctx.out.screens.push(sc);
    }

    // ---- masks (before looks: looks link them) ----
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    let keys = ids::sort_keys(ws.masks.len());
    for (i, (m, key)) in ws.masks.iter().zip(keys).enumerate() {
        let mut mk = mask(ctx, m, key, i);
        if !seen.insert(mk.id.0) {
            ctx.warn(
                Category::Other,
                format!("mask \"{}\" repeats an ID; a new one was derived", mk.name),
            );
            mk.id = MaskId::from_uuid(ids::derived("mask", &[rel_path, &i.to_string()]));
            seen.insert(mk.id.0);
        }
        let source = m
            .base_slide
            .as_ref()
            .and_then(|s| ids::parse_uuid(s.uuid.as_ref()));
        ctx.lookups
            .masks
            .entry(source.unwrap_or(mk.id.0))
            .or_insert_with(|| NamedId {
                id: mk.id,
                name: mk.name.clone(),
            });
        ctx.out.masks.push(mk);
    }

    // ---- video inputs (list order matters: liveVideoIndex indexes it) ----
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    let keys = ids::sort_keys(ws.video_inputs.len());
    for (i, (v, key)) in ws.video_inputs.iter().zip(keys).enumerate() {
        let mut vi = video_input(ctx, v, key, i);
        if !seen.insert(vi.id.0) {
            ctx.warn(
                Category::Other,
                format!(
                    "video input \"{}\" repeats an ID; a new one was derived",
                    vi.name
                ),
            );
            vi.id =
                VideoInputId::from_uuid(ids::derived("videoInput", &[rel_path, &i.to_string()]));
            seen.insert(vi.id.0);
        }
        ctx.lookups.video_inputs.push(NamedId {
            id: vi.id,
            name: vi.name.clone(),
        });
        if let Some(b) = input_binding(v, vi.id) {
            ctx.out.input_bindings.push(b);
        }
        ctx.out.video_inputs.push(vi);
    }

    // ---- audio inputs ----
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    let keys = ids::sort_keys(ws.audio_inputs.len());
    for (i, (a, key)) in ws.audio_inputs.iter().zip(keys).enumerate() {
        let mut ai = audio_input(ctx, a, key, i);
        if !seen.insert(ai.id.0) {
            ctx.warn(
                Category::Other,
                format!(
                    "audio input \"{}\" repeats an ID; a new one was derived",
                    ai.name
                ),
            );
            ai.id =
                AudioInputId::from_uuid(ids::derived("audioInput", &[rel_path, &i.to_string()]));
            seen.insert(ai.id.0);
        }
        ctx.lookups.audio_inputs.push(NamedId {
            id: ai.id,
            name: ai.name.clone(),
        });
        if let Some(b) = audio_input_binding(a, ai.id) {
            ctx.out.audio_input_bindings.push(b);
        }
        ctx.out.audio_inputs.push(ai);
    }

    // ---- looks + the live look ----
    import_looks(ctx, ws, rel_path);

    // ---- audio ----
    if let Some(setup) = ws.digital_audio_setup.as_ref() {
        match audio_device_config(setup) {
            Some(cfg) => ctx.out.audio_device_config = Some(cfg),
            None if setup.disable_main_output_device => ctx.warn(
                Category::NotImported,
                "the main audio output device is disabled; no audio device suggestion",
            ),
            None => {}
        }
        if setup.monitor_device.is_some() {
            ctx.count_unmapped("workspace.digitalAudioSetup.monitorDevice");
        }
        if setup.monitor_on_mains {
            ctx.count_unmapped("workspace.digitalAudioSetup.monitorOnMains");
        }
        if !setup.buses.is_empty() {
            ctx.count_unmapped("workspace.digitalAudioSetup.buses");
        }
        if setup.sdi_ndi_device.is_some() {
            // Only `enableSdiNdiDevice` is modelled (OutputBinding.embedAudio); its routing is not.
            ctx.count_unmapped("workspace.digitalAudioSetup.sdiNdiDevice");
        }
    }

    // ---- reported leftovers ----
    if !ws.selected_library_name.is_empty() {
        ctx.count_unmapped("workspace.selectedLibraryName");
    }
    if let Some(rec) = ws.record_settings.as_ref() {
        ctx.count_unmapped("workspace.recordSettings");
        ctx.warn(
            Category::NotImported,
            format!(
                "recording settings ({} stream(s), {} preset(s)) are not imported",
                rec.streams.len(),
                rec.presets.len()
            ),
        );
    }
    if ws.audio_settings.is_some() {
        ctx.count_unmapped("workspace.audioSettings");
    }
    if ws.audio_input_transition_time != 0.0 {
        ctx.count_unmapped("workspace.audioInputTransitionTime");
    }
    if ws.audio_channel_count != 0 {
        ctx.count_unmapped("workspace.audioChannelCount");
    }
    let cloud =
        ws.cloud_output_screens.len() + ws.cloud_video_inputs.len() + ws.cloud_audio_inputs.len();
    if cloud > 0 {
        ctx.count_unmapped("workspace.cloud");
        ctx.warn(
            Category::NotImported,
            format!("{cloud} cloud screen / input entries are not imported"),
        );
    }
}

/// Saved looks in list order, then the live look (see the module docs).
fn import_looks(ctx: &mut ImportContext, ws: &pp::ProPresenterWorkspace, rel_path: &str) {
    let live = ws.live_audience_look.as_ref();
    let original = live.and_then(|l| ids::parse_uuid(l.original_look_uuid.as_ref()));
    let has_original = original.is_some_and(|o| {
        ws.audience_looks
            .iter()
            .any(|l| ids::parse_uuid(l.uuid.as_ref()) == Some(o))
    });
    let extra_live = live.is_some() && !has_original;
    let keys = ids::sort_keys(ws.audience_looks.len() + usize::from(extra_live));
    let mut keys = keys.into_iter();
    let mut seen: BTreeSet<Uuid> = BTreeSet::new();
    let mut original_look: Option<Look> = None;

    for (i, l) in ws.audience_looks.iter().enumerate() {
        let key = keys.next().unwrap_or_default();
        let mut lk = look(ctx, l, key);
        if !seen.insert(lk.id.0) {
            ctx.warn(
                Category::Other,
                format!("look \"{}\" repeats an ID; a new one was derived", lk.name),
            );
            lk.id = LookId::from_uuid(ids::derived("look", &[rel_path, &i.to_string()]));
            seen.insert(lk.id.0);
        }
        let source = ids::parse_uuid(l.uuid.as_ref());
        ctx.lookups
            .looks
            .entry(source.unwrap_or(lk.id.0))
            .or_insert_with(|| NamedId {
                id: lk.id,
                name: lk.name.clone(),
            });
        if source.is_some() && source == original && original_look.is_none() {
            original_look = Some(lk.clone());
        }
        ctx.out.looks.push(lk);
    }

    let Some(live) = live else {
        return;
    };
    // Convert the live look (with a throwaway key) to compare it with its original.
    let mut live_look = look(ctx, live, SortKey::new());
    match original_look {
        Some(orig)
            if orig.screens == live_look.screens && orig.transition == live_look.transition =>
        {
            crate::convert::config::show_settings(ctx).startup_look = Some(orig.id);
        }
        Some(orig) => {
            // The live state was changed after it was copied: keep it as its own look, right
            // after its original (before the next saved look, never on its key).
            let next = ctx
                .out
                .looks
                .iter()
                .map(|l| l.sort_key.as_str())
                .filter(|k| *k > orig.sort_key.as_str())
                .min();
            live_look.sort_key = model::sort_key_between(Some(&orig.sort_key), next);
            if live_look.name == orig.name || live_look.name.is_empty() {
                live_look.name = format!("{} (live)", orig.name);
            }
            push_live_look(ctx, live_look, live, rel_path, &mut seen);
            ctx.warn(
                Category::Other,
                "the live look differs from its saved original; imported as its own look",
            );
        }
        None => {
            live_look.sort_key = keys.next().unwrap_or_default();
            if live_look.name.is_empty() {
                live_look.name = "Live".into();
            }
            push_live_look(ctx, live_look, live, rel_path, &mut seen);
        }
    }
}

fn push_live_look(
    ctx: &mut ImportContext,
    mut lk: Look,
    src: &pp::ProAudienceLook,
    rel_path: &str,
    seen: &mut BTreeSet<Uuid>,
) {
    if !seen.insert(lk.id.0) {
        lk.id = LookId::from_uuid(ids::derived("look", &[rel_path, "live"]));
        seen.insert(lk.id.0);
    }
    let source = ids::parse_uuid(src.uuid.as_ref());
    ctx.lookups
        .looks
        .entry(source.unwrap_or(lk.id.0))
        .or_insert_with(|| NamedId {
            id: lk.id,
            name: lk.name.clone(),
        });
    crate::convert::config::show_settings(ctx).startup_look = Some(lk.id);
    ctx.out.looks.push(lk);
}

/// One ProScreen → portable screen.
pub fn screen(
    ctx: &mut ImportContext,
    s: &pp::ProPresenterScreen,
    sort_key: SortKey,
    stage_mappings: &[pp::stage::ScreenAssignment],
) -> Screen {
    let path = ctx.current.path.clone().unwrap_or_default();
    let source = ids::parse_uuid(s.uuid.as_ref());
    let id =
        ScreenId::from_uuid(source.unwrap_or_else(|| ids::derived("screen", &[&path, &s.name])));
    let subs = sub_screens(s);

    let kind = if s.screen_type == pp::pro_presenter_screen::ScreenType::Stage as i32 {
        let layout = source.and_then(|u| {
            stage_mappings.iter().find_map(|m| {
                let screen = ids::parse_uuid(m.screen.as_ref()?.parameter_uuid.as_ref())?;
                (screen == u)
                    .then(|| ids::parse_uuid(m.layout.as_ref()?.parameter_uuid.as_ref()))
                    .flatten()
            })
        });
        ScreenKind::Stage {
            layout: layout.map(StageLayoutId::from_uuid),
        }
    } else {
        ScreenKind::Audience
    };

    let size = canvas_size(subs).unwrap_or(Size::HD);
    let frame_rate = screen_frame_rate(subs).unwrap_or_default();
    let alpha = subs
        .first()
        .map(|sub| alpha_mode(sub.alpha_settings.as_ref().map_or(0, |a| a.mode)))
        .unwrap_or_default();
    let background = if s.background_color_enabled {
        s.background_color
            .as_ref()
            .map(common::color)
            .unwrap_or(model::Color::TRANSPARENT)
    } else {
        model::Color::TRANSPARENT
    };

    let mut import = Some(ctx.document_info());
    screen_unmapped(ctx, s, &mut import);

    Screen {
        id,
        name: s.name.clone(),
        sort_key,
        kind,
        size,
        frame_rate,
        alpha,
        background,
        import,
        arrangement: arrangement(s, size),
        ..Screen::default()
    }
}

/// A combined arrangement → Grouped, an edge blend arrangement → Edge Blend (one row), with
/// the bezel / overlap from the first tile's bounds; tiles in sub-screen order. Single (and
/// anything that doesn't make a sane grid) → the default.
fn arrangement(s: &pp::ProPresenterScreen, size: Size) -> ScreenArrangement {
    use pp::pro_presenter_screen::Arrangement;
    let tile = |subs: &[pp::Screen]| {
        let r = common::rect(Some(subs.first()?.bounds.as_ref()?));
        (r.width > 0.0 && r.height > 0.0).then_some((r.width, r.height))
    };
    let a = match s.arrangement.as_ref() {
        Some(Arrangement::ArrangementCombined(c)) => {
            let (cols, rows) = (c.columns.max(1), c.rows.max(1));
            let Some((tw, th)) = tile(&c.screens) else {
                return ScreenArrangement::default();
            };
            let gap = |total: f64, n: u32, t: f64| {
                if n > 1 {
                    ((total - n as f64 * t) / (n - 1) as f64).max(0.0).round()
                } else {
                    0.0
                }
            };
            ScreenArrangement {
                kind: ArrangementKind::Grouped,
                columns: cols,
                rows,
                bezel: TileGap {
                    x: gap(size.width, cols, tw),
                    y: gap(size.height, rows, th),
                },
                ..ScreenArrangement::default()
            }
        }
        Some(Arrangement::ArrangementEdgeBlend(e)) => {
            let cols = if e.screen_count > 0 {
                e.screen_count
            } else {
                e.screens.len() as u32
            };
            let Some((tw, _)) = tile(&e.screens) else {
                return ScreenArrangement::default();
            };
            let overlap = if cols > 1 {
                ((cols as f64 * tw - size.width) / (cols - 1) as f64)
                    .max(0.0)
                    .round()
            } else {
                0.0
            };
            let curve = match e.edge_blends.first().map(|b| b.mode) {
                Some(m) if m == pp::edge_blend::Mode::Cubic as i32 => 3.0,
                Some(m) if m == pp::edge_blend::Mode::Linear as i32 => 1.0,
                _ => 2.0,
            };
            ScreenArrangement {
                kind: ArrangementKind::EdgeBlend,
                columns: cols.max(1),
                overlap: TileGap { x: overlap, y: 0.0 },
                blend: BlendCurve {
                    curve,
                    ..BlendCurve::default()
                },
                ..ScreenArrangement::default()
            }
        }
        _ => return ScreenArrangement::default(),
    };
    let (tw, th) = a.tile_size(size);
    let sane = (1..=8).contains(&a.columns)
        && (1..=8).contains(&a.rows)
        && tw >= 16.0
        && th >= 16.0
        && a.overlap.x < tw
        // The server's rule (F08-3): with three or more tiles across, at most half a tile.
        && (a.kind != ArrangementKind::EdgeBlend || a.columns < 3 || a.overlap.x <= tw / 2.0)
        && a.bezel.x <= 2000.0
        && a.bezel.y <= 2000.0;
    if sane {
        a
    } else {
        ScreenArrangement::default()
    }
}

/// The sub-screens (output tiles) of a ProScreen, whatever its arrangement.
fn sub_screens(s: &pp::ProPresenterScreen) -> &[pp::Screen] {
    use pp::pro_presenter_screen::Arrangement;
    match s.arrangement.as_ref() {
        Some(Arrangement::ArrangementSingle(a)) => &a.screens,
        Some(Arrangement::ArrangementCombined(a)) => &a.screens,
        Some(Arrangement::ArrangementEdgeBlend(a)) => &a.screens,
        None => &[],
    }
}

/// Canvas size: the union of the sub-screens' `bounds` (one tile = its bounds).
fn canvas_size(subs: &[pp::Screen]) -> Option<Size> {
    let rects: Vec<Rect> = subs
        .iter()
        .filter_map(|s| s.bounds.as_ref())
        .map(|b| common::rect(Some(b)))
        .filter(|r| r.width > 0.0 && r.height > 0.0 && r.width.is_finite() && r.height.is_finite())
        .collect();
    let first = rects.first()?;
    let (mut x0, mut y0) = (first.x, first.y);
    let (mut x1, mut y1) = (first.x + first.width, first.y + first.height);
    for r in &rects[1..] {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.x + r.width);
        y1 = y1.max(r.y + r.height);
    }
    Some(Size::new(x1 - x0, y1 - y0))
}

/// The screen's render clock: the first output display's mode refresh rate.
fn screen_frame_rate(subs: &[pp::Screen]) -> Option<FrameRate> {
    subs.iter().find_map(|s| {
        let mode = s.output_display.as_ref()?.mode.as_ref()?;
        frame_rate(mode.refresh_rate)
    })
}

/// A refresh rate in Hz as an exact rational: integers → `n/1`, NTSC rates (29.97, 59.94, 23.976)
/// → `n·1000/1001`, anything else → millihertz reduced. `None` for non-positive / non-finite.
pub fn frame_rate(hz: f64) -> Option<FrameRate> {
    if !hz.is_finite() || hz <= 0.0 || hz > 10_000.0 {
        return None;
    }
    let r = hz.round();
    if (hz - r).abs() < 0.005 {
        return Some(FrameRate::fps(r as u32));
    }
    let n = (hz * 1.001).round();
    if n > 0.0 && (hz - n * 1000.0 / 1001.0).abs() < 0.005 {
        return Some(FrameRate {
            num: n as u32 * 1000,
            den: 1001,
        });
    }
    let mut num = (hz * 1000.0).round() as u32;
    let mut den = 1000u32;
    let g = gcd(num, den);
    num /= g;
    den /= g;
    Some(FrameRate { num, den })
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// `Screen.AlphaSettings.Mode` → portable alpha intent.
fn alpha_mode(mode: i32) -> AlphaMode {
    use pp::screen::alpha_settings::Mode;
    match Mode::try_from(mode) {
        Ok(Mode::Straight) => AlphaMode::Straight,
        Ok(Mode::Premultiplied) => AlphaMode::Premultiplied,
        _ => AlphaMode::Opaque,
    }
}

fn output_alpha(mode: AlphaMode) -> OutputAlpha {
    match mode {
        AlphaMode::Straight => OutputAlpha::Straight,
        AlphaMode::Premultiplied => OutputAlpha::Premultiplied,
        AlphaMode::Opaque | AlphaMode::Unknown => OutputAlpha::Opaque,
    }
}

/// Keeps the portable parts of the arrangement that the model has no slot for.
fn screen_unmapped(
    ctx: &mut ImportContext,
    s: &pp::ProPresenterScreen,
    info: &mut Option<ImportInfo>,
) {
    use pp::pro_presenter_screen::Arrangement;
    match s.arrangement.as_ref() {
        Some(Arrangement::ArrangementCombined(a)) => {
            let mut a = a.clone();
            a.screens.iter_mut().for_each(strip_machine_local);
            let v = ctx.to_json(&a);
            ctx.keep_unmapped(info, "screen.arrangementCombined", v);
        }
        Some(Arrangement::ArrangementEdgeBlend(a)) => {
            let mut a = a.clone();
            a.screens.iter_mut().for_each(strip_machine_local);
            let v = ctx.to_json(&a);
            ctx.keep_unmapped(info, "screen.arrangementEdgeBlend", v);
        }
        Some(Arrangement::ArrangementSingle(a)) => {
            let residues: Vec<pp::Screen> = a.screens.iter().map(portable_residue).collect();
            if residues.iter().any(|r| *r != pp::Screen::default()) {
                let v = Value::Array(residues.iter().map(|r| ctx.to_json(r)).collect());
                ctx.keep_unmapped(info, "screen.subscreens", v);
            }
        }
        None => {}
    }
    if let Some(pp::pro_presenter_screen::RenderingEnabledOptional::RenderingEnabled(false)) =
        s.rendering_enabled_optional
    {
        ctx.keep_unmapped(info, "screen.renderingEnabled", Value::Bool(false));
    }
}

/// Drops the machine-local output binding of a sub-screen (kept as a suggestion instead).
fn strip_machine_local(s: &mut pp::Screen) {
    s.output_display = None;
    if let Some(a) = s.alpha_settings.as_mut()
        && let Some(d) = a.alpha_device.as_mut()
    {
        d.display = None;
    }
}

/// A sub-screen minus everything the model represents (canvas, crop, alpha, binding) and minus
/// "off" defaults (zero corner pin, empty colour adjustments). Default = nothing to keep.
fn portable_residue(s: &pp::Screen) -> pp::Screen {
    let mut r = s.clone();
    r.uuid = None;
    r.name.clear();
    r.bounds = None;
    r.subscreen_unit_rect = None;
    r.output_display = None;
    r.alpha_settings = None;
    if r.corner_values.as_ref().is_some_and(corners_zero) && !r.corner_pinning_enabled {
        r.corner_values = None;
    }
    if r.color_adjustment == Some(Default::default()) {
        r.color_adjustment = None;
    }
    if r.blend_compensation == Some(Default::default()) {
        r.blend_compensation = None;
    }
    r
}

fn corners_zero(c: &pp::CornerValues) -> bool {
    [&c.top_left, &c.top_right, &c.bottom_left, &c.bottom_right]
        .iter()
        .all(|p| p.as_ref().is_none_or(|p| p.x == 0.0 && p.y == 0.0))
}

/// Output-binding suggestions of one ProScreen (machine-local).
pub fn output_bindings(
    ctx: &mut ImportContext,
    s: &pp::ProPresenterScreen,
    screen: ScreenId,
    embed_audio: bool,
) -> Vec<OutputBinding> {
    let subs = sub_screens(s);
    let canvas = canvas_size(subs).unwrap_or(Size::HD);
    let screen_rate = screen_frame_rate(subs).unwrap_or_default();
    let screen_alpha = subs
        .first()
        .map(|sub| alpha_mode(sub.alpha_settings.as_ref().map_or(0, |a| a.mode)))
        .unwrap_or_default();
    let enabled = !matches!(
        s.rendering_enabled_optional,
        Some(pp::pro_presenter_screen::RenderingEnabledOptional::RenderingEnabled(false))
    );
    let several = subs.iter().filter(|x| x.output_display.is_some()).count() > 1;

    let tiled = matches!(
        s.arrangement,
        Some(
            pp::pro_presenter_screen::Arrangement::ArrangementCombined(_)
                | pp::pro_presenter_screen::Arrangement::ArrangementEdgeBlend(_)
        )
    ) && arrangement(s, canvas).is_tiled();
    let cells = arrangement(s, canvas).cells() as usize;
    let tile = |index: usize| (tiled && index < cells).then_some(index as u32);
    // A wall tile is cropped by its tile alone: a region would win over it on the server, and
    // Oriel's tile and arrangement edits (which never show or clear a region) would then move
    // only the blend ramps, not the picture (F09-1). Outputs without a tile keep their region.
    let crop = |sub: &pp::Screen, index: usize| {
        tile(index)
            .is_none()
            .then(|| region(sub.subscreen_unit_rect.as_ref(), canvas))
            .flatten()
    };
    let mut out = Vec::new();
    for (index, sub) in subs.iter().enumerate() {
        let Some(od) = sub.output_display.as_ref() else {
            // A wall tile without hardware: a placeholder keeps its slot.
            if tiled {
                out.push(OutputBinding {
                    id: ids::output_id(screen, index),
                    name: if sub.name.is_empty() {
                        format!("{} {}", s.name, index + 1)
                    } else {
                        sub.name.clone()
                    },
                    screen,
                    enabled,
                    transport: OutputTransport::Placeholder,
                    region: crop(sub, index),
                    tile: tile(index),
                    ..OutputBinding::default()
                });
            }
            continue;
        };
        use pp::output_display::Type;
        let device = first_non_empty(&[&od.device_name, &od.name, &od.serial]);
        let transport = match Type::try_from(od.r#type) {
            // ProPresenter's NDI outputs become IDN Open Protocol sources (compatible with
            // NDI®) under the same name.
            Ok(Type::Ndi) => OutputTransport::Idn {
                source_name: device.clone(),
                groups: Vec::new(),
            },
            Ok(Type::Screen) => OutputTransport::Display {
                selector: DisplaySelector {
                    connector: non_empty(&od.device_name),
                    name: non_empty(&od.model).or_else(|| non_empty(&od.name)),
                    index: None,
                },
                fullscreen: true,
                mode: None,
            },
            Ok(Type::Card) => OutputTransport::Sdi {
                device: device.clone(),
                port: 0,
            },
            other => {
                let t = other.map(|t| t.as_str_name()).unwrap_or("TYPE_UNKNOWN");
                ctx.warn(
                    Category::NotImported,
                    format!(
                        "screen \"{}\": output type {t} has no equivalent; the output binding \
                         suggestion has no transport",
                        s.name
                    ),
                );
                OutputTransport::Unknown
            }
        };
        let carries_audio = matches!(
            transport,
            OutputTransport::Idn { .. } | OutputTransport::Sdi { .. }
        );

        // Alpha: a separate key device → key/fill; a tile whose mode differs from the screen's.
        let sub_alpha = alpha_mode(sub.alpha_settings.as_ref().map_or(0, |a| a.mode));
        let stored_key_display = sub
            .alpha_settings
            .as_ref()
            .and_then(|a| a.alpha_device.as_ref())
            .and_then(|d| d.display.as_ref());
        // A key device stays stored when the tile's alpha is switched off: it drives nothing
        // then (E07-1), so no key output (it would show white on a monitor now used for
        // something else).
        let key_display = stored_key_display.filter(|_| sub_alpha != AlphaMode::Opaque);
        if key_display.is_none() && stored_key_display.is_some() {
            ctx.count_unmapped("alphaSettings.alphaDevice.disabled");
        }
        let key_device =
            key_display.map(|d| first_non_empty(&[&d.device_name, &d.name, &d.serial]));
        // A display or network output with a key device: a Fill/Key pair of two outputs, the
        // key on a device of the same kind.
        let key_transport = key_display.and_then(|d| match &transport {
            OutputTransport::Display { .. } => Some(OutputTransport::Display {
                selector: DisplaySelector {
                    connector: non_empty(&d.device_name),
                    name: non_empty(&d.model).or_else(|| non_empty(&d.name)),
                    index: None,
                },
                fullscreen: true,
                mode: None,
            }),
            OutputTransport::Idn { .. } => {
                non_empty(&first_non_empty(&[&d.device_name, &d.name, &d.serial])).map(
                    |source_name| OutputTransport::Idn {
                        source_name,
                        groups: Vec::new(),
                    },
                )
            }
            _ => None,
        });
        let external_key = matches!(transport, OutputTransport::Sdi { .. })
            && matches!(
                od.hardware_options,
                Some(pp::output_display::HardwareOptions::Blackmagic(ref b))
                    if b.enabled && b.key_mode == pp::output_display::blackmagic::KeyMode::External as i32
            );
        let alpha_override = if key_transport.is_some() {
            // The fill follows the tile's alpha type (straight: unshaped, premultiplied: shaped).
            (sub_alpha != screen_alpha).then(|| output_alpha(sub_alpha))
        } else if key_device.is_some() || external_key {
            Some(OutputAlpha::KeyFill {
                fill_device: non_empty(&device),
                key_device: key_device.and_then(|k| non_empty(&k)),
            })
        } else if sub_alpha != screen_alpha {
            Some(output_alpha(sub_alpha))
        } else {
            None
        };

        let mode = od.mode.as_ref();
        let frame_rate_override = mode
            .and_then(|m| frame_rate(m.refresh_rate))
            .filter(|r| *r != screen_rate);
        let resolution_override = mode
            .filter(|m| m.width > 0 && m.height > 0)
            .map(|m| Resolution {
                width: m.width,
                height: m.height,
            })
            .filter(|r| f64::from(r.width) != canvas.width || f64::from(r.height) != canvas.height);
        if mode.is_some_and(|m| m.interlaced) {
            ctx.count_unmapped("outputDisplay.mode.interlaced");
        }
        if od.video_delay != 0 {
            ctx.count_unmapped("outputDisplay.videoDelay");
        }
        if od.render_format != 0 || od.hdr_max_nits_offset != 0 {
            ctx.count_unmapped("outputDisplay.renderFormat");
        }

        let name = if !sub.name.is_empty() {
            sub.name.clone()
        } else if several {
            format!("{} {}", s.name, index + 1)
        } else {
            s.name.clone()
        };

        let fill = OutputBinding {
            id: ids::output_id(screen, index),
            name,
            screen,
            enabled,
            transport,
            alpha_override,
            frame_rate_override,
            resolution_override,
            region: crop(sub, index),
            embed_audio: embed_audio && carries_audio,
            tile: tile(index),
            ..OutputBinding::default()
        };
        match key_transport {
            Some(key_transport) => {
                let key = OutputBinding {
                    id: ids::key_output_id(screen, index),
                    name: format!("{} key", fill.name),
                    transport: key_transport,
                    alpha_override: None,
                    embed_audio: false,
                    target: OutputTarget::Key,
                    ..fill.clone()
                };
                out.push(OutputBinding {
                    target: OutputTarget::Fill,
                    ..fill
                });
                out.push(key);
            }
            None => out.push(fill),
        }
    }
    out
}

/// `subscreenUnitRect` (0..1 of the canvas) → canvas px; `None` when missing or the full canvas.
fn region(unit: Option<&pp::graphics::Rect>, canvas: Size) -> Option<Rect> {
    let u = common::rect(Some(unit?));
    let full = (u.x.abs() < 1e-9)
        && (u.y.abs() < 1e-9)
        && ((u.width - 1.0).abs() < 1e-9)
        && ((u.height - 1.0).abs() < 1e-9);
    if full || u.width <= 0.0 || u.height <= 0.0 {
        return None;
    }
    Some(Rect::new(
        u.x * canvas.width,
        u.y * canvas.height,
        u.width * canvas.width,
        u.height * canvas.height,
    ))
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn first_non_empty(c: &[&str]) -> String {
    c.iter().find_map(|s| non_empty(s)).unwrap_or_default()
}

/// One audience look.
pub fn look(ctx: &mut ImportContext, l: &pp::ProAudienceLook, sort_key: SortKey) -> Look {
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = LookId::from_uuid(ids::source_or_derived(
        l.uuid.as_ref(),
        "look",
        &[&path, &l.name],
    ));
    let mut screens = Vec::new();
    for sl in &l.screen_looks {
        if ids::parse_uuid(sl.pro_screen_uuid.as_ref()).is_none() {
            ctx.warn(
                Category::Other,
                format!(
                    "look \"{}\": a screen entry without a screen UUID was skipped",
                    l.name
                ),
            );
            continue;
        }
        screens.push(screen_look(ctx, sl));
    }
    let transition = (l.transition_duration.is_finite() && l.transition_duration > 0.0)
        .then(|| Transition::dissolve(l.transition_duration));
    Look {
        id,
        name: l.name.clone(),
        sort_key,
        screens,
        transition,
        import: Some(ctx.document_info()),
        ..Look::default()
    }
}

/// One per-screen look entry.
pub fn screen_look(
    ctx: &mut ImportContext,
    sl: &pp::pro_audience_look::ProScreenLook,
) -> ScreenLook {
    let screen_uuid = ids::parse_uuid(sl.pro_screen_uuid.as_ref()).unwrap_or_default();
    let screen = match ctx.lookups.screens.get(&screen_uuid) {
        Some(s) => s.id,
        None => {
            ctx.warn(
                Category::Other,
                format!("a look references screen {screen_uuid}, which is not in the workspace"),
            );
            ScreenId::from_uuid(screen_uuid)
        }
    };
    let mask = ids::parse_uuid(sl.mask_uuid.as_ref()).map(|u| match ctx.lookups.masks.get(&u) {
        Some(m) => m.id,
        None => {
            ctx.warn(
                Category::Other,
                format!("a look references mask {u}, which is not in the workspace"),
            );
            MaskId::from_uuid(u)
        }
    });
    let template_override = template_override(ctx, sl);
    ScreenLook {
        screen,
        layers: layer_visibility(sl),
        template_override,
        mask,
        language: Default::default(),
    }
}

/// The provisional template override of a per-screen look entry (theme link fixed later by
/// [`resolve_theme_links`]).
fn template_override(
    ctx: &mut ImportContext,
    sl: &pp::pro_audience_look::ProScreenLook,
) -> Option<TemplateOverride> {
    let folder = sl
        .template_document_file_path
        .as_ref()
        .and_then(theme_folder_from_url);
    let slide = ids::parse_uuid(sl.template_slide_uuid.as_ref());
    let Some(slide) = slide else {
        if folder.is_some() {
            ctx.warn(
                Category::Other,
                "a look names a template document without a template slide; no override imported",
            );
            ctx.count_unmapped("look.templateDocumentFilePath");
        }
        return None;
    };
    let theme = match folder {
        Some(f) => {
            let name = f.rsplit('/').next().unwrap_or(&f).to_string();
            Link::named(ids::theme_id(&f), name)
        }
        None => Link::to(ids::theme_id("")),
    };
    Some(TemplateOverride {
        theme,
        slide: TemplateSlideId::from_uuid(slide),
        mapping: Default::default(),
        keep_inline_formatting: true,
        hide_when_empty: true,
    })
}

/// The theme folder (relative to the backup root) of a template document URL
/// (`Themes/Key Feed/Theme` → `Themes/Key Feed`). `None` for an empty URL.
pub fn theme_folder_from_url(u: &pp::Url) -> Option<String> {
    use pp::url::{RelativeFilePath, Storage};
    let mut candidates: Vec<String> = Vec::new();
    if let Some(RelativeFilePath::Local(l)) = u.relative_file_path.as_ref()
        && !l.path.trim().is_empty()
    {
        candidates.push(l.path.clone());
    }
    if let Some(RelativeFilePath::External(e)) = u.relative_file_path.as_ref()
        && !e.path.trim().is_empty()
    {
        candidates.push(e.path.clone());
    }
    match u.storage.as_ref() {
        Some(Storage::AbsoluteString(s)) | Some(Storage::RelativePath(s))
            if !s.trim().is_empty() =>
        {
            candidates.push(percent_decode(s))
        }
        _ => {}
    }
    for c in candidates {
        let p = crate::source::normalize_rel(&c);
        // Keep the part from the "Themes" segment on (absolute paths of the source machine).
        let segs: Vec<&str> = p.split('/').collect();
        let start = segs
            .iter()
            .rposition(|s| s.eq_ignore_ascii_case(crate::source::THEMES))
            .unwrap_or(0);
        let mut rel = segs[start..].join("/");
        if let Some(stripped) = rel.strip_suffix(&format!("/{}", crate::source::THEME_FILE)) {
            rel = stripped.to_string();
        }
        if !rel.is_empty() {
            return Some(rel);
        }
    }
    None
}

/// Minimal `%XX` decoding for `file://` URLs (invalid escapes are kept verbatim).
fn percent_decode(s: &str) -> String {
    crate::source::percent_decode(s.strip_prefix("file://").unwrap_or(s))
}

/// Layer flags of a per-screen look entry.
pub fn layer_visibility(sl: &pp::pro_audience_look::ProScreenLook) -> LayerVisibility {
    LayerVisibility {
        video_input: sl.live_video_enabled,
        media: sl.presentation_background_enabled,
        slide: sl.presentation_foreground_enabled,
        announcements: sl.announcements_enabled,
        props: sl.props_enabled || sl.props_layer_enabled,
        messages: sl.messages_layer_enabled,
        // Vesper's own layer, not in the imported look: it follows the messages layer (a
        // key feed without messages has no captions either).
        captions: sl.messages_layer_enabled,
    }
}

/// One mask.
pub fn mask(ctx: &mut ImportContext, m: &pp::ProMask, sort_key: SortKey, index: usize) -> Mask {
    let path = ctx.current.path.clone().unwrap_or_default();
    let base = m.base_slide.clone().unwrap_or_default();
    let uuid = ids::source_or_derived(base.uuid.as_ref(), "mask", &[&path, &index.to_string()]);
    let size = common::size(base.size.as_ref()).unwrap_or(Size::HD);
    let body = slide::slide_from_pb(ctx, &base, SlideId::from_uuid(uuid));
    Mask {
        id: MaskId::from_uuid(uuid),
        name: m.name.clone(),
        sort_key,
        size,
        slide: body,
        ..Mask::default()
    }
}

/// One logical video input.
pub fn video_input(
    ctx: &mut ImportContext,
    v: &pp::VideoInput,
    sort_key: SortKey,
    index: usize,
) -> VideoInput {
    use pp::video_input::{AltAudioSource, AudioDeviceType};
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = VideoInputId::from_uuid(ids::source_or_derived(
        v.uuid.as_ref(),
        "videoInput",
        &[&path, &index.to_string()],
    ));
    let device = v.video_input_device.as_ref();
    let name = non_empty(&v.user_description)
        .or_else(|| device.and_then(|d| non_empty(&d.name)))
        .unwrap_or_else(|| format!("Input {}", index + 1));

    let mut import = Some(ctx.document_info());
    let audio = match AudioDeviceType::try_from(v.audio_type) {
        Ok(AudioDeviceType::None) => AudioSettings {
            muted: true,
            ..audio_routing(device.and_then(|d| d.audio_routing.as_ref()))
        },
        Ok(AudioDeviceType::Alternate) => {
            // Audio from another device: its routing is kept; the device itself is
            // machine-local and the model binds one source per input.
            let routing = match v.alt_audio_source.as_ref() {
                Some(AltAudioSource::AudioDevice(d)) => d.routing.as_ref(),
                Some(AltAudioSource::VideoDevice(d)) => d.audio_routing.as_ref(),
                None => None,
            };
            if let Some(alt) = v.alt_audio_source.as_ref() {
                let value = match alt {
                    AltAudioSource::AudioDevice(d) => serde_json::json!({
                        "audioDevice": { "name": d.name }
                    }),
                    AltAudioSource::VideoDevice(d) => serde_json::json!({
                        "videoDevice": { "name": d.name }
                    }),
                };
                ctx.keep_unmapped(&mut import, "videoInput.altAudioSource", value);
            }
            audio_routing(routing)
        }
        _ => audio_routing(device.and_then(|d| d.audio_routing.as_ref())),
    };
    let alpha = match pp::AlphaType::try_from(v.alpha_type) {
        Ok(pp::AlphaType::Straight) => SourceAlpha::Straight,
        Ok(pp::AlphaType::Premultiplied) => SourceAlpha::Premultiplied,
        _ => SourceAlpha::Auto,
    };
    let thumbnail = v
        .thumbnail_path
        .as_ref()
        .filter(|u| !url_is_empty(u))
        .map(|u| {
            let scope = ctx.current.asset_scope.clone();
            media::asset_for_url(ctx, u, MediaKind::Image, &scope)
        });
    VideoInput {
        id,
        name,
        sort_key,
        color: common::visible_color(v.display_color.as_ref()),
        alpha,
        audio,
        thumbnail,
        import,
        ..VideoInput::default()
    }
}

/// True when a URL names nothing (`{}` or only a platform).
pub fn url_is_empty(u: &pp::Url) -> bool {
    use pp::url::{RelativeFilePath, Storage};
    let storage_empty = match u.storage.as_ref() {
        Some(Storage::AbsoluteString(s)) | Some(Storage::RelativePath(s)) => s.trim().is_empty(),
        None => true,
    };
    let rel_empty = match u.relative_file_path.as_ref() {
        Some(RelativeFilePath::Local(l)) => l.path.trim().is_empty(),
        Some(RelativeFilePath::External(e)) => e.path.trim().is_empty(),
        None => true,
    };
    storage_empty && rel_empty
}

/// The binding suggestion of a video input (`None` without a device).
pub fn input_binding(v: &pp::VideoInput, input: model::VideoInputId) -> Option<InputBinding> {
    let device = v.video_input_device.as_ref()?;
    if device.name.trim().is_empty() && device.unique_id.trim().is_empty() {
        return None;
    }
    Some(InputBinding {
        input,
        source: input_source(device),
        ..InputBinding::default()
    })
}

/// One logical audio input.
pub fn audio_input(
    ctx: &mut ImportContext,
    a: &pp::AudioInput,
    sort_key: SortKey,
    index: usize,
) -> AudioInput {
    use pp::audio_input::Source;
    use pp::audio_input::behavior_mode::Mode;
    let path = ctx.current.path.clone().unwrap_or_default();
    let id = AudioInputId::from_uuid(ids::source_or_derived(
        a.uuid.as_ref(),
        "audioInput",
        &[&path, &index.to_string()],
    ));
    let source_name = match a.source.as_ref() {
        Some(Source::AudioDevice(d)) => non_empty(&d.name),
        Some(Source::VideoDevice(d)) => non_empty(&d.name),
        None => None,
    };
    let name = non_empty(&a.user_description)
        .or(source_name)
        .unwrap_or_else(|| format!("Audio Input {}", index + 1));
    let routing = match a.source.as_ref() {
        Some(Source::AudioDevice(d)) => d.routing.as_ref(),
        Some(Source::VideoDevice(d)) => d.audio_routing.as_ref(),
        None => None,
    };
    let mut linked_video_inputs = Vec::new();
    let mode = match a.behavior_mode.as_ref().and_then(|b| b.mode.as_ref()) {
        Some(Mode::On(_)) => AudioInputMode::On,
        Some(Mode::Off(_)) | None => AudioInputMode::Off,
        Some(Mode::AutoOff(_)) => AudioInputMode::AutoOff,
        Some(Mode::AutoOn(on)) => {
            for &i in &on.linked_video_inputs {
                match ctx.lookups.video_inputs.get(i as usize) {
                    Some(v) => linked_video_inputs.push(v.id),
                    None => ctx.warn(
                        Category::Other,
                        format!(
                            "audio input \"{name}\" links video input #{i}, which does not exist"
                        ),
                    ),
                }
            }
            AudioInputMode::AutoOn
        }
    };
    AudioInput {
        id,
        name,
        sort_key,
        mode,
        linked_video_inputs,
        audio: audio_routing(routing),
        import: Some(ctx.document_info()),
        ..AudioInput::default()
    }
}

/// The binding suggestion of an audio input (`None` without a source).
pub fn audio_input_binding(
    a: &pp::AudioInput,
    input: model::AudioInputId,
) -> Option<AudioInputBinding> {
    use pp::audio_input::Source;
    let source = match a.source.as_ref()? {
        Source::VideoDevice(d) => {
            if d.name.trim().is_empty() && d.unique_id.trim().is_empty() {
                return None;
            }
            input_source(d)
        }
        Source::AudioDevice(d) => {
            let name = first_non_empty(&[&d.name, &d.render_id]);
            if name.is_empty() {
                return None;
            }
            InputSource::Device { name }
        }
    };
    Some(AudioInputBinding {
        input,
        source,
        ..AudioInputBinding::default()
    })
}

/// A capture device → input source (`"HOST (name)"` names and `TYPE_NDI` → IDN, else device).
pub fn input_source(device: &pp::media::VideoDevice) -> InputSource {
    let name = first_non_empty(&[&device.name, &device.unique_id]);
    let pp_ndi = device.r#type == pp::media::video_device::Type::Ndi as i32;
    if pp_ndi || looks_like_network_source(&name) {
        InputSource::Idn { source_name: name }
    } else {
        InputSource::Device { name }
    }
}

/// True when a device name follows the network source `HOST (source)` convention: a host part without
/// whitespace (machine names cannot contain spaces), a space, and a non-empty source name in
/// parentheses at the end (`"STUDIO-PC (Program-Out)"`). Camera names such as
/// `"HD Pro Webcam C920 (046d:082d)"` have spaces before the parenthesis and do not match.
pub fn looks_like_network_source(name: &str) -> bool {
    let name = name.trim();
    let Some(inner) = name.strip_suffix(')') else {
        return false;
    };
    let Some(open) = inner.find(" (") else {
        return false;
    };
    let host = &inner[..open];
    let source = &inner[open + 2..];
    !host.is_empty()
        && !host.chars().any(char::is_whitespace)
        && !host.contains(['(', ')'])
        && !source.trim().is_empty()
        && !source.contains(['(', ')'])
}

/// A digital-audio routing → level and channel routing.
///
/// `masterChannel.level` → `volumeDb`; every `map` entry `channelIndex → mappedIndices[k]`
/// becomes one [`ChannelRoute`] carrying that source channel's `level` (dB) and `muteEnable`.
/// Without a map, routes are only written (identity) when some channel is muted or has a level,
/// so a plain pass-through stays the compact default. Channel solo / tone / delay have no model
/// slot (never used in practice) and are not carried.
pub fn audio_routing(r: Option<&pp::digital_audio::device::Routing>) -> AudioSettings {
    let Some(r) = r else {
        return AudioSettings::default();
    };
    let level = |c: Option<&pp::digital_audio::device::Channel>| {
        c.map(|c| c.level).filter(|l| l.is_finite()).unwrap_or(0.0)
    };
    let volume_db = level(r.master_channel.as_ref());
    let mut channels = Vec::new();
    if r.map.is_empty() {
        if r.channels.iter().any(|c| c.mute_enable || c.level != 0.0) {
            for (i, c) in r.channels.iter().enumerate() {
                channels.push(ChannelRoute {
                    source: i as u32,
                    dest: i as u32,
                    gain_db: level(Some(c)),
                    muted: c.mute_enable,
                });
            }
        }
    } else {
        for m in &r.map {
            let c = r.channels.get(m.channel_index as usize);
            for &dest in &m.mapped_indices {
                channels.push(ChannelRoute {
                    source: m.channel_index,
                    dest,
                    gain_db: level(c),
                    muted: c.is_some_and(|c| c.mute_enable),
                });
            }
        }
    }
    AudioSettings {
        muted: false,
        volume_db,
        channels,
    }
}

/// The main output device of the digital audio setup → machine-local suggestion. `None` when
/// there is no main device or it is disabled.
pub fn audio_device_config(setup: &pp::digital_audio::Setup) -> Option<AudioDeviceConfig> {
    if setup.disable_main_output_device {
        return None;
    }
    let d = setup.main_output_device.as_ref()?;
    let device = first_non_empty(&[&d.name, &d.render_id]);
    let routing = d.routing.as_ref();
    if device.is_empty() && routing.is_none() {
        return None;
    }
    let mut channel_map: Vec<u32> = Vec::new();
    if let Some(r) = routing {
        let len = r
            .map
            .iter()
            .filter(|m| !m.mapped_indices.is_empty())
            .map(|m| m.channel_index as usize + 1)
            .max()
            .unwrap_or(0)
            .min(1024);
        channel_map = (0..len as u32).collect();
        for m in &r.map {
            if let (Some(slot), Some(&dev)) = (
                channel_map.get_mut(m.channel_index as usize),
                m.mapped_indices.first(),
            ) {
                *slot = dev;
            }
        }
        if channel_map.iter().enumerate().all(|(i, &d)| i as u32 == d) {
            channel_map.clear();
        }
    }
    let master_gain_db = routing
        .and_then(|r| r.master_channel.as_ref())
        .map(|c| c.level)
        .filter(|l| l.is_finite())
        .unwrap_or(0.0);
    Some(AudioDeviceConfig {
        device,
        channel_map,
        master_gain_db,
        ..AudioDeviceConfig::default()
    })
}

/// Post-pass after themes: fixes look template overrides and message templates (see the module
/// docs). Also warns about stage screens whose layout does not exist.
pub fn resolve_theme_links(ctx: &mut ImportContext) {
    let mut warnings: Vec<String> = Vec::new();
    {
        let lookups = &ctx.lookups;
        for look in &mut ctx.out.looks {
            for sl in &mut look.screens {
                let Some(o) = sl.template_override.as_mut() else {
                    continue;
                };
                match resolve(lookups, &o.theme, o.slide) {
                    Some((theme, slide)) => {
                        o.theme = theme;
                        o.slide = slide;
                    }
                    None => warnings.push(format!(
                        "look \"{}\": template slide {} of theme \"{}\" was not found; the \
                         override is kept as a dangling link",
                        look.name,
                        o.slide,
                        o.theme.name.as_deref().unwrap_or("?")
                    )),
                }
            }
        }
        for m in &mut ctx.out.messages {
            let Some(t) = m.template.as_mut() else {
                continue;
            };
            match resolve(lookups, &t.theme, t.slide) {
                Some((theme, slide)) => {
                    t.theme = theme;
                    t.slide = slide;
                }
                None => warnings.push(format!(
                    "message \"{}\": template slide {} of theme \"{}\" was not found; kept as a \
                     dangling link",
                    m.name,
                    t.slide,
                    t.theme.name.as_deref().unwrap_or("?")
                )),
            }
        }
        for s in &ctx.out.screens {
            if let ScreenKind::Stage {
                layout: Some(layout),
            } = &s.kind
                && !lookups.stage_layouts.values().any(|l| l.id == *layout)
            {
                warnings.push(format!(
                    "stage screen \"{}\" shows stage layout {layout}, which does not exist",
                    s.name
                ));
            }
        }
    }
    for w in warnings {
        ctx.warn(Category::Other, w);
    }
}

/// Resolves a provisional (theme link, template slide) pair to the kept theme:
/// 1. the theme whose folder the provisional ID was derived from, if it holds the slide;
/// 2. else the template-slide UUID lookup (any kept theme);
/// 3. else a theme with the hinted name that holds the slide.
fn resolve(
    lookups: &crate::context::Lookups,
    theme: &Link<ThemeId>,
    slide: TemplateSlideId,
) -> Option<(Link<ThemeId>, TemplateSlideId)> {
    let named = |id: ThemeId| {
        let name = lookups
            .themes
            .values()
            .find(|t| t.id == id)
            .map(|t| t.name.clone());
        match name {
            Some(n) => Link::named(id, n),
            None => Link::to(id),
        }
    };
    if let Some((_, entry)) = lookups
        .themes
        .iter()
        .find(|(folder, _)| ids::theme_id(folder) == theme.id)
        && entry.slides.contains(&slide)
    {
        return Some((Link::named(entry.id, entry.name.clone()), slide));
    }
    if let Some(&(theme_id, slide_id)) = lookups.template_slides.get(&slide.0) {
        return Some((named(theme_id), slide_id));
    }
    let hint = theme.name.as_deref()?;
    lookups
        .themes
        .values()
        .find(|t| t.name == hint && t.slides.contains(&slide))
        .map(|t| (Link::named(t.id, t.name.clone()), slide))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ThemeEntry;
    use crate::{ImportContext, ImportOptions};
    use std::path::Path;

    fn ctx() -> ImportContext {
        let mut c = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        c.current.path = Some("Configuration/Workspace".into());
        c
    }

    fn uuid(s: &str) -> Option<pp::Uuid> {
        Some(pp::Uuid { string: s.into() })
    }

    const SCREEN_A: &str = "11111111-1111-4111-8111-111111111111";
    const SCREEN_B: &str = "22222222-2222-4222-8222-222222222222";
    const LOOK: &str = "33333333-3333-4333-8333-333333333333";
    const LIVE: &str = "44444444-4444-4444-8444-444444444444";
    const TSLIDE: &str = "55555555-5555-4555-8555-555555555555";

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Option<pp::graphics::Rect> {
        Some(pp::graphics::Rect {
            origin: Some(pp::graphics::Point { x, y }),
            size: Some(pp::graphics::Size {
                width: w,
                height: h,
            }),
        })
    }

    fn sub(kind: pp::output_display::Type, name: &str, hz: f64, alpha: i32) -> pp::Screen {
        pp::Screen {
            uuid: uuid("66666666-6666-4666-8666-666666666666"),
            bounds: rect(0.0, 0.0, 1920.0, 1080.0),
            subscreen_unit_rect: rect(0.0, 0.0, 1.0, 1.0),
            corner_values: Some(pp::CornerValues {
                top_left: Some(Default::default()),
                top_right: Some(Default::default()),
                bottom_left: Some(Default::default()),
                bottom_right: Some(Default::default()),
            }),
            color_adjustment: Some(Default::default()),
            blend_compensation: Some(Default::default()),
            output_display: Some(pp::OutputDisplay {
                name: name.into(),
                device_name: name.into(),
                r#type: kind as i32,
                mode: Some(pp::DisplayMode {
                    name: "mode".into(),
                    width: 1920,
                    height: 1080,
                    refresh_rate: hz,
                    interlaced: false,
                }),
                render_id: "ndi:00000000".into(),
                ..Default::default()
            }),
            alpha_settings: Some(pp::screen::AlphaSettings {
                mode: alpha,
                alpha_device: None,
            }),
            ..Default::default()
        }
    }

    fn pro_screen(id: &str, name: &str, subs: Vec<pp::Screen>) -> pp::ProPresenterScreen {
        pp::ProPresenterScreen {
            name: name.into(),
            screen_type: pp::pro_presenter_screen::ScreenType::Audience as i32,
            uuid: uuid(id),
            arrangement: Some(pp::pro_presenter_screen::Arrangement::ArrangementSingle(
                pp::pro_presenter_screen::SingleArrangement { screens: subs },
            )),
            ..Default::default()
        }
    }

    fn lower_third_look(id: &str, original: Option<&str>) -> pp::ProAudienceLook {
        pp::ProAudienceLook {
            uuid: uuid(id),
            name: "Key feed".into(),
            original_look_uuid: original.and_then(uuid),
            screen_looks: vec![
                pp::pro_audience_look::ProScreenLook {
                    pro_screen_uuid: uuid(SCREEN_A),
                    props_enabled: true,
                    live_video_enabled: true,
                    presentation_background_enabled: true,
                    presentation_foreground_enabled: true,
                    announcements_enabled: true,
                    props_layer_enabled: true,
                    messages_layer_enabled: true,
                    template_document_file_path: Some(pp::Url::default()),
                    ..Default::default()
                },
                pp::pro_audience_look::ProScreenLook {
                    pro_screen_uuid: uuid(SCREEN_B),
                    presentation_foreground_enabled: true,
                    template_document_file_path: Some(pp::Url {
                        platform: pp::url::Platform::Win32 as i32,
                        relative_file_path: Some(pp::url::RelativeFilePath::Local(
                            pp::url::LocalRelativePath {
                                root: pp::url::local_relative_path::Root::Show as i32,
                                path: "Themes/Example Lower/Theme".into(),
                            },
                        )),
                        storage: None,
                    }),
                    template_slide_uuid: uuid(TSLIDE),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    fn workspace() -> pp::ProPresenterWorkspace {
        use pp::output_display::Type;
        let mut keyed = sub(Type::Ndi, "Key-Feed", 25.0, 3);
        keyed.output_display.as_mut().unwrap().r#type = Type::Ndi as i32;
        pp::ProPresenterWorkspace {
            pro_screens: vec![
                pro_screen(
                    SCREEN_A,
                    "Program",
                    vec![sub(Type::Ndi, "Program-Out", 30.0, 1)],
                ),
                pp::ProPresenterScreen {
                    background_color: Some(pp::Color {
                        red: 1.0,
                        green: 1.0,
                        blue: 1.0,
                        alpha: 0.0,
                    }),
                    background_color_enabled: true,
                    ..pro_screen(SCREEN_B, "Lower", vec![keyed])
                },
                pro_screen(
                    "77777777-7777-4777-8777-777777777777",
                    "Unassigned",
                    vec![pp::Screen {
                        output_display: None,
                        ..sub(Type::Ndi, "", 30.0, 2)
                    }],
                ),
            ],
            audience_looks: vec![lower_third_look(LOOK, None)],
            live_audience_look: Some(lower_third_look(LIVE, Some(LOOK))),
            video_inputs: vec![pp::VideoInput {
                uuid: uuid("88888888-8888-4888-8888-888888888888"),
                user_description: "Camera Feed".into(),
                video_input_device: Some(pp::media::VideoDevice {
                    name: "STUDIO-PC (Camera Out)".into(),
                    unique_id: "STUDIO-PC (Camera Out)".into(),
                    ..Default::default()
                }),
                display_color: Some(pp::Color {
                    red: 0.5,
                    green: 0.25,
                    blue: 0.0,
                    alpha: 1.0,
                }),
                audio_type: pp::video_input::AudioDeviceType::None as i32,
                alpha_type: pp::AlphaType::Straight as i32,
                ..Default::default()
            }],
            audio_inputs: vec![pp::AudioInput {
                uuid: uuid("99999999-9999-4999-8999-999999999999"),
                user_description: "Mixer".into(),
                behavior_mode: Some(pp::audio_input::BehaviorMode {
                    mode: Some(pp::audio_input::behavior_mode::Mode::AutoOn(
                        pp::audio_input::behavior_mode::AutoOn {
                            linked_video_inputs: vec![0, 5],
                        },
                    )),
                }),
                source: Some(pp::audio_input::Source::AudioDevice(
                    pp::digital_audio::Device {
                        name: "USB Audio Interface".into(),
                        ..Default::default()
                    },
                )),
            }],
            digital_audio_setup: Some(pp::digital_audio::Setup {
                enable_sdi_ndi_device: true,
                main_output_device: Some(pp::digital_audio::Device {
                    name: "Example Device".into(),
                    routing: Some(pp::digital_audio::device::Routing {
                        map: vec![
                            pp::digital_audio::device::Map {
                                channel_index: 0,
                                mapped_indices: vec![1],
                            },
                            pp::digital_audio::device::Map {
                                channel_index: 1,
                                mapped_indices: vec![0],
                            },
                        ],
                        master_channel: Some(pp::digital_audio::device::Channel {
                            level: -6.0,
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            selected_library_name: "Songs".into(),
            ..Default::default()
        }
    }

    #[test]
    fn frame_rates_are_rational() {
        assert_eq!(frame_rate(30.0), Some(FrameRate::FPS30));
        assert_eq!(frame_rate(25.0), Some(FrameRate::FPS25));
        assert_eq!(frame_rate(29.97), Some(FrameRate::FPS29_97));
        assert_eq!(frame_rate(59.94), Some(FrameRate::FPS59_94));
        assert_eq!(
            frame_rate(23.976),
            Some(FrameRate {
                num: 24000,
                den: 1001
            })
        );
        assert_eq!(frame_rate(12.5), Some(FrameRate { num: 25, den: 2 }));
        assert_eq!(frame_rate(0.0), None);
        assert_eq!(frame_rate(f64::NAN), None);
    }

    #[test]
    fn network_source_names() {
        assert!(looks_like_network_source("STUDIO-PC (Program-Out)"));
        assert!(looks_like_network_source("host.local (Program)"));
        assert!(!looks_like_network_source("HD Pro Webcam C920 (046d:082d)"));
        assert!(!looks_like_network_source("Plain Camera"));
        assert!(!looks_like_network_source("HOST ()"));
        assert!(!looks_like_network_source("(x)"));
        let dev = |name: &str, t: pp::media::video_device::Type| pp::media::VideoDevice {
            name: name.into(),
            r#type: t as i32,
            ..Default::default()
        };
        use pp::media::video_device::Type;
        assert_eq!(
            input_source(&dev("PC (Out)", Type::Generic)),
            InputSource::Idn {
                source_name: "PC (Out)".into()
            }
        );
        assert_eq!(
            input_source(&dev("Studio Out", Type::Ndi)),
            InputSource::Idn {
                source_name: "Studio Out".into()
            }
        );
        assert_eq!(
            input_source(&dev("Capture Card", Type::Blackmagic)),
            InputSource::Device {
                name: "Capture Card".into()
            }
        );
    }

    #[test]
    fn layer_flags_absent_means_hidden() {
        let sl = pp::pro_audience_look::ProScreenLook {
            presentation_foreground_enabled: true,
            props_layer_enabled: true,
            ..Default::default()
        };
        let v = layer_visibility(&sl);
        assert_eq!(
            v,
            LayerVisibility {
                slide: true,
                props: true,
                ..LayerVisibility::NONE
            }
        );
        let all = pp::pro_audience_look::ProScreenLook {
            props_enabled: true,
            live_video_enabled: true,
            presentation_background_enabled: true,
            presentation_foreground_enabled: true,
            announcements_enabled: true,
            messages_layer_enabled: true,
            ..Default::default()
        };
        assert_eq!(layer_visibility(&all), LayerVisibility::ALL);
    }

    #[test]
    fn routing_maps_channels() {
        use pp::digital_audio::device::{Channel, Map, Routing};
        let r = Routing {
            channels: vec![
                Channel::default(),
                Channel::default(),
                Channel {
                    mute_enable: true,
                    ..Default::default()
                },
            ],
            map: vec![
                Map {
                    channel_index: 0,
                    mapped_indices: vec![0],
                },
                Map {
                    channel_index: 1,
                    mapped_indices: vec![1],
                },
                Map {
                    channel_index: 2,
                    mapped_indices: vec![0],
                },
            ],
            master_channel: Some(Channel {
                level: -4.5,
                ..Default::default()
            }),
            is_custom_map: false,
        };
        let a = audio_routing(Some(&r));
        assert_eq!(a.volume_db, -4.5);
        assert_eq!(a.channels.len(), 3);
        assert_eq!(
            a.channels[2],
            ChannelRoute {
                source: 2,
                dest: 0,
                gain_db: 0.0,
                muted: true
            }
        );
        // Plain pass-through stays compact.
        let plain = Routing {
            channels: vec![Channel::default(); 4],
            ..Default::default()
        };
        assert_eq!(audio_routing(Some(&plain)), AudioSettings::default());
        assert_eq!(audio_routing(None), AudioSettings::default());
    }

    #[test]
    fn workspace_end_to_end() {
        let mut ctx = ctx();
        let ws = workspace();
        import_workspace(&mut ctx, &ws, "Configuration/Workspace");

        // Screens.
        assert_eq!(ctx.out.screens.len(), 3);
        let a = &ctx.out.screens[0];
        assert_eq!(a.id.0.to_string(), SCREEN_A);
        assert_eq!(a.size, Size::new(1920.0, 1080.0));
        assert_eq!(a.frame_rate, FrameRate::FPS30);
        assert_eq!(a.alpha, AlphaMode::Opaque);
        assert_eq!(a.background, model::Color::TRANSPARENT);
        assert!(
            a.import.as_ref().unwrap().unmapped.is_null(),
            "default residue is not kept"
        );
        let b = &ctx.out.screens[1];
        assert_eq!(b.frame_rate, FrameRate::FPS25);
        assert_eq!(b.alpha, AlphaMode::Straight);
        assert_eq!(b.background, model::Color::rgba(1.0, 1.0, 1.0, 0.0));
        assert_eq!(ctx.out.screens[2].alpha, AlphaMode::Premultiplied);
        assert!(ctx.out.screens[0].sort_key < ctx.out.screens[1].sort_key);

        // Bindings: the unassigned screen gets none; IDN source names; audio embedded.
        assert_eq!(ctx.out.output_bindings.len(), 2);
        let ob = &ctx.out.output_bindings[1];
        assert_eq!(ob.screen, b.id);
        assert_eq!(
            ob.transport,
            OutputTransport::Idn {
                source_name: "Key-Feed".into(),
                groups: vec![]
            }
        );
        assert!(ob.embed_audio);
        assert_eq!(ob.frame_rate_override, None);
        assert_eq!(ob.resolution_override, None);
        assert_eq!(ob.region, None);
        assert_eq!(ob.alpha_override, None);
        assert_eq!(ob.name, "Lower");

        // Looks: the live look is a copy → startup look, one look only.
        assert_eq!(ctx.out.looks.len(), 1);
        let look = &ctx.out.looks[0];
        assert_eq!(look.id.0.to_string(), LOOK);
        assert_eq!(
            ctx.out.show_settings.as_ref().unwrap().startup_look,
            Some(look.id)
        );
        assert_eq!(look.screens[0].layers, LayerVisibility::ALL);
        assert!(look.screens[0].template_override.is_none());
        assert_eq!(
            look.screens[1].layers,
            LayerVisibility {
                slide: true,
                ..LayerVisibility::NONE
            }
        );
        let o = look.screens[1].template_override.as_ref().unwrap();
        assert_eq!(o.slide.0.to_string(), TSLIDE);
        assert_eq!(o.theme.id, ids::theme_id("Themes/Example Lower"));
        assert_eq!(o.theme.name.as_deref(), Some("Example Lower"));

        // Inputs.
        assert_eq!(ctx.out.video_inputs.len(), 1);
        let vi = &ctx.out.video_inputs[0];
        assert_eq!(vi.name, "Camera Feed");
        assert!(vi.audio.muted);
        assert_eq!(vi.alpha, SourceAlpha::Straight);
        assert_eq!(
            ctx.lookups.video_input_by_index(0).map(|v| v.id),
            Some(vi.id)
        );
        assert_eq!(
            ctx.out.input_bindings[0].source,
            InputSource::Idn {
                source_name: "STUDIO-PC (Camera Out)".into()
            }
        );
        let ai = &ctx.out.audio_inputs[0];
        assert_eq!(ai.mode, AudioInputMode::AutoOn);
        assert_eq!(ai.linked_video_inputs, vec![vi.id]);
        assert_eq!(
            ctx.out.audio_input_bindings[0].source,
            InputSource::Device {
                name: "USB Audio Interface".into()
            }
        );

        // Audio device.
        let dev = ctx.out.audio_device_config.as_ref().unwrap();
        assert_eq!(dev.device, "Example Device");
        assert_eq!(dev.channel_map, vec![1, 0]);
        assert_eq!(dev.master_gain_db, -6.0);

        // Leftovers are counted / reported.
        assert_eq!(
            ctx.report.unmapped_features["workspace.selectedLibraryName"],
            1
        );
        // One warning: the dangling linked video input #5.
        assert!(ctx.report.warnings.iter().any(|w| w.detail.contains("#5")));

        // IDs are unique per kind.
        let ids: BTreeSet<_> = ctx.out.output_bindings.iter().map(|b| b.id).collect();
        assert_eq!(ids.len(), 2);
    }

    /// A changed live look sorts right after its original, on a key of its own (A10-2).
    #[test]
    fn changed_live_look_sorts_after_its_original() {
        let mut ws = workspace();
        let ids = [
            "a0000001-0000-4000-8000-000000000001",
            "a0000002-0000-4000-8000-000000000002",
            "a0000003-0000-4000-8000-000000000003",
            "a0000004-0000-4000-8000-000000000004",
            "a0000005-0000-4000-8000-000000000005",
        ];
        ws.audience_looks = ids.iter().map(|id| lower_third_look(id, None)).collect();
        let mut live = lower_third_look(LIVE, Some(ids[1]));
        live.screen_looks[0].messages_layer_enabled = false;
        ws.live_audience_look = Some(live);
        let mut ctx = ctx();
        import_workspace(&mut ctx, &ws, "Configuration/Workspace");
        assert_eq!(ctx.out.looks.len(), 6);
        let key = |id: &str| {
            ctx.out
                .looks
                .iter()
                .find(|l| l.id.0.to_string() == id)
                .unwrap()
                .sort_key
                .clone()
        };
        let live_key = key(LIVE);
        assert!(
            key(ids[1]) < live_key && live_key < key(ids[2]),
            "{live_key}"
        );
        let keys: BTreeSet<_> = ctx.out.looks.iter().map(|l| l.sort_key.clone()).collect();
        assert_eq!(keys.len(), 6, "every key is its own");
    }

    #[test]
    fn changed_live_look_is_kept_and_unknown_original_too() {
        let mut ws = workspace();
        let mut live = lower_third_look(LIVE, Some(LOOK));
        live.screen_looks[0].messages_layer_enabled = false;
        ws.live_audience_look = Some(live);
        let mut ctx = ctx();
        import_workspace(&mut ctx, &ws, "Configuration/Workspace");
        assert_eq!(ctx.out.looks.len(), 2);
        let startup = ctx
            .out
            .show_settings
            .as_ref()
            .unwrap()
            .startup_look
            .unwrap();
        assert_eq!(startup.0.to_string(), LIVE);
        let live = ctx.out.looks.iter().find(|l| l.id == startup).unwrap();
        assert_eq!(live.name, "Key feed (live)");
        assert!(live.sort_key > ctx.out.looks[0].sort_key);

        // A live look without a saved original is imported as its own look.
        let mut ws = workspace();
        ws.audience_looks.clear();
        let mut ctx2 = self::ctx();
        import_workspace(&mut ctx2, &ws, "Configuration/Workspace");
        assert_eq!(ctx2.out.looks.len(), 1);
        assert_eq!(
            ctx2.out.show_settings.as_ref().unwrap().startup_look,
            Some(ctx2.out.looks[0].id)
        );
    }

    #[test]
    fn output_binding_overrides_and_transports() {
        use pp::output_display::Type;
        let mut ctx = ctx();
        let mut left = sub(Type::Screen, "\\\\.\\DISPLAY2", 60.0, 1);
        left.bounds = rect(0.0, 0.0, 1920.0, 1080.0);
        left.subscreen_unit_rect = rect(0.0, 0.0, 0.5, 1.0);
        left.output_display.as_mut().unwrap().model = "Example Monitor".into();
        let mut right = sub(Type::Card, "Card 1", 30.0, 3);
        right.bounds = rect(1920.0, 0.0, 1920.0, 1080.0);
        right.subscreen_unit_rect = rect(0.5, 0.0, 0.5, 1.0);
        let mut syphon = sub(Type::Syphon, "Out", 30.0, 1);
        syphon.subscreen_unit_rect = None;
        let s = pp::ProPresenterScreen {
            name: "Wide".into(),
            uuid: uuid(SCREEN_A),
            arrangement: Some(pp::pro_presenter_screen::Arrangement::ArrangementCombined(
                pp::pro_presenter_screen::CombinedArrangement {
                    screens: vec![left, right, syphon],
                    rows: 1,
                    columns: 2,
                },
            )),
            ..Default::default()
        };
        let sc = screen(&mut ctx, &s, "1".into(), &[]);
        assert_eq!(sc.size, Size::new(3840.0, 1080.0));
        assert_eq!(sc.frame_rate, FrameRate::FPS60);
        // Combined → Grouped, no bezel (the tiles touch).
        assert_eq!(sc.arrangement.kind, ArrangementKind::Grouped);
        assert_eq!((sc.arrangement.columns, sc.arrangement.rows), (2, 1));
        assert_eq!(sc.arrangement.bezel, TileGap::default());
        // The combined arrangement is kept without machine-local output displays.
        let unmapped = &sc.import.as_ref().unwrap().unmapped["arrangementCombined"];
        assert_eq!(unmapped["rows"], 1);
        assert!(unmapped["screens"][0].get("outputDisplay").is_none());

        let b = output_bindings(&mut ctx, &s, sc.id, true);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].name, "Wide 1");
        assert_eq!(
            b[0].transport,
            OutputTransport::Display {
                selector: DisplaySelector {
                    connector: Some("\\\\.\\DISPLAY2".into()),
                    name: Some("Example Monitor".into()),
                    index: None
                },
                fullscreen: true,
                mode: None
            }
        );
        assert!(!b[0].embed_audio, "displays never embed audio");
        // A tile is cropped by its tile, never by a region that would override it (F09-1).
        assert_eq!(b[0].region, None);
        assert_eq!(b[1].region, None);
        assert_eq!(
            b[0].resolution_override,
            Some(Resolution {
                width: 1920,
                height: 1080
            })
        );
        assert_eq!(b[1].frame_rate_override, Some(FrameRate::FPS30));
        assert_eq!(b[1].alpha_override, Some(OutputAlpha::Straight));
        assert!(b[1].embed_audio);
        assert!(
            matches!(b[1].transport, OutputTransport::Sdi { ref device, .. } if device == "Card 1")
        );
        assert_eq!(b[2].transport, OutputTransport::Unknown);
        // Tiles in sub-screen order; a sub-screen past the grid shows the whole screen.
        assert_eq!(
            b.iter().map(|o| o.tile).collect::<Vec<_>>(),
            vec![Some(0), Some(1), None]
        );
        assert!(
            ctx.report
                .warnings
                .iter()
                .any(|w| w.category == Category::NotImported)
        );
    }

    #[test]
    fn an_edge_blend_wall_keeps_its_overlap_and_empty_tiles_become_placeholders() {
        use pp::output_display::Type;
        let mut ctx = ctx();
        let mut left = sub(Type::Ndi, "Left", 60.0, 1);
        left.bounds = rect(0.0, 0.0, 1920.0, 1080.0);
        let mut right = sub(Type::Ndi, "Right", 60.0, 1);
        right.bounds = rect(1720.0, 0.0, 1920.0, 1080.0);
        right.output_display = None;
        right.name = "Right projector".into();
        let s = pp::ProPresenterScreen {
            name: "Wall".into(),
            uuid: uuid(SCREEN_A),
            arrangement: Some(pp::pro_presenter_screen::Arrangement::ArrangementEdgeBlend(
                pp::pro_presenter_screen::EdgeBlendArrangement {
                    screen_count: 2,
                    screens: vec![left, right],
                    edge_blends: vec![pp::EdgeBlend {
                        mode: pp::edge_blend::Mode::Linear as i32,
                        ..Default::default()
                    }],
                },
            )),
            ..Default::default()
        };
        let sc = screen(&mut ctx, &s, "1".into(), &[]);
        assert_eq!(sc.size, Size::new(3640.0, 1080.0));
        assert_eq!(sc.arrangement.kind, ArrangementKind::EdgeBlend);
        assert_eq!(sc.arrangement.columns, 2);
        assert_eq!(sc.arrangement.overlap, TileGap { x: 200.0, y: 0.0 });
        assert_eq!(sc.arrangement.blend.curve, 1.0);
        let b = output_bindings(&mut ctx, &s, sc.id, false);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].tile, Some(0));
        assert!(matches!(b[0].transport, OutputTransport::Idn { .. }));
        assert_eq!(b[1].transport, OutputTransport::Placeholder);
        assert_eq!(b[1].tile, Some(1));
        assert_eq!(b[1].name, "Right projector");
    }

    /// H06-1: three projectors overlapping by more than half a tile is a blend the server
    /// refuses: imported as a single screen, like any other grid that isn't sane.
    #[test]
    fn an_edge_blend_of_three_overlapping_over_half_a_tile_is_not_kept() {
        use pp::output_display::Type;
        let wall = |step: f64| {
            let subs = (0..3)
                .map(|i| {
                    let mut t = sub(Type::Ndi, &format!("P{i}"), 60.0, 1);
                    t.bounds = rect(i as f64 * step, 0.0, 1920.0, 1080.0);
                    t
                })
                .collect();
            pp::ProPresenterScreen {
                name: "Wall".into(),
                uuid: uuid(SCREEN_A),
                arrangement: Some(pp::pro_presenter_screen::Arrangement::ArrangementEdgeBlend(
                    pp::pro_presenter_screen::EdgeBlendArrangement {
                        screen_count: 3,
                        screens: subs,
                        edge_blends: vec![],
                    },
                )),
                ..Default::default()
            }
        };
        // Overlap 1320 px of a 1920 px tile: over half.
        let sc = screen(&mut ctx(), &wall(600.0), "1".into(), &[]);
        assert_eq!(sc.arrangement.kind, ArrangementKind::Single);
        // Overlap 960 px: exactly half, kept.
        let sc = screen(&mut ctx(), &wall(960.0), "1".into(), &[]);
        assert_eq!(sc.arrangement.kind, ArrangementKind::EdgeBlend);
        assert_eq!(sc.arrangement.overlap.x, 960.0);
    }

    #[test]
    fn a_display_with_a_key_device_becomes_a_fill_and_key_pair() {
        use pp::output_display::Type;
        let mut ctx = ctx();
        let straight = pp::screen::alpha_settings::Mode::Straight as i32;
        let mut tile = sub(Type::Screen, "\\\\.\\DISPLAY2", 60.0, straight);
        tile.alpha_settings.as_mut().unwrap().alpha_device =
            Some(pp::screen::alpha_settings::AlphaDevice {
                display: Some(pp::OutputDisplay {
                    name: "Key Out".into(),
                    device_name: "\\\\.\\DISPLAY3".into(),
                    model: "Key Monitor".into(),
                    r#type: Type::Screen as i32,
                    ..Default::default()
                }),
                ..Default::default()
            });
        let s = pro_screen(SCREEN_A, "Lower Thirds", vec![tile]);
        let sc = screen(&mut ctx, &s, "1".into(), &[]);
        let b = output_bindings(&mut ctx, &s, sc.id, false);
        assert_eq!(b.len(), 2, "{b:#?}");
        assert_eq!(b[0].target, OutputTarget::Fill);
        assert_eq!(
            b[0].alpha_override, None,
            "the screen's straight alpha: unshaped"
        );
        assert!(
            matches!(&b[0].transport, OutputTransport::Display { selector, .. }
            if selector.connector.as_deref() == Some("\\\\.\\DISPLAY2"))
        );
        assert_eq!(b[1].target, OutputTarget::Key);
        assert_eq!(b[1].name, format!("{} key", b[0].name));
        assert_ne!(b[1].id, b[0].id);
        assert_eq!(
            b[1].transport,
            OutputTransport::Display {
                selector: DisplaySelector {
                    connector: Some("\\\\.\\DISPLAY3".into()),
                    name: Some("Key Monitor".into()),
                    index: None
                },
                fullscreen: true,
                mode: None
            }
        );
        assert_eq!(b[1].screen, b[0].screen);
    }

    /// E07-1: a key device left stored on a tile whose alpha is disabled makes no key output.
    #[test]
    fn a_key_device_of_a_tile_with_alpha_disabled_makes_no_key_output() {
        use pp::output_display::Type;
        let mut ctx = ctx();
        let disabled = pp::screen::alpha_settings::Mode::Disabled as i32;
        let mut tile = sub(Type::Screen, "\\\\.\\DISPLAY2", 60.0, disabled);
        tile.alpha_settings.as_mut().unwrap().alpha_device =
            Some(pp::screen::alpha_settings::AlphaDevice {
                display: Some(pp::OutputDisplay {
                    name: "Old Key Out".into(),
                    device_name: "\\\\.\\DISPLAY3".into(),
                    r#type: Type::Screen as i32,
                    ..Default::default()
                }),
                ..Default::default()
            });
        let s = pro_screen(SCREEN_A, "Audience", vec![tile]);
        let sc = screen(&mut ctx, &s, "1".into(), &[]);
        let b = output_bindings(&mut ctx, &s, sc.id, false);
        assert_eq!(b.len(), 1, "{b:#?}");
        assert_eq!(b[0].target, OutputTarget::Full);
        assert_eq!(b[0].alpha_override, None, "no key/fill either");
    }

    #[test]
    fn stage_screen_uses_mapping() {
        let mut ctx = ctx();
        let layout = "abababab-abab-4bab-8bab-abababababab";
        let s = pp::ProPresenterScreen {
            screen_type: pp::pro_presenter_screen::ScreenType::Stage as i32,
            ..pro_screen(SCREEN_A, "Stage", vec![])
        };
        let maps = [pp::stage::ScreenAssignment {
            screen: Some(pp::CollectionElementType {
                parameter_uuid: uuid(SCREEN_A),
                ..Default::default()
            }),
            layout: Some(pp::CollectionElementType {
                parameter_uuid: uuid(layout),
                parameter_name: "Layout".into(),
                ..Default::default()
            }),
        }];
        let sc = screen(&mut ctx, &s, "1".into(), &maps);
        assert_eq!(
            sc.kind,
            ScreenKind::Stage {
                layout: Some(StageLayoutId::from_uuid(Uuid::parse_str(layout).unwrap()))
            }
        );
        assert_eq!(sc.size, Size::HD);
        ctx.out.screens.push(sc);
        resolve_theme_links(&mut ctx);
        assert!(
            ctx.report
                .warnings
                .iter()
                .any(|w| w.detail.contains("stage layout"))
        );
    }

    #[test]
    fn theme_links_resolve_to_kept_theme() {
        let mut ctx = ctx();
        let ws = pp::ProPresenterWorkspace {
            pro_screens: workspace().pro_screens,
            audience_looks: vec![lower_third_look(LOOK, None)],
            ..Default::default()
        };
        import_workspace(&mut ctx, &ws, "Configuration/Workspace");
        // The theme folder was a duplicate of a kept theme; the slide UUID resolves through
        // the template-slide lookup to the kept copy.
        let kept = ThemeId::from_uuid(ids::derived("theme", &["Themes/Kept"]));
        let kept_slide = TemplateSlideId::from_uuid(Uuid::parse_str(TSLIDE).unwrap());
        ctx.lookups.themes.insert(
            "Themes/Example Lower".into(),
            ThemeEntry {
                id: kept,
                name: "Kept".into(),
                folder: "Themes/Kept".into(),
                slides: vec![kept_slide],
            },
        );
        resolve_theme_links(&mut ctx);
        let o = ctx.out.looks[0].screens[1]
            .template_override
            .as_ref()
            .unwrap();
        assert_eq!(o.theme, Link::named(kept, "Kept"));
        assert_eq!(o.slide, kept_slide);
        assert!(ctx.report.warnings.is_empty());

        // Unknown slide → dangling link kept with a warning.
        let mut ctx = self::ctx();
        import_workspace(&mut ctx, &ws, "Configuration/Workspace");
        resolve_theme_links(&mut ctx);
        assert!(ctx.out.looks[0].screens[1].template_override.is_some());
        assert_eq!(ctx.report.warnings.len(), 1);
    }

    #[test]
    fn theme_folder_from_absolute_url() {
        let u = pp::Url {
            storage: Some(pp::url::Storage::AbsoluteString(
                "file:///C:/Users/Someone/Documents/ProPresenter/Themes/My%20Theme/Theme".into(),
            )),
            ..Default::default()
        };
        assert_eq!(
            theme_folder_from_url(&u).as_deref(),
            Some("Themes/My Theme")
        );
        assert_eq!(theme_folder_from_url(&pp::Url::default()), None);
        assert!(url_is_empty(&pp::Url::default()));
    }

    #[test]
    fn audio_device_disabled_or_identity() {
        let mut setup = pp::digital_audio::Setup {
            main_output_device: Some(pp::digital_audio::Device {
                name: "Out".into(),
                routing: Some(pp::digital_audio::device::Routing {
                    map: vec![pp::digital_audio::device::Map {
                        channel_index: 1,
                        mapped_indices: vec![1],
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let cfg = audio_device_config(&setup).unwrap();
        assert!(cfg.channel_map.is_empty(), "identity map is compact");
        setup.disable_main_output_device = true;
        assert!(audio_device_config(&setup).is_none());
    }

    #[test]
    fn repeated_screen_ids_are_rederived() {
        use pp::output_display::Type;
        let mut ctx = ctx();
        let ws = pp::ProPresenterWorkspace {
            pro_screens: vec![
                pro_screen(SCREEN_A, "One", vec![sub(Type::Ndi, "A", 30.0, 1)]),
                pro_screen(SCREEN_A, "Two", vec![sub(Type::Ndi, "B", 30.0, 1)]),
            ],
            ..Default::default()
        };
        import_workspace(&mut ctx, &ws, "Configuration/Workspace");
        assert_ne!(ctx.out.screens[0].id, ctx.out.screens[1].id);
        assert_ne!(ctx.out.output_bindings[0].id, ctx.out.output_bindings[1].id);
        assert_eq!(ctx.out.output_bindings[1].screen, ctx.out.screens[1].id);
    }
}
