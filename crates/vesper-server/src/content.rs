//! Editing library content: presentations (edit ops and CRUD), libraries, playlists and
//! playlist folders (CRUD and item ops), media bins (CRUD and item ops) and global groups
//! (CRUD).
//!
//! Like [`crate::config`], every change is **planned** here against a collection's
//! [`CollectionContent`] without side effects ([`plan`]); the Codex writer
//! ([`crate::codex::writer`]) then persists the plan's writes atomically
//! ([`crate::store::apply`], never over a newer schema), has the show actor swap in the new data
//! ([`ShowData::with_content`](crate::data::ShowData::with_content)) and the show state follow
//! ([`ShowState::revalidate_edit`]: an edit of the live slide re-renders at once, a deleted live
//! slide stays on screen until the next trigger), and broadcasts `documentChanged` with the new
//! revision, the sender's client ID, the collection and, for edits, the applied ops.
//!
//! - **Edits** (`POST …/edits`) apply to the current revision; see [`crate::edit`]. A
//!   presentation edit with `If-Match` applies only to that revision (`409` otherwise).
//! - **CRUD** routes take `If-Match` (the document's revision, a content hash) on updates,
//!   moves and deletes; a mismatch is `409` with the current revision.
//! - A library can only be deleted when it holds no presentation. Deleting a presentation
//!   leaves playlist items that refer to it (they show as missing). Deleting a folder deletes
//!   its subtree. Delete answers carry the deleted documents, so undo can recreate them
//!   (`POST` with `{"document": …}` keeps their IDs).
//!
//! [`ShowState::revalidate_edit`]: crate::show::ShowState::revalidate_edit

use crate::config::{ConfigError, revision};
use crate::data::{CollectionContent, ContentChange};
use crate::edit;
use crate::store::DocWrite;
use model::{
    Document, DocumentKind, GlobalGroup, GlobalGroupId, Library, LibraryId, MediaBin, MediaBinId,
    MediaBinKind, PlaylistId, PlaylistNode, PlaylistNodeKind, Presentation, PresentationId, Size,
    Theme, ThemeId, sort_key_between,
};
use protocol::{
    ApiError, CreateGlobalGroup, CreateLibrary, CreateMediaBin, CreatePlaylistNode,
    CreatePresentation, CreateTheme, DeletedDocuments, DocumentChange, DocumentRevision,
    DuplicatePlaylist, DuplicatePresentation, DuplicateTheme, EditConflict, EditOps, ErrorCode,
    MediaBinEdit, MediaBinEditResult, MovePresentation, PlaylistEdit, PlaylistEditResult,
    PlaylistNodeType, PresentationEdit, PresentationEditResult, UpdateGlobalGroup, UpdateLibrary,
    UpdateMediaBin, UpdatePlaylistNode, UpdateTheme,
};
use serde_json::Value;
use std::sync::Arc;

/// Longest library / presentation / playlist name, characters.
pub const MAX_NAME: usize = 200;

/// A content change.
#[derive(Clone, Debug)]
pub enum ContentOp {
    /// `POST /presentations/{id}/edits` (with its `If-Match`, if any).
    EditPresentation(PresentationId, PresentationEdit, Option<String>),
    /// `POST /playlists/{id}/edits`.
    EditPlaylist(PlaylistId, PlaylistEdit),
    /// `POST /libraries`.
    CreateLibrary(Box<CreateLibrary>),
    /// `PUT /libraries/{id}`.
    UpdateLibrary(LibraryId, UpdateLibrary, Option<String>),
    /// `DELETE /libraries/{id}`.
    DeleteLibrary(LibraryId, Option<String>),
    /// `POST /presentations`.
    CreatePresentation(Box<CreatePresentation>),
    /// `POST /presentations/{id}/duplicate`.
    DuplicatePresentation(PresentationId, DuplicatePresentation),
    /// `POST /presentations/{id}/move`.
    MovePresentation(PresentationId, MovePresentation, Option<String>),
    /// `DELETE /presentations/{id}`.
    DeletePresentation(PresentationId, Option<String>),
    /// `POST /playlists`.
    CreatePlaylist(Box<CreatePlaylistNode>),
    /// `PUT /playlists/{id}`.
    UpdatePlaylist(PlaylistId, UpdatePlaylistNode, Option<String>),
    /// `POST /playlists/{id}/duplicate`.
    DuplicatePlaylist(PlaylistId, DuplicatePlaylist),
    /// `DELETE /playlists/{id}`.
    DeletePlaylist(PlaylistId, Option<String>),
    /// `POST /themes`.
    CreateTheme(Box<CreateTheme>, Size),
    /// `PUT /themes/{id}`.
    UpdateTheme(ThemeId, UpdateTheme, Option<String>),
    /// `POST /themes/{id}/duplicate`.
    DuplicateTheme(ThemeId, DuplicateTheme),
    /// `DELETE /themes/{id}`.
    DeleteTheme(ThemeId, DeleteTheme, Option<String>),
    /// `POST /themes/{id}/edits`.
    EditTheme(ThemeId, PresentationEdit),
    /// `POST /media-bins`.
    CreateMediaBin(Box<CreateMediaBin>),
    /// `PUT /media-bins/{id}`.
    UpdateMediaBin(MediaBinId, UpdateMediaBin, Option<String>),
    /// `DELETE /media-bins/{id}`.
    DeleteMediaBin(MediaBinId, Option<String>),
    /// `POST /media-bins/{id}/edits`.
    EditMediaBin(MediaBinId, MediaBinEdit),
    /// `POST /groups`.
    CreateGlobalGroup(Box<CreateGlobalGroup>),
    /// `PUT /groups/{id}`.
    UpdateGlobalGroup(GlobalGroupId, UpdateGlobalGroup, Option<String>),
    /// `DELETE /groups/{id}`.
    DeleteGlobalGroup(GlobalGroupId, Option<String>),
}

/// What happens to the presentations that follow a theme being deleted.
#[derive(Clone, Debug, Default)]
pub struct DeleteTheme {
    /// They follow this theme instead (template slides matched by name, else its first).
    pub reassign: Option<ThemeId>,
    /// They keep their look as plain copies.
    pub unlink: bool,
}

/// What a successful change answers.
#[derive(Clone, Debug)]
pub enum ContentResult {
    /// A presentation edit.
    PresentationEdit(PresentationEditResult),
    /// A playlist edit.
    PlaylistEdit(PlaylistEditResult),
    /// A media bin edit.
    MediaBinEdit(MediaBinEditResult),
    /// A created / updated document (`201` for creates).
    Revision(DocumentRevision, bool),
    /// Deleted documents.
    Deleted(DeletedDocuments),
}

pub use crate::data::ChangeEvent;

/// A validated change.
#[derive(Debug)]
pub struct ContentPlan {
    /// New / removed documents.
    pub change: ContentChange,
    /// Files to write / delete, in order.
    pub writes: Vec<DocWrite>,
    /// Events once applied.
    pub events: Vec<ChangeEvent>,
    /// The answer.
    pub result: ContentResult,
}

fn err(code: ErrorCode, msg: impl Into<String>) -> ConfigError {
    ApiError::new(code, msg).into()
}

fn now() -> String {
    crate::auth::rfc3339(std::time::SystemTime::now(), true)
}

/// True for a trimmed, non-empty name the content rules take: at most [`MAX_NAME`]
/// characters, no control characters.
pub fn name_ok(t: &str) -> bool {
    !t.is_empty() && t.chars().count() <= MAX_NAME && !t.chars().any(char::is_control)
}

/// The default name of a copy: `"<name> copy"`, the name shortened (and any control
/// characters an older document kept made spaces) so the result is a valid name.
pub fn copy_name(name: &str) -> String {
    const SUFFIX: &str = " copy";
    let base: String = name
        .trim()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_NAME - SUFFIX.chars().count())
        .collect();
    let base = base.trim_end();
    if base.is_empty() {
        "Copy".to_string()
    } else {
        format!("{base}{SUFFIX}")
    }
}

fn clean_name(name: &str, field: &str) -> Result<String, ConfigError> {
    let t = name.trim();
    if t.is_empty() {
        return Err(ApiError::invalid(field, "a name is needed").into());
    }
    if !name_ok(t) {
        return Err(ApiError::invalid(
            field,
            format!("names are at most {MAX_NAME} characters, without control characters"),
        )
        .into());
    }
    Ok(t.to_string())
}

fn check_rev<T: serde::Serialize>(
    doc: &T,
    if_match: &Option<String>,
    what: &str,
) -> Result<(), ConfigError> {
    match if_match.as_deref().filter(|w| *w != "*") {
        Some(want) if revision(doc) != want => Err(ConfigError {
            error: ApiError::new(
                ErrorCode::Conflict,
                format!("{what} was changed by another operator; reload it"),
            ),
            current: Some(serde_json::json!({ "revision": revision(doc) })),
        }),
        _ => Ok(()),
    }
}

fn put<T: Document>(doc: &T) -> Result<DocWrite, ConfigError> {
    DocWrite::put(doc).map_err(|e| err(ErrorCode::Internal, format!("{e:#}")))
}

fn doc_rev<T: Document>(doc: &T, created: bool) -> ContentResult {
    ContentResult::Revision(
        DocumentRevision {
            kind: T::KIND,
            id: doc.id().to_string(),
            revision: revision(doc),
        },
        created,
    )
}

fn event<T: Document>(doc: &T, change: DocumentChange, ops: Option<EditOps>) -> ChangeEvent {
    ChangeEvent {
        kind: T::KIND,
        id: doc.id(),
        change,
        revision: Some(revision(doc)),
        ops,
    }
}

fn removed(kind: DocumentKind, id: uuid::Uuid) -> ChangeEvent {
    ChangeEvent {
        kind,
        id,
        change: DocumentChange::Removed,
        revision: None,
        ops: None,
    }
}

fn presentation(
    data: &CollectionContent,
    id: PresentationId,
) -> Result<&Presentation, ConfigError> {
    data.presentation(id).ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("presentation {} not found", id.0),
        )
    })
}

fn theme(data: &CollectionContent, id: ThemeId) -> Result<&Theme, ConfigError> {
    data.themes
        .get(&id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("theme {} not found", id.0)))
}

/// `base`, or `base 2`, `base 3`, … when a theme has that name.
fn free_theme_name(data: &CollectionContent, base: &str) -> String {
    let taken = |n: &str| data.themes.values().any(|t| t.name.eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_string())
}

/// A changed theme: written with every presentation that follows it, re-resolved.
#[allow(clippy::too_many_arguments)]
fn theme_changed(
    data: &CollectionContent,
    cur: &Theme,
    t: Theme,
    ops: Option<EditOps>,
    change: &mut ContentChange,
    writes: &mut Vec<DocWrite>,
    events: &mut Vec<ChangeEvent>,
) -> Result<ContentResult, ConfigError> {
    let r = doc_rev(&t, false);
    if t != *cur {
        let mut themes = (*data.themes).clone();
        themes.insert(t.id, t.clone());
        writes.push(put(&t)?);
        events.push(event(&t, DocumentChange::Changed, ops));
        for p in crate::themes::propagate(data, t.id, &themes, None) {
            writes.push(put(&p)?);
            events.push(event(&p, DocumentChange::Changed, None));
            change.presentations.push((p.id, Some(Arc::new(p))));
        }
    }
    change.themes.push((t.id, Some(t)));
    Ok(r)
}

fn library_exists(data: &CollectionContent, id: LibraryId) -> Result<(), ConfigError> {
    if data.libraries.contains_key(&id) {
        Ok(())
    } else {
        Err(ApiError::invalid("library", format!("library {} not found", id.0)).into())
    }
}

/// Siblings of a playlist node position in display order (sort key, then name, then ID).
fn siblings(
    data: &CollectionContent,
    parent: Option<PlaylistId>,
    except: PlaylistId,
) -> Vec<PlaylistNode> {
    let mut v: Vec<PlaylistNode> = data
        .playlists
        .values()
        .filter(|n| n.id != except && n.parent.filter(|p| data.playlists.contains_key(p)) == parent)
        .cloned()
        .collect();
    v.sort_by(|a, b| {
        a.sort_key
            .cmp(&b.sort_key)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.0.cmp(&b.id.0))
    });
    v
}

/// Places `node` at index `at` among `sibs` (display order, without `node`): sets its sort key, renumbering the siblings when their keys are not strictly increasing.
/// Returns the siblings whose key changed.
fn place(node: &mut PlaylistNode, sibs: &[PlaylistNode], at: usize) -> Vec<PlaylistNode> {
    let at = at.min(sibs.len());
    let keys_ok = sibs.windows(2).all(|w| w[0].sort_key < w[1].sort_key)
        && sibs.iter().all(|s| !s.sort_key.is_empty());
    if keys_ok {
        let before = at.checked_sub(1).map(|i| sibs[i].sort_key.as_str());
        let after = sibs.get(at).map(|s| s.sort_key.as_str());
        node.sort_key = sort_key_between(before, after);
        return Vec::new();
    }
    let mut changed = Vec::new();
    let mut prev: Option<String> = None;
    for i in 0..=sibs.len() {
        if i == at {
            let k = sort_key_between(prev.as_deref(), None);
            node.sort_key = k.clone();
            prev = Some(k);
        }
        if let Some(s) = sibs.get(i) {
            let k = sort_key_between(prev.as_deref(), None);
            if s.sort_key != k {
                let mut s = s.clone();
                s.sort_key = k.clone();
                changed.push(s);
            }
            prev = Some(k);
        }
    }
    changed
}

fn position_after(sibs: &[PlaylistNode], after: Option<PlaylistId>) -> Result<usize, ConfigError> {
    match after {
        None => Ok(0),
        Some(a) => sibs
            .iter()
            .position(|s| s.id == a)
            .map(|i| i + 1)
            .ok_or_else(|| ApiError::invalid("after", "that sibling is not in this folder").into()),
    }
}

fn check_parent(
    data: &CollectionContent,
    parent: Option<PlaylistId>,
    node: Option<PlaylistId>,
) -> Result<(), ConfigError> {
    let Some(pid) = parent else { return Ok(()) };
    match data.playlists.get(&pid).map(|n| &n.kind) {
        Some(PlaylistNodeKind::Folder) => {}
        Some(_) => return Err(ApiError::invalid("parent", "the parent must be a folder").into()),
        None => return Err(ApiError::invalid("parent", "that folder does not exist").into()),
    }
    // No cycles: the new parent must not be the node or below it.
    let mut cur = Some(pid);
    let mut guard = 0;
    while let Some(c) = cur {
        if Some(c) == node {
            return Err(ApiError::invalid("parent", "a folder cannot go inside itself").into());
        }
        cur = data.playlists.get(&c).and_then(|n| n.parent);
        guard += 1;
        if guard > 1000 {
            break;
        }
    }
    Ok(())
}

/// Places a document at index `at` among its siblings `sibs` (display order, without it):
/// its new sort key, and the siblings whose keys had to be renumbered (when theirs were not
/// strictly increasing) with their new keys.
fn place_key<T: Clone>(
    sibs: &[T],
    at: usize,
    key: impl Fn(&T) -> &str,
    set: impl Fn(&mut T, String),
) -> (String, Vec<T>) {
    let at = at.min(sibs.len());
    let keys_ok =
        sibs.windows(2).all(|w| key(&w[0]) < key(&w[1])) && sibs.iter().all(|s| !key(s).is_empty());
    if keys_ok {
        let before = at.checked_sub(1).map(|i| key(&sibs[i]));
        let after = sibs.get(at).map(&key);
        return (sort_key_between(before, after), Vec::new());
    }
    let mut changed = Vec::new();
    let mut prev: Option<String> = None;
    let mut mine = String::new();
    for i in 0..=sibs.len() {
        if i == at {
            mine = sort_key_between(prev.as_deref(), None);
            prev = Some(mine.clone());
        }
        if let Some(s) = sibs.get(i) {
            let k = sort_key_between(prev.as_deref(), None);
            if key(s) != k {
                let mut s = s.clone();
                set(&mut s, k.clone());
                changed.push(s);
            }
            prev = Some(k);
        }
    }
    (mine, changed)
}

/// The index after `after` among `ids` (`None` = first).
fn index_after(ids: &[uuid::Uuid], after: Option<uuid::Uuid>) -> Result<usize, ConfigError> {
    match after {
        None => Ok(0),
        Some(a) => ids
            .iter()
            .position(|s| *s == a)
            .map(|i| i + 1)
            .ok_or_else(|| ApiError::invalid("after", "that sibling is not in this list").into()),
    }
}

/// The bins of `folder` and `browser` in display order, without `except`.
fn bin_siblings(
    data: &CollectionContent,
    folder: &[String],
    browser: model::BinBrowser,
    except: MediaBinId,
) -> Vec<MediaBin> {
    // `media_bins` is kept sorted (folder, sort key, name, ID).
    data.media_bins
        .iter()
        .filter(|b| b.id != except && b.folder == folder && same_browser(b.browser, browser))
        .cloned()
        .collect()
}

fn same_browser(a: model::BinBrowser, b: model::BinBrowser) -> bool {
    use model::BinBrowser::Audio;
    (a == Audio) == (b == Audio)
}

/// A folder path the content rules take: each segment a valid name.
fn clean_folder(folder: &[String], field: &str) -> Result<Vec<String>, ConfigError> {
    if folder.len() > 8 {
        return Err(ApiError::invalid(field, "folders nest at most 8 deep").into());
    }
    folder.iter().map(|f| clean_name(f, field)).collect()
}

fn media_bin(data: &CollectionContent, id: MediaBinId) -> Result<&MediaBin, ConfigError> {
    data.media_bins
        .iter()
        .find(|b| b.id == id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("media bin {} not found", id.0)))
}

fn global_group(data: &CollectionContent, id: GlobalGroupId) -> Result<&GlobalGroup, ConfigError> {
    data.global_groups
        .iter()
        .find(|g| g.id == id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("group {} not found", id.0)))
}

/// Puts a bin (with its renumbered siblings) into the plan.
fn put_bin(
    b: MediaBin,
    sibs: Vec<MediaBin>,
    change: &mut ContentChange,
    writes: &mut Vec<DocWrite>,
    events: &mut Vec<ChangeEvent>,
    kind: DocumentChange,
) -> Result<(), ConfigError> {
    for s in sibs {
        writes.push(put(&s)?);
        events.push(event(&s, DocumentChange::Changed, None));
        change.media_bins.push((s.id, Some(s)));
    }
    writes.push(put(&b)?);
    events.push(event(&b, kind, None));
    change.media_bins.push((b.id, Some(b)));
    Ok(())
}

/// Puts a group (with its renumbered siblings) into the plan.
fn put_group(
    g: GlobalGroup,
    sibs: Vec<GlobalGroup>,
    change: &mut ContentChange,
    writes: &mut Vec<DocWrite>,
    events: &mut Vec<ChangeEvent>,
    kind: DocumentChange,
) -> Result<(), ConfigError> {
    for s in sibs {
        writes.push(put(&s)?);
        events.push(event(&s, DocumentChange::Changed, None));
        change.global_groups.push((s.id, Some(s)));
    }
    writes.push(put(&g)?);
    events.push(event(&g, kind, None));
    change.global_groups.push((g.id, Some(g)));
    Ok(())
}

/// Plans a change (no side effects).
pub fn plan(data: &CollectionContent, op: ContentOp) -> Result<ContentPlan, ConfigError> {
    let mut change = ContentChange::default();
    let mut writes = Vec::new();
    let mut events = Vec::new();
    let result = match op {
        ContentOp::EditPresentation(id, edit, if_match) => {
            let cur = presentation(data, id)?;
            check_rev(cur, &if_match, "this presentation")?;
            let mut p = cur.clone();
            let applied = edit::apply_presentation_with(&mut p, &edit.ops, Some(&data.themes))
                .map_err(|f| ConfigError {
                    error: ApiError::new(f.code, f.message),
                    current: serde_json::to_value(EditConflict {
                        revision: Some(revision(cur)),
                        op_index: f.op_index,
                        slide: f.slide,
                        element: f.element,
                        text: f.text,
                    })
                    .ok(),
                })?;
            if p != *cur {
                p.modified_at = Some(now());
                writes.push(put(&p)?);
                events.push(event(
                    &p,
                    DocumentChange::Changed,
                    Some(EditOps::Presentation(applied.applied.clone())),
                ));
            }
            let rev = revision(&p);
            change.presentations.push((id, Some(Arc::new(p))));
            ContentResult::PresentationEdit(PresentationEditResult {
                revision: rev,
                applied: applied.applied,
                inverse: applied.inverse,
            })
        }
        ContentOp::EditPlaylist(id, edit) => {
            let cur = data
                .playlists
                .get(&id)
                .ok_or_else(|| err(ErrorCode::NotFound, format!("playlist {} not found", id.0)))?;
            let mut node = cur.clone();
            let PlaylistNodeKind::Playlist { items, .. } = &mut node.kind else {
                return Err(err(ErrorCode::BadRequest, "folders have no items"));
            };
            let applied = edit::apply_playlist(items, &edit.ops).map_err(|f| ConfigError {
                error: ApiError::new(f.code, f.message),
                current: serde_json::to_value(EditConflict {
                    revision: Some(revision(cur)),
                    op_index: f.op_index,
                    ..EditConflict::default()
                })
                .ok(),
            })?;
            if node != *cur {
                writes.push(put(&node)?);
                events.push(event(
                    &node,
                    DocumentChange::Changed,
                    Some(EditOps::Playlist(applied.applied.clone())),
                ));
            }
            let rev = revision(&node);
            change.playlists.push((id, Some(node)));
            ContentResult::PlaylistEdit(PlaylistEditResult {
                revision: rev,
                applied: applied.applied,
                inverse: applied.inverse,
            })
        }
        ContentOp::CreateLibrary(body) => {
            let body = *body;
            let lib = match body.document {
                Some(mut l) => {
                    if data.libraries.contains_key(&l.id) {
                        return Err(err(
                            ErrorCode::Conflict,
                            format!("library {} already exists", l.id.0),
                        ));
                    }
                    l.schema_version = model::SCHEMA_VERSION;
                    l.name = clean_name(&l.name, "document.name")?;
                    l
                }
                None => {
                    let last = data.libraries.values().map(|l| l.sort_key.as_str()).max();
                    Library {
                        name: clean_name(body.name.as_deref().unwrap_or("Library"), "name")?,
                        sort_key: sort_key_between(last.filter(|k| !k.is_empty()), None),
                        ..Library::default()
                    }
                }
            };
            writes.push(put(&lib)?);
            events.push(event(&lib, DocumentChange::Added, None));
            let r = doc_rev(&lib, true);
            change.libraries.push((lib.id, Some(lib)));
            r
        }
        ContentOp::UpdateLibrary(id, body, if_match) => {
            let cur = data
                .libraries
                .get(&id)
                .ok_or_else(|| err(ErrorCode::NotFound, format!("library {} not found", id.0)))?;
            check_rev(cur, &if_match, "this library")?;
            let mut l = cur.clone();
            l.name = clean_name(&body.name, "name")?;
            if l != *cur {
                writes.push(put(&l)?);
                events.push(event(&l, DocumentChange::Changed, None));
            }
            let r = doc_rev(&l, false);
            change.libraries.push((id, Some(l)));
            r
        }
        ContentOp::DeleteLibrary(id, if_match) => {
            let cur = data
                .libraries
                .get(&id)
                .ok_or_else(|| err(ErrorCode::NotFound, format!("library {} not found", id.0)))?;
            check_rev(cur, &if_match, "this library")?;
            let n = data
                .presentations
                .iter()
                .filter(|p| p.library == Some(id))
                .count();
            if n > 0 {
                return Err(err(
                    ErrorCode::Conflict,
                    format!(
                        "the library “{}” still holds {n} presentation{}; delete or move them first",
                        cur.name,
                        if n == 1 { "" } else { "s" }
                    ),
                ));
            }
            writes.push(DocWrite::delete::<Library>(id.0));
            events.push(removed(DocumentKind::Library, id.0));
            change.libraries.push((id, None));
            ContentResult::Deleted(DeletedDocuments {
                kind: DocumentKind::Library,
                documents: vec![serde_json::to_value(cur).unwrap_or(Value::Null)],
            })
        }
        ContentOp::CreatePresentation(body) => {
            let body = *body;
            let p = match body.document {
                Some(mut p) => {
                    if data.presentation(p.id).is_some() {
                        return Err(err(
                            ErrorCode::Conflict,
                            format!("presentation {} already exists", p.id.0),
                        ));
                    }
                    if let Some(l) = p.library {
                        library_exists(data, l)?;
                    }
                    p.schema_version = model::SCHEMA_VERSION;
                    // A language that is not a code is dropped (as `setPresentation` refuses it).
                    p.language = p.language.as_deref().and_then(model::language_code);
                    p
                }
                None => {
                    if let Some(l) = body.library {
                        library_exists(data, l)?;
                    }
                    let mut p = edit::blank_presentation(&clean_name(
                        body.name.as_deref().unwrap_or("Untitled"),
                        "name",
                    )?);
                    p.library = body.library;
                    p.created_at = Some(now());
                    p.modified_at = p.created_at.clone();
                    p
                }
            };
            writes.push(put(&p)?);
            events.push(event(&p, DocumentChange::Added, None));
            let r = doc_rev(&p, true);
            change.presentations.push((p.id, Some(Arc::new(p))));
            r
        }
        ContentOp::DuplicatePresentation(id, body) => {
            let cur = presentation(data, id)?;
            let mut p = edit::duplicate_presentation(cur);
            p.name = clean_name(&body.name.unwrap_or_else(|| copy_name(&cur.name)), "name")?;
            if let Some(l) = body.library {
                library_exists(data, l)?;
                p.library = Some(l);
            }
            p.created_at = Some(now());
            p.modified_at = p.created_at.clone();
            writes.push(put(&p)?);
            events.push(event(&p, DocumentChange::Added, None));
            let r = doc_rev(&p, true);
            change.presentations.push((p.id, Some(Arc::new(p))));
            r
        }
        ContentOp::MovePresentation(id, body, if_match) => {
            let cur = presentation(data, id)?;
            check_rev(cur, &if_match, "this presentation")?;
            library_exists(data, body.library)?;
            let mut p = cur.clone();
            p.library = Some(body.library);
            if p != *cur {
                p.modified_at = Some(now());
                writes.push(put(&p)?);
                events.push(event(&p, DocumentChange::Changed, None));
            }
            let r = doc_rev(&p, false);
            change.presentations.push((id, Some(Arc::new(p))));
            r
        }
        ContentOp::DeletePresentation(id, if_match) => {
            let cur = presentation(data, id)?;
            check_rev(cur, &if_match, "this presentation")?;
            writes.push(DocWrite::delete::<Presentation>(id.0));
            events.push(removed(DocumentKind::Presentation, id.0));
            change.presentations.push((id, None));
            ContentResult::Deleted(DeletedDocuments {
                kind: DocumentKind::Presentation,
                documents: vec![serde_json::to_value(cur).unwrap_or(Value::Null)],
            })
        }
        ContentOp::CreatePlaylist(body) => {
            let body = *body;
            let (mut node, created) = match body.document {
                Some(mut n) => {
                    if data.playlists.contains_key(&n.id) {
                        return Err(err(
                            ErrorCode::Conflict,
                            format!("playlist {} already exists", n.id.0),
                        ));
                    }
                    check_parent(data, n.parent, Some(n.id))?;
                    n.schema_version = model::SCHEMA_VERSION;
                    (n, false)
                }
                None => {
                    check_parent(data, body.parent, None)?;
                    let folder = body.kind == Some(PlaylistNodeType::Folder);
                    let n = PlaylistNode {
                        parent: body.parent,
                        name: clean_name(
                            body.name.as_deref().unwrap_or(if folder {
                                "Folder"
                            } else {
                                "Playlist"
                            }),
                            "name",
                        )?,
                        kind: if folder {
                            PlaylistNodeKind::Folder
                        } else {
                            PlaylistNodeKind::Playlist {
                                purpose: body.purpose.unwrap_or_default(),
                                items: Vec::new(),
                            }
                        },
                        ..PlaylistNode::default()
                    };
                    (n, true)
                }
            };
            if created {
                let sibs = siblings(data, node.parent, node.id);
                let at = match body.after {
                    Some(a) => position_after(&sibs, Some(a))?,
                    None => sibs.len(),
                };
                for s in place(&mut node, &sibs, at) {
                    writes.push(put(&s)?);
                    events.push(event(&s, DocumentChange::Changed, None));
                    change.playlists.push((s.id, Some(s)));
                }
            }
            writes.push(put(&node)?);
            events.push(event(&node, DocumentChange::Added, None));
            let r = doc_rev(&node, true);
            change.playlists.push((node.id, Some(node)));
            r
        }
        ContentOp::UpdatePlaylist(id, body, if_match) => {
            let cur = data
                .playlists
                .get(&id)
                .ok_or_else(|| err(ErrorCode::NotFound, format!("playlist {} not found", id.0)))?;
            check_rev(cur, &if_match, "this playlist")?;
            let mut n = cur.clone();
            if let Some(name) = &body.name {
                n.name = clean_name(name, "name")?;
            }
            if let Some(parent) = body.parent {
                check_parent(data, parent, Some(id))?;
                n.parent = parent;
            }
            if body.parent.is_some() || body.after.is_some() {
                let sibs = siblings(data, n.parent, id);
                let at = match body.after {
                    Some(a) => position_after(&sibs, a)?,
                    None => sibs.len(),
                };
                for s in place(&mut n, &sibs, at) {
                    writes.push(put(&s)?);
                    events.push(event(&s, DocumentChange::Changed, None));
                    change.playlists.push((s.id, Some(s)));
                }
            }
            if n != *cur {
                writes.push(put(&n)?);
                events.push(event(&n, DocumentChange::Changed, None));
            }
            let r = doc_rev(&n, false);
            change.playlists.push((id, Some(n)));
            r
        }
        ContentOp::DuplicatePlaylist(id, body) => {
            let cur = data
                .playlists
                .get(&id)
                .ok_or_else(|| err(ErrorCode::NotFound, format!("playlist {} not found", id.0)))?;
            let PlaylistNodeKind::Playlist { purpose, items } = &cur.kind else {
                return Err(err(
                    ErrorCode::BadRequest,
                    "only playlists can be duplicated",
                ));
            };
            let mut n = PlaylistNode {
                parent: cur.parent,
                name: clean_name(&body.name.unwrap_or_else(|| copy_name(&cur.name)), "name")?,
                kind: PlaylistNodeKind::Playlist {
                    purpose: *purpose,
                    items: items
                        .iter()
                        .map(|i| model::PlaylistItem {
                            id: model::PlaylistItemId::new(),
                            ..i.clone()
                        })
                        .collect(),
                },
                ..PlaylistNode::default()
            };
            let sibs = siblings(data, cur.parent, n.id);
            let at = position_after(&sibs, Some(id))?;
            for s in place(&mut n, &sibs, at) {
                writes.push(put(&s)?);
                events.push(event(&s, DocumentChange::Changed, None));
                change.playlists.push((s.id, Some(s)));
            }
            writes.push(put(&n)?);
            events.push(event(&n, DocumentChange::Added, None));
            let r = doc_rev(&n, true);
            change.playlists.push((n.id, Some(n)));
            r
        }
        ContentOp::CreateTheme(body, canvas) => {
            let body = *body;
            let t = match body.document {
                Some(mut t) => {
                    if data.themes.contains_key(&t.id) {
                        return Err(err(
                            ErrorCode::Conflict,
                            format!("theme {} already exists", t.id.0),
                        ));
                    }
                    t.schema_version = model::SCHEMA_VERSION;
                    t.name = clean_name(&t.name, "document.name")?;
                    t
                }
                None => {
                    let name = match body.name.as_deref() {
                        Some(n) => clean_name(n, "name")?,
                        None => free_theme_name(data, "Theme"),
                    };
                    let mut t = crate::themes::starter(&name, canvas);
                    let last = data.themes.values().map(|t| t.sort_key.as_str()).max();
                    t.sort_key = sort_key_between(last.filter(|k| !k.is_empty()), None);
                    t
                }
            };
            writes.push(put(&t)?);
            events.push(event(&t, DocumentChange::Added, None));
            let r = doc_rev(&t, true);
            change.themes.push((t.id, Some(t)));
            r
        }
        ContentOp::UpdateTheme(id, body, if_match) => {
            let cur = theme(data, id)?;
            check_rev(cur, &if_match, "this theme")?;
            let mut t = cur.clone();
            if let Some(n) = &body.name {
                t.name = clean_name(n, "name")?;
            }
            if let Some(sl) = &body.second_language {
                t.second_language = sl.sane();
            }
            theme_changed(data, cur, t, None, &mut change, &mut writes, &mut events)?
        }
        ContentOp::DuplicateTheme(id, body) => {
            let cur = theme(data, id)?;
            let mut t = crate::themes::duplicate(cur);
            t.name = match body.name {
                Some(n) => clean_name(&n, "name")?,
                None => free_theme_name(data, &format!("{} copy", cur.name)),
            };
            let last = data.themes.values().map(|t| t.sort_key.as_str()).max();
            t.sort_key = sort_key_between(last.filter(|k| !k.is_empty()), None);
            writes.push(put(&t)?);
            events.push(event(&t, DocumentChange::Added, None));
            let r = doc_rev(&t, true);
            change.themes.push((t.id, Some(t)));
            r
        }
        ContentOp::DeleteTheme(id, opts, if_match) => {
            let cur = theme(data, id)?;
            check_rev(cur, &if_match, "this theme")?;
            let users = crate::themes::users(data, id);
            let to = match opts.reassign {
                Some(r) if r == id => {
                    return Err(ApiError::invalid("reassign", "reassign to another theme").into());
                }
                Some(r) => Some(data.themes.get(&r).ok_or_else(|| {
                    ConfigError::from(ApiError::invalid(
                        "reassign",
                        format!("theme {} not found", r.0),
                    ))
                })?),
                None => None,
            };
            if !users.is_empty() && to.is_none() && !opts.unlink {
                let n = users.len();
                return Err(ConfigError {
                    error: ApiError::new(
                        ErrorCode::Conflict,
                        format!(
                            "the theme “{}” is used by {n} presentation{}; choose a theme for them or unlink them",
                            cur.name,
                            if n == 1 { "" } else { "s" }
                        ),
                    ),
                    current: Some(serde_json::json!({ "usedBy": users })),
                });
            }
            let mut themes = (*data.themes).clone();
            themes.remove(&id);
            for p in crate::themes::propagate(data, id, &themes, to) {
                writes.push(put(&p)?);
                events.push(event(&p, DocumentChange::Changed, None));
                change.presentations.push((p.id, Some(Arc::new(p))));
            }
            writes.push(DocWrite::delete::<Theme>(id.0));
            events.push(removed(DocumentKind::Theme, id.0));
            change.themes.push((id, None));
            ContentResult::Deleted(DeletedDocuments {
                kind: DocumentKind::Theme,
                documents: vec![serde_json::to_value(cur).unwrap_or(Value::Null)],
            })
        }
        ContentOp::EditTheme(id, edit) => {
            let cur = theme(data, id)?;
            if let Some((k, op)) = edit
                .ops
                .iter()
                .enumerate()
                .find(|(_, o)| !crate::themes::edit_op_allowed(o))
            {
                return Err(err(
                    ErrorCode::BadRequest,
                    format!(
                        "edit refused (op {k}, {}): a theme has one list of template slides",
                        op.name()
                    ),
                ));
            }
            let mut doc = crate::themes::theme_document(cur);
            let applied =
                edit::apply_presentation(&mut doc, &edit.ops).map_err(|f| ConfigError {
                    error: ApiError::new(f.code, f.message),
                    current: serde_json::to_value(EditConflict {
                        revision: Some(revision(cur)),
                        op_index: f.op_index,
                        slide: f.slide,
                        element: f.element,
                        text: f.text,
                    })
                    .ok(),
                })?;
            let t = crate::themes::from_document(cur, &doc);
            if t.slides.is_empty() {
                return Err(err(
                    ErrorCode::BadRequest,
                    "a theme needs at least one template slide",
                ));
            }
            let ops = EditOps::Presentation(applied.applied.clone());
            theme_changed(
                data,
                cur,
                t,
                Some(ops),
                &mut change,
                &mut writes,
                &mut events,
            )?;
            let rev = change
                .themes
                .last()
                .and_then(|(_, t)| t.as_ref())
                .map(revision)
                .unwrap_or_else(|| revision(cur));
            ContentResult::PresentationEdit(PresentationEditResult {
                revision: rev,
                applied: applied.applied,
                inverse: applied.inverse,
            })
        }
        ContentOp::CreateMediaBin(body) => {
            let body = *body;
            match body.document {
                Some(mut b) => {
                    if data.media_bins.iter().any(|x| x.id == b.id) {
                        return Err(err(
                            ErrorCode::Conflict,
                            format!("media bin {} already exists", b.id.0),
                        ));
                    }
                    b.schema_version = model::SCHEMA_VERSION;
                    b.name = clean_name(&b.name, "document.name")?;
                    b.folder = clean_folder(&b.folder, "document.folder")?;
                    put_bin(
                        b.clone(),
                        Vec::new(),
                        &mut change,
                        &mut writes,
                        &mut events,
                        DocumentChange::Added,
                    )?;
                    doc_rev(&b, true)
                }
                None => {
                    let browser = body.browser.unwrap_or_default();
                    let folder = clean_folder(&body.folder.unwrap_or_default(), "folder")?;
                    let mut b = MediaBin {
                        name: clean_name(body.name.as_deref().unwrap_or("Bin"), "name")?,
                        folder,
                        browser,
                        ..MediaBin::default()
                    };
                    let sibs = bin_siblings(data, &b.folder, browser, b.id);
                    let ids: Vec<uuid::Uuid> = sibs.iter().map(|s| s.id.0).collect();
                    let at = match body.after {
                        Some(a) => index_after(&ids, Some(a.0))?,
                        None => ids.len(),
                    };
                    let (k, renumbered) =
                        place_key(&sibs, at, |s| s.sort_key.as_str(), |s, k| s.sort_key = k);
                    b.sort_key = k;
                    let r = doc_rev(&b, true);
                    put_bin(
                        b,
                        renumbered,
                        &mut change,
                        &mut writes,
                        &mut events,
                        DocumentChange::Added,
                    )?;
                    r
                }
            }
        }
        ContentOp::UpdateMediaBin(id, body, if_match) => {
            let cur = media_bin(data, id)?;
            check_rev(cur, &if_match, "this media bin")?;
            let mut b = cur.clone();
            if let Some(name) = &body.name {
                b.name = clean_name(name, "name")?;
            }
            if let Some(folder) = &body.folder {
                b.folder = clean_folder(folder, "folder")?;
            }
            let mut renumbered = Vec::new();
            if body.folder.is_some() || body.after.is_some() {
                let sibs = bin_siblings(data, &b.folder, b.browser, id);
                let ids: Vec<uuid::Uuid> = sibs.iter().map(|s| s.id.0).collect();
                let at = match body.after {
                    Some(a) => index_after(&ids, a.map(|a| a.0))?,
                    None => ids.len(),
                };
                let (k, r) = place_key(&sibs, at, |s| s.sort_key.as_str(), |s, k| s.sort_key = k);
                b.sort_key = k;
                renumbered = r;
            }
            let r = doc_rev(&b, false);
            if b != *cur || !renumbered.is_empty() {
                put_bin(
                    b,
                    renumbered,
                    &mut change,
                    &mut writes,
                    &mut events,
                    DocumentChange::Changed,
                )?;
            }
            r
        }
        ContentOp::DeleteMediaBin(id, if_match) => {
            let cur = media_bin(data, id)?;
            check_rev(cur, &if_match, "this media bin")?;
            writes.push(DocWrite::delete::<MediaBin>(id.0));
            events.push(removed(DocumentKind::MediaBin, id.0));
            change.media_bins.push((id, None));
            ContentResult::Deleted(DeletedDocuments {
                kind: DocumentKind::MediaBin,
                documents: vec![serde_json::to_value(cur).unwrap_or(Value::Null)],
            })
        }
        ContentOp::EditMediaBin(id, edit) => {
            let cur = media_bin(data, id)?;
            let mut b = cur.clone();
            let MediaBinKind::Manual { items } = &mut b.kind else {
                return Err(err(
                    ErrorCode::BadRequest,
                    "a watched folder's items follow the folder; they can't be edited",
                ));
            };
            for op in &edit.ops {
                if let protocol::MediaBinOp::AddItem { item, .. } = op
                    && !data.assets.contains_key(&item.trigger.asset)
                {
                    return Err(ApiError::invalid(
                        "ops",
                        format!("media {} is not in this collection", item.trigger.asset.0),
                    )
                    .into());
                }
            }
            let applied = edit::apply_media_bin(items, &edit.ops).map_err(|f| ConfigError {
                error: ApiError::new(f.code, f.message),
                current: serde_json::to_value(EditConflict {
                    revision: Some(revision(cur)),
                    op_index: f.op_index,
                    ..EditConflict::default()
                })
                .ok(),
            })?;
            if b != *cur {
                writes.push(put(&b)?);
                events.push(event(&b, DocumentChange::Changed, None));
            }
            let rev = revision(&b);
            change.media_bins.push((id, Some(b)));
            ContentResult::MediaBinEdit(MediaBinEditResult {
                revision: rev,
                applied: applied.applied,
                inverse: applied.inverse,
            })
        }
        ContentOp::CreateGlobalGroup(body) => {
            let body = *body;
            match body.document {
                Some(mut g) => {
                    if data.global_groups.iter().any(|x| x.id == g.id) {
                        return Err(err(
                            ErrorCode::Conflict,
                            format!("group {} already exists", g.id.0),
                        ));
                    }
                    g.schema_version = model::SCHEMA_VERSION;
                    g.name = clean_name(&g.name, "document.name")?;
                    put_group(
                        g.clone(),
                        Vec::new(),
                        &mut change,
                        &mut writes,
                        &mut events,
                        DocumentChange::Added,
                    )?;
                    doc_rev(&g, true)
                }
                None => {
                    let mut g = GlobalGroup {
                        name: clean_name(body.name.as_deref().unwrap_or("Group"), "name")?,
                        color: body.color,
                        hotkey: body.hotkey,
                        ..GlobalGroup::default()
                    };
                    let sibs: Vec<GlobalGroup> = data.global_groups.to_vec();
                    let ids: Vec<uuid::Uuid> = sibs.iter().map(|s| s.id.0).collect();
                    let at = match body.after {
                        Some(a) => index_after(&ids, Some(a.0))?,
                        None => ids.len(),
                    };
                    let (k, renumbered) =
                        place_key(&sibs, at, |s| s.sort_key.as_str(), |s, k| s.sort_key = k);
                    g.sort_key = k;
                    let r = doc_rev(&g, true);
                    put_group(
                        g,
                        renumbered,
                        &mut change,
                        &mut writes,
                        &mut events,
                        DocumentChange::Added,
                    )?;
                    r
                }
            }
        }
        ContentOp::UpdateGlobalGroup(id, body, if_match) => {
            let cur = global_group(data, id)?;
            check_rev(cur, &if_match, "this group")?;
            let mut g = cur.clone();
            if let Some(name) = &body.name {
                g.name = clean_name(name, "name")?;
            }
            if let Some(c) = body.color {
                g.color = c;
            }
            if let Some(h) = body.hotkey {
                g.hotkey = h;
            }
            let mut renumbered = Vec::new();
            if let Some(after) = body.after {
                let sibs: Vec<GlobalGroup> = data
                    .global_groups
                    .iter()
                    .filter(|x| x.id != id)
                    .cloned()
                    .collect();
                let ids: Vec<uuid::Uuid> = sibs.iter().map(|s| s.id.0).collect();
                let at = index_after(&ids, after.map(|a| a.0))?;
                let (k, r) = place_key(&sibs, at, |s| s.sort_key.as_str(), |s, k| s.sort_key = k);
                g.sort_key = k;
                renumbered = r;
            }
            let r = doc_rev(&g, false);
            if g != *cur || !renumbered.is_empty() {
                put_group(
                    g,
                    renumbered,
                    &mut change,
                    &mut writes,
                    &mut events,
                    DocumentChange::Changed,
                )?;
            }
            r
        }
        ContentOp::DeleteGlobalGroup(id, if_match) => {
            let cur = global_group(data, id)?;
            check_rev(cur, &if_match, "this group")?;
            writes.push(DocWrite::delete::<GlobalGroup>(id.0));
            events.push(removed(DocumentKind::GlobalGroup, id.0));
            change.global_groups.push((id, None));
            ContentResult::Deleted(DeletedDocuments {
                kind: DocumentKind::GlobalGroup,
                documents: vec![serde_json::to_value(cur).unwrap_or(Value::Null)],
            })
        }
        ContentOp::DeletePlaylist(id, if_match) => {
            let cur = data
                .playlists
                .get(&id)
                .ok_or_else(|| err(ErrorCode::NotFound, format!("playlist {} not found", id.0)))?;
            check_rev(cur, &if_match, "this playlist")?;
            // The subtree, parents first.
            let mut order = vec![cur.clone()];
            let mut i = 0;
            while i < order.len() {
                let parent = order[i].id;
                let mut kids: Vec<PlaylistNode> = data
                    .playlists
                    .values()
                    .filter(|n| n.parent == Some(parent) && !order.iter().any(|o| o.id == n.id))
                    .cloned()
                    .collect();
                kids.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.id.0.cmp(&b.id.0)));
                order.extend(kids);
                i += 1;
            }
            // Children are deleted first, so an interrupted delete never orphans a node.
            for n in order.iter().rev() {
                writes.push(DocWrite::delete::<PlaylistNode>(n.id.0));
                events.push(removed(DocumentKind::PlaylistNode, n.id.0));
                change.playlists.push((n.id, None));
            }
            ContentResult::Deleted(DeletedDocuments {
                kind: DocumentKind::PlaylistNode,
                documents: order
                    .iter()
                    .map(|n| serde_json::to_value(n).unwrap_or(Value::Null))
                    .collect(),
            })
        }
    };
    Ok(ContentPlan {
        change,
        writes,
        events,
        result,
    })
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn copy_names_stay_valid() {
        assert_eq!(copy_name("Song"), "Song copy");
        let long = "x".repeat(MAX_NAME);
        let c = copy_name(&long);
        assert!(name_ok(&c), "{c}");
        assert!(c.ends_with(" copy"));
        let c = copy_name("Line one\nline two");
        assert_eq!(c, "Line one line two copy");
        assert!(name_ok(&c));
        assert_eq!(copy_name("\n"), "Copy");
    }
}
