//! **Area: content.** `Themes/**/Theme` (`rv.data.Template.Document`) → [`Theme`] documents.
//!
//! ## Owns
//! - [`import_theme`]: byte-identical de-duplication first (`ctx.lookups.theme_contents`: a
//!   duplicate is not imported; `report.duplicate_themes` + `ctx.lookups.themes[dup folder]`
//!   pointing at the kept theme, so references to either folder resolve). Otherwise converts,
//!   pushes to `ctx.out.themes`, registers `ctx.lookups.themes[folder]` and
//!   `ctx.lookups.template_slides[slide uuid]`.
//!   While converting, sets `ctx.current.asset_scope = AssetScope::Theme { theme }` and
//!   `ctx.current.resource_dir = Some(folder)` so media fills / actions resolve
//!   `ROOT_CURRENT_RESOURCE` (`Assets/…`) inside the theme folder (the media area reads them).
//! - [`theme`]: ID `ids::theme_id(folder)`; `name` from the folder name (the legacy name in the
//!   undecoded field 2, [`legacy_theme_name`], only as a fallback, else kept in
//!   `import.originalName`); `folder` = parent folders below `Themes/` ([`theme_folder`]);
//!   `canvasSize` from the slides; `import = ctx.document_info()`; `sortKey` = the argument
//!   (the orchestrator assigns `ids::sort_keys` in `source::theme_files` order); template slides
//!   in order ([`template_slide`]).
//! - [`template_slide`]: ID = `baseSlide.uuid` (derived when missing); `name`; slide body via
//!   `slide::slide_from_pb`; template `actions` → `slide::slide_actions` onto the slide; guides
//!   from `baseSlide.guidelines`.
//!
//! ## Details
//! - Template-slide UUIDs are unique per kept theme. A UUID already used by another (non
//!   identical) theme gets a derived ID (`ids::derived("templateSlide", [theme, uuid])`) and keeps
//!   the source UUID in `import.originalId`; `lookups.template_slides` keeps the first owner.
//! - The legacy name is read from the raw bytes with a minimal protobuf wire scanner (field 2 of
//!   the top-level message, length-delimited, UTF-8).
//!
//! ## May call
//! `common::*`, `slide::slide_from_pb`, `slide::slide_actions`, `ImportContext` helpers.

use model::{
    AssetScope, Guide, GuideId, GuideOrientation, ImportInfo, Size, Slide, SlideAction, SlideId,
    SortKey, TemplateSlide, TemplateSlideId, Theme, ThemeId,
};

use crate::context::{ImportContext, ThemeEntry, ids};
use crate::convert::{common, slide};
use crate::pb::rv::data as pp;
use crate::report::{Category, DuplicateTheme};

/// The slides-area converters a theme needs (unit tests pass stubs).
#[derive(Clone, Copy)]
struct SlideHooks {
    body: fn(&mut ImportContext, &pp::Slide, SlideId) -> Slide,
    actions: fn(&mut ImportContext, &[pp::Action], Option<usize>) -> Vec<SlideAction>,
}

const SLIDES_AREA: SlideHooks = SlideHooks {
    body: slide::slide_from_pb,
    actions: slide::slide_actions,
};

/// Imports one theme document (see the module docs). Returns the kept theme's ID (also for a
/// duplicate), `None` when it could not be imported.
pub fn import_theme(
    ctx: &mut ImportContext,
    doc: &pp::template::Document,
    rel_path: &str,
    raw: &[u8],
    sort_key: SortKey,
) -> Option<ThemeId> {
    import_theme_with(ctx, doc, rel_path, raw, sort_key, SLIDES_AREA)
}

fn import_theme_with(
    ctx: &mut ImportContext,
    doc: &pp::template::Document,
    rel_path: &str,
    raw: &[u8],
    sort_key: SortKey,
    hooks: SlideHooks,
) -> Option<ThemeId> {
    let (folder, _, _) = theme_folder(rel_path);

    // Byte-identical copy of a theme that is already imported.
    if let Some(kept) = ctx.lookups.theme_contents.get(raw).cloned() {
        let entry = ctx.lookups.themes.get(&kept).cloned()?;
        ctx.report.duplicate_themes.push(DuplicateTheme {
            kept: kept.clone(),
            duplicate: folder.clone(),
        });
        ctx.warn(
            Category::DuplicateThemes,
            format!("theme \"{folder}\" is byte-identical to \"{kept}\"; not imported again"),
        );
        let id = entry.id;
        ctx.lookups.themes.insert(folder, entry);
        return Some(id);
    }

    let t = theme_with(ctx, doc, rel_path, raw, sort_key, hooks);

    for (ts, source) in t.slides.iter().zip(&doc.slides) {
        let source_uuid = source
            .base_slide
            .as_ref()
            .and_then(|b| ids::parse_uuid(b.uuid.as_ref()));
        if let Some(u) = source_uuid {
            ctx.lookups
                .template_slides
                .entry(u)
                .or_insert((t.id, ts.id));
        }
    }
    ctx.lookups.themes.insert(
        folder.clone(),
        ThemeEntry {
            id: t.id,
            name: t.name.clone(),
            folder: folder.clone(),
            slides: t.slides.iter().map(|s| s.id).collect(),
        },
    );
    ctx.lookups.theme_contents.insert(raw.to_vec(), folder);
    let id = t.id;
    ctx.out.themes.push(t);
    Some(id)
}

/// Converts one theme (pure: no push, no registration). `rel_path` is the `Theme` file.
pub fn theme(
    ctx: &mut ImportContext,
    doc: &pp::template::Document,
    rel_path: &str,
    raw: &[u8],
    sort_key: SortKey,
) -> Theme {
    theme_with(ctx, doc, rel_path, raw, sort_key, SLIDES_AREA)
}

fn theme_with(
    ctx: &mut ImportContext,
    doc: &pp::template::Document,
    rel_path: &str,
    raw: &[u8],
    sort_key: SortKey,
    hooks: SlideHooks,
) -> Theme {
    ctx.set_writer(doc.application_info.as_ref());
    let (folder, groups, folder_name) = theme_folder(rel_path);
    let id = ids::theme_id(&folder);
    // Theme media is scoped to the theme while its slides convert (restored below).
    let saved_scope = std::mem::replace(
        &mut ctx.current.asset_scope,
        AssetScope::Theme { theme: id },
    );
    let saved_dir = ctx.current.resource_dir.replace(folder.clone());

    let legacy = legacy_theme_name(raw);
    let mut import = ctx.document_info();
    let name = if folder_name.trim().is_empty() {
        legacy.clone().unwrap_or_else(|| "Theme".to_string())
    } else {
        if let Some(l) = legacy.as_ref().filter(|l| **l != folder_name) {
            import.original_name = Some(l.clone());
        }
        folder_name
    };

    // Canvas: the first slide's size; others are warned.
    let sizes: Vec<Size> = doc
        .slides
        .iter()
        .filter_map(|s| common::size(s.base_slide.as_ref()?.size.as_ref()))
        .collect();
    let canvas = sizes.first().copied().unwrap_or(Size::HD);
    let other = sizes.iter().filter(|s| **s != canvas).count();
    if other > 0 {
        ctx.warn(
            Category::Other,
            format!(
                "{other} template slide(s) differ from the theme canvas {}x{}",
                canvas.width, canvas.height
            ),
        );
    }

    let mut slides: Vec<TemplateSlide> = Vec::with_capacity(doc.slides.len());
    for (i, s) in doc.slides.iter().enumerate() {
        let mut ts = template_slide_with(ctx, s, canvas, i, hooks);
        // Unique inside this theme and against other kept themes.
        let used_elsewhere = ids::parse_uuid(s.base_slide.as_ref().and_then(|b| b.uuid.as_ref()))
            .and_then(|u| ctx.lookups.template_slides.get(&u))
            .is_some_and(|(t, _)| *t != id);
        if used_elsewhere || slides.iter().any(|x| x.id == ts.id) {
            let original = ts.id;
            ts.id = TemplateSlideId::from_uuid(ids::derived(
                "templateSlide",
                &[&id.to_string(), &original.to_string(), &i.to_string()],
            ));
            ts.slide.id = SlideId::from_uuid(ts.id.0);
            ts.import
                .get_or_insert_with(ImportInfo::default)
                .original_id = Some(original.to_string());
        }
        slides.push(ts);
    }
    ctx.current.asset_scope = saved_scope;
    ctx.current.resource_dir = saved_dir;

    Theme {
        id,
        name,
        folder: groups,
        sort_key,
        canvas_size: canvas,
        slides,
        import: Some(import),
        ..Theme::default()
    }
}

/// One template slide.
pub fn template_slide(
    ctx: &mut ImportContext,
    slide: &pp::template::Slide,
    canvas: Size,
    index: usize,
) -> TemplateSlide {
    template_slide_with(ctx, slide, canvas, index, SLIDES_AREA)
}

fn template_slide_with(
    ctx: &mut ImportContext,
    slide: &pp::template::Slide,
    canvas: Size,
    index: usize,
    hooks: SlideHooks,
) -> TemplateSlide {
    let _ = canvas; // slides have no size of their own (the theme owns it)
    let doc_path = ctx.current.path.clone().unwrap_or_default();
    let base = slide.base_slide.as_ref();
    let uuid = ids::source_or_derived(
        base.and_then(|b| b.uuid.as_ref()),
        "templateSlide",
        &[&doc_path, &index.to_string()],
    );
    let id = TemplateSlideId::from_uuid(uuid);
    let mut body = match base {
        Some(b) => (hooks.body)(ctx, b, SlideId::from_uuid(uuid)),
        None => Slide {
            id: SlideId::from_uuid(uuid),
            ..Slide::new()
        },
    };
    body.actions
        .extend((hooks.actions)(ctx, &slide.actions, None));
    TemplateSlide {
        id,
        name: slide.name.clone(),
        slide: body,
        guides: base.map(|b| guides(&b.guidelines)).unwrap_or_default(),
        import: None,
    }
}

/// Editor guides of a template slide.
pub fn guides(guidelines: &[pp::AlignmentGuide]) -> Vec<Guide> {
    use pp::alignment_guide::GuidelineOrientation as O;
    guidelines
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let orientation = match O::try_from(g.orientation) {
                Ok(O::Horizontal) => GuideOrientation::Horizontal,
                Ok(O::Vertical) => GuideOrientation::Vertical,
                Err(_) => GuideOrientation::Unknown,
            };
            let id = GuideId::from_uuid(ids::source_or_derived(
                g.uuid.as_ref(),
                "guide",
                &[
                    &i.to_string(),
                    &g.orientation.to_string(),
                    &g.location.to_string(),
                ],
            ));
            Guide {
                id,
                orientation,
                position: g.location,
            }
        })
        .collect()
}

/// The legacy theme name stored in `Template.Document` field 2 (a length-delimited string the
/// schema marks reserved; prost drops it, so it is read from the raw bytes). `None` when absent.
pub fn legacy_theme_name(raw: &[u8]) -> Option<String> {
    fn varint(b: &[u8], pos: &mut usize) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *b.get(*pos)?;
            *pos += 1;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
    let mut pos = 0usize;
    while pos < raw.len() {
        let key = varint(raw, &mut pos)?;
        let (field, wire) = (key >> 3, key & 7);
        match wire {
            0 => {
                varint(raw, &mut pos)?;
            }
            1 => pos = pos.checked_add(8)?,
            5 => pos = pos.checked_add(4)?,
            2 => {
                let len = usize::try_from(varint(raw, &mut pos)?).ok()?;
                let end = pos.checked_add(len)?;
                let bytes = raw.get(pos..end)?;
                pos = end;
                if field == 2 {
                    let s = std::str::from_utf8(bytes).ok()?.trim();
                    return (!s.is_empty()).then(|| s.to_string());
                }
            }
            _ => return None, // groups (3/4) and invalid wire types
        }
    }
    None
}

/// Splits a `Theme` file path into (folder path of the theme relative to the root, grouping
/// folders below `Themes/`, theme name): `"Themes/Lyric Styles/Lines/Theme"` →
/// `("Themes/Lyric Styles/Lines", ["Lyric Styles"], "Lines")`.
pub fn theme_folder(rel_path: &str) -> (String, Vec<String>, String) {
    let norm = crate::source::normalize_rel(rel_path);
    let mut segs: Vec<&str> = norm.split('/').filter(|s| !s.is_empty()).collect();
    if segs.last() == Some(&crate::source::THEME_FILE) {
        segs.pop();
    }
    let folder = segs.join("/");
    let below: &[&str] = match segs.first() {
        Some(first) if *first == crate::source::THEMES => &segs[1..],
        _ => &segs[..],
    };
    let (name, groups) = match below.split_last() {
        Some((name, groups)) => (
            name.to_string(),
            groups.iter().map(|s| s.to_string()).collect(),
        ),
        None => (String::new(), Vec::new()),
    };
    (folder, groups, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImportOptions;
    use prost::Message;
    use std::path::Path;
    use uuid::Uuid;

    fn stub_body(_: &mut ImportContext, s: &pp::Slide, id: SlideId) -> Slide {
        let mut out = Slide::new();
        out.id = id;
        out.background = s.background_color.as_ref().map(common::color);
        out
    }

    fn stub_actions(_: &mut ImportContext, a: &[pp::Action], _: Option<usize>) -> Vec<SlideAction> {
        a.iter()
            .map(|_| SlideAction::new(model::ActionKind::Unknown))
            .collect()
    }

    const STUBS: SlideHooks = SlideHooks {
        body: stub_body,
        actions: stub_actions,
    };

    fn uuid(n: u128) -> pp::Uuid {
        pp::Uuid {
            string: Uuid::from_u128(n).to_string(),
        }
    }

    fn doc(first_slide: u128) -> pp::template::Document {
        let slide = |n: u128, name: &str| pp::template::Slide {
            name: name.into(),
            base_slide: Some(pp::Slide {
                uuid: Some(uuid(n)),
                size: Some(pp::graphics::Size {
                    width: 1920.0,
                    height: 1080.0,
                }),
                guidelines: vec![pp::AlignmentGuide {
                    uuid: Some(uuid(n + 100)),
                    orientation: pp::alignment_guide::GuidelineOrientation::Vertical as i32,
                    location: 960.0,
                }],
                ..Default::default()
            }),
            actions: vec![pp::Action::default()],
        };
        pp::template::Document {
            application_info: None,
            slides: vec![
                slide(first_slide, "Lyrics"),
                slide(first_slide + 1, "Title"),
            ],
        }
    }

    /// Encodes `doc` plus a legacy field 2 string.
    fn raw_with_legacy(doc: &pp::template::Document, legacy: &str) -> Vec<u8> {
        let mut raw = doc.encode_to_vec();
        raw.push(0x12); // field 2, wire type 2
        raw.push(legacy.len() as u8);
        raw.extend_from_slice(legacy.as_bytes());
        raw
    }

    #[test]
    fn folders() {
        assert_eq!(
            theme_folder("Themes/Lyric Styles/Lines/Theme"),
            (
                "Themes/Lyric Styles/Lines".to_string(),
                vec!["Lyric Styles".to_string()],
                "Lines".to_string()
            )
        );
        assert_eq!(
            theme_folder("Themes/Lower Thirds/Theme"),
            (
                "Themes/Lower Thirds".to_string(),
                vec![],
                "Lower Thirds".to_string()
            )
        );
    }

    #[test]
    fn legacy_name_from_raw_bytes() {
        let d = doc(1);
        assert_eq!(legacy_theme_name(&d.encode_to_vec()), None);
        let raw = raw_with_legacy(&d, "Old Name");
        assert_eq!(legacy_theme_name(&raw).as_deref(), Some("Old Name"));
        // Still decodes (prost skips the reserved field).
        assert_eq!(pp::template::Document::decode(raw.as_slice()).unwrap(), d);
        assert_eq!(legacy_theme_name(&[0xff, 0xff]), None);
        assert_eq!(legacy_theme_name(&[0x12, 0x40, b'a']), None);
    }

    #[test]
    fn import_dedup_and_registration() {
        let mut ctx = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        let d = doc(1);
        let raw = raw_with_legacy(&d, "Legacy");
        let keys = ids::sort_keys(3);
        let a = import_theme_with(
            &mut ctx,
            &d,
            "Themes/Group/Alpha/Theme",
            &raw,
            keys[0].clone(),
            STUBS,
        )
        .unwrap();
        let b = import_theme_with(
            &mut ctx,
            &d,
            "Themes/Alpha 1/Theme",
            &raw,
            keys[1].clone(),
            STUBS,
        )
        .unwrap();
        assert_eq!(a, b);
        assert_eq!(ctx.out.themes.len(), 1);
        let t = &ctx.out.themes[0];
        assert_eq!(t.id, ids::theme_id("Themes/Group/Alpha"));
        assert_eq!(t.name, "Alpha");
        assert_eq!(t.folder, vec!["Group".to_string()]);
        assert_eq!(t.sort_key, keys[0]);
        assert_eq!(
            t.import.as_ref().unwrap().original_name.as_deref(),
            Some("Legacy")
        );
        assert_eq!(t.slides.len(), 2);
        assert_eq!(t.slides[0].id.0, Uuid::from_u128(1));
        assert_eq!(t.slides[0].slide.id.0, Uuid::from_u128(1));
        assert_eq!(t.slides[0].name, "Lyrics");
        assert_eq!(t.slides[0].slide.actions.len(), 1);
        assert_eq!(t.slides[0].guides.len(), 1);
        assert_eq!(
            t.slides[0].guides[0].orientation,
            GuideOrientation::Vertical
        );
        assert_eq!(t.slides[0].guides[0].position, 960.0);
        assert_eq!(ctx.report.duplicate_themes.len(), 1);
        assert_eq!(ctx.report.duplicate_themes[0].kept, "Themes/Group/Alpha");
        assert_eq!(ctx.report.duplicate_themes[0].duplicate, "Themes/Alpha 1");
        // Both folders resolve to the kept theme.
        assert_eq!(
            ctx.lookups
                .theme_by_path("Themes/Alpha 1/Theme")
                .unwrap()
                .id,
            a
        );
        assert_eq!(
            ctx.lookups
                .theme_by_path("Themes/Group/Alpha")
                .unwrap()
                .slides
                .len(),
            2
        );
        assert_eq!(
            ctx.lookups.template_slides[&Uuid::from_u128(2)],
            (a, t.slides[1].id)
        );
        // The asset scope is restored after the theme.
        assert_eq!(ctx.current.asset_scope, AssetScope::Library);
    }

    #[test]
    fn colliding_slide_uuids_in_different_themes() {
        let mut ctx = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        let d1 = doc(1);
        let mut d2 = doc(1);
        d2.slides[0].name = "Changed".into(); // not byte-identical
        let k = ids::sort_keys(2);
        let a = import_theme_with(
            &mut ctx,
            &d1,
            "Themes/A/Theme",
            &d1.encode_to_vec(),
            k[0].clone(),
            STUBS,
        )
        .unwrap();
        let b = import_theme_with(
            &mut ctx,
            &d2,
            "Themes/B/Theme",
            &d2.encode_to_vec(),
            k[1].clone(),
            STUBS,
        )
        .unwrap();
        assert_ne!(a, b);
        let tb = &ctx.out.themes[1];
        assert_ne!(tb.slides[0].id.0, Uuid::from_u128(1));
        assert_eq!(
            tb.slides[0].import.as_ref().unwrap().original_id.as_deref(),
            Some(Uuid::from_u128(1).to_string().as_str())
        );
        // The lookup keeps the first owner.
        assert_eq!(ctx.lookups.template_slides[&Uuid::from_u128(1)].0, a);
        assert!(ctx.report.duplicate_themes.is_empty());
    }

    #[test]
    fn legacy_name_is_fallback_for_nameless_folder() {
        let mut ctx = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        let d = pp::template::Document::default();
        let raw = raw_with_legacy(&d, "Legacy");
        let t = theme_with(&mut ctx, &d, "Themes/Theme", &raw, SortKey::new(), STUBS);
        assert_eq!(t.name, "Legacy");
        assert!(t.slides.is_empty());
        assert_eq!(t.canvas_size, Size::HD);
    }
}
