//! Presence and edit locks: `GET /presence`, `POST /presence/locks`,
//! `DELETE /presence/locks/{kind}/{id}` (see [`crate::presence`]).

use super::edit::client_header;
use super::*;
use model::DocumentKind;
use protocol::{DocumentLock, PresenceList, TakeLock};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/presence", get(list))
        .route("/presence/locks", post(take))
        .route("/presence/locks/{kind}/{id}", delete(release))
}

async fn list(State(app): State<AppState>) -> ApiResult<PresenceList> {
    ok(app.shared.presence.list())
}

fn caller(headers: &HeaderMap) -> Result<String, Error> {
    client_header(headers).ok_or_else(|| {
        err(
            ErrorCode::BadRequest,
            "edit locks need the X-Client-Id header (the clientId of the client's presence)",
        )
    })
}

async fn take(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<DocumentLock> {
    let client = caller(&headers)?;
    let b = parse_body::<TakeLock>(&body)?;
    // A show box of a site's Codex: the lock is the site's, on the Codex.
    let sh = app.shared.clone();
    let (kind, id, c, over) = (
        b.kind,
        b.id.clone(),
        client.clone(),
        b.take_over.unwrap_or(false),
    );
    let forwarded = tokio::task::spawn_blocking(move || {
        crate::codex::site_presence::forward_lock(
            &sh,
            crate::codex::site_presence::LockOp::Take,
            kind,
            &id,
            &c,
            over,
        )
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    if let Some(r) = forwarded {
        let l = r?.ok_or_else(|| err(ErrorCode::Internal, "the site's Codex answered no lock"))?;
        return ok(l);
    }
    let collection = crate::presence::request_collection(&app.shared);
    let l = app.shared.presence.take(
        collection,
        b.kind,
        &b.id,
        &client,
        b.take_over.unwrap_or(false),
    )?;
    ok(l)
}

async fn release(
    State(app): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, Error> {
    let client = caller(&headers)?;
    let kind: DocumentKind = serde_json::from_value(serde_json::Value::String(kind.clone()))
        .map_err(|_| err(ErrorCode::BadRequest, format!("unknown kind {kind:?}")))?;
    let sh = app.shared.clone();
    let (i, c) = (id.clone(), client.clone());
    let forwarded = tokio::task::spawn_blocking(move || {
        crate::codex::site_presence::forward_lock(
            &sh,
            crate::codex::site_presence::LockOp::Release,
            kind,
            &i,
            &c,
            false,
        )
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    if let Some(r) = forwarded {
        r?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let collection = crate::presence::request_collection(&app.shared);
    app.shared.presence.release(collection, kind, &id, &client);
    Ok(StatusCode::NO_CONTENT)
}
