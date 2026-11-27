//! `/codex/dormant/upload` (on a show box): its dormant collection uploaded to the site's
//! Codex ([`crate::codex::adopt`]).

use crate::api::{AppState, Error};
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use protocol::{ErrorCode, StartDormantUpload};
use std::net::SocketAddr;

/// `GET/POST/DELETE /codex/dormant/upload`.
pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/codex/dormant/upload",
        post(start_route).get(status_route).delete(cancel_route),
    )
}

async fn status_route(State(app): State<AppState>) -> Response {
    match app.shared.adopt.status(&app.codex) {
        Ok(s) => axum::Json(s).into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn start_route(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<crate::auth::Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let body: StartDormantUpload = if body.iter().all(u8::is_ascii_whitespace) {
        StartDormantUpload::default()
    } else {
        match crate::api::parse_body(&body) {
            Ok(b) => b,
            Err(e) => return e.into_response(),
        }
    };
    let action = "codex.uploadDormant";
    let detail = serde_json::json!({"name": body.name});
    let actor = match super::admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let sh = app.shared.clone();
    let r =
        match tokio::task::spawn_blocking(move || sh.adopt.start(&sh.codex_server(), body)).await {
            Ok(r) => r,
            Err(e) => Err(protocol::ApiError::new(ErrorCode::Internal, e.to_string())),
        };
    let (detail, error) = match &r {
        Ok(s) => (
            serde_json::json!({"name": detail["name"], "source": s.source, "collection": s.collection}),
            None,
        ),
        Err(e) => (detail, Some(e.message.clone())),
    };
    super::audit(&app, &actor, action, detail, error).await;
    match r {
        Ok(s) => (StatusCode::ACCEPTED, axum::Json(s)).into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn cancel_route(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<crate::auth::Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
) -> Response {
    let action = "codex.cancelDormantUpload";
    let detail = serde_json::json!({});
    let actor = match super::admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app.shared.adopt.cancel(&app.codex);
    let error = r.as_ref().err().map(|e| e.message.clone());
    super::audit(&app, &actor, action, detail, error).await;
    match r {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => Error(e).into_response(),
    }
}
