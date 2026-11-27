//! Library documents: libraries, presentations, playlists, looks, clear groups, themes.

use super::edit::{
    create_global_group, create_library, create_playlist, create_presentation, delete_global_group,
    delete_library, delete_playlist, delete_presentation, duplicate_playlist,
    duplicate_presentation, edit_playlist, edit_presentation, global_groups, move_presentation,
    update_global_group, update_library, update_playlist,
};
use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/libraries", get(libraries).post(create_library))
        .route(
            "/libraries/{id}",
            axum::routing::put(update_library).delete(delete_library),
        )
        .route("/libraries/{id}/presentations", get(library_presentations))
        .route(
            "/presentations",
            get(presentations).post(create_presentation),
        )
        .route(
            "/presentations/{id}",
            get(presentation).delete(delete_presentation),
        )
        .route("/presentations/{id}/edits", post(edit_presentation))
        .route(
            "/presentations/{id}/duplicate",
            post(duplicate_presentation),
        )
        .route("/presentations/{id}/move", post(move_presentation))
        .route("/groups", get(global_groups).post(create_global_group))
        .route(
            "/groups/{id}",
            axum::routing::put(update_global_group).delete(delete_global_group),
        )
        .route("/playlists", get(playlists).post(create_playlist))
        .route(
            "/playlists/{id}",
            get(playlist).put(update_playlist).delete(delete_playlist),
        )
        .route("/playlists/{id}/edits", post(edit_playlist))
        .route("/playlists/{id}/duplicate", post(duplicate_playlist))
        .route("/looks", get(looks))
        .route("/clear-groups", get(clear_groups))
}

fn summary(d: &ShowData, p: &Presentation) -> PresentationSummary {
    let song = p.song_info.as_ref();
    let theme = crate::themes::presentation_theme(p);
    PresentationSummary {
        id: p.id,
        name: p.name.clone(),
        library: p.library,
        category: p.category.clone(),
        slide_count: p.slides().count() as u32,
        destination: p.destination,
        modified_at: p.modified_at.clone(),
        kind: p.kind(),
        authors: song
            .and_then(|s| s.authors.clone())
            .filter(|a| !a.is_empty()),
        copyright_year: song.and_then(|s| s.copyright_year),
        ccli_number: song
            .and_then(|s| s.ccli_number.clone())
            .filter(|c| !c.is_empty()),
        theme: theme.map(|t| t.id),
        theme_name: theme.and_then(|t| {
            d.content
                .themes
                .get(&t.id)
                .map(|th| th.name.clone())
                .or_else(|| t.name.clone())
        }),
    }
}

async fn libraries(State(app): State<AppState>) -> ApiResult<Vec<LibrarySummary>> {
    let d = app.shared.data();
    let mut counts: HashMap<LibraryId, u32> = HashMap::new();
    for p in d.content.presentations.iter() {
        if let Some(l) = p.library {
            *counts.entry(l).or_default() += 1;
        }
    }
    let mut out: Vec<LibrarySummary> = d
        .content
        .libraries
        .values()
        .map(|l| LibrarySummary {
            id: l.id,
            name: l.name.clone(),
            sort_key: l.sort_key.clone(),
            presentation_count: counts.get(&l.id).copied().unwrap_or(0),
        })
        .collect();
    out.sort_by(|a, b| {
        a.sort_key
            .cmp(&b.sort_key)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.0.cmp(&b.id.0))
    });
    ok(out)
}

async fn library_presentations(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<PresentationSummary>> {
    let id: LibraryId = parse_id(&id, "library id")?;
    let d = app.shared.data();
    if !d.content.libraries.contains_key(&id) {
        return Err(err(
            ErrorCode::NotFound,
            format!("library {} not found", id.0),
        ));
    }
    ok(d.content
        .presentations
        .iter()
        .filter(|p| p.library == Some(id))
        .map(|p| summary(&d, p))
        .collect())
}

async fn presentations(State(app): State<AppState>) -> ApiResult<Vec<PresentationSummary>> {
    let d = app.shared.data();
    ok(d.content
        .presentations
        .iter()
        .map(|p| summary(&d, p))
        .collect())
}

async fn presentation(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let id: PresentationId = parse_id(&id, "presentation id")?;
    let d = app.shared.data();
    // Serialize on the blocking pool: large presentations are a few hundred KB of JSON.
    let body = tokio::task::spawn_blocking(move || {
        d.content
            .presentation(id)
            .map(|p| (serde_json::to_vec(p), crate::config::revision(p)))
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
    .ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("presentation {} not found", id.0),
        )
    })?;
    let (body, rev) = body;
    let body = body.map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    let etag =
        HeaderValue::from_str(&format!("\"{rev}\"")).unwrap_or(HeaderValue::from_static("\"\""));
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::ETAG, etag),
        ],
        body,
    )
        .into_response())
}

fn tree(d: &ShowData) -> Vec<PlaylistTreeNode> {
    let mut children: HashMap<Option<PlaylistId>, Vec<&model::PlaylistNode>> = HashMap::new();
    for n in d.content.playlists.values() {
        let parent = n.parent.filter(|p| d.content.playlists.contains_key(p));
        children.entry(parent).or_default().push(n);
    }
    for v in children.values_mut() {
        v.sort_by(|a, b| {
            a.sort_key
                .cmp(&b.sort_key)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then(a.id.0.cmp(&b.id.0))
        });
    }
    fn build(
        parent: Option<PlaylistId>,
        children: &HashMap<Option<PlaylistId>, Vec<&model::PlaylistNode>>,
        seen: &mut HashSet<PlaylistId>,
    ) -> Vec<PlaylistTreeNode> {
        let mut out = Vec::new();
        for n in children.get(&parent).map(Vec::as_slice).unwrap_or(&[]) {
            if !seen.insert(n.id) {
                continue; // a cycle in the parent links
            }
            let (kind, item_count, purpose) = match &n.kind {
                PlaylistNodeKind::Playlist { items, purpose } => (
                    PlaylistNodeType::Playlist,
                    items.len() as u32,
                    Some(*purpose),
                ),
                _ => (PlaylistNodeType::Folder, 0, None),
            };
            out.push(PlaylistTreeNode {
                id: n.id,
                name: n.name.clone(),
                kind,
                purpose,
                item_count,
                children: build(Some(n.id), children, seen),
            });
        }
        out
    }
    build(None, &children, &mut HashSet::new())
}

async fn playlists(State(app): State<AppState>) -> ApiResult<Vec<PlaylistTreeNode>> {
    ok(tree(&app.shared.data()))
}

async fn playlist(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<PlaylistDetail> {
    let id: PlaylistId = parse_id(&id, "playlist id")?;
    let d = app.shared.data();
    let n = d
        .content
        .playlists
        .get(&id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("playlist {} not found", id.0)))?;
    let PlaylistNodeKind::Playlist { purpose, items } = &n.kind else {
        return Err(err(ErrorCode::BadRequest, format!("{} is a folder", id.0)));
    };
    let items = items
        .iter()
        .map(|it| {
            let presentation = match &it.kind {
                PlaylistItemKind::Presentation { presentation, .. } => d
                    .content
                    .presentation(presentation.id)
                    .map(|p| summary(&d, p)),
                _ => None,
            };
            let fallback = match &it.kind {
                PlaylistItemKind::Presentation {
                    presentation: l, ..
                } => presentation
                    .as_ref()
                    .map(|p| p.name.clone())
                    .or_else(|| l.name.clone())
                    .unwrap_or_else(|| "Missing presentation".into()),
                PlaylistItemKind::Media(t) => d.content.asset_name(t.asset),
                PlaylistItemKind::Header { name, .. } => name.clone(),
                PlaylistItemKind::VideoInput(_) => "Video input".into(),
                PlaylistItemKind::Placeholder => "Placeholder".into(),
                PlaylistItemKind::Unsupported { source_type, .. } => source_type.clone(),
                PlaylistItemKind::Unknown => "Unknown item".into(),
            };
            PlaylistItemDetail {
                id: it.id,
                display_name: it
                    .name
                    .clone()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(fallback),
                enabled: it.enabled,
                kind: it.kind.clone(),
                presentation,
            }
        })
        .collect();
    ok(PlaylistDetail {
        id: n.id,
        name: n.name.clone(),
        parent: n.parent,
        purpose: *purpose,
        items,
    })
}

async fn looks(State(app): State<AppState>) -> ApiResult<Vec<model::Look>> {
    ok(app.shared.data().looks.clone())
}

async fn clear_groups(State(app): State<AppState>) -> ApiResult<Vec<model::ClearGroup>> {
    ok((*app.shared.data().content.clear_groups).clone())
}
