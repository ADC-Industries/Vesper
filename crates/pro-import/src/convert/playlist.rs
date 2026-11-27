//! **Area: content.** Playlist documents → [`PlaylistNode`] documents (`Playlists/Library` as
//! service playlists, `Playlists/Audio` as audio playlists). `Playlists/Media` belongs to the
//! media area (`media::import_media_bins`).
//!
//! ## Owns
//! - [`import_playlists`]: walks `rootNode` (type ROOT, not imported itself: its children are
//!   top level). Folder vs playlist is decided only by the `ChildrenType` oneof (`playlists` →
//!   folder, `items` → playlist; unset → empty playlist); `Playlist.type` is ignored except ROOT.
//!   Node ID = source UUID; siblings get `ids::sort_keys`; `parent` links; top-level nodes carry
//!   `ctx.document_info()`. `purpose` = the argument.
//! - [`playlist_item`]: item ID `ids::playlist_item_id(playlist, index)` (source UUIDs collide;
//!   the source UUID goes to `import.originalId`); `name`; `enabled = !isHidden`; kinds:
//!   - presentation → [`presentation_item`];
//!   - cue (inline media) → the cue's media action through `media::media_action`, mapped to
//!     `PlaylistItemKind::Media` / `VideoInput` (anything else → `Unsupported`);
//!   - header → `Header { name, color }` (header actions → unmapped);
//!   - placeholder → `Placeholder`; planning center → `Unsupported { payload }`.
//! - [`presentation_item`] / [`resolve_presentation`]: `documentPath` is ROOT_SHOW-relative;
//!   resolve exactly through `ctx.lookups.presentation_by_path`; fallback is LIBRARY-SCOPED: the
//!   library from the path's `Libraries/<folder>` segment, then same basename
//!   (`lookups.presentations_in_library`), then `item.name` within that library; never across
//!   libraries. Unresolved → `Presentation { presentation: Link to a derived ID with the item
//!   name, unresolved: Some(MediaOrigin of the path) }` + `report.unresolved_playlist_items`.
//!   Arrangement kept only when it exists in the resolved presentation; otherwise `None` and
//!   `report.dangling_arrangements.playlist_items += 1`. Provenance split (IMPORTER.md wins over
//!   the model's doc comment): `import.originalId` holds the item's own source UUID; a dropped
//!   arrangement UUID goes to `import.unmapped.arrangement`.
//!
//! ## Details
//! - Resolution: the `local` ROOT_SHOW path first (exact, then case-insensitive: the source is
//!   usually Windows). Without a usable local path, the `Libraries/…` tail of `absoluteString`
//!   (or `relativePath`) is used the same way. A library-scoped fallback match is warned.
//! - Item names: a presentation item whose name equals the resolved presentation's name gets
//!   `name: None` (it follows the presentation); every other item keeps its source name. A
//!   header's name moves into `Header { name }`.
//! - Node IDs: the source UUID unless it is missing or already used by another node (a second
//!   playlist document, a repeated UUID); then a derived ID with the source UUID in
//!   `import.originalId`.
//! - Unmodelled node / item fields (hot keys, smart directories and Planning Center links on
//!   service playlists, item tags, item content destination / music key / arrangement name,
//!   header actions, extra cue actions) → `import.unmapped`. UI state (`expanded`) is dropped.
//!
//! ## May call
//! `common::*`, `media::media_action`, `media::origin`, `ImportContext` helpers.

use model::{
    ActionKind, ArrangementId, ImportInfo, Link, PlaylistId, PlaylistItem, PlaylistItemKind,
    PlaylistNode, PlaylistNodeKind, PlaylistPurpose, PresentationId, SortKey,
};
use serde_json::Value;

use crate::context::{ImportContext, PresentationEntry, ids};
use crate::convert::{common, media};
use crate::pb::rv::data as pp;
use crate::report::{Category, UnresolvedPlaylistItem};

/// The media area's `media::media_action` (unit tests pass a stub).
type MediaActionFn = fn(&mut ImportContext, &pp::Action, &pp::action::MediaType) -> ActionKind;

/// How a playlist presentation reference resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresentationResolution {
    /// Exact `documentPath` match.
    Exact(PresentationEntry),
    /// Library-scoped fallback (basename or item name inside the same library).
    LibraryFallback(PresentationEntry),
    /// Not found.
    Unresolved {
        /// Why (for the report).
        reason: String,
    },
}

/// Imports one playlist document's tree (see the module docs).
pub fn import_playlists(
    ctx: &mut ImportContext,
    doc: &pp::PlaylistDocument,
    rel_path: &str,
    purpose: PlaylistPurpose,
) {
    import_playlists_with(ctx, doc, rel_path, purpose, media::media_action)
}

fn import_playlists_with(
    ctx: &mut ImportContext,
    doc: &pp::PlaylistDocument,
    rel_path: &str,
    purpose: PlaylistPurpose,
    media_action: MediaActionFn,
) {
    if ctx.current.path.is_none() {
        ctx.current.path = Some(crate::source::normalize_rel(rel_path));
    }
    ctx.set_writer(doc.application_info.as_ref());
    let Some(root) = doc.root_node.as_ref() else {
        return;
    };
    if root.r#type != pp::playlist::Type::Root as i32 {
        ctx.warn(
            Category::Other,
            "the playlist root node is not marked TYPE_ROOT; treated as the root anyway",
        );
    }
    // The root is not a document: whatever it carries besides its children is only reported.
    for (key, _) in node_unmapped(ctx, root) {
        ctx.warn(
            Category::UnmappedFeatures,
            format!("playlist root carries \"{key}\"; not imported"),
        );
        ctx.count_unmapped(&format!("playlist.root.{key}"));
    }
    match &root.children_type {
        Some(pp::playlist::ChildrenType::Playlists(arr)) => {
            let keys = ids::sort_keys(arr.playlists.len());
            for (child, key) in arr.playlists.iter().zip(keys) {
                import_node_with(ctx, child, None, key, &[], purpose, media_action);
            }
        }
        // A root that holds items directly: one top-level playlist.
        Some(pp::playlist::ChildrenType::Items(_)) => {
            let key = ids::sort_keys(1).remove(0);
            import_node_with(ctx, root, None, key, &[], purpose, media_action);
        }
        None => {}
    }
}

/// Imports one node and its subtree. `path` is the node's ancestors' names (for the report).
pub fn import_node(
    ctx: &mut ImportContext,
    node: &pp::Playlist,
    parent: Option<PlaylistId>,
    sort_key: SortKey,
    path: &[String],
    purpose: PlaylistPurpose,
) -> PlaylistId {
    import_node_with(
        ctx,
        node,
        parent,
        sort_key,
        path,
        purpose,
        media::media_action,
    )
}

fn import_node_with(
    ctx: &mut ImportContext,
    node: &pp::Playlist,
    parent: Option<PlaylistId>,
    sort_key: SortKey,
    path: &[String],
    purpose: PlaylistPurpose,
    media_action: MediaActionFn,
) -> PlaylistId {
    let doc_path = ctx.current.path.clone().unwrap_or_default();
    let mut id_parts: Vec<&str> = vec![&doc_path];
    id_parts.extend(path.iter().map(String::as_str));
    id_parts.push(&node.name);
    id_parts.push(&sort_key);
    let derived = || PlaylistId::from_uuid(ids::derived("playlist", &id_parts));

    let mut import = if parent.is_none() {
        Some(ctx.document_info())
    } else {
        None
    };
    let taken = |id: PlaylistId| ctx.out.playlist_nodes.iter().any(|n| n.id == id);
    let mut id = match ids::parse_uuid(node.uuid.as_ref()) {
        Some(u) if !taken(PlaylistId::from_uuid(u)) => PlaylistId::from_uuid(u),
        Some(u) => {
            import.get_or_insert_with(ImportInfo::default).original_id = Some(u.to_string());
            derived()
        }
        None => derived(),
    };
    // Ancestors are named, not keyed, so same-named folders can derive the same ID: number the
    // later ones (deterministic, as nodes are imported in source order).
    let mut n = 1usize;
    while taken(id) {
        let n_str = n.to_string();
        let mut parts = id_parts.clone();
        parts.push(&n_str);
        id = PlaylistId::from_uuid(ids::derived("playlist", &parts));
        n += 1;
    }

    for (key, value) in node_unmapped(ctx, node) {
        ctx.keep_unmapped(&mut import, &format!("playlist.{key}"), value);
    }

    let name = node.name.clone();
    let mut child_path = path.to_vec();
    child_path.push(name.clone());
    let playlist_path = child_path.join("/");

    // Push the node before its children so the tree reads top-down in insertion order.
    let index = ctx.out.playlist_nodes.len();
    ctx.out.playlist_nodes.push(PlaylistNode {
        id,
        parent,
        name,
        sort_key,
        kind: PlaylistNodeKind::Folder,
        import,
        ..PlaylistNode::default()
    });

    let kind = match &node.children_type {
        Some(pp::playlist::ChildrenType::Playlists(arr)) => {
            let keys = ids::sort_keys(arr.playlists.len());
            for (child, key) in arr.playlists.iter().zip(keys) {
                import_node_with(
                    ctx,
                    child,
                    Some(id),
                    key,
                    &child_path,
                    purpose,
                    media_action,
                );
            }
            PlaylistNodeKind::Folder
        }
        Some(pp::playlist::ChildrenType::Items(items)) => PlaylistNodeKind::Playlist {
            purpose,
            items: items
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| playlist_item_with(ctx, item, id, i, &playlist_path, media_action))
                .collect(),
        },
        None => PlaylistNodeKind::Playlist {
            purpose,
            items: Vec::new(),
        },
    };
    ctx.out.playlist_nodes[index].kind = kind;
    id
}

/// Node fields the model has no slot for, as `(json key, value)` pairs.
fn node_unmapped(ctx: &ImportContext, node: &pp::Playlist) -> Vec<(String, Value)> {
    let rest = pp::Playlist {
        uuid: None,
        name: String::new(),
        r#type: 0,
        expanded: false,
        children_type: None,
        ..node.clone()
    };
    object_entries(ctx.to_json(&rest))
}

fn object_entries(v: Value) -> Vec<(String, Value)> {
    match v {
        Value::Object(map) => map.into_iter().collect(),
        _ => Vec::new(),
    }
}

/// One playlist item (see the module docs). `playlist_path` names the playlist in the report.
pub fn playlist_item(
    ctx: &mut ImportContext,
    item: &pp::PlaylistItem,
    playlist: PlaylistId,
    index: usize,
    playlist_path: &str,
) -> PlaylistItem {
    playlist_item_with(
        ctx,
        item,
        playlist,
        index,
        playlist_path,
        media::media_action,
    )
}

fn playlist_item_with(
    ctx: &mut ImportContext,
    item: &pp::PlaylistItem,
    playlist: PlaylistId,
    index: usize,
    playlist_path: &str,
    media_action: MediaActionFn,
) -> PlaylistItem {
    use pp::playlist_item::ItemType;

    let mut import = ids::parse_uuid(item.uuid.as_ref()).map(|u| ImportInfo {
        original_id: Some(u.to_string()),
        ..ImportInfo::default()
    });
    let mut name = Some(item.name.clone()).filter(|n| !n.trim().is_empty());
    let mut enabled = !item.is_hidden;

    if !item.tags.is_empty() {
        let tags = Value::Array(
            item.tags
                .iter()
                .map(|t| Value::String(t.string.clone()))
                .collect(),
        );
        ctx.keep_unmapped(&mut import, "playlistItem.tags", tags);
    }

    let kind = match &item.item_type {
        Some(ItemType::Presentation(p)) => {
            let (kind, dropped) = presentation_item(ctx, p, &item.name, playlist_path);
            if let Some(a) = dropped {
                ctx.keep_unmapped(&mut import, "playlistItem.arrangement", Value::String(a));
            }
            let rest = pp::playlist_item::Presentation {
                document_path: None,
                arrangement: None,
                ..p.clone()
            };
            for (key, value) in object_entries(ctx.to_json(&rest)) {
                ctx.keep_unmapped(
                    &mut import,
                    &format!("playlistItem.presentation.{key}"),
                    value,
                );
            }
            // The name follows the presentation unless it was renamed in the playlist.
            if let PlaylistItemKind::Presentation {
                presentation,
                unresolved: None,
                ..
            } = &kind
                && presentation.name.as_deref() == Some(item.name.as_str())
            {
                name = None;
            }
            kind
        }
        Some(ItemType::Cue(cue)) => {
            enabled &= cue.is_enabled;
            cue_item(ctx, cue, &mut import, media_action)
        }
        Some(ItemType::Header(h)) => {
            if !h.actions.is_empty() {
                let v = Value::Array(h.actions.iter().map(|a| ctx.to_json(a)).collect());
                ctx.keep_unmapped(&mut import, "playlistItem.header.actions", v);
            }
            PlaylistItemKind::Header {
                name: name.take().unwrap_or_default(),
                color: common::visible_color(h.color.as_ref()),
            }
        }
        Some(ItemType::Placeholder(p)) => {
            if let Some(linked) = p.linked_data.as_deref() {
                let v = ctx.to_json(linked);
                ctx.keep_unmapped(&mut import, "playlistItem.placeholder.linkedData", v);
            }
            PlaylistItemKind::Placeholder
        }
        Some(ItemType::PlanningCenter(pc)) => {
            ctx.warn(
                Category::UnsupportedActions,
                format!("playlist \"{playlist_path}\": Planning Center item kept as unsupported"),
            );
            PlaylistItemKind::Unsupported {
                source_type: "planningCenter".into(),
                payload: ctx.to_json(pc.as_ref()),
            }
        }
        None => {
            ctx.warn(
                Category::Other,
                format!(
                    "playlist \"{playlist_path}\": an item has no content; kept as unsupported"
                ),
            );
            PlaylistItemKind::Unsupported {
                source_type: "empty".into(),
                payload: Value::Null,
            }
        }
    };

    PlaylistItem {
        id: ids::playlist_item_id(playlist, index),
        name,
        enabled,
        kind,
        import,
    }
}

/// An inline cue item: its media action becomes a `Media` / `VideoInput` item.
fn cue_item(
    ctx: &mut ImportContext,
    cue: &pp::Cue,
    import: &mut Option<ImportInfo>,
    media_action: MediaActionFn,
) -> PlaylistItemKind {
    let media = cue
        .actions
        .iter()
        .enumerate()
        .find_map(|(i, a)| match &a.action_type_data {
            Some(pp::action::ActionTypeData::Media(m)) => Some((i, a, m)),
            _ => None,
        });
    let Some((mi, action, m)) = media else {
        ctx.warn(
            Category::UnsupportedActions,
            "a playlist cue item has no media action; kept as unsupported",
        );
        return PlaylistItemKind::Unsupported {
            source_type: "cue".into(),
            payload: ctx.to_json(cue),
        };
    };
    let others: Vec<Value> = cue
        .actions
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != mi)
        .map(|(_, a)| ctx.to_json(a))
        .collect();
    if !others.is_empty() {
        ctx.keep_unmapped(import, "playlistItem.cue.actions", Value::Array(others));
    }
    match media_action(ctx, action, m) {
        ActionKind::Media(t) => PlaylistItemKind::Media(t),
        ActionKind::VideoInput(v) => PlaylistItemKind::VideoInput(v),
        ActionKind::Unsupported {
            source_type,
            payload,
        } => PlaylistItemKind::Unsupported {
            source_type,
            payload,
        },
        _ => PlaylistItemKind::Unsupported {
            source_type: "cue".into(),
            payload: ctx.to_json(cue),
        },
    }
}

/// A presentation reference → item kind, plus the arrangement UUID that did not resolve (for
/// provenance).
pub fn presentation_item(
    ctx: &mut ImportContext,
    p: &pp::playlist_item::Presentation,
    item_name: &str,
    playlist_path: &str,
) -> (PlaylistItemKind, Option<String>) {
    let arrangement_src = ids::parse_uuid(p.arrangement.as_ref());
    let resolution = match p.document_path.as_ref() {
        Some(url) => resolve_presentation(ctx, url, item_name),
        None => PresentationResolution::Unresolved {
            reason: "the item has no document path".into(),
        },
    };
    let entry = match resolution {
        PresentationResolution::Exact(e) => e,
        PresentationResolution::LibraryFallback(e) => {
            ctx.warn(
                Category::UnresolvedPlaylistItems,
                format!(
                    "playlist \"{playlist_path}\": \"{item_name}\" resolved by a same-library fallback"
                ),
            );
            e
        }
        PresentationResolution::Unresolved { reason } => {
            let written = p
                .document_path
                .as_ref()
                .map(written_path)
                .unwrap_or_default();
            ctx.report
                .unresolved_playlist_items
                .push(UnresolvedPlaylistItem {
                    playlist: playlist_path.to_string(),
                    item: item_name.to_string(),
                    document_path: written.clone(),
                    reason,
                });
            let key = p
                .document_path
                .as_ref()
                .and_then(local_show_path)
                .unwrap_or(written);
            let id = PresentationId::from_uuid(ids::derived("unresolvedPresentation", &[&key]));
            let dropped = arrangement_src.map(|_| {
                ctx.report.dangling_arrangements.playlist_items += 1;
                p.arrangement
                    .as_ref()
                    .map(|a| a.string.clone())
                    .unwrap_or_default()
            });
            return (
                PlaylistItemKind::Presentation {
                    presentation: Link::named(id, item_name),
                    arrangement: None,
                    unresolved: Some(
                        p.document_path
                            .as_ref()
                            .map(media::origin)
                            .unwrap_or_default(),
                    ),
                },
                dropped,
            );
        }
    };

    let (arrangement, dropped) = match arrangement_src {
        None => (None, None),
        Some(u) if entry.arrangements.contains(&ArrangementId::from_uuid(u)) => {
            (Some(ArrangementId::from_uuid(u)), None)
        }
        Some(_) => {
            ctx.report.dangling_arrangements.playlist_items += 1;
            (None, p.arrangement.as_ref().map(|a| a.string.clone()))
        }
    };
    (
        PlaylistItemKind::Presentation {
            presentation: Link::named(entry.id, entry.name.clone()),
            arrangement,
            unresolved: None,
        },
        dropped,
    )
}

/// Resolves a `documentPath` (see the module docs).
pub fn resolve_presentation(
    ctx: &mut ImportContext,
    document_path: &pp::Url,
    item_name: &str,
) -> PresentationResolution {
    let Some(rel) = local_show_path(document_path).or_else(|| libraries_tail(document_path)) else {
        return PresentationResolution::Unresolved {
            reason: "the document path is neither show-relative nor inside a Libraries folder"
                .into(),
        };
    };
    let lookups = &ctx.lookups;
    if let Some(e) = lookups.presentation_by_path(&rel) {
        return PresentationResolution::Exact(e.clone());
    }
    if let Some(e) = lookups
        .presentations
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&rel))
        .map(|(_, e)| e)
    {
        return PresentationResolution::Exact(e.clone());
    }

    // Library-scoped fallbacks.
    let segs: Vec<&str> = rel.split('/').collect();
    let folder = match segs.as_slice() {
        [lib, folder, ..] if lib.eq_ignore_ascii_case(crate::source::LIBRARIES) => *folder,
        _ => {
            return PresentationResolution::Unresolved {
                reason: format!("\"{rel}\" is not inside Libraries/<library>/"),
            };
        }
    };
    let library = lookups.libraries.get(folder).copied().or_else(|| {
        lookups
            .libraries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(folder))
            .map(|(_, id)| *id)
    });
    let Some(library) = library else {
        return PresentationResolution::Unresolved {
            reason: format!("library \"{folder}\" was not imported"),
        };
    };
    let file = segs.last().copied().unwrap_or_default();
    let basename = match file.rfind('.') {
        Some(i) if file[i..].eq_ignore_ascii_case(".pro") => &file[..i],
        _ => file,
    };
    if let Some(e) = lookups.presentations_in_library(library, basename).first() {
        return PresentationResolution::LibraryFallback((*e).clone());
    }
    let name = item_name.trim();
    if !name.is_empty()
        && let Some(e) = lookups
            .presentations
            .values()
            .find(|p| p.library == library && p.name.trim().eq_ignore_ascii_case(name))
    {
        return PresentationResolution::LibraryFallback(e.clone());
    }
    PresentationResolution::Unresolved {
        reason: format!("no presentation \"{basename}\" in library \"{folder}\""),
    }
}

/// The ROOT_SHOW-relative path of a URL (normalized), if it has one.
fn local_show_path(url: &pp::Url) -> Option<String> {
    use pp::url::local_relative_path::Root;
    match &url.relative_file_path {
        Some(pp::url::RelativeFilePath::Local(l))
            if l.root == Root::Show as i32 && !l.path.trim().is_empty() =>
        {
            Some(crate::source::normalize_rel(&l.path))
        }
        _ => None,
    }
}

/// `Libraries/<library>/<file>` cut from the URL's relative path, else its absolute or
/// relative string (`C:\Users\x\Documents\ProPresenter\Libraries\Songs\A.pro`, or the
/// `file://` URL of it, percent-decoded → `Libraries/Songs/A.pro`).
fn libraries_tail(url: &pp::Url) -> Option<String> {
    let relative = match &url.relative_file_path {
        Some(pp::url::RelativeFilePath::Local(l)) => Some(l.path.as_str()),
        Some(pp::url::RelativeFilePath::External(e)) => Some(e.path.as_str()),
        None => None,
    };
    let stored = match &url.storage {
        Some(pp::url::Storage::AbsoluteString(s)) | Some(pp::url::Storage::RelativePath(s)) => {
            Some(crate::source::file_url_path(s))
        }
        None => None,
    };
    let tail = |s: &str| {
        let norm = crate::source::normalize_rel(s);
        let segs: Vec<&str> = norm.split('/').collect();
        let i = segs
            .iter()
            .rposition(|s| s.eq_ignore_ascii_case(crate::source::LIBRARIES))?;
        (segs.len() > i + 2).then(|| segs[i..].join("/"))
    };
    relative
        .and_then(tail)
        .or_else(|| stored.as_deref().and_then(tail))
}

/// The document path as written (local path, else absolute string).
fn written_path(url: &pp::Url) -> String {
    match (&url.relative_file_path, &url.storage) {
        (Some(pp::url::RelativeFilePath::Local(l)), _) if !l.path.is_empty() => l.path.clone(),
        (Some(pp::url::RelativeFilePath::External(e)), _) if !e.path.is_empty() => e.path.clone(),
        (_, Some(pp::url::Storage::AbsoluteString(s)))
        | (_, Some(pp::url::Storage::RelativePath(s))) => s.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImportOptions;
    use model::{LibraryId, MediaTrigger, VideoInputTrigger};
    use std::path::Path;
    use uuid::Uuid;

    fn stub_media(_: &mut ImportContext, _: &pp::Action, m: &pp::action::MediaType) -> ActionKind {
        match m.media_type {
            Some(pp::action::media_type::MediaType::LiveVideo(_)) => {
                ActionKind::VideoInput(VideoInputTrigger {
                    input: Link::to(model::VideoInputId::from_uuid(Uuid::nil())),
                    display: Default::default(),
                    audio: Default::default(),
                    transition: None,
                })
            }
            _ => ActionKind::Media(MediaTrigger::new(
                model::MediaAssetId::from_uuid(Uuid::nil()),
                model::MediaLayer::Background,
            )),
        }
    }

    fn uuid(n: u128) -> pp::Uuid {
        pp::Uuid {
            string: Uuid::from_u128(n).to_string(),
        }
    }

    fn show_url(path: &str) -> pp::Url {
        pp::Url {
            platform: pp::url::Platform::Win32 as i32,
            storage: Some(pp::url::Storage::AbsoluteString(format!(
                "C:\\Users\\u\\Documents\\ProPresenter\\{}",
                path.replace('/', "\\")
            ))),
            relative_file_path: Some(pp::url::RelativeFilePath::Local(
                pp::url::LocalRelativePath {
                    root: pp::url::local_relative_path::Root::Show as i32,
                    path: path.into(),
                },
            )),
        }
    }

    fn ctx_with_presentations() -> ImportContext {
        let mut ctx = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        ctx.current.path = Some("Playlists/Library".into());
        for lib in ["Songs", "Talks"] {
            ctx.lookups
                .libraries
                .insert(lib.into(), ids::library_id(lib));
        }
        let mut add = |path: &str, lib: &str, name: &str, n: u128, arr: Vec<u128>| {
            ctx.lookups.presentations.insert(
                path.into(),
                PresentationEntry {
                    id: PresentationId::from_uuid(Uuid::from_u128(n)),
                    library: ids::library_id(lib),
                    name: name.into(),
                    basename: name.into(),
                    arrangements: arr
                        .into_iter()
                        .map(|a| ArrangementId::from_uuid(Uuid::from_u128(a)))
                        .collect(),
                },
            );
        };
        add("Libraries/Songs/Alpha.pro", "Songs", "Alpha", 1, vec![50]);
        add("Libraries/Songs/Shared.pro", "Songs", "Shared", 2, vec![]);
        add("Libraries/Talks/Shared.pro", "Talks", "Shared", 3, vec![]);
        ctx
    }

    fn pres_item(n: u128, name: &str, path: &str, arrangement: Option<u128>) -> pp::PlaylistItem {
        pp::PlaylistItem {
            uuid: Some(uuid(n)),
            name: name.into(),
            item_type: Some(pp::playlist_item::ItemType::Presentation(
                pp::playlist_item::Presentation {
                    document_path: Some(show_url(path)),
                    arrangement: arrangement.map(uuid),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    fn media_item(n: u128, name: &str, live: bool) -> pp::PlaylistItem {
        let media = pp::action::MediaType {
            media_type: Some(if live {
                pp::action::media_type::MediaType::LiveVideo(Default::default())
            } else {
                pp::action::media_type::MediaType::Video(Default::default())
            }),
            ..Default::default()
        };
        pp::PlaylistItem {
            uuid: Some(uuid(n)),
            name: name.into(),
            item_type: Some(pp::playlist_item::ItemType::Cue(pp::Cue {
                uuid: Some(uuid(n)),
                name: name.into(),
                is_enabled: true,
                actions: vec![pp::Action {
                    action_type_data: Some(pp::action::ActionTypeData::Media(media)),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn node(n: u128, name: &str, children: pp::playlist::ChildrenType) -> pp::Playlist {
        pp::Playlist {
            uuid: Some(uuid(n)),
            name: name.into(),
            children_type: Some(children),
            ..Default::default()
        }
    }

    fn items(v: Vec<pp::PlaylistItem>) -> pp::playlist::ChildrenType {
        pp::playlist::ChildrenType::Items(pp::playlist::PlaylistItems { items: v })
    }

    fn folders(v: Vec<pp::Playlist>) -> pp::playlist::ChildrenType {
        pp::playlist::ChildrenType::Playlists(pp::playlist::PlaylistArray { playlists: v })
    }

    #[test]
    fn tree_items_and_resolution() {
        let mut ctx = ctx_with_presentations();
        let service = node(
            102,
            "Sunday",
            items(vec![
                pres_item(1, "Alpha", "Libraries/Songs/Alpha.pro", Some(50)),
                pres_item(201, "Alpha", "Libraries/Songs/Alpha.pro", Some(51)),
                pres_item(202, "Shared", "Libraries/Talks/Shared.pro", None),
                media_item(203, "walk-in.mp4", false),
                media_item(204, "Camera", true),
            ]),
        );
        let mut hidden = pres_item(205, "Renamed", "libraries/songs/ALPHA.pro", None);
        hidden.is_hidden = true;
        let other = node(103, "Other", items(vec![hidden]));
        let empty = pp::Playlist {
            uuid: Some(uuid(104)),
            name: "Empty".into(),
            ..Default::default()
        };
        let doc = pp::PlaylistDocument {
            root_node: Some(pp::Playlist {
                uuid: Some(uuid(100)),
                r#type: pp::playlist::Type::Root as i32,
                children_type: Some(folders(vec![
                    node(101, "Archive", folders(vec![other])),
                    service,
                    empty,
                ])),
                ..Default::default()
            }),
            ..Default::default()
        };
        import_playlists_with(
            &mut ctx,
            &doc,
            "Playlists/Library",
            PlaylistPurpose::Service,
            stub_media,
        );

        let nodes = &ctx.out.playlist_nodes;
        assert_eq!(nodes.len(), 4); // root is not imported
        let by_name = |n: &str| nodes.iter().find(|x| x.name == n).unwrap();
        let archive = by_name("Archive");
        assert_eq!(archive.kind, PlaylistNodeKind::Folder);
        assert_eq!(archive.parent, None);
        assert!(archive.import.as_ref().unwrap().document_path.is_some());
        assert_eq!(by_name("Other").parent, Some(archive.id));
        assert!(by_name("Other").import.is_none());
        assert!(archive.sort_key < by_name("Sunday").sort_key);
        assert!(by_name("Sunday").sort_key < by_name("Empty").sort_key);
        assert!(matches!(
            &by_name("Empty").kind,
            PlaylistNodeKind::Playlist { items, .. } if items.is_empty()
        ));

        let sunday = by_name("Sunday");
        assert_eq!(sunday.id.0, Uuid::from_u128(102));
        let PlaylistNodeKind::Playlist { items, purpose } = &sunday.kind else {
            panic!("not a playlist")
        };
        assert_eq!(*purpose, PlaylistPurpose::Service);
        assert_eq!(items.len(), 5);
        // New item IDs, source UUID in provenance (item 1 collides with presentation 1).
        assert_eq!(items[0].id, ids::playlist_item_id(sunday.id, 0));
        assert_ne!(items[0].id.0, Uuid::from_u128(1));
        assert_eq!(
            items[0].import.as_ref().unwrap().original_id.as_deref(),
            Some(Uuid::from_u128(1).to_string().as_str())
        );
        // Resolved, arrangement kept; the name follows the presentation.
        match &items[0].kind {
            PlaylistItemKind::Presentation {
                presentation,
                arrangement,
                unresolved,
            } => {
                assert_eq!(presentation.id.0, Uuid::from_u128(1));
                assert_eq!(arrangement.map(|a| a.0), Some(Uuid::from_u128(50)));
                assert!(unresolved.is_none());
            }
            k => panic!("{k:?}"),
        }
        assert_eq!(items[0].name, None);
        // Dangling arrangement dropped, original kept in unmapped.
        assert!(matches!(
            &items[1].kind,
            PlaylistItemKind::Presentation {
                arrangement: None,
                ..
            }
        ));
        assert_eq!(
            items[1].import.as_ref().unwrap().unmapped["arrangement"],
            Uuid::from_u128(51).to_string()
        );
        assert_eq!(ctx.report.dangling_arrangements.playlist_items, 1);
        // Same name in two libraries: the path decides.
        assert!(matches!(
            &items[2].kind,
            PlaylistItemKind::Presentation { presentation, .. } if presentation.id.0 == Uuid::from_u128(3)
        ));
        assert!(matches!(items[3].kind, PlaylistItemKind::Media(_)));
        assert_eq!(items[3].name.as_deref(), Some("walk-in.mp4"));
        assert!(matches!(items[4].kind, PlaylistItemKind::VideoInput(_)));

        // Case-insensitive path, renamed and hidden item.
        let PlaylistNodeKind::Playlist { items, .. } = &by_name("Other").kind else {
            panic!()
        };
        assert!(!items[0].enabled);
        assert_eq!(items[0].name.as_deref(), Some("Renamed"));
        assert!(matches!(
            &items[0].kind,
            PlaylistItemKind::Presentation { presentation, .. } if presentation.id.0 == Uuid::from_u128(1)
        ));
        assert!(ctx.report.unresolved_playlist_items.is_empty());
    }

    #[test]
    fn library_scoped_fallbacks_and_unresolved() {
        let mut ctx = ctx_with_presentations();
        // Basename fallback inside the library named by the path.
        let mut url = show_url("Libraries/Songs/Sub/Alpha.pro");
        let r = resolve_presentation(&mut ctx, &url, "x");
        assert!(
            matches!(r, PresentationResolution::LibraryFallback(e) if e.id.0 == Uuid::from_u128(1))
        );
        // Item-name fallback.
        url = show_url("Libraries/Songs/Gone.pro");
        let r = resolve_presentation(&mut ctx, &url, "shared");
        assert!(
            matches!(r, PresentationResolution::LibraryFallback(e) if e.id.0 == Uuid::from_u128(2))
        );
        // Never across libraries: "Alpha" exists only in Songs.
        url = show_url("Libraries/Talks/Alpha.pro");
        assert!(matches!(
            resolve_presentation(&mut ctx, &url, "Alpha"),
            PresentationResolution::Unresolved { .. }
        ));
        // Unknown library.
        url = show_url("Libraries/Nope/Alpha.pro");
        assert!(matches!(
            resolve_presentation(&mut ctx, &url, "Alpha"),
            PresentationResolution::Unresolved { .. }
        ));
        // No local part: the Libraries tail of the absolute string is used.
        let abs_only = pp::Url {
            platform: pp::url::Platform::Win32 as i32,
            storage: Some(pp::url::Storage::AbsoluteString(
                "D:\\Data\\ProPresenter\\Libraries\\Talks\\Shared.pro".into(),
            )),
            relative_file_path: None,
        };
        assert!(matches!(
            resolve_presentation(&mut ctx, &abs_only, ""),
            PresentationResolution::Exact(e) if e.id.0 == Uuid::from_u128(3)
        ));

        // An unresolved item keeps its source reference and is reported.
        let p = pp::playlist_item::Presentation {
            document_path: Some(show_url("Libraries/Talks/Alpha.pro")),
            arrangement: Some(uuid(9)),
            ..Default::default()
        };
        let (kind, dropped) = presentation_item(&mut ctx, &p, "Alpha", "Folder/Sunday");
        let PlaylistItemKind::Presentation {
            presentation,
            unresolved: Some(origin),
            arrangement: None,
        } = kind
        else {
            panic!("{kind:?}")
        };
        assert_eq!(presentation.name.as_deref(), Some("Alpha"));
        assert_eq!(origin.root_kind.as_deref(), Some("ROOT_SHOW"));
        assert_eq!(
            origin.relative_path.as_deref(),
            Some("Libraries/Talks/Alpha.pro")
        );
        assert_eq!(origin.platform, model::SourcePlatform::Windows);
        assert_eq!(dropped, Some(Uuid::from_u128(9).to_string()));
        let rep = &ctx.report.unresolved_playlist_items[0];
        assert_eq!(rep.playlist, "Folder/Sunday");
        assert_eq!(rep.document_path, "Libraries/Talks/Alpha.pro");
    }

    #[test]
    fn header_placeholder_planning_center_and_unmapped() {
        let mut ctx = ctx_with_presentations();
        let header = pp::PlaylistItem {
            uuid: Some(uuid(1)),
            name: "Worship".into(),
            item_type: Some(pp::playlist_item::ItemType::Header(
                pp::playlist_item::Header {
                    color: Some(pp::Color {
                        red: 1.0,
                        green: 0.0,
                        blue: 0.0,
                        alpha: 1.0,
                    }),
                    actions: vec![pp::Action::default()],
                },
            )),
            tags: vec![uuid(7)],
            ..Default::default()
        };
        let pid = PlaylistId::from_uuid(Uuid::from_u128(5));
        let h = playlist_item_with(&mut ctx, &header, pid, 0, "P", stub_media);
        assert!(
            matches!(&h.kind, PlaylistItemKind::Header { name, color: Some(_) } if name == "Worship")
        );
        assert_eq!(h.name, None);
        let un = &h.import.as_ref().unwrap().unmapped;
        assert!(un["actions"].is_array());
        assert!(un["tags"].is_array());

        let ph = pp::PlaylistItem {
            name: "Special song".into(),
            item_type: Some(pp::playlist_item::ItemType::Placeholder(Box::default())),
            ..Default::default()
        };
        let p = playlist_item_with(&mut ctx, &ph, pid, 1, "P", stub_media);
        assert_eq!(p.kind, PlaylistItemKind::Placeholder);
        assert_eq!(p.name.as_deref(), Some("Special song"));
        assert!(p.import.is_none());

        let pc = pp::PlaylistItem {
            item_type: Some(pp::playlist_item::ItemType::PlanningCenter(Box::default())),
            ..Default::default()
        };
        let p = playlist_item_with(&mut ctx, &pc, pid, 2, "P", stub_media);
        assert!(
            matches!(p.kind, PlaylistItemKind::Unsupported { ref source_type, .. } if source_type == "planningCenter")
        );

        // A cue without media.
        let cue = pp::PlaylistItem {
            item_type: Some(pp::playlist_item::ItemType::Cue(pp::Cue::default())),
            ..Default::default()
        };
        let p = playlist_item_with(&mut ctx, &cue, pid, 3, "P", stub_media);
        assert!(
            matches!(p.kind, PlaylistItemKind::Unsupported { ref source_type, .. } if source_type == "cue")
        );
        // IDs are distinct per index.
        assert_ne!(ids::playlist_item_id(pid, 0), ids::playlist_item_id(pid, 1));
    }

    #[test]
    fn audio_purpose_and_repeated_node_uuid() {
        let mut ctx = ctx_with_presentations();
        let doc = pp::PlaylistDocument {
            root_node: Some(pp::Playlist {
                r#type: pp::playlist::Type::Root as i32,
                children_type: Some(folders(vec![
                    node(1, "Default", items(vec![])),
                    node(1, "Playlist", items(vec![])),
                ])),
                ..Default::default()
            }),
            ..Default::default()
        };
        import_playlists_with(
            &mut ctx,
            &doc,
            "Playlists/Audio",
            PlaylistPurpose::Audio,
            stub_media,
        );
        let nodes = &ctx.out.playlist_nodes;
        assert_eq!(nodes.len(), 2);
        assert_ne!(nodes[0].id, nodes[1].id);
        assert_eq!(
            nodes[1].import.as_ref().unwrap().original_id.as_deref(),
            Some(Uuid::from_u128(1).to_string().as_str())
        );
        assert!(nodes.iter().all(|n| matches!(
            n.kind,
            PlaylistNodeKind::Playlist {
                purpose: PlaylistPurpose::Audio,
                ..
            }
        )));
        let _ = LibraryId::from_uuid(Uuid::nil());
    }

    #[test]
    fn derived_node_ids_never_collide() {
        // Same-named folders whose children have no usable UUID (missing, or a third copy of
        // a repeated one) used to derive the same child ID from (document, ancestor names,
        // name, sort key): two documents with one ID, one of them overwritten on write.
        let bare = |name: &str, children| pp::Playlist {
            name: name.into(),
            children_type: Some(children),
            ..Default::default()
        };
        let doc = pp::PlaylistDocument {
            root_node: Some(pp::Playlist {
                r#type: pp::playlist::Type::Root as i32,
                children_type: Some(folders(vec![
                    bare("Sunday", folders(vec![bare("Service", items(vec![]))])),
                    bare("Sunday", folders(vec![bare("Service", items(vec![]))])),
                    node(5, "Copy", folders(vec![node(6, "Service", items(vec![]))])),
                    node(5, "Copy", folders(vec![node(6, "Service", items(vec![]))])),
                    node(5, "Copy", folders(vec![node(6, "Service", items(vec![]))])),
                ])),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut ctx = ctx_with_presentations();
        import_playlists_with(
            &mut ctx,
            &doc,
            "Playlists/Library",
            PlaylistPurpose::Service,
            stub_media,
        );
        let nodes = &ctx.out.playlist_nodes;
        assert_eq!(nodes.len(), 10);
        let unique: std::collections::BTreeSet<_> = nodes.iter().map(|n| n.id).collect();
        assert_eq!(unique.len(), nodes.len(), "duplicate playlist node IDs");
        // Every child still points at its own parent.
        for pair in nodes.chunks(2) {
            assert_eq!(pair[1].parent, Some(pair[0].id));
        }
        // Deterministic across runs.
        let mut again = ctx_with_presentations();
        import_playlists_with(
            &mut again,
            &doc,
            "Playlists/Library",
            PlaylistPurpose::Service,
            stub_media,
        );
        let ids = |c: &ImportContext| {
            c.out
                .playlist_nodes
                .iter()
                .map(|n| n.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&ctx), ids(&again));
    }

    #[test]
    fn libraries_tail_cuts_paths() {
        let u = |s: &str| pp::Url {
            storage: Some(pp::url::Storage::AbsoluteString(s.into())),
            ..Default::default()
        };
        assert_eq!(
            libraries_tail(&u("C:\\x\\Libraries\\A\\b.pro")).as_deref(),
            Some("Libraries/A/b.pro")
        );
        assert_eq!(libraries_tail(&u("C:\\x\\Libraries\\b.pro")), None);
        assert_eq!(libraries_tail(&u("/x/y.pro")), None);
        // A file URL is percent-decoded (A10-3).
        assert_eq!(
            libraries_tail(&u(
                "file:///Volumes/Data/Show/Libraries/Sunday%20Service/Opening%20Song.pro"
            ))
            .as_deref(),
            Some("Libraries/Sunday Service/Opening Song.pro")
        );
        // The plain relative path wins over the encoded string.
        let both = pp::Url {
            storage: Some(pp::url::Storage::AbsoluteString(
                "file:///Volumes/Data/Show/Libraries/Old%20Name/A.pro".into(),
            )),
            relative_file_path: Some(pp::url::RelativeFilePath::External(
                pp::url::ExternalRelativePath {
                    path: "Show/Libraries/Sunday Service/A.pro".into(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        assert_eq!(
            libraries_tail(&both).as_deref(),
            Some("Libraries/Sunday Service/A.pro")
        );
    }
}
