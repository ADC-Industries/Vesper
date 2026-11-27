//! Content editing: libraries, presentations, playlists, media bins, global groups.

use super::*;

/// The `X-Client-Id` header (echoed in `documentChanged`).
pub(super) fn client_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().chars().take(80).collect::<String>())
        .filter(|s| !s.is_empty())
}

pub(super) async fn content(app: &AppState, op: ContentOp, client: Option<String>) -> Response {
    match crate::codex::server::content(&app.shared, &app.show, op, client, change_by(&app.shared))
        .await
    {
        Ok(ContentResult::PresentationEdit(r)) => with_etag(StatusCode::OK, &r.revision.clone(), r),
        Ok(ContentResult::PlaylistEdit(r)) => with_etag(StatusCode::OK, &r.revision.clone(), r),
        Ok(ContentResult::MediaBinEdit(r)) => with_etag(StatusCode::OK, &r.revision.clone(), r),
        Ok(ContentResult::Revision(r, created)) => {
            let status = if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            with_etag(status, &r.revision.clone(), r)
        }
        Ok(ContentResult::Deleted(d)) => axum::Json(d).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Parses `{id}` and the JSON body, then runs `make`'s op.
async fn content_with<T: serde::de::DeserializeOwned, I: From<uuid::Uuid>>(
    app: &AppState,
    id: &str,
    what: &str,
    body: &[u8],
    client: Option<String>,
    make: impl FnOnce(I, T) -> (ContentOp, Option<String>),
) -> Response {
    let parsed = parse_id::<I>(id, what).and_then(|id| Ok((id, parse_body::<T>(body)?)));
    match parsed {
        Ok((id, b)) => {
            let (op, from_body) = make(id, b);
            content(app, op, from_body.or(client)).await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn global_groups(
    State(app): State<AppState>,
) -> ApiResult<Vec<model::GlobalGroup>> {
    ok((*app.shared.data().content.global_groups).clone())
}

pub(super) async fn create_library(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match parse_body::<protocol::CreateLibrary>(&body) {
        Ok(b) => {
            content(
                &app,
                ContentOp::CreateLibrary(Box::new(b)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn update_library(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let m = if_match(&headers);
    content_with(
        &app,
        &id,
        "library id",
        &body,
        client_header(&headers),
        |id, b| (ContentOp::UpdateLibrary(id, b, m), None),
    )
    .await
}

pub(super) async fn delete_library(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match parse_id::<LibraryId>(&id, "library id") {
        Ok(id) => {
            content(
                &app,
                ContentOp::DeleteLibrary(id, if_match(&headers)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn create_presentation(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match parse_body::<protocol::CreatePresentation>(&body) {
        Ok(b) => {
            content(
                &app,
                ContentOp::CreatePresentation(Box::new(b)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn edit_presentation(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let m = if_match(&headers);
    content_with(
        &app,
        &id,
        "presentation id",
        &body,
        client_header(&headers),
        |id, b: protocol::PresentationEdit| {
            let c = b.client_id.clone();
            (ContentOp::EditPresentation(id, b, m), c)
        },
    )
    .await
}

pub(super) async fn duplicate_presentation(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body = if body.is_empty() {
        Bytes::from_static(b"{}")
    } else {
        body
    };
    content_with(
        &app,
        &id,
        "presentation id",
        &body,
        client_header(&headers),
        |id, b| (ContentOp::DuplicatePresentation(id, b), None),
    )
    .await
}

pub(super) async fn move_presentation(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let m = if_match(&headers);
    content_with(
        &app,
        &id,
        "presentation id",
        &body,
        client_header(&headers),
        |id, b| (ContentOp::MovePresentation(id, b, m), None),
    )
    .await
}

pub(super) async fn delete_presentation(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match parse_id::<PresentationId>(&id, "presentation id") {
        Ok(id) => {
            content(
                &app,
                ContentOp::DeletePresentation(id, if_match(&headers)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn create_playlist(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match parse_body::<protocol::CreatePlaylistNode>(&body) {
        Ok(b) => {
            content(
                &app,
                ContentOp::CreatePlaylist(Box::new(b)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn update_playlist(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let m = if_match(&headers);
    content_with(
        &app,
        &id,
        "playlist id",
        &body,
        client_header(&headers),
        |id, b| (ContentOp::UpdatePlaylist(id, b, m), None),
    )
    .await
}

pub(super) async fn edit_playlist(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    content_with(
        &app,
        &id,
        "playlist id",
        &body,
        client_header(&headers),
        |id, b: protocol::PlaylistEdit| {
            let c = b.client_id.clone();
            (ContentOp::EditPlaylist(id, b), c)
        },
    )
    .await
}

pub(super) async fn duplicate_playlist(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body = if body.is_empty() {
        Bytes::from_static(b"{}")
    } else {
        body
    };
    content_with(
        &app,
        &id,
        "playlist id",
        &body,
        client_header(&headers),
        |id, b| (ContentOp::DuplicatePlaylist(id, b), None),
    )
    .await
}

pub(super) async fn delete_playlist(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match parse_id::<PlaylistId>(&id, "playlist id") {
        Ok(id) => {
            content(
                &app,
                ContentOp::DeletePlaylist(id, if_match(&headers)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn create_media_bin(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match parse_body::<protocol::CreateMediaBin>(&body) {
        Ok(b) => {
            content(
                &app,
                ContentOp::CreateMediaBin(Box::new(b)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn update_media_bin(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let m = if_match(&headers);
    content_with(
        &app,
        &id,
        "media bin id",
        &body,
        client_header(&headers),
        |id, b| (ContentOp::UpdateMediaBin(id, b, m), None),
    )
    .await
}

pub(super) async fn delete_media_bin(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match parse_id::<MediaBinId>(&id, "media bin id") {
        Ok(id) => {
            content(
                &app,
                ContentOp::DeleteMediaBin(id, if_match(&headers)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn edit_media_bin(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    content_with(
        &app,
        &id,
        "media bin id",
        &body,
        client_header(&headers),
        |id, b: protocol::MediaBinEdit| {
            let c = b.client_id.clone();
            (ContentOp::EditMediaBin(id, b), c)
        },
    )
    .await
}

pub(super) async fn create_global_group(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match parse_body::<protocol::CreateGlobalGroup>(&body) {
        Ok(b) => {
            content(
                &app,
                ContentOp::CreateGlobalGroup(Box::new(b)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn update_global_group(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let m = if_match(&headers);
    content_with(
        &app,
        &id,
        "group id",
        &body,
        client_header(&headers),
        |id, b| (ContentOp::UpdateGlobalGroup(id, b, m), None),
    )
    .await
}

pub(super) async fn delete_global_group(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match parse_id::<model::GlobalGroupId>(&id, "group id") {
        Ok(id) => {
            content(
                &app,
                ContentOp::DeleteGlobalGroup(id, if_match(&headers)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}
