//! "Used by": the documents of the active collection that use a media asset or a prop.

use super::*;
use crate::data::CollectionContent;
use model::{DocumentKind, PropId};
use protocol::DocumentUse;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/media/{id}/used-by", get(media_used_by))
        .route("/props/{id}/used-by", get(prop_used_by))
}

async fn media_used_by(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<DocumentUse>> {
    let id: MediaAssetId = parse_id(&id, "media id")?;
    let content = app.shared.data().content.clone();
    if !content.assets.contains_key(&id) {
        return Err(err(
            ErrorCode::NotFound,
            format!("media asset {} not found", id.0),
        ));
    }
    let uses = tokio::task::spawn_blocking(move || users_of(&content, id.0))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    ok(uses)
}

async fn prop_used_by(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<DocumentUse>> {
    let id: PropId = parse_id(&id, "prop id")?;
    let content = app.shared.data().content.clone();
    if !content.props.iter().any(|p| p.id == id) {
        return Err(err(ErrorCode::NotFound, format!("prop {} not found", id.0)));
    }
    let uses = tokio::task::spawn_blocking(move || users_of(&content, id.0))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    ok(uses)
}

/// The documents that name `id` anywhere in them (an element's media, a media or prop
/// action, a playlist item, a bin's item, a theme's asset), but `id`'s own: by kind, then name.
/// IDs are UUIDs, so a document holding the text of one refers to it.
pub(crate) fn users_of(c: &CollectionContent, id: uuid::Uuid) -> Vec<DocumentUse> {
    let needle = id.to_string();
    let mut out = Vec::new();
    let mut look =
        |kind: DocumentKind, doc_id: uuid::Uuid, name: &str, json: serde_json::Result<String>| {
            if doc_id != id && json.is_ok_and(|j| j.contains(&needle)) {
                out.push(DocumentUse {
                    kind,
                    id: doc_id.to_string(),
                    name: name.to_string(),
                });
            }
        };
    for p in c.presentations.iter() {
        look(
            DocumentKind::Presentation,
            p.id.0,
            &p.name,
            serde_json::to_string(&**p),
        );
    }
    for p in c.playlists.values() {
        look(
            DocumentKind::PlaylistNode,
            p.id.0,
            &p.name,
            serde_json::to_string(p),
        );
    }
    for p in c.props.iter() {
        look(
            DocumentKind::Prop,
            p.id.0,
            &p.name,
            serde_json::to_string(p),
        );
    }
    for m in c.macros.iter() {
        look(
            DocumentKind::Macro,
            m.id.0,
            &m.name,
            serde_json::to_string(m),
        );
    }
    for t in c.themes.values() {
        look(
            DocumentKind::Theme,
            t.id.0,
            &t.name,
            serde_json::to_string(t),
        );
    }
    for b in c.media_bins.iter() {
        look(
            DocumentKind::MediaBin,
            b.id.0,
            &b.name,
            serde_json::to_string(b),
        );
    }
    for s in c.stage_layouts.iter() {
        look(
            DocumentKind::StageLayout,
            s.id.0,
            &s.name,
            serde_json::to_string(s),
        );
    }
    for m in c.messages.iter() {
        look(
            DocumentKind::Message,
            m.id.0,
            &m.name,
            serde_json::to_string(m),
        );
    }
    for t in c.timers.iter() {
        look(
            DocumentKind::Timer,
            t.id.0,
            &t.name,
            serde_json::to_string(t),
        );
    }
    out.sort_by_key(|u| (kind_order(u.kind), u.name.to_lowercase()));
    out
}

/// The order "Used by" lists kinds in: what people look for first.
fn kind_order(k: DocumentKind) -> u8 {
    match k {
        DocumentKind::Presentation => 0,
        DocumentKind::PlaylistNode => 1,
        DocumentKind::Prop => 2,
        DocumentKind::Macro => 3,
        DocumentKind::MediaBin => 4,
        DocumentKind::Theme => 5,
        _ => 6,
    }
}
