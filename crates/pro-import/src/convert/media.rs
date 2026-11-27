//! **Area: media.** Media references → [`MediaAsset`]s under [`MediaRoot`]s, per-use media
//! settings ([`MediaTrigger`], media fills, [`VideoInputTrigger`]), media bins and the relink
//! report. Never copies media, never touches the network; reads only file metadata / headers
//! inside the backup.
//!
//! ## Owns
//! - [`asset_for_url`] / [`asset_for_media`]: one asset per distinct (root, normalized relative
//!   path), de-duplicated through `ctx.lookups.media_assets` (index into `ctx.out.media_assets`);
//!   every call counts a reference in `ctx.lookups.asset_references`. Location rules (IMPORTER.md
//!   "Media"):
//!   - `local { root: ROOT_SHOW, path }` → `Linked { root: <backup root>, path }`. The show folder
//!     *is* the backup, so a file that exists there is `Online` (probed when
//!     `ctx.options.probe_media`), and a file that is gone is still linked to the backup root
//!     but `Missing`: restoring it into the bound folder fixes every use without a relink, and
//!     the relink report lists it under the backup root;
//!   - `external { win32 { driveLetter, volumeName } | macos { volumeName }, path }` (G:, Z:,
//!     UNC) → `Linked { root: <one MediaRoot per distinct volume, VolumeHints>, path }`,
//!     `Missing` (external volumes are never touched). See *Volume roots* below;
//!   - `ROOT_CURRENT_RESOURCE` inside a theme (`ctx.current.resource_dir`) → `ThemeAsset { theme,
//!     path }` with `scope: Theme`, status from the file's existence. Outside a theme it is
//!     relative to the current document's folder (→ `Linked` under the backup root);
//!   - a URL with only an absolute Windows string (`X:\…`, `\\host\share\…`) is treated like
//!     the matching external volume;
//!   - anything else (other roots such as `ROOT_USER_HOME`, paths escaping their root with `..`,
//!     empty URLs) → `Unresolved`, `Missing`, ID `ids::unresolved_asset_id(<absolute string or
//!     ROOT/path>)`.
//!
//!   Paths are `/`-separated and normalized (`\` → `/`, empty and `.` segments dropped, `..`
//!   resolved; escaping the root makes the reference unresolved). ProPresenter paths come from
//!   case-insensitive file systems, so a backup file is found case-insensitively (the stored
//!   path is the on-disk spelling), and a missing file whose path differs from an existing
//!   asset of the same root only by case reuses that asset.
//!
//!   Always resolve from `media.url` (never `<kind>.file.localUrl`); keep the verbatim source in
//!   [`MediaOrigin`] (`absoluteString` only as a hint). IDs: `MediaAssetId::for_linked(root,
//!   path)`, `ids::theme_asset_id`, `ids::unresolved_asset_id`. Assets in a theme folder are
//!   theme-scoped; every other asset is library-scoped, even when a theme references it, since
//!   presentations may share it. `checkedAt` = `ctx.options.imported_at`; `import` = the
//!   provenance of the first referencing document (+ `unmapped.metadata` for artist / info
//!   metadata).
//! - Media roots: [`media_root_for_backup`] (name "ProPresenter backup", key `"backup"`, hints:
//!   the show-folder prefixes seen in `ROOT_SHOW` absolute strings) and [`media_root_for_volume`]
//!   (ID `ids::media_root_id(<root key>)`), pushed to `ctx.out.media_roots`. Keys in
//!   `ctx.lookups.media_roots`: `"backup"`; root keys `"unc:<host>/<share>"`,
//!   `"volume:<label>"`, `"drive:<letter>"` (lowercase, from the hint that created the root);
//!   and one alias key `"hint:<drive>|<volume>|<unc>"` per distinct hint seen.
//!
//!   **Volume roots.** A new hint joins an existing root when the evidence says it is the same
//!   folder: two UNC shares with the same share name whose hosts are aliases (same name, or a
//!   short name and its FQDN) *or* that are mapped under the same drive letter (an IP and a
//!   host name of the same NAS); two non-network volumes with the same volume label (on any
//!   drive letter); a bare drive letter and any volume on that letter. Different share names
//!   never merge (a root is one folder; one binding cannot serve two shares). Matching is
//!   incremental in import order, which is deterministic.
//! - [`media_action`]: an `ACTION_TYPE_MEDIA` action → `ActionKind::Media(MediaTrigger)` or
//!   `ActionKind::VideoInput(VideoInputTrigger)` (live video), `Unsupported` (counted) for web
//!   content or media without a file. Called by the slides area (slide actions) and the content
//!   area (inline playlist cues).
//! - [`media_trigger`] / [`media_layer`] / [`display`] / [`playback`] / [`audio_settings`] /
//!   [`effect`]: per-use settings from `Action.MediaType` + `rv.data.Media` (drawing, transport,
//!   video, audio properties). Element-level settings win; the (always empty in practice)
//!   action-level `Video` / `Audio` playback fields fill in where the element has defaults.
//!   Fade flags without a duration → `Fade { enabled: true, seconds: None }`; a duration with the
//!   flag off is kept (`enabled: false`). `RETRIGGER_SETTING_UNSET` → `Automatic`,
//!   `alwaysRetrigger` → `Always`. `outPoint` equal to `endPoint` (the file duration) → `None`.
//!   Volumes: the media volume is a linear factor (`1.0` = unity; `0` / unset = unity, since
//!   proto3 cannot tell them apart) → dB; per-channel volumes are dB already. Each channel output
//!   becomes a [`ChannelRoute`]; a channel without outputs is a muted route onto itself.
//! - [`media_fill`]: an element / text media fill → `FillKind::Media { asset, display, playback }`.
//! - [`video_input_trigger`]: live video → `VideoInputTrigger { input: Link (by
//!   `liveVideoIndex` through `ctx.lookups.video_inputs`, else a derived dangling link + warning) }`.
//! - [`import_media_bins`]: `Playlists/Media` → [`MediaBin`]s (smart directories → `Watched {
//!   folder: root + path, importBehavior, lastScan = cached cue list (ids
//!   `MediaBinItemId::for_watched`) }`; plain playlists → `Manual`; folder names → `MediaBin.folder`;
//!   sort keys in tree order; `browser: Audio` for an audio playlist document). The watched
//!   folder's import behaviour takes the layer from `importBehavior`, plus the scale mode (and
//!   video repeat mode) when every cached item agrees on it, so newly scanned files look like
//!   the cached ones. The verbatim smart-directory URL is kept in `import.unmapped`. The
//!   `liveVideoPlaylist` is derived from the video-input config: not imported (warn
//!   `notImported` if it holds anything unexpected).
//! - [`finalize_media`]: last phase. Sort keys for roots (backup first, then by name), fills
//!   `ctx.report.missing_media` (grouped by root, files sorted, reference counts; theme assets
//!   per theme, unresolved references in their own group) and suggests a
//!   [`MediaRootBinding`] for the backup root (→ the backup's absolute path) in
//!   `ctx.out.media_root_bindings`.
//! - [`probe_file`]: cheap header sniffing (PNG / JPEG / GIF / WebP dimensions, PNG / WebP
//!   alpha); video probing is not done (video / audio assets get `fromImportHints` probes from
//!   natural size, frame rate, end point and metadata format). An image whose header cannot be
//!   read is `Unsupported`.
//!
//! ## May call
//! `common::*`, `ImportContext` helpers. Never calls slides / content / config.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};

use model::{
    ActionKind, Anchor, AssetLocation, AssetScope, AudioSettings, BinBrowser, ChannelRoute, Color,
    Effect, EffectKind, EndBehavior, Fade, FillKind, Flip, FrameRate, Link, MediaAsset,
    MediaAssetId, MediaBin, MediaBinId, MediaBinItem, MediaBinItemId, MediaBinKind, MediaDisplay,
    MediaKind, MediaLayer, MediaOrigin, MediaProbe, MediaRoot, MediaRootBinding, MediaRootId,
    MediaStatus, MediaTrigger, Playback, QuarterTurn, Repeat, Retrigger, ScaleMode, SourceAlpha,
    SourcePlatform, ThemeId, Transition, TriggerDefaults, VideoInputId, VideoInputTrigger,
    VolumeHint, WatchedFolder,
};

use crate::context::{ImportContext, ids};
use crate::convert::common;
use crate::pb::rv::data as pp;
use crate::report::{Category, MissingFile, MissingMediaRoot};

use pp::url::local_relative_path::Root as UrlRoot;

/// Key of the backup root in `ctx.lookups.media_roots`.
const BACKUP_ROOT_KEY: &str = "backup";
/// Display name of the backup root.
pub const BACKUP_ROOT_NAME: &str = "ProPresenter backup";
/// ProPresenter render IDs of the built-in effects mapped to typed kinds (vendor constants).
const ADJUST_COLOR_ID: &str = "D4C199A9-64D7-4CCB-8C26-443E7A9C5C03";
const TWO_COLOR_ID: &str = "258D3830-8496-4712-AAAE-A68BCA23D805";
const ALPHA_ADJUST_ID: &str = "56C8A640-8BFF-4AFF-BBA5-23134683DB2F";
/// Soft-loop cross-fade when the flag is set without a duration (ProPresenter default).
const DEFAULT_SOFT_LOOP: f64 = 0.5;
/// Image extensions whose headers [`probe_file`] can read.
const SNIFFABLE: &[&str] = &["png", "jpg", "jpeg", "jpe", "jfif", "gif", "webp"];

// ---- assets -------------------------------------------------------------------------------------

/// The asset for a URL (created on first use, de-duplicated after). `kind` is the media kind of
/// the use site; `scope` the owner (`Library`, or `Theme` inside a theme).
pub fn asset_for_url(
    ctx: &mut ImportContext,
    url: &pp::Url,
    kind: MediaKind,
    scope: &AssetScope,
) -> MediaAssetId {
    let located = locate(ctx, url, scope);
    let id = located.id;
    *ctx.lookups.asset_references.entry(id).or_default() += 1;
    if let Some(&i) = ctx.lookups.media_assets.get(&id) {
        if let Some(asset) = ctx.out.media_assets.get_mut(i)
            && asset.kind == MediaKind::Unknown
            && kind != MediaKind::Unknown
        {
            asset.kind = kind;
        }
        return id;
    }

    let path_for_name = match &located.location {
        AssetLocation::Linked { path, .. } | AssetLocation::ThemeAsset { path, .. } => path.clone(),
        _ => located.reference.clone(),
    };
    let kind = if kind == MediaKind::Unknown {
        kind_from_extension(&path_for_name)
    } else {
        kind
    };
    let mut status = located.status;
    let mut probe = None;
    if let Some(file) = &located.file
        && ctx.options.probe_media
        && kind == MediaKind::Image
    {
        probe = probe_file(file);
        if probe.is_none() && SNIFFABLE.contains(&extension(&path_for_name).as_str()) {
            status = MediaStatus::Unsupported;
        }
    }
    let scope = match &located.location {
        AssetLocation::ThemeAsset { theme, .. } => AssetScope::Theme { theme: *theme },
        _ => AssetScope::Library,
    };
    let asset = MediaAsset {
        id,
        kind,
        name: file_stem(&path_for_name),
        location: located.location,
        origin: Some(origin(url)),
        status,
        checked_at: ctx.options.imported_at.clone(),
        probe,
        scope,
        import: Some(ctx.document_info()),
        ..MediaAsset::default()
    };
    ctx.lookups
        .media_assets
        .insert(id, ctx.out.media_assets.len());
    ctx.out.media_assets.push(asset);
    id
}

/// The asset of an `rv.data.Media` element (URL from `media.url`, kind from `typeProperties`,
/// probe hints from `metadata` / natural size). `None` when the media has no URL (live video).
pub fn asset_for_media(
    ctx: &mut ImportContext,
    media: &pp::Media,
    scope: &AssetScope,
) -> Option<MediaAssetId> {
    let url = media.url.as_ref()?;
    if url.storage.is_none() && url.relative_file_path.is_none() {
        return None;
    }
    let kind = media_kind(media);
    let id = asset_for_url(ctx, url, kind, scope);
    let i = *ctx.lookups.media_assets.get(&id)?;

    // Import hints fill in what probing the file could not give.
    let hints = probe_hints(media);
    if let Some(asset) = ctx.out.media_assets.get_mut(i) {
        match &mut asset.probe {
            None => asset.probe = hints,
            Some(p) if p.from_import_hints => {
                if let Some(h) = hints {
                    p.width = p.width.or(h.width);
                    p.height = p.height.or(h.height);
                    p.duration = p.duration.or(h.duration);
                    p.frame_rate = p.frame_rate.or(h.frame_rate);
                    p.codec = p.codec.take().or(h.codec);
                }
            }
            Some(_) => {}
        }
    }

    // Descriptive metadata (artist, ...) has no model slot: keep it once per asset.
    if let Some(meta) = &media.metadata
        && (!meta.artist.is_empty()
            || !meta.information.is_empty()
            || !meta.manufacture_name.is_empty()
            || meta.manufacture_url.is_some())
    {
        let already = ctx.out.media_assets[i]
            .import
            .as_ref()
            .is_some_and(|info| info.unmapped.get("metadata").is_some());
        if !already {
            let value = ctx.to_json(meta);
            let mut info = ctx.out.media_assets[i].import.take();
            ctx.keep_unmapped(&mut info, "media.metadata", value);
            ctx.out.media_assets[i].import = info;
        }
    }
    Some(id)
}

/// Media kind from `typeProperties` (image / video / audio; live video and web → `Unknown`).
/// Media without type properties falls back to the URL's file extension.
pub fn media_kind(media: &pp::Media) -> MediaKind {
    use pp::media::TypeProperties as T;
    match &media.type_properties {
        Some(T::Image(_)) => MediaKind::Image,
        Some(T::Video(_)) => MediaKind::Video,
        Some(T::Audio(_)) => MediaKind::Audio,
        Some(T::LiveVideo(_)) | Some(T::WebContent(_)) => MediaKind::Unknown,
        None => media
            .url
            .as_ref()
            .and_then(url_path)
            .map(|p| kind_from_extension(&p))
            .unwrap_or(MediaKind::Unknown),
    }
}

/// The verbatim source reference of a URL.
pub fn origin(url: &pp::Url) -> MediaOrigin {
    use pp::url::{RelativeFilePath as R, Storage};
    let mut o = MediaOrigin {
        platform: platform(url.platform),
        ..MediaOrigin::default()
    };
    match &url.storage {
        Some(Storage::AbsoluteString(s)) if !s.is_empty() => o.absolute_hint = Some(s.clone()),
        Some(Storage::RelativePath(s)) if !s.is_empty() && url.relative_file_path.is_none() => {
            o.relative_path = Some(s.clone())
        }
        _ => {}
    }
    match &url.relative_file_path {
        Some(R::Local(l)) => {
            o.root_kind = Some(root_name_of(l.root));
            o.relative_path = non_empty(&l.path);
        }
        Some(R::External(e)) => {
            o.relative_path = non_empty(&e.path);
            if let Some(w) = &e.win32 {
                o.drive_letter = drive_letter(&w.drive_letter);
                o.volume_name = non_empty(&w.volume_name);
                if parse_unc(&w.volume_name).is_some() || w.network_share {
                    o.unc_share = non_empty(&w.volume_name);
                }
            } else if let Some(m) = &e.macos {
                o.volume_name = non_empty(&m.volume_name);
            }
        }
        None => {}
    }
    o
}

// ---- actions and triggers -------------------------------------------------------------------------

/// An `ACTION_TYPE_MEDIA` payload → `Media`, `VideoInput` or `Unsupported` action kind. Assets
/// are scoped by `ctx.current.asset_scope`.
pub fn media_action(
    ctx: &mut ImportContext,
    action: &pp::Action,
    media: &pp::action::MediaType,
) -> ActionKind {
    let scope = ctx.current.asset_scope.clone();
    if let Some(trigger) = media_trigger(ctx, media, &scope) {
        return ActionKind::Media(trigger);
    }
    if let Some(trigger) = video_input_trigger(ctx, media) {
        return ActionKind::VideoInput(trigger);
    }
    const SOURCE: &str = "ACTION_TYPE_MEDIA";
    ctx.count_unsupported_action(SOURCE);
    ActionKind::Unsupported {
        source_type: SOURCE.into(),
        payload: ctx.to_json(action),
    }
}

/// A complete media use from an action payload; `None` when it is not file media (live video,
/// web content, no element).
pub fn media_trigger(
    ctx: &mut ImportContext,
    media: &pp::action::MediaType,
    scope: &AssetScope,
) -> Option<MediaTrigger> {
    use pp::action::media_type::MediaType as A;
    use pp::media::TypeProperties as T;
    let element = media.element.as_ref()?;
    match (&element.type_properties, &media.media_type) {
        (Some(T::LiveVideo(_)) | Some(T::WebContent(_)), _) | (None, Some(A::LiveVideo(_))) => {
            return None;
        }
        _ => {}
    }
    let mut kind = media_kind(element);
    if kind == MediaKind::Unknown {
        kind = match &media.media_type {
            Some(A::Image(_)) => MediaKind::Image,
            Some(A::Video(_)) => MediaKind::Video,
            Some(A::Audio(_)) => MediaKind::Audio,
            _ => MediaKind::Unknown,
        };
    }
    let asset = asset_for_media(ctx, element, scope)?;
    // The asset may know its kind better (extension) when nothing else says.
    if kind == MediaKind::Unknown
        && let Some(&i) = ctx.lookups.media_assets.get(&asset)
    {
        kind = ctx.out.media_assets[i].kind;
    }

    let parts = MediaParts::of(element);
    let mut trigger = MediaTrigger::new(asset, media_layer(media.layer_type, kind));
    if kind != MediaKind::Audio {
        trigger.display = display(ctx, parts.drawing);
    }
    for e in &media.effects {
        let e = effect(ctx, e);
        trigger.display.effects.push(e);
    }
    if kind != MediaKind::Image {
        trigger.playback = playback(parts.transport, parts.video);
        apply_action_playback(&mut trigger.playback, media);
        trigger.audio = audio_settings(parts.audio);
        count_audio_unmapped(ctx, parts.audio);
    }
    if media.always_retrigger {
        trigger.playback.retrigger = Retrigger::Always;
    }
    trigger.transition = action_transition(ctx, media);
    count_action_unmapped(ctx, media);
    Some(trigger)
}

/// A live-video use from an action payload; `None` when it is not live video.
pub fn video_input_trigger(
    ctx: &mut ImportContext,
    media: &pp::action::MediaType,
) -> Option<VideoInputTrigger> {
    use pp::action::media_type::MediaType as A;
    use pp::media::TypeProperties as T;
    let live = match media
        .element
        .as_ref()
        .and_then(|e| e.type_properties.as_ref())
    {
        Some(T::LiveVideo(l)) => Some(l),
        Some(_) => return None,
        None if matches!(media.media_type, Some(A::LiveVideo(_))) => None,
        None => return None,
    };
    let props = live.and_then(|l| l.live_video.as_ref());
    let input = live_input(ctx, props);
    let trigger = VideoInputTrigger {
        input,
        display: display(ctx, live.and_then(|l| l.drawing.as_ref())),
        audio: audio_settings(live.and_then(|l| l.audio.as_ref())),
        transition: action_transition(ctx, media),
    };
    let mut trigger = trigger;
    for e in &media.effects {
        let e = effect(ctx, e);
        trigger.display.effects.push(e);
    }
    count_action_unmapped(ctx, media);
    Some(trigger)
}

/// The logical input a live-video use names (by its index in the workspace's input list); a
/// dangling link (named after the device) when the index is not a configured input.
fn live_input(
    ctx: &mut ImportContext,
    props: Option<&pp::media::LiveVideoProperties>,
) -> Link<VideoInputId> {
    let index = props.map(|p| p.live_video_index).unwrap_or(0);
    match ctx.lookups.video_input_by_index(index) {
        Some(n) => Link::named(n.id, n.name.clone()),
        None => {
            let device = props
                .and_then(|p| p.video_device.as_ref())
                .map(|d| d.name.trim().to_string())
                .filter(|n| !n.is_empty());
            ctx.warn(
                Category::Other,
                format!(
                    "live video input index {index} is not a configured video input; kept as a \
                     dangling link"
                ),
            );
            Link {
                id: VideoInputId::from_uuid(ids::derived("videoInputIndex", &[&index.to_string()])),
                name: Some(device.unwrap_or_else(|| format!("Input {}", i64::from(index) + 1))),
            }
        }
    }
}

/// A live-video element fill (a camera box on a slide): the logical input by index, and
/// the device's name as the source when it names a network source (`HOST (name)`), used
/// where the input isn't bound.
fn live_video_fill(ctx: &mut ImportContext, live: &pp::media::LiveVideoTypeProperties) -> FillKind {
    let props = live.live_video.as_ref();
    let input = live_input(ctx, props);
    let source = props
        .and_then(|p| p.video_device.as_ref())
        .map(|d| d.name.trim().to_string())
        .filter(|n| super::workspace::looks_like_network_source(n))
        .unwrap_or_default();
    let display = display(ctx, live.drawing.as_ref());
    count_audio_unmapped(ctx, live.audio.as_ref());
    FillKind::LiveSource {
        source,
        input: Some(input.id),
        display,
    }
}

/// `Action.LayerType` + media kind → [`MediaLayer`] (audio files → `Audio`).
///
/// `BACKGROUND` → `Background`, `FOREGROUND` → `Foreground`, `INPUT` → `VideoInput`, `FILL`
/// (unused by ProPresenter 7 content) → `Background`, unknown values → `Unknown`.
pub fn media_layer(layer_type: i32, kind: MediaKind) -> MediaLayer {
    use pp::action::LayerType as L;
    if kind == MediaKind::Audio {
        return MediaLayer::Audio;
    }
    match L::try_from(layer_type) {
        Ok(L::Background) | Ok(L::Fill) => MediaLayer::Background,
        Ok(L::Foreground) => MediaLayer::Foreground,
        Ok(L::Input) => MediaLayer::VideoInput,
        Err(_) => MediaLayer::Unknown,
    }
}

/// Drawing properties → [`MediaDisplay`] (scale, anchor, crop, rotation, flip, alpha, effects).
/// Unmodelled parts (`customImageBounds`, `selectedEffectPresetUuid`, …) are counted with
/// `ctx.count_unmapped`.
///
/// `nativeRotation` is the clockwise corrective rotation; crop insets apply only when
/// `cropEnable` is set (disabled insets are counted as unmapped); `SCALE_BEHAVIOR_CUSTOM`
/// (custom image bounds, counted as unmapped) becomes `ScaleMode::None`; `customImageBounds`
/// with any other scale mode is inert and ignored.
pub fn display(
    ctx: &mut ImportContext,
    drawing: Option<&pp::media::DrawingProperties>,
) -> MediaDisplay {
    use pp::media::drawing_properties::NativeRotationType as Rot;
    use pp::media::{ScaleAlignment as SA, ScaleBehavior as SB};
    let Some(d) = drawing else {
        return MediaDisplay::default();
    };
    let mut m = MediaDisplay {
        scale: match SB::try_from(d.scale_behavior) {
            Ok(SB::Fit) => ScaleMode::Fit,
            Ok(SB::Fill) => ScaleMode::Fill,
            Ok(SB::Stretch) => ScaleMode::Stretch,
            Ok(SB::Custom) => {
                ctx.count_unmapped("media.drawing.scaleBehavior.custom");
                ScaleMode::None
            }
            Err(_) => ScaleMode::Unknown,
        },
        anchor: match SA::try_from(d.scale_alignment) {
            Ok(SA::MiddleCenter) => Anchor::Center,
            Ok(SA::TopLeft) => Anchor::TopLeft,
            Ok(SA::TopCenter) => Anchor::Top,
            Ok(SA::TopRight) => Anchor::TopRight,
            Ok(SA::MiddleRight) => Anchor::Right,
            Ok(SA::BottomRight) => Anchor::BottomRight,
            Ok(SA::BottomCenter) => Anchor::Bottom,
            Ok(SA::BottomLeft) => Anchor::BottomLeft,
            Ok(SA::MiddleLeft) => Anchor::Left,
            Err(_) => Anchor::Unknown,
        },
        rotation: match Rot::try_from(d.native_rotation) {
            Ok(Rot::RotateStandard) => QuarterTurn::None,
            Ok(Rot::Rotate90) => QuarterTurn::Cw90,
            Ok(Rot::Rotate180) => QuarterTurn::Cw180,
            Ok(Rot::Rotate270) => QuarterTurn::Cw270,
            Err(_) => QuarterTurn::Unknown,
        },
        flip: Flip {
            horizontal: d.flipped_horizontally,
            vertical: d.flipped_vertically,
        },
        alpha: match pp::AlphaType::try_from(d.alpha_type) {
            Ok(pp::AlphaType::Unknown) => SourceAlpha::Auto,
            Ok(pp::AlphaType::Straight) => SourceAlpha::Straight,
            Ok(pp::AlphaType::Premultiplied) => SourceAlpha::Premultiplied,
            Err(_) => SourceAlpha::Unknown,
        },
        ..MediaDisplay::default()
    };
    let insets = common::edge_insets(d.crop_insets.as_ref());
    if d.crop_enable {
        m.crop = insets;
    } else if insets != model::EdgeInsets::default() {
        ctx.count_unmapped("media.drawing.cropInsets.disabled");
    }
    m.effects = d.effects.iter().map(|e| effect(ctx, e)).collect();

    if d.alpha_inverted {
        ctx.count_unmapped("media.drawing.alphaInverted");
    }
    if d.is_blurred {
        ctx.count_unmapped("media.drawing.isBlurred");
    }
    if d.custom_image_rotation != 0.0 {
        ctx.count_unmapped("media.drawing.customImageRotation");
    }
    // Custom image bounds only take effect with SCALE_BEHAVIOR_CUSTOM (counted above); PowerPoint
    // imports write them on almost every fill, where they are inert.
    if ids::parse_uuid(d.selected_effect_preset_uuid.as_ref()).is_some() {
        ctx.count_unmapped("media.drawing.selectedEffectPresetUuid");
    }
    m
}

/// Transport + video properties → [`Playback`].
pub fn playback(
    transport: Option<&pp::media::TransportProperties>,
    video: Option<&pp::media::VideoProperties>,
) -> Playback {
    use pp::media::transport_properties::RetriggerSetting as RS;
    let mut p = Playback::default();
    if let Some(t) = transport {
        p.repeat = repeat(t.playback_behavior, t.times_to_loop, t.loop_time);
        p.in_point = finite_non_negative(t.in_point);
        p.out_point = out_point(t);
        p.rate = if t.play_rate.is_finite() && t.play_rate > 0.0 {
            t.play_rate
        } else {
            1.0
        };
        p.fade_in = fade(t.should_fade_in, t.fade_in_duration);
        p.fade_out = fade(t.should_fade_out, t.fade_out_duration);
        p.retrigger = match RS::try_from(t.retrigger) {
            Ok(RS::Always) => Retrigger::Always,
            Ok(RS::Never) => Retrigger::Never,
            Ok(RS::Unset) | Ok(RS::Automatic) | Err(_) => Retrigger::Automatic,
        };
    }
    if let Some(v) = video {
        p.end_behavior = end_behavior(v.end_behavior);
        if v.soft_loop {
            p.soft_loop = Some(soft_loop_seconds(v.soft_loop_duration));
        }
    }
    p
}

/// Audio properties → [`AudioSettings`] (volume, channel routing).
pub fn audio_settings(audio: Option<&pp::media::AudioProperties>) -> AudioSettings {
    let Some(a) = audio else {
        return AudioSettings::default();
    };
    let mut s = AudioSettings::default();
    if a.volume.is_finite() && a.volume > 0.0 {
        let db = 20.0 * a.volume.log10();
        s.volume_db = if db.abs() < 1e-9 { 0.0 } else { db };
    }
    for ch in &a.audio_channels {
        let gain_db = if ch.volume.is_finite() {
            ch.volume
        } else {
            0.0
        };
        if ch.outputs.is_empty() {
            s.channels.push(ChannelRoute {
                source: ch.index,
                dest: ch.index,
                gain_db,
                muted: true,
            });
        }
        for o in &ch.outputs {
            s.channels.push(ChannelRoute {
                source: ch.index,
                dest: u32::try_from(o.channel_index).unwrap_or(0),
                gain_db,
                muted: ch.muted,
            });
        }
    }
    s
}

/// A media effect (known names → typed kinds, else `Vendor`).
///
/// "Adjust Color" → `ColorAdjust` (hue radians → degrees), "Two Color" → `TwoColor`, "Alpha
/// Adjust" → `AlphaAdjust`; matched by name or ProPresenter render ID. A variable that is
/// present without a value is 0 (proto3); an absent variable takes the model default.
pub fn effect(ctx: &mut ImportContext, e: &pp::Effect) -> Effect {
    let _ = ctx; // reserved for warnings about odd effects
    let name = e.name.trim().to_ascii_lowercase();
    let rid = e.render_id.trim().to_ascii_uppercase();
    let kind = if name == "adjust color" || name == "color adjust" || rid == ADJUST_COLOR_ID {
        EffectKind::ColorAdjust {
            brightness: effect_number(e, "brightness").unwrap_or(0.0),
            contrast: effect_number(e, "contrast").unwrap_or(1.0),
            saturation: effect_number(e, "saturation").unwrap_or(1.0),
            hue: effect_number(e, "hue").map(f64::to_degrees).unwrap_or(0.0),
        }
    } else if name == "two color" || rid == TWO_COLOR_ID {
        EffectKind::TwoColor {
            dark: effect_color(e, "dark color").unwrap_or(Color::BLACK),
            bright: effect_color(e, "bright color").unwrap_or(Color::WHITE),
        }
    } else if name == "alpha adjust" || rid == ALPHA_ADJUST_ID {
        EffectKind::AlphaAdjust {
            amount: effect_number(e, "amount").unwrap_or(1.0),
        }
    } else {
        EffectKind::Vendor(common::vendor_effect(e))
    };
    Effect {
        enabled: e.enabled,
        kind,
    }
}

/// A media fill of an element or text (`FillKind::Media`; a fill without a URL → `Unsupported`).
pub fn media_fill(ctx: &mut ImportContext, media: &pp::Media, scope: &AssetScope) -> FillKind {
    if let Some(pp::media::TypeProperties::LiveVideo(l)) = &media.type_properties {
        return live_video_fill(ctx, l);
    }
    let Some(asset) = asset_for_media(ctx, media, scope) else {
        ctx.count_unmapped("fill.media.withoutFile");
        return FillKind::Unsupported {
            source_type: "propresenter.fill.media".into(),
            payload: ctx.to_json(media),
        };
    };
    let parts = MediaParts::of(media);
    let display = display(ctx, parts.drawing);
    let playback = if matches!(
        media.type_properties,
        Some(pp::media::TypeProperties::Image(_))
    ) {
        Playback::default()
    } else {
        playback(parts.transport, parts.video)
    };
    count_audio_unmapped(ctx, parts.audio);
    FillKind::Media {
        asset,
        display,
        playback,
        // A video element's sound: level, mute and routing (default: as is).
        audio: audio_settings(parts.audio),
    }
}

// ---- media roots --------------------------------------------------------------------------------

/// The media root of the backup folder itself (created on first use).
pub fn media_root_for_backup(ctx: &mut ImportContext) -> MediaRootId {
    if let Some(id) = ctx.lookups.media_roots.get(BACKUP_ROOT_KEY) {
        return *id;
    }
    let id = ids::media_root_id(BACKUP_ROOT_KEY);
    ctx.out.media_roots.push(MediaRoot {
        id,
        name: BACKUP_ROOT_NAME.into(),
        ..MediaRoot::default()
    });
    ctx.lookups
        .media_roots
        .insert(BACKUP_ROOT_KEY.to_string(), id);
    id
}

/// The media root for an external volume (created on first use, de-duplicated by hints; see the
/// module docs for the evidence rules).
pub fn media_root_for_volume(ctx: &mut ImportContext, hint: &VolumeHint) -> MediaRootId {
    let hint = clean_hint(hint);
    let alias = hint_key(&hint);
    if let Some(id) = ctx.lookups.media_roots.get(&alias) {
        return *id;
    }
    let backup = ctx.lookups.media_roots.get(BACKUP_ROOT_KEY).copied();
    let existing = ctx
        .out
        .media_roots
        .iter_mut()
        .find(|r| Some(r.id) != backup && r.hints.iter().any(|h| same_volume(h, &hint)));
    let id = match existing {
        Some(root) => {
            if !root.hints.iter().any(|h| hint_key(h) == alias) {
                root.hints.push(hint.clone());
            }
            root.id
        }
        None => {
            let base = root_key(&hint);
            let mut key = base.clone();
            let mut n = 1;
            while ctx.lookups.media_roots.contains_key(&key) {
                n += 1;
                key = format!("{base}#{n}");
            }
            let id = ids::media_root_id(&key);
            ctx.out.media_roots.push(MediaRoot {
                id,
                name: root_name(&hint),
                hints: vec![hint.clone()],
                ..MediaRoot::default()
            });
            ctx.lookups.media_roots.insert(key, id);
            id
        }
    };
    ctx.lookups.media_roots.insert(alias, id);
    id
}

// ---- probing ------------------------------------------------------------------------------------

/// Image dimensions (and codec name) by header sniffing; `None` when unknown or unreadable.
///
/// Reads only headers: PNG (`IHDR`, alpha from the colour type), JPEG (first `SOFn` marker),
/// GIF (logical screen), WebP (`VP8 ` / `VP8L` / `VP8X`, alpha flag).
pub fn probe_file(path: &Path) -> Option<MediaProbe> {
    let file = std::fs::File::open(path).ok()?;
    let mut r = BufReader::new(file);
    let mut buf = [0u8; 30];
    let n = read_up_to(&mut r, &mut buf)?;
    let head = &buf[..n];
    let probe = |w: u32, h: u32, codec: &str, alpha: Option<bool>| {
        (w > 0 && h > 0).then(|| MediaProbe {
            width: Some(w),
            height: Some(h),
            has_alpha: alpha,
            codec: Some(codec.into()),
            ..MediaProbe::default()
        })
    };
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        if head.len() < 26 || &head[12..16] != b"IHDR" {
            return None;
        }
        let w = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
        let h = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
        let alpha = match head[25] {
            4 | 6 => Some(true),
            0 | 2 => Some(false),
            _ => None,
        };
        return probe(w, h, "png", alpha);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        if head.len() < 10 {
            return None;
        }
        let w = u16::from_le_bytes([head[6], head[7]]);
        let h = u16::from_le_bytes([head[8], head[9]]);
        return probe(w.into(), h.into(), "gif", None);
    }
    if head.starts_with(b"RIFF") && head.len() >= 12 && &head[8..12] == b"WEBP" {
        if head.len() < 30 {
            return None;
        }
        let le24 = |b: &[u8]| u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16);
        return match &head[12..16] {
            b"VP8 " if head[23..26] == [0x9d, 0x01, 0x2a] => {
                let w = u16::from_le_bytes([head[26], head[27]]) & 0x3fff;
                let h = u16::from_le_bytes([head[28], head[29]]) & 0x3fff;
                probe(w.into(), h.into(), "webp", Some(false))
            }
            b"VP8L" if head[20] == 0x2f => {
                let b = &head[21..25];
                let w = 1 + (u32::from(b[0]) | (u32::from(b[1] & 0x3f) << 8));
                let h = 1
                    + ((u32::from(b[1]) >> 6)
                        | (u32::from(b[2]) << 2)
                        | (u32::from(b[3] & 0x0f) << 10));
                probe(w, h, "webp", Some((b[3] >> 4) & 1 == 1))
            }
            b"VP8X" => {
                let w = 1 + le24(&head[24..27]);
                let h = 1 + le24(&head[27..30]);
                probe(w, h, "webp", Some(head[20] & 0x10 != 0))
            }
            _ => None,
        };
    }
    if head.starts_with(&[0xff, 0xd8]) {
        r.seek(std::io::SeekFrom::Start(2)).ok()?;
        let (w, h) = jpeg_size(&mut r)?;
        return probe(w, h, "jpeg", Some(false));
    }
    None
}

// ---- media bins ---------------------------------------------------------------------------------

/// `Playlists/Media` → media bins (see the module docs).
pub fn import_media_bins(ctx: &mut ImportContext, doc: &pp::PlaylistDocument, rel_path: &str) {
    ctx.set_writer(doc.application_info.as_ref());
    let audio_doc = doc.r#type == pp::playlist_document::Type::Audio as i32
        || rel_path.rsplit('/').next() == Some("Audio");
    let browser = if audio_doc {
        BinBrowser::Audio
    } else {
        BinBrowser::Media
    };

    let mut leaves = Vec::new();
    if let Some(root) = &doc.root_node {
        let mut empty_folders = Vec::new();
        collect_bins(root, &mut Vec::new(), true, &mut leaves, &mut empty_folders);
        for folder in empty_folders {
            ctx.warn(
                Category::NotImported,
                format!(
                    "empty media bin folder \"{folder}\" (bin folders exist only while they hold \
                     a bin)"
                ),
            );
        }
    }
    let keys = ids::sort_keys(leaves.len());
    for (index, ((folder, playlist), sort_key)) in leaves.into_iter().zip(keys).enumerate() {
        let bin = media_bin(ctx, playlist, folder, sort_key, browser, rel_path, index);
        ctx.out.media_bins.push(bin);
    }

    if let Some(live) = &doc.live_video_playlist {
        check_live_video_playlist(ctx, live);
    }
    if let Some(downloads) = &doc.downloads_playlist
        && playlist_has_content(downloads)
    {
        ctx.warn(
            Category::NotImported,
            "downloads playlist (ProContent downloads) is not imported",
        );
    }
    if !doc.tags.is_empty() {
        ctx.count_unmapped("mediaBin.tags");
    }
}

/// Final phase: missing-media report and media-root binding suggestions.
pub fn finalize_media(ctx: &mut ImportContext) {
    let backup = ctx.lookups.media_roots.get(BACKUP_ROOT_KEY).copied();

    // Root order: backup first, then by name.
    let mut order: Vec<usize> = (0..ctx.out.media_roots.len()).collect();
    order.sort_by_key(|&i| {
        let r = &ctx.out.media_roots[i];
        (Some(r.id) != backup, r.name.to_lowercase(), r.id)
    });
    let keys = ids::sort_keys(order.len());
    for (i, key) in order.into_iter().zip(keys) {
        ctx.out.media_roots[i].sort_key = key;
    }

    // Missing media, grouped by root.
    let roots: BTreeMap<MediaRootId, &MediaRoot> =
        ctx.out.media_roots.iter().map(|r| (r.id, r)).collect();
    let theme_folders: BTreeMap<ThemeId, &str> = ctx
        .lookups
        .themes
        .values()
        .map(|t| (t.id, t.folder.as_str()))
        .collect();
    let mut groups: BTreeMap<String, MissingMediaRoot> = BTreeMap::new();
    for asset in &ctx.out.media_assets {
        if asset.status != MediaStatus::Missing {
            continue;
        }
        let references = ctx
            .lookups
            .asset_references
            .get(&asset.id)
            .copied()
            .unwrap_or(0);
        let (key, root, name, hints, path) = match &asset.location {
            AssetLocation::Linked { root, path } => {
                let r = roots.get(root);
                (
                    format!("root:{root}"),
                    Some(*root),
                    r.map(|r| r.name.clone())
                        .unwrap_or_else(|| root.to_string()),
                    r.map(|r| r.hints.clone()).unwrap_or_default(),
                    path.clone(),
                )
            }
            AssetLocation::ThemeAsset { theme, path } => (
                format!("theme:{theme}"),
                None,
                format!(
                    "Theme assets: {}",
                    theme_folders
                        .get(theme)
                        .map(|f| f.to_string())
                        .unwrap_or_else(|| theme.to_string())
                ),
                Vec::new(),
                path.clone(),
            ),
            _ => (
                "unresolved".into(),
                None,
                "Unresolved references".into(),
                Vec::new(),
                asset
                    .origin
                    .as_ref()
                    .map(best_reference)
                    .filter(|r| !r.is_empty())
                    .unwrap_or_else(|| asset.name.clone()),
            ),
        };
        groups
            .entry(key)
            .or_insert_with(|| MissingMediaRoot {
                root,
                name,
                hints,
                files: Vec::new(),
            })
            .files
            .push(MissingFile {
                path,
                asset: asset.id,
                references,
            });
    }
    let mut missing: Vec<MissingMediaRoot> = groups.into_values().collect();
    for g in &mut missing {
        g.files
            .sort_by(|a, b| a.path.cmp(&b.path).then(a.asset.cmp(&b.asset)));
    }
    missing.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.root.cmp(&b.root))
    });
    ctx.report.missing_media = missing;

    // The backup root binds to the backup folder on this machine.
    if let Some(root) = backup
        && !ctx.out.media_root_bindings.iter().any(|b| b.root == root)
    {
        let local = std::path::absolute(&ctx.root).unwrap_or_else(|_| ctx.root.clone());
        ctx.out.media_root_bindings.push(MediaRootBinding {
            schema_version: model::SCHEMA_VERSION,
            root,
            local_path: local.to_string_lossy().into_owned(),
        });
    }
}

// ---- private: locating URLs -----------------------------------------------------------------------

/// Where a URL points, before an asset exists.
enum Target {
    /// Relative to the ProPresenter show folder (= the backup root).
    Show(String),
    /// Relative to an external volume.
    Volume(VolumeHint, String),
    /// Relative to a theme folder (`folder` relative to the backup, when known).
    Theme {
        theme: ThemeId,
        folder: Option<String>,
        path: String,
    },
    /// Not locatable; the key identifies the reference.
    Unresolved(String),
}

/// A located URL: asset ID, location and status (not yet de-duplicated by ID).
struct Located {
    id: MediaAssetId,
    location: AssetLocation,
    status: MediaStatus,
    /// The file inside the backup, when it exists.
    file: Option<PathBuf>,
    /// The best textual source reference (names unresolved assets).
    reference: String,
}

fn target(ctx: &ImportContext, url: &pp::Url, scope: &AssetScope) -> Target {
    use pp::url::RelativeFilePath as R;
    let reference = url_reference(url);
    match &url.relative_file_path {
        Some(R::Local(l)) => {
            let Some(path) = clean_path(&l.path).filter(|p| !p.is_empty()) else {
                return Target::Unresolved(reference);
            };
            match UrlRoot::try_from(l.root) {
                Ok(UrlRoot::Show) => Target::Show(path),
                Ok(UrlRoot::CurrentResource) => current_resource(ctx, scope, path, reference),
                _ => Target::Unresolved(reference),
            }
        }
        Some(R::External(e)) => {
            let Some(path) = clean_path(&e.path) else {
                return Target::Unresolved(reference);
            };
            let mut hint = VolumeHint::default();
            if let Some(w) = &e.win32 {
                hint.drive_letter = drive_letter(&w.drive_letter);
                hint.volume_name = non_empty(w.volume_name.trim());
            } else if let Some(m) = &e.macos {
                hint.volume_name = non_empty(m.volume_name.trim());
            }
            let mut hint = clean_hint(&hint);
            if hint.drive_letter.is_none() && hint.volume_name.is_none() {
                return Target::Unresolved(reference);
            }
            if let Some(abs) = absolute_path(url)
                && let Some(prefix) = absolute_prefix(&abs, &path)
                && is_informative_prefix(&prefix, &hint)
            {
                hint.path_prefix = Some(prefix);
            }
            Target::Volume(hint, path)
        }
        None => match absolute_path(url).and_then(|a| parse_windows_absolute(&a)) {
            Some((hint, path)) => Target::Volume(hint, path),
            None => Target::Unresolved(reference),
        },
    }
}

/// `ROOT_CURRENT_RESOURCE`: the theme folder inside a theme, else the current document's folder.
fn current_resource(
    ctx: &ImportContext,
    scope: &AssetScope,
    path: String,
    reference: String,
) -> Target {
    let theme = match scope {
        AssetScope::Theme { theme } => Some(*theme),
        _ => ctx.current.resource_dir.as_deref().map(ids::theme_id),
    };
    if let Some(theme) = theme {
        return Target::Theme {
            theme,
            folder: ctx.current.resource_dir.clone(),
            path,
        };
    }
    match ctx.current.path.as_deref() {
        Some(doc) => match doc.rsplit_once('/') {
            Some((dir, _)) => clean_path(&format!("{dir}/{path}"))
                .map(Target::Show)
                .unwrap_or(Target::Unresolved(reference)),
            None => Target::Show(path),
        },
        None => Target::Unresolved(reference),
    }
}

fn locate(ctx: &mut ImportContext, url: &pp::Url, scope: &AssetScope) -> Located {
    let reference = url_reference(url);
    match target(ctx, url, scope) {
        Target::Show(path) => {
            let root = media_root_for_backup(ctx);
            if let Some(abs) = absolute_path(url)
                && let Some(prefix) = absolute_prefix(&abs, &path)
            {
                add_backup_hint(ctx, root, prefix);
            }
            match find_in_backup(ctx, &path) {
                Some(actual) => Located {
                    id: MediaAssetId::for_linked(root, &actual),
                    file: Some(ctx.path(&actual)),
                    location: AssetLocation::Linked { root, path: actual },
                    status: MediaStatus::Online,
                    reference,
                },
                None => missing_linked(ctx, root, path, reference),
            }
        }
        Target::Volume(hint, path) => {
            let root = media_root_for_volume(ctx, &hint);
            missing_linked(ctx, root, path, reference)
        }
        Target::Theme {
            theme,
            folder,
            path,
        } => {
            let found = folder.as_deref().and_then(|f| {
                let actual = find_in_backup(ctx, &format!("{f}/{path}"))?;
                let inner = actual.get(f.len() + 1..)?.to_string();
                Some((ctx.path(&actual), inner))
            });
            match found {
                Some((file, inner)) => Located {
                    id: ids::theme_asset_id(theme, &inner),
                    location: AssetLocation::ThemeAsset { theme, path: inner },
                    status: MediaStatus::Online,
                    file: Some(file),
                    reference,
                },
                None => {
                    let id = ids::theme_asset_id(theme, &path);
                    let id = if ctx.lookups.media_assets.contains_key(&id) {
                        id
                    } else {
                        existing_asset_ci(ctx, |l| match l {
                            AssetLocation::ThemeAsset { theme: t, path: p } => {
                                *t == theme && same_name(p, &path)
                            }
                            _ => false,
                        })
                        .unwrap_or(id)
                    };
                    Located {
                        id,
                        location: AssetLocation::ThemeAsset { theme, path },
                        status: MediaStatus::Missing,
                        file: None,
                        reference,
                    }
                }
            }
        }
        Target::Unresolved(key) => Located {
            id: ids::unresolved_asset_id(&key),
            location: AssetLocation::Unresolved,
            status: MediaStatus::Missing,
            file: None,
            reference: key,
        },
    }
}

/// A missing file under a root; re-uses an asset whose path differs only by case.
fn missing_linked(
    ctx: &ImportContext,
    root: MediaRootId,
    path: String,
    reference: String,
) -> Located {
    let id = MediaAssetId::for_linked(root, &path);
    let id = if ctx.lookups.media_assets.contains_key(&id) {
        id
    } else {
        existing_asset_ci(ctx, |l| match l {
            AssetLocation::Linked { root: r, path: p } => *r == root && same_name(p, &path),
            _ => false,
        })
        .unwrap_or(id)
    };
    Located {
        id,
        location: AssetLocation::Linked { root, path },
        status: MediaStatus::Missing,
        file: None,
        reference,
    }
}

fn existing_asset_ci(
    ctx: &ImportContext,
    matches: impl Fn(&AssetLocation) -> bool,
) -> Option<MediaAssetId> {
    ctx.out
        .media_assets
        .iter()
        .find(|a| matches(&a.location))
        .map(|a| a.id)
}

/// The on-disk spelling of a backup-relative file path, found exactly or case-insensitively.
/// Every segment is looked up in its folder's listing, not with `exists()`: on a
/// case-insensitive file system (Windows, macOS) `exists()` takes any spelling, and the stored
/// path must be the one on disk (two spellings of one file are one asset, and the path must
/// work on Linux).
fn find_in_backup(ctx: &ImportContext, rel: &str) -> Option<String> {
    if rel.is_empty() {
        return None;
    }
    let mut dir = ctx.root.clone();
    let mut parts = Vec::new();
    for seg in rel.split('/') {
        let names = ctx.dir_names(&dir)?;
        let name = if names.binary_search_by(|n| n.as_str().cmp(seg)).is_ok() {
            seg.to_string()
        } else {
            names.iter().find(|n| same_name(n, seg))?.clone()
        };
        dir.push(&name);
        parts.push(name);
    }
    dir.is_file().then(|| parts.join("/"))
}

fn add_backup_hint(ctx: &mut ImportContext, root: MediaRootId, prefix: String) {
    if let Some(r) = ctx.out.media_roots.iter_mut().find(|r| r.id == root)
        && !r.hints.iter().any(|h| {
            h.path_prefix
                .as_deref()
                .is_some_and(|p| same_name(p, &prefix))
        })
    {
        r.hints.push(VolumeHint {
            path_prefix: Some(prefix),
            ..VolumeHint::default()
        });
    }
}

// ---- private: volumes -----------------------------------------------------------------------------

/// `(host, share)` of a UNC string (`\\host\share[\...]`, `/` accepted).
fn parse_unc(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    let rest = s.strip_prefix("\\\\").or_else(|| s.strip_prefix("//"))?;
    let mut it = rest.split(['\\', '/']).filter(|p| !p.is_empty());
    let host = it.next()?.to_string();
    let share = it.next()?.to_string();
    Some((host, share))
}

/// Hint fields trimmed; the UNC prefix normalized (also derived from a UNC volume name); the
/// drive letter upper-case without colon.
fn clean_hint(h: &VolumeHint) -> VolumeHint {
    let volume_name = h.volume_name.as_deref().map(str::trim).and_then(non_empty);
    let unc_prefix = h
        .unc_prefix
        .as_deref()
        .and_then(parse_unc)
        .or_else(|| volume_name.as_deref().and_then(parse_unc))
        .map(|(host, share)| format!("\\\\{host}\\{share}"));
    VolumeHint {
        drive_letter: h.drive_letter.as_deref().and_then(drive_letter),
        volume_name,
        unc_prefix,
        path_prefix: h.path_prefix.clone(),
    }
}

/// Alias key of a hint (path prefix ignored).
fn hint_key(h: &VolumeHint) -> String {
    format!(
        "hint:{}|{}|{}",
        h.drive_letter.as_deref().unwrap_or("").to_uppercase(),
        h.volume_name.as_deref().unwrap_or("").to_lowercase(),
        h.unc_prefix.as_deref().unwrap_or("").to_lowercase()
    )
}

/// Root key of the hint that creates a root.
fn root_key(h: &VolumeHint) -> String {
    if let Some((host, share)) = h.unc_prefix.as_deref().and_then(parse_unc) {
        return format!("unc:{}/{}", host.to_lowercase(), share.to_lowercase());
    }
    if let Some(v) = &h.volume_name {
        return format!("volume:{}", v.to_lowercase());
    }
    if let Some(d) = &h.drive_letter {
        return format!("drive:{}", d.to_lowercase());
    }
    "volume:unknown".into()
}

/// Display name of a root created from `h`.
fn root_name(h: &VolumeHint) -> String {
    let base = h
        .unc_prefix
        .clone()
        .or_else(|| h.volume_name.clone())
        .unwrap_or_default();
    match (&h.drive_letter, base.is_empty()) {
        (Some(d), false) => format!("{base} ({d}:)"),
        (Some(d), true) => format!("Drive {d}:"),
        (None, false) => base,
        (None, true) => "External volume".into(),
    }
}

/// Evidence that two hints name the same folder (see the module docs).
fn same_volume(a: &VolumeHint, b: &VolumeHint) -> bool {
    let drives_equal = match (&a.drive_letter, &b.drive_letter) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
        _ => false,
    };
    let ua = a.unc_prefix.as_deref().and_then(parse_unc);
    let ub = b.unc_prefix.as_deref().and_then(parse_unc);
    match (ua, ub) {
        (Some((ha, sa)), Some((hb, sb))) => {
            same_name(&sa, &sb) && (hosts_alias(&ha, &hb) || drives_equal)
        }
        (Some(_), None) => b.volume_name.is_none() && drives_equal,
        (None, Some(_)) => a.volume_name.is_none() && drives_equal,
        (None, None) => match (&a.volume_name, &b.volume_name) {
            (Some(x), Some(y)) => same_name(x, y),
            _ => drives_equal,
        },
    }
}

/// Two host names of the same machine: equal, or a short name and its FQDN (IP literals only
/// match themselves).
fn hosts_alias(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    if a == b {
        return true;
    }
    let is_ip = |s: &str| s.chars().all(|c| c.is_ascii_digit() || c == '.') || s.contains(':');
    if is_ip(&a) || is_ip(&b) {
        return false;
    }
    let first = |s: &str| s.split('.').next().unwrap_or("").to_string();
    (!a.contains('.') || !b.contains('.')) && first(&a) == first(&b)
}

/// `X:\rest` or `\\host\share\rest` → (hint, normalized rest).
fn parse_windows_absolute(abs: &str) -> Option<(VolumeHint, String)> {
    let s = abs.trim();
    if let Some((host, share)) = parse_unc(s) {
        let rest = s
            .trim_start_matches(['\\', '/'])
            .splitn(3, ['\\', '/'])
            .nth(2)
            .unwrap_or("");
        let hint = VolumeHint {
            unc_prefix: Some(format!("\\\\{host}\\{share}")),
            ..VolumeHint::default()
        };
        let path = clean_path(rest)?;
        return (!path.is_empty()).then_some((hint, path));
    }
    let b = s.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
    {
        let hint = VolumeHint {
            drive_letter: Some((b[0] as char).to_ascii_uppercase().to_string()),
            ..VolumeHint::default()
        };
        let path = clean_path(&s[3..])?;
        return (!path.is_empty()).then_some((hint, path));
    }
    None
}

/// A path prefix worth keeping on a volume hint (not just the drive root or the UNC share).
fn is_informative_prefix(prefix: &str, hint: &VolumeHint) -> bool {
    let p = prefix.trim_end_matches(['\\', '/']);
    if p.is_empty() {
        return false;
    }
    if let Some(d) = &hint.drive_letter
        && p.eq_ignore_ascii_case(&format!("{d}:"))
    {
        return false;
    }
    if let Some(u) = &hint.unc_prefix
        && same_name(&p.replace('/', "\\"), u)
    {
        return false;
    }
    true
}

// ---- private: per-use helpers ------------------------------------------------------------------

/// The parts of a media element's type properties.
#[derive(Default)]
struct MediaParts<'a> {
    drawing: Option<&'a pp::media::DrawingProperties>,
    transport: Option<&'a pp::media::TransportProperties>,
    video: Option<&'a pp::media::VideoProperties>,
    audio: Option<&'a pp::media::AudioProperties>,
}

impl<'a> MediaParts<'a> {
    fn of(media: &'a pp::Media) -> Self {
        use pp::media::TypeProperties as T;
        match &media.type_properties {
            Some(T::Image(i)) => MediaParts {
                drawing: i.drawing.as_ref(),
                ..MediaParts::default()
            },
            Some(T::Video(v)) => MediaParts {
                drawing: v.drawing.as_ref(),
                transport: v.transport.as_ref(),
                video: v.video.as_ref(),
                audio: v.audio.as_ref(),
            },
            Some(T::Audio(a)) => MediaParts {
                transport: a.transport.as_ref(),
                audio: a.audio.as_ref(),
                ..MediaParts::default()
            },
            Some(T::LiveVideo(l)) => MediaParts {
                drawing: l.drawing.as_ref(),
                audio: l.audio.as_ref(),
                ..MediaParts::default()
            },
            Some(T::WebContent(w)) => MediaParts {
                drawing: w.drawing.as_ref(),
                ..MediaParts::default()
            },
            None => MediaParts::default(),
        }
    }
}

/// Probe hints from the element (natural size, frame rate, end point, metadata format).
fn probe_hints(media: &pp::Media) -> Option<MediaProbe> {
    let parts = MediaParts::of(media);
    let mut p = MediaProbe {
        from_import_hints: true,
        ..MediaProbe::default()
    };
    if let Some(size) = parts.drawing.and_then(|d| d.natural_size.as_ref()) {
        let dim = |v: f64| (v.is_finite() && (1.0..1e6).contains(&v)).then(|| v.round() as u32);
        if let (Some(w), Some(h)) = (dim(size.width), dim(size.height)) {
            p.width = Some(w);
            p.height = Some(h);
        }
    }
    p.frame_rate = parts.video.and_then(|v| frame_rate(v.frame_rate));
    p.duration = parts
        .transport
        .map(|t| t.end_point)
        .filter(|d| d.is_finite() && *d > 0.0);
    p.codec = media
        .metadata
        .as_ref()
        .map(|m| m.format.trim().to_lowercase())
        .filter(|f| !f.is_empty());
    let empty =
        p.width.is_none() && p.frame_rate.is_none() && p.duration.is_none() && p.codec.is_none();
    (!empty).then_some(p)
}

/// A frame rate as an exact rational (integer and NTSC rates recognized).
fn frame_rate(fps: f64) -> Option<FrameRate> {
    if !(fps.is_finite() && fps > 0.0 && fps < 1000.0) {
        return None;
    }
    let whole = fps.round();
    if (fps - whole).abs() < 0.005 {
        return Some(FrameRate::fps(whole as u32));
    }
    let ntsc = (fps * 1.001).round();
    if (fps - ntsc * 1000.0 / 1001.0).abs() < 0.005 {
        return Some(FrameRate {
            num: ntsc as u32 * 1000,
            den: 1001,
        });
    }
    Some(FrameRate {
        num: (fps * 1000.0).round() as u32,
        den: 1000,
    })
}

/// Playback behaviour (0 stop, 1 loop, 2 loop for count, 3 loop for time; the same numbering in
/// `Media.TransportProperties` and `Action.MediaType`) → [`Repeat`].
fn repeat(behavior: i32, times: u32, seconds: f64) -> Repeat {
    match behavior {
        1 => Repeat::Loop,
        2 => Repeat::LoopCount {
            count: times.max(1),
        },
        3 if seconds.is_finite() && seconds > 0.0 => Repeat::LoopFor { seconds },
        3 => Repeat::Loop,
        _ => Repeat::Once,
    }
}

/// Video end behaviour (the same numbering in `Media.VideoProperties` and `Action.MediaType`).
fn end_behavior(v: i32) -> EndBehavior {
    use pp::media::video_properties::EndBehavior as E;
    match E::try_from(v) {
        Ok(E::Stop) => EndBehavior::HoldLastFrame,
        Ok(E::StopOnBlack) => EndBehavior::Black,
        Ok(E::StopOnClear) => EndBehavior::Clear,
        Ok(E::FadeToBlack) => EndBehavior::FadeToBlack,
        Ok(E::FadeToClear) => EndBehavior::FadeToClear,
        Err(_) => EndBehavior::Unknown,
    }
}

fn fade(enabled: bool, seconds: f64) -> Fade {
    Fade {
        enabled,
        seconds: (seconds.is_finite() && seconds > 0.0).then_some(seconds),
    }
}

fn soft_loop_seconds(d: f64) -> f64 {
    if d.is_finite() && d > 0.0 {
        d
    } else {
        DEFAULT_SOFT_LOOP
    }
}

/// The out point, or `None` when it is unset or reaches the end point (no trim).
fn out_point(t: &pp::media::TransportProperties) -> Option<f64> {
    let out = t.out_point;
    if !(out.is_finite() && out > 0.0) {
        return None;
    }
    if t.end_point.is_finite() && t.end_point > 0.0 && out >= t.end_point - 1e-3 {
        return None;
    }
    Some(out)
}

fn finite_non_negative(v: f64) -> f64 {
    if v.is_finite() && v > 0.0 { v } else { 0.0 }
}

/// Action-level `Video` / `Audio` playback fields where the element left defaults.
fn apply_action_playback(p: &mut Playback, media: &pp::action::MediaType) {
    use pp::action::media_type::MediaType as A;
    let (behavior, times, seconds, end, soft) = match &media.media_type {
        Some(A::Video(v)) => (
            v.playback_behavior,
            v.times_to_loop,
            v.loop_time,
            v.end_behavior,
            v.soft_loop.then_some(v.soft_loop_duration),
        ),
        Some(A::Audio(a)) => (a.playback_behavior, a.times_to_loop, a.loop_time, 0, None),
        _ => return,
    };
    if p.repeat == Repeat::Once && behavior != 0 {
        p.repeat = repeat(behavior, times, seconds);
    }
    if p.end_behavior == EndBehavior::HoldLastFrame && end != 0 {
        p.end_behavior = end_behavior(end);
    }
    if p.soft_loop.is_none()
        && let Some(d) = soft
    {
        p.soft_loop = Some(soft_loop_seconds(d));
    }
}

fn action_transition(ctx: &mut ImportContext, media: &pp::action::MediaType) -> Option<Transition> {
    match &media.transition {
        Some(t) => Some(common::transition(ctx, t)),
        None if media.transition_duration.is_finite() && media.transition_duration > 0.0 => {
            Some(Transition::dissolve(media.transition_duration))
        }
        None => None,
    }
}

fn count_action_unmapped(ctx: &mut ImportContext, media: &pp::action::MediaType) {
    use pp::action::media_type::MediaType as A;
    if ids::parse_uuid(media.selected_effect_preset_uuid.as_ref()).is_some() {
        ctx.count_unmapped("media.selectedEffectPresetUuid");
    }
    if !media.markers.is_empty() {
        ctx.count_unmapped("media.markers");
    }
    if let Some(A::Audio(a)) = &media.media_type
        && a.audio_type != 0
    {
        ctx.count_unmapped("media.audio.audioType");
    }
}

fn count_audio_unmapped(ctx: &mut ImportContext, audio: Option<&pp::media::AudioProperties>) {
    let Some(a) = audio else { return };
    if a.audio_channels.iter().any(|c| c.compress_limit) {
        ctx.count_unmapped("media.audio.compressLimit");
    }
    if a.volume == 0.0 && !a.audio_channels.is_empty() {
        // proto3 cannot tell "unset" from "0"; played at unity.
        ctx.count_unmapped("media.audio.volumeZero");
    }
}

fn effect_variable<'a>(e: &'a pp::Effect, name: &str) -> Option<&'a pp::effect::EffectVariable> {
    e.variables
        .iter()
        .find(|v| v.name.trim().eq_ignore_ascii_case(name))
}

fn effect_number(e: &pp::Effect, name: &str) -> Option<f64> {
    use pp::effect::effect_variable::Type;
    let v = match effect_variable(e, name)?.r#type.as_ref()? {
        Type::Float(f) => f64::from(f.value),
        Type::Double(d) => d.value,
        Type::Int(i) => f64::from(i.value),
        _ => return None,
    };
    v.is_finite().then_some(v)
}

fn effect_color(e: &pp::Effect, name: &str) -> Option<Color> {
    use pp::effect::effect_variable::Type;
    match effect_variable(e, name)?.r#type.as_ref()? {
        Type::Color(c) => Some(c.color.as_ref().map(common::color).unwrap_or_default()),
        _ => None,
    }
}

// ---- private: probing ---------------------------------------------------------------------------

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> Option<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(n)
}

/// Width and height from the first JPEG start-of-frame marker (reader positioned after SOI).
fn jpeg_size<R: Read + Seek>(r: &mut R) -> Option<(u32, u32)> {
    fn byte(r: &mut impl Read) -> Option<u8> {
        let mut b = [0u8; 1];
        r.read_exact(&mut b).ok()?;
        Some(b[0])
    }
    for _ in 0..10_000 {
        // Find the marker prefix; skip fill bytes.
        let mut b = byte(r)?;
        while b != 0xff {
            b = byte(r)?;
        }
        let mut marker = byte(r)?;
        while marker == 0xff {
            marker = byte(r)?;
        }
        match marker {
            0x00 | 0x01 | 0xd0..=0xd8 => continue,
            0xd9 | 0xda => return None,
            _ => {}
        }
        let mut len = [0u8; 2];
        r.read_exact(&mut len).ok()?;
        let len = u16::from_be_bytes(len);
        if len < 2 {
            return None;
        }
        let is_sof = matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
        if is_sof {
            let mut sof = [0u8; 5];
            r.read_exact(&mut sof).ok()?;
            let h = u16::from_be_bytes([sof[1], sof[2]]);
            let w = u16::from_be_bytes([sof[3], sof[4]]);
            return Some((w.into(), h.into()));
        }
        r.seek_relative(i64::from(len) - 2).ok()?;
    }
    None
}

// ---- private: media bins ------------------------------------------------------------------------

fn collect_bins<'a>(
    p: &'a pp::Playlist,
    path: &mut Vec<String>,
    is_root: bool,
    out: &mut Vec<(Vec<String>, &'a pp::Playlist)>,
    empty_folders: &mut Vec<String>,
) {
    use pp::playlist::{ChildrenType, LinkData, Type};
    let children: Vec<&pp::Playlist> = match &p.children_type {
        Some(ChildrenType::Playlists(a)) => a.playlists.iter().chain(&p.children).collect(),
        _ => p.children.iter().collect(),
    };
    let is_bin = matches!(p.children_type, Some(ChildrenType::Items(_)))
        || matches!(p.link_data, Some(LinkData::SmartDirectory(_)))
        || p.smart_directory_path.is_some()
        || !p.cues.is_empty()
        || (!is_root
            && children.is_empty()
            && matches!(
                Type::try_from(p.r#type),
                Ok(Type::Playlist) | Ok(Type::Smart)
            ));
    if is_bin {
        out.push((path.clone(), p));
        return;
    }
    if !is_root {
        if children.is_empty() {
            let mut full = path.clone();
            full.push(p.name.clone());
            empty_folders.push(full.join("/"));
            return;
        }
        path.push(p.name.clone());
    }
    for c in children {
        collect_bins(c, path, false, out, empty_folders);
    }
    if !is_root {
        path.pop();
    }
}

/// A bin entry: the cue and the item fields that matter.
struct BinEntry<'a> {
    cue: &'a pp::Cue,
    name: &'a str,
    uuid: Option<&'a pp::Uuid>,
    hidden: bool,
}

fn bin_entries<'a>(ctx: &mut ImportContext, p: &'a pp::Playlist) -> Vec<BinEntry<'a>> {
    use pp::playlist::ChildrenType;
    use pp::playlist_item::ItemType;
    let mut out = Vec::new();
    if let Some(ChildrenType::Items(items)) = &p.children_type {
        for item in &items.items {
            match &item.item_type {
                Some(ItemType::Cue(cue)) => out.push(BinEntry {
                    cue,
                    name: &item.name,
                    uuid: item.uuid.as_ref(),
                    hidden: item.is_hidden,
                }),
                _ => ctx.warn(
                    Category::NotImported,
                    format!(
                        "media bin \"{}\": item \"{}\" is not a media cue; not imported",
                        p.name, item.name
                    ),
                ),
            }
        }
    }
    for cue in &p.cues {
        out.push(BinEntry {
            cue,
            name: &cue.name,
            uuid: cue.uuid.as_ref(),
            hidden: false,
        });
    }
    out
}

/// The trigger of a bin cue (its first media action, file media only).
fn cue_trigger(ctx: &mut ImportContext, bin: &str, entry: &BinEntry) -> Option<MediaTrigger> {
    use pp::action::ActionTypeData;
    let media = entry
        .cue
        .actions
        .iter()
        .find_map(|a| match &a.action_type_data {
            Some(ActionTypeData::Media(m)) => Some(m),
            _ => None,
        });
    let Some(media) = media else {
        ctx.warn(
            Category::NotImported,
            format!(
                "media bin \"{bin}\": item \"{}\" has no media action",
                entry.name
            ),
        );
        return None;
    };
    if entry.cue.actions.len() > 1 {
        ctx.count_unmapped("mediaBin.item.extraActions");
    }
    if common::hotkey(entry.cue.hot_key.as_ref()).is_some() {
        ctx.count_unmapped("mediaBin.item.hotKey");
    }
    let trigger = media_trigger(ctx, media, &AssetScope::Library);
    if trigger.is_none() {
        ctx.warn(
            Category::NotImported,
            format!(
                "media bin \"{bin}\": item \"{}\" is not a media file (live video / web content); \
                 not imported",
                entry.name
            ),
        );
    }
    trigger
}

fn media_bin(
    ctx: &mut ImportContext,
    p: &pp::Playlist,
    folder: Vec<String>,
    sort_key: model::SortKey,
    browser: BinBrowser,
    rel_path: &str,
    index: usize,
) -> MediaBin {
    use pp::playlist::LinkData;
    let derived =
        || MediaBinId::from_uuid(ids::derived("mediaBin", &[rel_path, &index.to_string()]));
    let mut info = Some(ctx.document_info());
    let id = match ids::parse_uuid(p.uuid.as_ref()).map(MediaBinId::from_uuid) {
        // A copied bin can repeat an earlier bin's UUID: give it its own ID.
        Some(u) if ctx.out.media_bins.iter().any(|b| b.id == u) => {
            if let Some(i) = info.as_mut() {
                i.original_id = Some(u.to_string());
            }
            derived()
        }
        Some(u) => u,
        None => derived(),
    };
    if let Some(h) = &p.hot_key
        && common::hotkey(Some(h)).is_some()
    {
        let v = ctx.to_json(h);
        ctx.keep_unmapped(&mut info, "mediaBin.hotKey", v);
    }
    if let Some(u) = &p.targeted_layer_uuid
        && ids::parse_uuid(Some(u)).is_some()
    {
        let v = ctx.to_json(u);
        ctx.keep_unmapped(&mut info, "mediaBin.targetedLayerUuid", v);
    }
    if let Some(s) = &p.startup_info
        && s.trigger_on_startup
    {
        let v = ctx.to_json(s);
        ctx.keep_unmapped(&mut info, "mediaBin.startupInfo", v);
    }
    if let Some(LinkData::PcoPlan(plan)) = &p.link_data {
        let v = ctx.to_json(plan);
        ctx.keep_unmapped(&mut info, "mediaBin.pcoPlan", v);
    }
    if p.timecode_enabled || p.timing != 0 {
        ctx.count_unmapped("mediaBin.timing");
    }

    let smart = match &p.link_data {
        Some(LinkData::SmartDirectory(fd)) => {
            fd.smart_directory.as_ref().map(|u| (u, fd.import_behavior))
        }
        _ => None,
    }
    .or_else(|| p.smart_directory_path.as_ref().map(|u| (u, 0)));

    let entries = bin_entries(ctx, p);
    let mut triggers = Vec::new();
    for e in &entries {
        if e.hidden {
            ctx.count_unmapped("mediaBin.item.isHidden");
        }
        if let Some(t) = cue_trigger(ctx, &p.name, e) {
            triggers.push((e, t));
        }
    }

    let watched = smart.and_then(|(url, behavior)| {
        let v = ctx.to_json(url);
        ctx.keep_unmapped(&mut info, "mediaBin.smartDirectory", v);
        let located = folder_location(ctx, url);
        if located.is_none() {
            ctx.warn(
                Category::MissingMedia,
                format!(
                    "smart media bin \"{}\": its folder cannot be located; imported as a manual \
                     bin",
                    p.name
                ),
            );
        }
        located.map(|l| (l, behavior))
    });

    let kind = match watched {
        Some(((root, path), behavior)) => {
            let mut seen = BTreeSet::new();
            let mut last_scan = Vec::new();
            for (e, trigger) in triggers {
                let rel = path_in_folder(ctx, trigger.asset, root, &path);
                let item_id = MediaBinItemId::for_watched(id, &rel);
                if !seen.insert(item_id) {
                    ctx.warn(
                        Category::Other,
                        format!(
                            "smart media bin \"{}\": duplicate cached item \"{rel}\" skipped",
                            p.name
                        ),
                    );
                    continue;
                }
                last_scan.push(MediaBinItem {
                    id: item_id,
                    name: item_name(ctx, e.name, trigger.asset),
                    enabled: e.cue.is_enabled,
                    trigger,
                });
            }
            let import_behavior = trigger_defaults(ctx, behavior, &last_scan);
            MediaBinKind::Watched {
                folder: WatchedFolder {
                    root,
                    path,
                    recursive: false,
                },
                import_behavior,
                last_scan,
                scanned_at: None,
            }
        }
        None => {
            let mut seen = BTreeSet::new();
            let mut items = Vec::new();
            for (i, (e, trigger)) in triggers.into_iter().enumerate() {
                let fallback = || {
                    MediaBinItemId::from_uuid(ids::derived(
                        "mediaBinItem",
                        &[&id.to_string(), &i.to_string()],
                    ))
                };
                let mut item_id = ids::parse_uuid(e.uuid)
                    .map(MediaBinItemId::from_uuid)
                    .unwrap_or_else(fallback);
                if !seen.insert(item_id) {
                    item_id = fallback();
                    seen.insert(item_id);
                }
                items.push(MediaBinItem {
                    id: item_id,
                    name: item_name(ctx, e.name, trigger.asset),
                    enabled: e.cue.is_enabled,
                    trigger,
                });
            }
            MediaBinKind::Manual { items }
        }
    };

    MediaBin {
        id,
        name: p.name.clone(),
        folder,
        sort_key,
        browser,
        kind,
        import: info,
        ..MediaBin::default()
    }
}

/// The root and folder path of a smart directory URL (backup or external volume).
fn folder_location(ctx: &mut ImportContext, url: &pp::Url) -> Option<(MediaRootId, String)> {
    match target(ctx, url, &AssetScope::Library) {
        Target::Show(path) => Some((media_root_for_backup(ctx), path)),
        Target::Volume(hint, path) => Some((media_root_for_volume(ctx, &hint), path)),
        Target::Theme { .. } | Target::Unresolved(_) => None,
    }
}

/// An asset's path relative to a watched folder (its full root path when it lies elsewhere).
fn path_in_folder(
    ctx: &ImportContext,
    asset: MediaAssetId,
    root: MediaRootId,
    folder: &str,
) -> String {
    let Some(a) = ctx
        .lookups
        .media_assets
        .get(&asset)
        .and_then(|&i| ctx.out.media_assets.get(i))
    else {
        return asset.to_string();
    };
    match &a.location {
        AssetLocation::Linked { root: r, path } if *r == root => {
            if folder.is_empty() {
                return path.clone();
            }
            match (path.get(..folder.len()), path.get(folder.len()..)) {
                (Some(head), Some(rest)) if same_name(head, folder) && rest.starts_with('/') => {
                    rest[1..].to_string()
                }
                _ => path.clone(),
            }
        }
        AssetLocation::Linked { path, .. } | AssetLocation::ThemeAsset { path, .. } => path.clone(),
        _ => a.id.to_string(),
    }
}

/// The item's display name when it differs from the asset name.
fn item_name(ctx: &ImportContext, name: &str, asset: MediaAssetId) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let asset_name = ctx
        .lookups
        .media_assets
        .get(&asset)
        .and_then(|&i| ctx.out.media_assets.get(i))
        .map(|a| a.name.as_str());
    (asset_name != Some(name)).then(|| name.to_string())
}

/// Trigger defaults of a watched folder: the layer from `importBehavior`, and the scale mode /
/// video repeat mode when every cached item agrees.
fn trigger_defaults(ctx: &ImportContext, behavior: i32, items: &[MediaBinItem]) -> TriggerDefaults {
    use pp::playlist::folder_directory::ImportBehavior as IB;
    let mut d = TriggerDefaults {
        layer: match IB::try_from(behavior) {
            Ok(IB::Foreground) => MediaLayer::Foreground,
            _ => MediaLayer::Background,
        },
        ..TriggerDefaults::default()
    };
    let visual: Vec<&MediaTrigger> = items
        .iter()
        .map(|i| &i.trigger)
        .filter(|t| t.layer != MediaLayer::Audio)
        .collect();
    if let Some(first) = visual.first()
        && visual
            .iter()
            .all(|t| t.display.scale == first.display.scale)
    {
        d.display.scale = first.display.scale;
    }
    let kind_of = |t: &MediaTrigger| {
        ctx.lookups
            .media_assets
            .get(&t.asset)
            .and_then(|&i| ctx.out.media_assets.get(i))
            .map(|a| a.kind)
    };
    let videos: Vec<&MediaTrigger> = items
        .iter()
        .map(|i| &i.trigger)
        .filter(|t| kind_of(t) == Some(MediaKind::Video))
        .collect();
    if let Some(first) = videos.first()
        && videos
            .iter()
            .all(|t| t.playback.repeat == first.playback.repeat)
    {
        d.playback.repeat = first.playback.repeat;
    }
    d
}

fn playlist_has_content(p: &pp::Playlist) -> bool {
    use pp::playlist::ChildrenType;
    !p.cues.is_empty()
        || !p.children.is_empty()
        || match &p.children_type {
            Some(ChildrenType::Items(i)) => !i.items.is_empty(),
            Some(ChildrenType::Playlists(a)) => !a.playlists.is_empty(),
            None => false,
        }
}

/// The live-video playlist mirrors the configured video inputs; report anything else.
fn check_live_video_playlist(ctx: &mut ImportContext, p: &pp::Playlist) {
    use pp::action::ActionTypeData;
    use pp::playlist::ChildrenType;
    use pp::playlist_item::ItemType;
    let mut cues: Vec<(&str, &pp::Cue)> = p.cues.iter().map(|c| (c.name.as_str(), c)).collect();
    match &p.children_type {
        Some(ChildrenType::Items(items)) => {
            for item in &items.items {
                match &item.item_type {
                    Some(ItemType::Cue(c)) => cues.push((&item.name, c)),
                    _ => ctx.warn(
                        Category::NotImported,
                        format!(
                            "live video playlist: item \"{}\" is not a live video cue",
                            item.name
                        ),
                    ),
                }
            }
        }
        Some(ChildrenType::Playlists(a)) if !a.playlists.is_empty() => ctx.warn(
            Category::NotImported,
            "live video playlist has sub-playlists; not imported",
        ),
        _ => {}
    }
    for (name, cue) in cues {
        let live_index = cue.actions.iter().find_map(|a| match &a.action_type_data {
            Some(ActionTypeData::Media(m)) => {
                match m.element.as_ref().and_then(|e| e.type_properties.as_ref()) {
                    Some(pp::media::TypeProperties::LiveVideo(l)) => Some(
                        l.live_video
                            .as_ref()
                            .map(|v| v.live_video_index)
                            .unwrap_or(0),
                    ),
                    _ => None,
                }
            }
            _ => None,
        });
        match live_index {
            Some(i) if ctx.lookups.video_input_by_index(i).is_some() => {}
            Some(i) => ctx.warn(
                Category::NotImported,
                format!(
                    "live video playlist: item \"{name}\" uses input index {i}, which is not a \
                     configured video input"
                ),
            ),
            None => ctx.warn(
                Category::NotImported,
                format!(
                    "live video playlist: item \"{name}\" is not a live video cue; not imported"
                ),
            ),
        }
    }
}

// ---- private: strings and paths -----------------------------------------------------------------

/// Normalized `/`-separated path; `None` when `..` escapes the root.
fn clean_path(p: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s),
        }
    }
    Some(out.join("/"))
}

/// The absolute source string up to (not including) the trailing relative path.
fn absolute_prefix(abs: &str, rel: &str) -> Option<String> {
    let abs = abs.trim();
    if rel.is_empty() {
        return non_empty(abs.trim_end_matches(['\\', '/']));
    }
    let cut = abs.len().checked_sub(rel.len())?;
    if !abs.is_char_boundary(cut) {
        return None;
    }
    let (head, tail) = abs.split_at(cut);
    if !same_name(&tail.replace('\\', "/"), rel)
        || !(head.is_empty() || head.ends_with(['\\', '/']))
    {
        return None;
    }
    non_empty(head.trim_end_matches(['\\', '/']))
}

fn absolute_string(url: &pp::Url) -> Option<&str> {
    match &url.storage {
        Some(pp::url::Storage::AbsoluteString(s)) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

/// [`absolute_string`] as a path: a `file://` URL decoded (`source::file_url_path`).
fn absolute_path(url: &pp::Url) -> Option<std::borrow::Cow<'_, str>> {
    absolute_string(url).map(crate::source::file_url_path)
}

/// The relative path a URL names (local / external path, else the stored string).
fn url_path(url: &pp::Url) -> Option<String> {
    use pp::url::{RelativeFilePath as R, Storage};
    match (&url.relative_file_path, &url.storage) {
        (Some(R::Local(l)), _) => Some(l.path.clone()),
        (Some(R::External(e)), _) => Some(e.path.clone()),
        (None, Some(Storage::AbsoluteString(s) | Storage::RelativePath(s))) => Some(s.clone()),
        _ => None,
    }
}

fn root_name_of(root: i32) -> String {
    UrlRoot::try_from(root)
        .map(|r| r.as_str_name().to_string())
        .unwrap_or_else(|_| format!("ROOT_{root}"))
}

/// The most specific textual reference of a URL (unresolved asset key).
fn url_reference(url: &pp::Url) -> String {
    use pp::url::RelativeFilePath as R;
    if let Some(abs) = absolute_string(url) {
        return abs.to_string();
    }
    match &url.relative_file_path {
        Some(R::Local(l)) => format!("{}/{}", root_name_of(l.root), l.path),
        Some(R::External(e)) => {
            let vol = e
                .win32
                .as_ref()
                .map(|w| format!("{}:{}", w.drive_letter, w.volume_name))
                .or_else(|| e.macos.as_ref().map(|m| m.volume_name.clone()))
                .unwrap_or_default();
            format!("{vol}/{}", e.path)
        }
        None => match &url.storage {
            Some(pp::url::Storage::RelativePath(s)) => s.clone(),
            _ => String::new(),
        },
    }
}

/// The best reference for the report of an unresolved asset.
fn best_reference(o: &MediaOrigin) -> String {
    if let Some(a) = &o.absolute_hint {
        return a.clone();
    }
    match (&o.root_kind, &o.relative_path) {
        (Some(k), Some(p)) => format!("{k}/{p}"),
        (None, Some(p)) => p.clone(),
        _ => String::new(),
    }
}

fn platform(p: i32) -> SourcePlatform {
    match pp::url::Platform::try_from(p) {
        Ok(pp::url::Platform::Win32) => SourcePlatform::Windows,
        Ok(pp::url::Platform::Macos) => SourcePlatform::Macos,
        _ => SourcePlatform::Unknown,
    }
}

fn drive_letter(s: &str) -> Option<String> {
    let d = s.trim().trim_end_matches(['\\', '/']).trim_end_matches(':');
    (!d.is_empty()).then(|| d.to_uppercase())
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// Case-insensitive name comparison (Windows / macOS default file systems).
fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn extension(path: &str) -> String {
    match file_name(path).rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn file_stem(path: &str) -> String {
    let name = file_name(path);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => name.to_string(),
    }
}

/// Media kind from a file extension (`Unknown` when not recognized).
fn kind_from_extension(path: &str) -> MediaKind {
    match extension(path).as_str() {
        "png" | "jpg" | "jpeg" | "jpe" | "jfif" | "gif" | "webp" | "bmp" | "tif" | "tiff"
        | "heic" | "heif" | "svg" | "tga" | "psd" => MediaKind::Image,
        "mp4" | "m4v" | "mov" | "avi" | "wmv" | "mkv" | "webm" | "mpg" | "mpeg" | "mts"
        | "m2ts" | "ts" | "flv" | "hevc" => MediaKind::Video,
        "mp3" | "m4a" | "wav" | "aac" | "aif" | "aiff" | "flac" | "ogg" | "wma" | "opus" => {
            MediaKind::Audio
        }
        _ => MediaKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic messages only (no backup data). Temporary backups live under
    //! `std::env::temp_dir()`.
    use super::*;
    use crate::ImportOptions;
    use crate::context::NamedId;
    use pp::url::external_relative_path::{MacOsExternalVolume, Win32ExternalVolume};
    use pp::url::{ExternalRelativePath, LocalRelativePath, RelativeFilePath, Storage};

    // ---- fixtures ---------------------------------------------------------------------------

    struct TempBackup(PathBuf);

    impl TempBackup {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("pp-media-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempBackup(p)
        }
        fn write(&self, rel: &str, bytes: &[u8]) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
        fn ctx(&self) -> ImportContext {
            ImportContext::new(&self.0, ImportOptions::default())
        }
    }

    impl Drop for TempBackup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn nowhere() -> ImportContext {
        ImportContext::new(
            Path::new("/nonexistent-pp-backup"),
            ImportOptions::default(),
        )
    }

    const SHOW: &str = "C:\\Users\\someone\\Documents\\ProPresenter";

    fn local(root: UrlRoot, path: &str, abs: Option<&str>) -> pp::Url {
        pp::Url {
            platform: pp::url::Platform::Win32 as i32,
            storage: abs.map(|a| Storage::AbsoluteString(a.into())),
            relative_file_path: Some(RelativeFilePath::Local(LocalRelativePath {
                root: root as i32,
                path: path.into(),
            })),
        }
    }

    fn show(path: &str) -> pp::Url {
        local(
            UrlRoot::Show,
            path,
            Some(&format!("{SHOW}\\{}", path.replace('/', "\\"))),
        )
    }

    fn external(drive: &str, volume: &str, path: &str) -> pp::Url {
        pp::Url {
            platform: pp::url::Platform::Win32 as i32,
            storage: Some(Storage::AbsoluteString(format!(
                "{drive}:\\{}",
                path.replace('/', "\\")
            ))),
            relative_file_path: Some(RelativeFilePath::External(ExternalRelativePath {
                win32: Some(Win32ExternalVolume {
                    drive_letter: drive.into(),
                    volume_name: volume.into(),
                    network_share: false,
                }),
                macos: None,
                path: path.into(),
            })),
        }
    }

    fn png(w: u32, h: u32, color_type: u8) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, color_type, 0, 0, 0]);
        b.extend([0; 8]);
        b
    }

    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
        b.extend(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        b.extend([0xff, 0xc4, 0x00, 0x04, 0x00, 0x00]); // DHT before SOF
        b.extend([0xff, 0xc0, 0x00, 0x11, 0x08]);
        b.extend(h.to_be_bytes());
        b.extend(w.to_be_bytes());
        b.extend([0x03, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        b.extend([0xff, 0xd9]);
        b
    }

    fn image(url: pp::Url, drawing: pp::media::DrawingProperties) -> pp::Media {
        pp::Media {
            url: Some(url),
            type_properties: Some(pp::media::TypeProperties::Image(
                pp::media::ImageTypeProperties {
                    drawing: Some(drawing),
                    file: None,
                },
            )),
            ..Default::default()
        }
    }

    fn video(
        url: pp::Url,
        transport: pp::media::TransportProperties,
        props: pp::media::VideoProperties,
        audio: Option<pp::media::AudioProperties>,
    ) -> pp::Media {
        pp::Media {
            url: Some(url),
            type_properties: Some(pp::media::TypeProperties::Video(
                pp::media::VideoTypeProperties {
                    drawing: Some(pp::media::DrawingProperties::default()),
                    audio,
                    transport: Some(transport),
                    video: Some(props),
                    file: None,
                },
            )),
            ..Default::default()
        }
    }

    fn action(
        media: pp::Media,
        layer: pp::action::LayerType,
    ) -> (pp::Action, pp::action::MediaType) {
        use pp::action::media_type::{Image, MediaType as A, Video};
        let oneof = match media.type_properties {
            Some(pp::media::TypeProperties::Video(_)) => Some(A::Video(Video::default())),
            Some(pp::media::TypeProperties::Image(_)) => Some(A::Image(Image {})),
            _ => None,
        };
        let m = pp::action::MediaType {
            element: Some(media),
            layer_type: layer as i32,
            media_type: oneof,
            ..Default::default()
        };
        let a = pp::Action {
            r#type: pp::action::ActionType::Media as i32,
            is_enabled: true,
            action_type_data: Some(pp::action::ActionTypeData::Media(m.clone())),
            ..Default::default()
        };
        (a, m)
    }

    fn asset(ctx: &ImportContext, id: MediaAssetId) -> &MediaAsset {
        &ctx.out.media_assets[ctx.lookups.media_assets[&id]]
    }

    fn root_of(ctx: &ImportContext, id: MediaAssetId) -> MediaRootId {
        match &asset(ctx, id).location {
            AssetLocation::Linked { root, .. } => *root,
            other => panic!("not linked: {other:?}"),
        }
    }

    fn variable(name: &str, t: pp::effect::effect_variable::Type) -> pp::effect::EffectVariable {
        pp::effect::EffectVariable {
            name: name.into(),
            r#type: Some(t),
            ..Default::default()
        }
    }

    fn float(v: f32) -> pp::effect::effect_variable::Type {
        pp::effect::effect_variable::Type::Float(pp::effect::effect_variable::EffectFloat {
            value: v,
            ..Default::default()
        })
    }

    // ---- assets -------------------------------------------------------------------------------

    #[test]
    fn show_files_link_to_the_backup_root() {
        let b = TempBackup::new("show");
        b.write("Media/Assets/Sunrise.png", &png(4, 3, 6));
        let mut ctx = b.ctx();
        let lib = AssetScope::Library;

        let a = asset_for_url(
            &mut ctx,
            &show("Media/Assets/Sunrise.png"),
            MediaKind::Image,
            &lib,
        );
        let backup = ids::media_root_id("backup");
        assert_eq!(
            a,
            MediaAssetId::for_linked(backup, "Media/Assets/Sunrise.png")
        );
        let x = asset(&ctx, a);
        assert_eq!(x.status, MediaStatus::Online);
        assert_eq!(x.name, "Sunrise");
        assert_eq!(x.kind, MediaKind::Image);
        assert_eq!(x.scope, AssetScope::Library);
        let p = x.probe.as_ref().unwrap();
        assert_eq!(
            (p.width, p.height, p.has_alpha),
            (Some(4), Some(3), Some(true))
        );
        assert_eq!(p.codec.as_deref(), Some("png"));
        assert!(!p.from_import_hints);
        let o = x.origin.as_ref().unwrap();
        assert_eq!(o.platform, SourcePlatform::Windows);
        assert_eq!(o.root_kind.as_deref(), Some("ROOT_SHOW"));
        assert_eq!(o.relative_path.as_deref(), Some("Media/Assets/Sunrise.png"));

        // Case-insensitive match on disk → the same asset (on-disk spelling).
        let b2 = asset_for_url(
            &mut ctx,
            &show("media/assets/SUNRISE.PNG"),
            MediaKind::Image,
            &lib,
        );
        assert_eq!(a, b2);
        assert_eq!(ctx.lookups.asset_references[&a], 2);

        // A missing show file stays linked to the backup root, de-duplicated by case.
        let m1 = asset_for_url(
            &mut ctx,
            &show("Media/Assets/Gone.jpg"),
            MediaKind::Image,
            &lib,
        );
        let m2 = asset_for_url(
            &mut ctx,
            &show("Media/Assets/GONE.JPG"),
            MediaKind::Image,
            &lib,
        );
        assert_eq!(m1, m2);
        assert_eq!(asset(&ctx, m1).status, MediaStatus::Missing);
        assert_eq!(root_of(&ctx, m1), backup);
        assert_eq!(ctx.out.media_assets.len(), 2);

        // The backup root remembers the source show folder.
        assert_eq!(ctx.out.media_roots.len(), 1);
        assert_eq!(ctx.out.media_roots[0].name, BACKUP_ROOT_NAME);
        assert_eq!(
            ctx.out.media_roots[0].hints[0].path_prefix.as_deref(),
            Some(SHOW)
        );
    }

    #[test]
    fn unreadable_images_are_unsupported_and_probing_can_be_disabled() {
        let b = TempBackup::new("unsupported");
        b.write("Media/Assets/Broken.jpg", b"not a jpeg");
        b.write("Media/Assets/Fine.png", &png(2, 2, 2));
        let mut ctx = b.ctx();
        let a = asset_for_url(
            &mut ctx,
            &show("Media/Assets/Broken.jpg"),
            MediaKind::Image,
            &AssetScope::Library,
        );
        assert_eq!(asset(&ctx, a).status, MediaStatus::Unsupported);

        let mut ctx = ImportContext::new(
            &b.0,
            ImportOptions {
                probe_media: false,
                imported_at: Some("2026-01-01T00:00:00Z".into()),
            },
        );
        let a = asset_for_url(
            &mut ctx,
            &show("Media/Assets/Fine.png"),
            MediaKind::Image,
            &AssetScope::Library,
        );
        let x = asset(&ctx, a);
        assert_eq!(x.status, MediaStatus::Online);
        assert!(x.probe.is_none());
        assert_eq!(x.checked_at.as_deref(), Some("2026-01-01T00:00:00Z"));
    }

    #[test]
    fn volume_roots_merge_aliases_on_evidence() {
        let mut ctx = nowhere();
        let lib = AssetScope::Library;
        let id = |ctx: &mut ImportContext, url: pp::Url| {
            asset_for_url(ctx, &url, MediaKind::Image, &lib)
        };
        let ip = id(
            &mut ctx,
            external("Z", "\\\\10.0.0.5\\Media", "Backgrounds/A.jpg"),
        );
        let fqdn = id(
            &mut ctx,
            external("Z", "\\\\nas.example.local\\Media", "Backgrounds/B.jpg"),
        );
        let short = id(
            &mut ctx,
            external("Y", "\\\\nas\\media", "Backgrounds/C.jpg"),
        );
        let other_share = id(&mut ctx, external("Z", "\\\\nas\\Other", "D.jpg"));
        let label_e = id(&mut ctx, external("E", "Shared Media", "E.jpg"));
        let label_f = id(&mut ctx, external("F", "shared media", "F.jpg"));
        let cloud = id(&mut ctx, external("G", "Cloud Drive", "G.jpg"));

        // Same share on the same drive letter (IP vs name), and short name vs FQDN.
        assert_eq!(root_of(&ctx, ip), root_of(&ctx, fqdn));
        assert_eq!(root_of(&ctx, fqdn), root_of(&ctx, short));
        // Different share names never merge.
        assert_ne!(root_of(&ctx, ip), root_of(&ctx, other_share));
        // Same label on different drive letters merges; other labels do not.
        assert_eq!(root_of(&ctx, label_e), root_of(&ctx, label_f));
        assert_ne!(root_of(&ctx, label_e), root_of(&ctx, cloud));
        assert_eq!(ctx.out.media_roots.len(), 4);

        let nas = ctx
            .out
            .media_roots
            .iter()
            .find(|r| r.id == root_of(&ctx, ip))
            .unwrap();
        assert_eq!(nas.hints.len(), 3);
        assert_eq!(nas.name, "\\\\10.0.0.5\\Media (Z:)");
        assert_eq!(nas.id, ids::media_root_id("unc:10.0.0.5/media"));
        assert_eq!(
            nas.hints[0].unc_prefix.as_deref(),
            Some("\\\\10.0.0.5\\Media")
        );
        assert_eq!(nas.hints[0].drive_letter.as_deref(), Some("Z"));
        for a in [ip, fqdn, short, other_share, label_e, label_f, cloud] {
            assert_eq!(asset(&ctx, a).status, MediaStatus::Missing);
        }
        let o = asset(&ctx, ip).origin.clone().unwrap();
        assert_eq!(o.drive_letter.as_deref(), Some("Z"));
        assert_eq!(o.unc_share.as_deref(), Some("\\\\10.0.0.5\\Media"));
        assert_eq!(o.absolute_hint.as_deref(), Some("Z:\\Backgrounds\\A.jpg"));

        // Re-importing gives the same IDs.
        let mut again = nowhere();
        let ip2 = asset_for_url(
            &mut again,
            &external("Z", "\\\\10.0.0.5\\Media", "Backgrounds/A.jpg"),
            MediaKind::Image,
            &lib,
        );
        assert_eq!(ip, ip2);
    }

    #[test]
    fn macos_volumes_and_absolute_only_urls() {
        let mut ctx = nowhere();
        let mac = pp::Url {
            platform: pp::url::Platform::Macos as i32,
            storage: None,
            relative_file_path: Some(RelativeFilePath::External(ExternalRelativePath {
                macos: Some(MacOsExternalVolume {
                    volume_name: "Shared Media".into(),
                }),
                win32: None,
                path: "Loops/Clouds.mov".into(),
            })),
        };
        let a = asset_for_url(&mut ctx, &mac, MediaKind::Video, &AssetScope::Library);
        let b = asset_for_url(
            &mut ctx,
            &external("E", "Shared Media", "Loops/Other.mov"),
            MediaKind::Video,
            &AssetScope::Library,
        );
        assert_eq!(root_of(&ctx, a), root_of(&ctx, b));
        assert_eq!(
            asset(&ctx, a).origin.as_ref().unwrap().platform,
            SourcePlatform::Macos
        );

        let abs_only = pp::Url {
            platform: pp::url::Platform::Win32 as i32,
            storage: Some(Storage::AbsoluteString(
                "\\\\nas\\media\\Backgrounds\\X.jpg".into(),
            )),
            relative_file_path: None,
        };
        let x = asset_for_url(&mut ctx, &abs_only, MediaKind::Image, &AssetScope::Library);
        match &asset(&ctx, x).location {
            AssetLocation::Linked { path, .. } => assert_eq!(path, "Backgrounds/X.jpg"),
            other => panic!("{other:?}"),
        }

        // The same as a file URL, percent-encoded (A10-3): the same volume-linked path.
        let url_only = pp::Url {
            platform: pp::url::Platform::Win32 as i32,
            storage: Some(Storage::AbsoluteString(
                "file:///D:/Media/Sunday%20Loops/Rain.mov".into(),
            )),
            relative_file_path: None,
        };
        let y = asset_for_url(&mut ctx, &url_only, MediaKind::Video, &AssetScope::Library);
        match &asset(&ctx, y).location {
            AssetLocation::Linked { path, .. } => assert_eq!(path, "Media/Sunday Loops/Rain.mov"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn theme_resources_are_theme_assets() {
        let b = TempBackup::new("theme");
        b.write("Themes/Example/Assets/Loop.mp4", b"....");
        let mut ctx = b.ctx();
        let theme = ids::theme_id("Themes/Example");
        ctx.current.path = Some("Themes/Example/Theme".into());
        ctx.current.resource_dir = Some("Themes/Example".into());
        ctx.current.asset_scope = AssetScope::Theme { theme };
        let scope = ctx.current.asset_scope.clone();

        let url = local(UrlRoot::CurrentResource, "Assets/Loop.mp4", None);
        let a = asset_for_url(&mut ctx, &url, MediaKind::Video, &scope);
        assert_eq!(a, ids::theme_asset_id(theme, "Assets/Loop.mp4"));
        let x = asset(&ctx, a);
        assert_eq!(x.status, MediaStatus::Online);
        assert_eq!(x.scope, AssetScope::Theme { theme });
        assert_eq!(
            x.location,
            AssetLocation::ThemeAsset {
                theme,
                path: "Assets/Loop.mp4".into()
            }
        );
        let gone = asset_for_url(
            &mut ctx,
            &local(UrlRoot::CurrentResource, "Assets/Gone.mp4", None),
            MediaKind::Video,
            &scope,
        );
        assert_eq!(asset(&ctx, gone).status, MediaStatus::Missing);

        // A show-relative reference from a theme stays a library asset.
        let shared = asset_for_url(
            &mut ctx,
            &show("Media/Assets/Shared.jpg"),
            MediaKind::Image,
            &scope,
        );
        assert_eq!(asset(&ctx, shared).scope, AssetScope::Library);

        // Outside a theme, the current resource is the document's folder.
        let mut ctx = b.ctx();
        ctx.current.path = Some("Libraries/Songs/Song.pro".into());
        let a = asset_for_url(
            &mut ctx,
            &local(UrlRoot::CurrentResource, "Assets/X.jpg", None),
            MediaKind::Image,
            &AssetScope::Library,
        );
        match &asset(&ctx, a).location {
            AssetLocation::Linked { path, .. } => assert_eq!(path, "Libraries/Songs/Assets/X.jpg"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn other_roots_and_escaping_paths_are_unresolved() {
        let b = TempBackup::new("escape");
        let mut ctx = b.ctx();
        let home = local(
            UrlRoot::UserHome,
            "AppData/Local/Temp/x.jpg",
            Some("C:\\Users\\someone\\AppData\\Local\\Temp\\x.jpg"),
        );
        let a = asset_for_url(&mut ctx, &home, MediaKind::Image, &AssetScope::Library);
        assert_eq!(
            a,
            ids::unresolved_asset_id("C:\\Users\\someone\\AppData\\Local\\Temp\\x.jpg")
        );
        let x = asset(&ctx, a);
        assert_eq!(x.location, AssetLocation::Unresolved);
        assert_eq!(x.status, MediaStatus::Missing);
        assert_eq!(x.name, "x");

        let escape = local(UrlRoot::Show, "../../etc/passwd", None);
        let e = asset_for_url(&mut ctx, &escape, MediaKind::Image, &AssetScope::Library);
        assert_eq!(asset(&ctx, e).location, AssetLocation::Unresolved);

        // A media element without a URL has no asset.
        assert!(asset_for_media(&mut ctx, &pp::Media::default(), &AssetScope::Library).is_none());
        let empty_url = pp::Media {
            url: Some(pp::Url::default()),
            ..Default::default()
        };
        assert!(asset_for_media(&mut ctx, &empty_url, &AssetScope::Library).is_none());
    }

    // ---- triggers -----------------------------------------------------------------------------

    #[test]
    fn video_trigger_keeps_playback_and_audio_settings() {
        use pp::media::transport_properties::{PlaybackBehavior as PB, RetriggerSetting as RS};
        let mut ctx = nowhere();
        let transport = pp::media::TransportProperties {
            play_rate: 1.0,
            out_point: 120.0,
            end_point: 120.0,
            should_fade_in: true,
            should_fade_out: true,
            playback_behavior: PB::Loop as i32,
            times_to_loop: 1,
            loop_time: 1.0,
            retrigger: RS::Unset as i32,
            ..Default::default()
        };
        let audio = pp::media::AudioProperties {
            volume: 1.0,
            audio_channels: vec![
                pp::media::audio::Channel {
                    index: 0,
                    outputs: vec![pp::media::audio::channel::Output { channel_index: 0 }],
                    ..Default::default()
                },
                pp::media::audio::Channel {
                    index: 1,
                    volume: -12.0,
                    outputs: vec![pp::media::audio::channel::Output { channel_index: 1 }],
                    ..Default::default()
                },
                pp::media::audio::Channel {
                    index: 2,
                    ..Default::default()
                },
            ],
            is_custom_mapping: false,
        };
        let props = pp::media::VideoProperties {
            frame_rate: 29.97,
            ..Default::default()
        };
        let media = video(
            external("Z", "\\\\nas\\media", "Videos/Intro.mp4"),
            transport,
            props,
            Some(audio),
        );
        let (a, m) = action(media, pp::action::LayerType::Foreground);
        let ActionKind::Media(t) = media_action(&mut ctx, &a, &m) else {
            panic!("not media")
        };
        assert_eq!(t.layer, MediaLayer::Foreground);
        let fade = Fade {
            enabled: true,
            seconds: None,
        };
        assert_eq!(t.playback.fade_in, fade);
        assert_eq!(t.playback.fade_out, fade);
        assert_eq!(t.playback.retrigger, Retrigger::Automatic);
        assert_eq!(t.playback.repeat, Repeat::Loop);
        assert_eq!(t.playback.out_point, None);
        assert_eq!(t.playback.rate, 1.0);
        assert_eq!(t.audio.volume_db, 0.0);
        assert_eq!(
            t.audio.channels,
            vec![
                ChannelRoute {
                    source: 0,
                    dest: 0,
                    gain_db: 0.0,
                    muted: false
                },
                ChannelRoute {
                    source: 1,
                    dest: 1,
                    gain_db: -12.0,
                    muted: false
                },
                ChannelRoute {
                    source: 2,
                    dest: 2,
                    gain_db: 0.0,
                    muted: true
                },
            ]
        );
        let probe = asset(&ctx, t.asset).probe.clone().unwrap();
        assert!(probe.from_import_hints);
        assert_eq!(probe.frame_rate, Some(FrameRate::FPS29_97));
        assert_eq!(probe.duration, Some(120.0));
        assert_eq!(asset(&ctx, t.asset).kind, MediaKind::Video);
    }

    #[test]
    fn playback_edge_cases() {
        use pp::media::transport_properties::{PlaybackBehavior as PB, RetriggerSetting as RS};
        let t = pp::media::TransportProperties {
            out_point: 351.8,
            end_point: 800.0,
            fade_in_duration: 2.0,
            should_fade_in: false,
            playback_behavior: PB::LoopForCount as i32,
            times_to_loop: 3,
            retrigger: RS::Never as i32,
            ..Default::default()
        };
        let v = pp::media::VideoProperties {
            end_behavior: pp::media::video_properties::EndBehavior::FadeToBlack as i32,
            soft_loop: true,
            ..Default::default()
        };
        let p = playback(Some(&t), Some(&v));
        assert_eq!(p.out_point, Some(351.8));
        assert_eq!(p.rate, 1.0);
        assert_eq!(
            p.fade_in,
            Fade {
                enabled: false,
                seconds: Some(2.0)
            }
        );
        assert_eq!(p.repeat, Repeat::LoopCount { count: 3 });
        assert_eq!(p.retrigger, Retrigger::Never);
        assert_eq!(p.end_behavior, EndBehavior::FadeToBlack);
        assert_eq!(p.soft_loop, Some(DEFAULT_SOFT_LOOP));
        let lf = pp::media::TransportProperties {
            playback_behavior: PB::LoopForTime as i32,
            loop_time: 30.0,
            ..Default::default()
        };
        assert_eq!(
            playback(Some(&lf), None).repeat,
            Repeat::LoopFor { seconds: 30.0 }
        );
        assert_eq!(playback(None, None), Playback::default());
        // Out-of-range enum values never panic.
        let odd = pp::media::TransportProperties {
            playback_behavior: 99,
            retrigger: 42,
            ..Default::default()
        };
        assert_eq!(playback(Some(&odd), None).repeat, Repeat::Once);
        assert!(
            (audio_settings(Some(&pp::media::AudioProperties {
                volume: 0.5,
                ..Default::default()
            }))
            .volume_db
                + 6.0206)
                .abs()
                < 1e-3
        );
    }

    #[test]
    fn audio_and_images() {
        let mut ctx = nowhere();
        let audio = pp::Media {
            url: Some(show("Media/Assets/Walk In.mp3")),
            type_properties: Some(pp::media::TypeProperties::Audio(
                pp::media::AudioTypeProperties {
                    audio: Some(pp::media::AudioProperties {
                        volume: 1.0,
                        ..Default::default()
                    }),
                    transport: Some(pp::media::TransportProperties {
                        should_fade_in: true,
                        ..Default::default()
                    }),
                    file: None,
                },
            )),
            ..Default::default()
        };
        let (a, m) = action(audio, pp::action::LayerType::Background);
        let ActionKind::Media(t) = media_action(&mut ctx, &a, &m) else {
            panic!()
        };
        assert_eq!(t.layer, MediaLayer::Audio);
        assert_eq!(t.display, MediaDisplay::default());
        assert!(t.playback.fade_in.enabled);
        assert_eq!(asset(&ctx, t.asset).kind, MediaKind::Audio);

        let img = image(
            show("Media/Assets/Still.jpg"),
            pp::media::DrawingProperties {
                scale_behavior: pp::media::ScaleBehavior::Fill as i32,
                natural_size: Some(pp::graphics::Size {
                    width: 1920.0,
                    height: 1080.0,
                }),
                alpha_type: pp::AlphaType::Straight as i32,
                ..Default::default()
            },
        );
        let (a, m) = action(img, pp::action::LayerType::Background);
        let ActionKind::Media(t) = media_action(&mut ctx, &a, &m) else {
            panic!()
        };
        assert_eq!(t.layer, MediaLayer::Background);
        assert_eq!(t.display.scale, ScaleMode::Fill);
        assert_eq!(t.display.alpha, SourceAlpha::Straight);
        assert_eq!(t.playback, Playback::default());
        let probe = asset(&ctx, t.asset).probe.clone().unwrap();
        assert_eq!((probe.width, probe.height), (Some(1920), Some(1080)));
        assert!(probe.from_import_hints);
    }

    #[test]
    fn display_settings() {
        let mut ctx = nowhere();
        let d = pp::media::DrawingProperties {
            scale_behavior: pp::media::ScaleBehavior::Stretch as i32,
            scale_alignment: pp::media::ScaleAlignment::TopLeft as i32,
            flipped_horizontally: true,
            native_rotation: pp::media::drawing_properties::NativeRotationType::Rotate90 as i32,
            crop_enable: true,
            crop_insets: Some(pp::graphics::EdgeInsets {
                left: 10.0,
                right: 0.0,
                top: 5.0,
                bottom: 0.0,
            }),
            alpha_type: pp::AlphaType::Premultiplied as i32,
            selected_effect_preset_uuid: Some(pp::Uuid {
                string: "00000000-0000-0000-0000-000000000000".into(),
            }),
            ..Default::default()
        };
        let m = display(&mut ctx, Some(&d));
        assert_eq!(m.scale, ScaleMode::Stretch);
        assert_eq!(m.anchor, Anchor::TopLeft);
        assert!(m.flip.horizontal && !m.flip.vertical);
        assert_eq!(m.rotation, QuarterTurn::Cw90);
        assert_eq!((m.crop.left, m.crop.top), (10.0, 5.0));
        assert_eq!(m.alpha, SourceAlpha::Premultiplied);
        assert!(
            ctx.report.unmapped_features.is_empty(),
            "nil preset UUID is not data"
        );

        let disabled = pp::media::DrawingProperties {
            crop_insets: Some(pp::graphics::EdgeInsets {
                left: 3.0,
                ..Default::default()
            }),
            is_blurred: true,
            scale_behavior: pp::media::ScaleBehavior::Custom as i32,
            ..Default::default()
        };
        let m = display(&mut ctx, Some(&disabled));
        assert_eq!(m.crop, model::EdgeInsets::default());
        assert_eq!(m.scale, ScaleMode::None);
        assert_eq!(
            ctx.report.unmapped_features["media.drawing.cropInsets.disabled"],
            1
        );
        assert_eq!(ctx.report.unmapped_features["media.drawing.isBlurred"], 1);
    }

    #[test]
    fn effects_map_to_typed_kinds() {
        use pp::effect::effect_variable::{EffectColor, Type};
        let mut ctx = nowhere();
        let adjust = pp::Effect {
            enabled: true,
            name: "Adjust Color".into(),
            variables: vec![
                variable("Hue", float(std::f32::consts::FRAC_PI_2)),
                variable("Saturation", float(0.0)),
                variable("Contrast", float(1.5)),
            ],
            ..Default::default()
        };
        let e = effect(&mut ctx, &adjust);
        assert!(e.enabled);
        let EffectKind::ColorAdjust {
            brightness,
            contrast,
            saturation,
            hue,
        } = e.kind
        else {
            panic!("{:?}", e.kind)
        };
        assert_eq!((brightness, contrast, saturation), (0.0, 1.5, 0.0));
        assert!((hue - 90.0).abs() < 1e-4);

        let two = pp::Effect {
            enabled: true,
            render_id: TWO_COLOR_ID.into(),
            variables: vec![variable(
                "Dark Color",
                Type::Color(EffectColor {
                    color: Some(pp::Color {
                        red: 0.0,
                        green: 0.5,
                        blue: 1.0,
                        alpha: 1.0,
                    }),
                    default_color: None,
                }),
            )],
            ..Default::default()
        };
        let EffectKind::TwoColor { dark, bright } = effect(&mut ctx, &two).kind else {
            panic!()
        };
        assert_eq!(dark, Color::rgba(0.0, 0.5, 1.0, 1.0));
        assert_eq!(bright, Color::WHITE);

        let alpha = pp::Effect {
            name: "Alpha Adjust".into(),
            variables: vec![variable("Amount", float(0.5))],
            ..Default::default()
        };
        let e = effect(&mut ctx, &alpha);
        assert!(!e.enabled);
        assert_eq!(e.kind, EffectKind::AlphaAdjust { amount: 0.5 });

        let other = pp::Effect {
            enabled: true,
            name: "Example Glow".into(),
            render_id: "ABC".into(),
            ..Default::default()
        };
        let EffectKind::Vendor(v) = effect(&mut ctx, &other).kind else {
            panic!()
        };
        assert_eq!(v.vendor_id.as_deref(), Some("ABC"));
    }

    #[test]
    fn live_video_becomes_a_video_input_trigger() {
        let mut ctx = nowhere();
        for i in 0..3 {
            ctx.lookups.video_inputs.push(NamedId {
                id: VideoInputId::from_uuid(ids::derived("test", &[&i.to_string()])),
                name: format!("Camera {i}"),
            });
        }
        let live = |index: i32| pp::action::MediaType {
            element: Some(pp::Media {
                type_properties: Some(pp::media::TypeProperties::LiveVideo(
                    pp::media::LiveVideoTypeProperties {
                        live_video: Some(pp::media::LiveVideoProperties {
                            live_video_index: index,
                            ..Default::default()
                        }),
                        audio: Some(pp::media::AudioProperties {
                            volume: 1.0,
                            ..Default::default()
                        }),
                        drawing: None,
                    },
                )),
                ..Default::default()
            }),
            layer_type: pp::action::LayerType::Input as i32,
            media_type: Some(pp::action::media_type::MediaType::LiveVideo(
                Default::default(),
            )),
            ..Default::default()
        };
        let m = live(2);
        assert!(media_trigger(&mut ctx, &m, &AssetScope::Library).is_none());
        let a = pp::Action::default();
        let ActionKind::VideoInput(t) = media_action(&mut ctx, &a, &m) else {
            panic!()
        };
        assert_eq!(t.input.id, ctx.lookups.video_inputs[2].id);
        assert_eq!(t.input.name.as_deref(), Some("Camera 2"));
        assert!(ctx.report.warnings.is_empty());

        let t = video_input_trigger(&mut ctx, &live(7)).unwrap();
        assert_eq!(t.input.name.as_deref(), Some("Input 8"));
        assert_eq!(ctx.report.warnings.len(), 1);
        assert!(ctx.out.media_assets.is_empty());
    }

    /// A live-video element fill becomes a live-source fill naming the input (and the
    /// device as the source when it is a network source name).
    #[test]
    fn live_video_fills_name_the_input() {
        let mut ctx = nowhere();
        ctx.lookups.video_inputs.push(NamedId {
            id: VideoInputId::from_uuid(ids::derived("test", &["0"])),
            name: "Camera 1".into(),
        });
        let fill = |device: &str| pp::Media {
            type_properties: Some(pp::media::TypeProperties::LiveVideo(
                pp::media::LiveVideoTypeProperties {
                    live_video: Some(pp::media::LiveVideoProperties {
                        live_video_index: 0,
                        video_device: Some(pp::media::VideoDevice {
                            name: device.into(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let FillKind::LiveSource { source, input, .. } =
            media_fill(&mut ctx, &fill("STUDIO-PC (Cam A)"), &AssetScope::Library)
        else {
            panic!()
        };
        assert_eq!(input, Some(ctx.lookups.video_inputs[0].id));
        assert_eq!(source, "STUDIO-PC (Cam A)");
        let FillKind::LiveSource { source, .. } =
            media_fill(&mut ctx, &fill("HD Pro Webcam C920"), &AssetScope::Library)
        else {
            panic!()
        };
        assert!(
            source.is_empty(),
            "a capture device is not a network source"
        );
        assert!(ctx.out.media_assets.is_empty());
    }

    #[test]
    fn web_content_and_empty_payloads_are_unsupported() {
        let mut ctx = nowhere();
        let web = pp::action::MediaType {
            element: Some(pp::Media {
                type_properties: Some(pp::media::TypeProperties::WebContent(Default::default())),
                ..Default::default()
            }),
            ..Default::default()
        };
        let a = pp::Action {
            r#type: pp::action::ActionType::Media as i32,
            ..Default::default()
        };
        assert!(
            matches!(media_action(&mut ctx, &a, &web), ActionKind::Unsupported { ref source_type, .. } if source_type == "ACTION_TYPE_MEDIA")
        );
        assert!(matches!(
            media_action(&mut ctx, &a, &pp::action::MediaType::default()),
            ActionKind::Unsupported { .. }
        ));
        assert_eq!(ctx.report.unsupported_actions["ACTION_TYPE_MEDIA"], 2);
    }

    #[test]
    fn media_fills() {
        let mut ctx = nowhere();
        let fill = image(
            show("Media/Import/slide.jpg"),
            pp::media::DrawingProperties {
                scale_behavior: pp::media::ScaleBehavior::Fill as i32,
                ..Default::default()
            },
        );
        let FillKind::Media {
            asset: a,
            display,
            playback,
            ..
        } = media_fill(&mut ctx, &fill, &AssetScope::Library)
        else {
            panic!()
        };
        assert_eq!(display.scale, ScaleMode::Fill);
        assert_eq!(playback, Playback::default());
        assert_eq!(asset(&ctx, a).status, MediaStatus::Missing);
        assert!(matches!(
            media_fill(&mut ctx, &pp::Media::default(), &AssetScope::Library),
            FillKind::Unsupported { .. }
        ));
    }

    // ---- probing ------------------------------------------------------------------------------

    #[test]
    fn header_sniffing() {
        let b = TempBackup::new("probe");
        let check = |name: &str, bytes: &[u8]| {
            b.write(name, bytes);
            probe_file(&b.0.join(name))
        };
        let p = check("a.png", &png(640, 480, 2)).unwrap();
        assert_eq!(
            (p.width, p.height, p.has_alpha),
            (Some(640), Some(480), Some(false))
        );
        let p = check("b.jpg", &jpeg(1920, 1080)).unwrap();
        assert_eq!((p.width, p.height), (Some(1920), Some(1080)));
        assert_eq!(p.codec.as_deref(), Some("jpeg"));
        let mut gif = b"GIF89a".to_vec();
        gif.extend([0x20, 0x03, 0x58, 0x02, 0, 0, 0]);
        let p = check("c.gif", &gif).unwrap();
        assert_eq!((p.width, p.height), (Some(800), Some(600)));

        let riff = |chunk: &[u8; 4], body: &[u8]| {
            let mut v = b"RIFF\0\0\0\0WEBP".to_vec();
            v.extend(chunk);
            v.extend([0, 0, 0, 0]);
            v.extend(body);
            v.resize(40, 0);
            v
        };
        // VP8X: alpha flag, canvas 1920 x 1080 (minus one, 24-bit LE).
        let mut x = vec![0x10, 0, 0, 0];
        x.extend([0x7f, 0x07, 0x00, 0x37, 0x04, 0x00]);
        let p = check("d.webp", &riff(b"VP8X", &x)).unwrap();
        assert_eq!(
            (p.width, p.height, p.has_alpha),
            (Some(1920), Some(1080), Some(true))
        );
        // VP8 (lossy): 3-byte frame tag, start code, 14-bit sizes.
        let mut l = vec![0, 0, 0, 0x9d, 0x01, 0x2a];
        l.extend(320u16.to_le_bytes());
        l.extend(240u16.to_le_bytes());
        let p = check("e.webp", &riff(b"VP8 ", &l)).unwrap();
        assert_eq!((p.width, p.height), (Some(320), Some(240)));
        // VP8L (lossless): 100 x 50 → (99, 49) packed in 14-bit fields, alpha bit set.
        let bits: u32 = 99 | (49 << 14) | (1 << 28);
        let mut ll = vec![0x2f];
        ll.extend(bits.to_le_bytes());
        let p = check("f.webp", &riff(b"VP8L", &ll)).unwrap();
        assert_eq!(
            (p.width, p.height, p.has_alpha),
            (Some(100), Some(50), Some(true))
        );

        assert!(check("g.jpg", b"\xff\xd8\xff\xd9").is_none());
        assert!(check("h.bin", b"").is_none());
        assert!(probe_file(&b.0.join("missing.png")).is_none());
    }

    #[test]
    fn frame_rates() {
        assert_eq!(frame_rate(30.0), Some(FrameRate::fps(30)));
        assert_eq!(frame_rate(29.97), Some(FrameRate::FPS29_97));
        assert_eq!(
            frame_rate(23.976),
            Some(FrameRate {
                num: 24000,
                den: 1001
            })
        );
        assert_eq!(frame_rate(59.94), Some(FrameRate::FPS59_94));
        assert_eq!(
            frame_rate(12.5),
            Some(FrameRate {
                num: 12500,
                den: 1000
            })
        );
        assert_eq!(frame_rate(0.0), None);
        assert_eq!(frame_rate(f64::NAN), None);
    }

    // ---- bins and finalize ----------------------------------------------------------------------

    fn uuid(n: u8) -> pp::Uuid {
        pp::Uuid {
            string: format!("00000000-0000-4000-8000-0000000000{n:02x}"),
        }
    }

    fn bin_item(n: u8, url: pp::Url, name: &str) -> pp::PlaylistItem {
        let media = image(
            url,
            pp::media::DrawingProperties {
                scale_behavior: pp::media::ScaleBehavior::Fill as i32,
                ..Default::default()
            },
        );
        let (a, _) = action(media, pp::action::LayerType::Background);
        pp::PlaylistItem {
            uuid: Some(uuid(n)),
            name: name.into(),
            item_type: Some(pp::playlist_item::ItemType::Cue(pp::Cue {
                uuid: Some(uuid(n)),
                is_enabled: true,
                actions: vec![a],
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn folder(n: u8, name: &str, children: Vec<pp::Playlist>) -> pp::Playlist {
        pp::Playlist {
            uuid: Some(uuid(n)),
            name: name.into(),
            r#type: pp::playlist::Type::Group as i32,
            children_type: Some(pp::playlist::ChildrenType::Playlists(
                pp::playlist::PlaylistArray {
                    playlists: children,
                },
            )),
            ..Default::default()
        }
    }

    fn items(n: u8, name: &str, items: Vec<pp::PlaylistItem>) -> pp::Playlist {
        pp::Playlist {
            uuid: Some(uuid(n)),
            name: name.into(),
            r#type: pp::playlist::Type::Playlist as i32,
            children_type: Some(pp::playlist::ChildrenType::Items(
                pp::playlist::PlaylistItems { items },
            )),
            ..Default::default()
        }
    }

    #[test]
    fn repeated_bin_uuid_gets_its_own_id() {
        // A copied bin can keep its source UUID (as playlists do). Both bins used to get that
        // UUID as their ID, so writing the bundle overwrote one with the other.
        let mut ctx = nowhere();
        let root = pp::Playlist {
            r#type: pp::playlist::Type::Root as i32,
            children_type: Some(pp::playlist::ChildrenType::Playlists(
                pp::playlist::PlaylistArray {
                    playlists: vec![
                        items(11, "Loops", vec![bin_item(23, show("A.jpg"), "A")]),
                        items(11, "Loops copy", vec![bin_item(24, show("B.jpg"), "B")]),
                    ],
                },
            )),
            ..Default::default()
        };
        let doc = pp::PlaylistDocument {
            r#type: pp::playlist_document::Type::Media as i32,
            root_node: Some(root),
            ..Default::default()
        };
        ctx.current.path = Some("Playlists/Media".into());
        import_media_bins(&mut ctx, &doc, "Playlists/Media");
        let bins = &ctx.out.media_bins;
        assert_eq!(bins.len(), 2);
        assert_eq!(bins[0].id.0.to_string(), uuid(11).string);
        assert_ne!(bins[0].id, bins[1].id);
        assert_eq!(
            bins[1].import.as_ref().unwrap().original_id.as_deref(),
            Some(uuid(11).string.as_str())
        );
    }

    #[test]
    fn media_bins_from_a_playlist_document() {
        let mut ctx = nowhere();
        let vol = "\\\\nas\\media";
        let mut smart = items(
            10,
            "Nature",
            vec![
                bin_item(
                    20,
                    external("Z", vol, "Backgrounds/Nature/Lake.jpg"),
                    "Lake",
                ),
                bin_item(
                    21,
                    external("Z", vol, "Backgrounds/Nature/Hills.jpg"),
                    "Rolling hills",
                ),
                bin_item(
                    22,
                    external("Z", vol, "backgrounds/nature/lake.JPG"),
                    "Lake",
                ),
            ],
        );
        smart.link_data = Some(pp::playlist::LinkData::SmartDirectory(
            pp::playlist::FolderDirectory {
                smart_directory: Some(external("Z", vol, "Backgrounds/Nature")),
                import_behavior: 0,
            },
        ));
        let manual = items(
            11,
            "Favorites",
            vec![bin_item(23, show("Media/Assets/Fav.jpg"), "Fav")],
        );
        let root = pp::Playlist {
            r#type: pp::playlist::Type::Root as i32,
            children_type: Some(pp::playlist::ChildrenType::Playlists(
                pp::playlist::PlaylistArray {
                    playlists: vec![
                        folder(
                            1,
                            "Media",
                            vec![
                                folder(2, "Backgrounds", vec![smart]),
                                folder(3, "Empty", vec![]),
                            ],
                        ),
                        manual,
                    ],
                },
            )),
            ..Default::default()
        };
        let live_cue = pp::PlaylistItem {
            name: "Input 1".into(),
            item_type: Some(pp::playlist_item::ItemType::Cue(pp::Cue {
                actions: vec![pp::Action {
                    action_type_data: Some(pp::action::ActionTypeData::Media(
                        pp::action::MediaType {
                            element: Some(pp::Media {
                                type_properties: Some(pp::media::TypeProperties::LiveVideo(
                                    Default::default(),
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        let doc = pp::PlaylistDocument {
            r#type: pp::playlist_document::Type::Media as i32,
            root_node: Some(root),
            live_video_playlist: Some(items(30, "Video Input", vec![live_cue])),
            ..Default::default()
        };
        ctx.current.path = Some("Playlists/Media".into());
        import_media_bins(&mut ctx, &doc, "Playlists/Media");

        assert_eq!(ctx.out.media_bins.len(), 2);
        let nature = &ctx.out.media_bins[0];
        let favorites = &ctx.out.media_bins[1];
        assert!(nature.sort_key < favorites.sort_key);
        assert_eq!(nature.name, "Nature");
        assert_eq!(
            nature.folder,
            vec!["Media".to_string(), "Backgrounds".to_string()]
        );
        assert_eq!(nature.browser, BinBrowser::Media);
        assert_eq!(
            nature.id,
            MediaBinId::from_uuid(ids::parse_uuid(Some(&uuid(10))).unwrap())
        );
        let MediaBinKind::Watched {
            folder,
            import_behavior,
            last_scan,
            ..
        } = &nature.kind
        else {
            panic!("{:?}", nature.kind)
        };
        assert_eq!(folder.path, "Backgrounds/Nature");
        assert!(!folder.recursive);
        assert_eq!(import_behavior.layer, MediaLayer::Background);
        assert_eq!(import_behavior.display.scale, ScaleMode::Fill);
        // The case-variant duplicate is the same file: skipped.
        assert_eq!(last_scan.len(), 2);
        assert_eq!(
            last_scan[0].id,
            MediaBinItemId::for_watched(nature.id, "Lake.jpg")
        );
        assert_eq!(last_scan[0].name, None);
        assert_eq!(last_scan[1].name.as_deref(), Some("Rolling hills"));
        assert_eq!(
            import_behavior.trigger(last_scan[0].trigger.asset),
            last_scan[0].trigger
        );
        assert!(
            nature
                .import
                .as_ref()
                .unwrap()
                .unmapped
                .get("smartDirectory")
                .is_some()
        );
        let root_id = folder.root;
        assert_eq!(root_of(&ctx, last_scan[0].trigger.asset), root_id);

        let MediaBinKind::Manual {
            items: manual_items,
        } = &favorites.kind
        else {
            panic!()
        };
        assert_eq!(manual_items.len(), 1);
        assert_eq!(
            manual_items[0].id,
            MediaBinItemId::from_uuid(ids::parse_uuid(Some(&uuid(23))).unwrap())
        );
        assert!(favorites.folder.is_empty());

        // Empty folder and the unconfigured live input are reported.
        let not_imported = ctx
            .report
            .warnings
            .iter()
            .filter(|w| w.category == Category::NotImported)
            .count();
        assert_eq!(not_imported, 2);

        // Audio documents produce audio bins.
        let mut ctx = nowhere();
        let audio_doc = pp::PlaylistDocument {
            r#type: pp::playlist_document::Type::Audio as i32,
            root_node: Some(pp::Playlist {
                r#type: pp::playlist::Type::Root as i32,
                children_type: Some(pp::playlist::ChildrenType::Playlists(
                    pp::playlist::PlaylistArray {
                        playlists: vec![items(40, "Walk-in", vec![])],
                    },
                )),
                ..Default::default()
            }),
            ..Default::default()
        };
        import_media_bins(&mut ctx, &audio_doc, "Playlists/Audio");
        assert_eq!(ctx.out.media_bins.len(), 1);
        assert_eq!(ctx.out.media_bins[0].browser, BinBrowser::Audio);
    }

    #[test]
    fn finalize_groups_missing_media_by_root() {
        let b = TempBackup::new("finalize");
        b.write("Media/Assets/Here.png", &png(1, 1, 6));
        let mut ctx = b.ctx();
        let lib = AssetScope::Library;
        for url in [
            show("Media/Assets/Here.png"),
            show("Media/Assets/Gone.jpg"),
            show("Media/Assets/Gone.jpg"),
            show("Media/Assets/Also gone.jpg"),
            external("Z", "\\\\nas\\media", "B.jpg"),
            external("G", "Cloud Drive", "A.jpg"),
            local(UrlRoot::UserHome, "Temp/x.jpg", None),
        ] {
            asset_for_url(&mut ctx, &url, MediaKind::Image, &lib);
        }
        finalize_media(&mut ctx);

        let names: Vec<&str> = ctx
            .report
            .missing_media
            .iter()
            .map(|g| g.name.as_str())
            .collect();
        let expected: BTreeSet<&str> = [
            "\\\\nas\\media (Z:)",
            "Cloud Drive (G:)",
            BACKUP_ROOT_NAME,
            "Unresolved references",
        ]
        .into();
        assert_eq!(names.len(), 4);
        assert_eq!(names.iter().copied().collect::<BTreeSet<_>>(), expected);
        let lower: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
        let mut sorted = lower.clone();
        sorted.sort();
        assert_eq!(lower, sorted);
        let backup = ctx
            .report
            .missing_media
            .iter()
            .find(|g| g.name == BACKUP_ROOT_NAME)
            .unwrap();
        let files: Vec<(&str, usize)> = backup
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.references))
            .collect();
        assert_eq!(
            files,
            vec![
                ("Media/Assets/Also gone.jpg", 1),
                ("Media/Assets/Gone.jpg", 2)
            ]
        );
        let unresolved = ctx
            .report
            .missing_media
            .iter()
            .find(|g| g.root.is_none())
            .unwrap();
        assert_eq!(unresolved.files[0].path, "ROOT_USER_HOME/Temp/x.jpg");

        // Root order: backup first; binding suggestion for the backup root.
        let first = ctx
            .out
            .media_roots
            .iter()
            .min_by(|a, b| a.sort_key.cmp(&b.sort_key))
            .unwrap();
        assert_eq!(first.name, BACKUP_ROOT_NAME);
        assert_eq!(ctx.out.media_root_bindings.len(), 1);
        assert_eq!(
            ctx.out.media_root_bindings[0].root,
            ids::media_root_id("backup")
        );
        assert!(Path::new(&ctx.out.media_root_bindings[0].local_path).is_absolute());
        finalize_media(&mut ctx);
        assert_eq!(ctx.out.media_root_bindings.len(), 1);
    }

    #[test]
    fn helpers() {
        assert_eq!(clean_path("a\\b/./c//d").as_deref(), Some("a/b/c/d"));
        assert_eq!(clean_path("a/../b").as_deref(), Some("b"));
        assert_eq!(clean_path("../x"), None);
        assert_eq!(
            absolute_prefix("Z:\\Media\\A.jpg", "Media/A.jpg").as_deref(),
            Some("Z:")
        );
        assert_eq!(absolute_prefix("Z:\\XMedia\\A.jpg", "Media/A.jpg"), None);
        assert!(hosts_alias("NAS", "nas.example.local"));
        assert!(!hosts_alias("10.0.0.5", "nas"));
        assert!(!hosts_alias("nas.a.local", "nas.b.local"));
        assert_eq!(
            parse_unc("\\\\host\\share\\x").unwrap(),
            ("host".into(), "share".into())
        );
        assert_eq!(parse_unc("C:\\x"), None);
        assert_eq!(kind_from_extension("a/B.MOV"), MediaKind::Video);
        assert_eq!(file_stem("a/b.c.jpg"), "b.c");
        assert_eq!(file_stem(".hidden"), ".hidden");
        assert_eq!(
            media_layer(pp::action::LayerType::Input as i32, MediaKind::Video),
            MediaLayer::VideoInput
        );
        assert_eq!(media_layer(9, MediaKind::Image), MediaLayer::Unknown);
    }
}
