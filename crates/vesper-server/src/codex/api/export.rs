//! The export and import routes (`/api/v1/codex/export…`), merged into the Codex's routes.
//! Every one is the site's admin's (audited: `codex.export`, `codex.exportSettings`,
//! `codex.exportPassphrase`, `codex.importUpload`, `codex.importPreview`, `codex.import`,
//! `codex.importDrop`); passphrases are never in an audit entry or a log line. On a show box
//! they answer `409 codexElsewhere` ([`crate::codex::write_through`]).
//!
//! | Route | |
//! |---|---|
//! | `POST /codex/export` | `CreateExport {passphrase, includeSite?}` → the encrypted file (`application/octet-stream`, a download); the site's identity only with `includeSite` (`409` at a box that doesn't run its site's Site Manager) |
//! | `GET`, `PUT /codex/export/settings` | `CodexExportStatus`; `UpdateExportSettings` → `CodexExportStatus` |
//! | `PUT`, `DELETE /codex/export/passphrase` | `SetExportPassphrase` → `CodexExportStatus`; clear it → `CodexExportStatus` |
//! | `GET /codex/export/scheduled` | `ScheduledExport[]`, newest first |
//! | `POST /codex/export/imports` | the file as the body → `201 ImportUpload` |
//! | `DELETE /codex/export/imports/{id}` | `204` |
//! | `POST /codex/export/imports/{id}/preview` | `OpenImport {passphrase}` → `ImportPreview` |
//! | `POST /codex/export/imports/{id}/apply` | `ImportRequest` → `ImportResult` |

use crate::api::{AppState, Error, parse_body};
use crate::auth::Access;
use crate::codex::api::{admin, audit};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use serde_json::json;
use std::net::SocketAddr;

/// The routes, merged into the Codex's.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/codex/export", post(export_now))
        .route("/codex/export/settings", get(settings).put(put_settings))
        .route(
            "/codex/export/passphrase",
            put(set_passphrase).delete(clear_passphrase),
        )
        .route("/codex/export/scheduled", get(scheduled))
        .route(
            "/codex/export/imports",
            post(upload).layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route(
            "/codex/export/imports/{id}",
            axum::routing::delete(drop_import),
        )
        .route("/codex/export/imports/{id}/preview", post(preview))
        .route("/codex/export/imports/{id}/apply", post(apply))
}

fn respond<T: serde::Serialize>(r: Result<T, protocol::ApiError>) -> Response {
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn export_now(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let action = "codex.export";
    let actor = match admin(&app, &access, peer, action, &json!({})).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let req: protocol::CreateExport = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let r = app
        .shared
        .exports
        .export_now(&app.codex, req.passphrase, req.include_site)
        .await;
    let (detail, error) = match &r {
        Ok((bytes, m)) => (
            json!({"bytes": bytes.len(), "collections": m.collections.len(), "backupSets": m.backup_sets, "site": m.site}),
            None,
        ),
        Err(e) => (
            json!({"includeSite": req.include_site}),
            Some(e.message.clone()),
        ),
    };
    audit(&app, &actor, action, detail, error).await;
    match r {
        Ok((bytes, _)) => {
            let name = format!(
                "vesper-codex-{}.{}",
                chrono::Local::now().format("%Y-%m-%d-%H%M%S"),
                crate::codex::export::EXTENSION
            );
            let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
                .unwrap_or(HeaderValue::from_static("attachment"));
            (
                StatusCode::OK,
                [
                    (
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/octet-stream"),
                    ),
                    (header::CONTENT_DISPOSITION, disposition),
                    (
                        header::HeaderName::from_static("x-codex-media-included"),
                        HeaderValue::from_static("false"),
                    ),
                    (
                        header::HeaderName::from_static("x-codex-export-note"),
                        HeaderValue::from_static(protocol::MEDIA_NOTE),
                    ),
                ],
                Body::from(bytes),
            )
                .into_response()
        }
        Err(e) => Error(e).into_response(),
    }
}

/// (A read: any paired client, like the site's other reads.)
async fn settings(State(app): State<AppState>) -> Response {
    respond(app.shared.exports.status(&app.codex).await)
}

async fn put_settings(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let req: protocol::UpdateExportSettings = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let action = "codex.exportSettings";
    let detail = serde_json::to_value(&req).unwrap_or_default();
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app.shared.exports.put_settings(&app.codex, req).await;
    audit(
        &app,
        &actor,
        action,
        detail,
        r.as_ref().err().map(|e| e.message.clone()),
    )
    .await;
    respond(r)
}

async fn set_passphrase(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let action = "codex.exportPassphrase";
    let detail = json!({"set": true});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let req: protocol::SetExportPassphrase = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let r = app
        .shared
        .exports
        .set_passphrase(&app.codex, Some(req.passphrase))
        .await;
    audit(
        &app,
        &actor,
        action,
        detail,
        r.as_ref().err().map(|e| e.message.clone()),
    )
    .await;
    respond(r)
}

async fn clear_passphrase(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
) -> Response {
    let action = "codex.exportPassphrase";
    let detail = json!({"set": false});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app.shared.exports.set_passphrase(&app.codex, None).await;
    audit(
        &app,
        &actor,
        action,
        detail,
        r.as_ref().err().map(|e| e.message.clone()),
    )
    .await;
    respond(r)
}

/// (A read: any paired client, like the site's other reads.)
async fn scheduled(State(app): State<AppState>) -> Response {
    respond(app.shared.exports.scheduled(&app.codex).await)
}

async fn upload(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Body,
) -> Response {
    let action = "codex.importUpload";
    let actor = match admin(&app, &access, peer, action, &json!({})).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app.shared.exports.upload(&app.codex, body).await;
    let detail = match &r {
        Ok(u) => json!({"import": u.id, "bytes": u.bytes}),
        Err(_) => json!({}),
    };
    audit(
        &app,
        &actor,
        action,
        detail,
        r.as_ref().err().map(|e| e.message.clone()),
    )
    .await;
    match r {
        Ok(u) => (StatusCode::CREATED, axum::Json(u)).into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn drop_import(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    let action = "codex.importDrop";
    let detail = json!({"import": id});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app.shared.exports.drop_import(&app.codex, &id).await;
    audit(
        &app,
        &actor,
        action,
        detail,
        r.as_ref().err().map(|e| e.message.clone()),
    )
    .await;
    match r {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn preview(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let action = "codex.importPreview";
    let detail = json!({"import": id});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let req: protocol::OpenImport = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let r = app
        .shared
        .exports
        .preview(&app.codex, &id, req.passphrase)
        .await;
    audit(
        &app,
        &actor,
        action,
        detail,
        r.as_ref().err().map(|e| e.message.clone()),
    )
    .await;
    respond(r)
}

async fn apply(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let action = "codex.import";
    let actor = match admin(&app, &access, peer, action, &json!({"import": id})).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let req: protocol::ImportRequest = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let mut detail = json!({"import": id, "replace": req.replace, "skip": req.skip, "backups": req.backups, "site": req.site});
    let by = crate::api::change_by(&app.shared);
    let r = app.shared.exports.apply(&app.codex, &id, req, by).await;
    if let Ok(res) = &r {
        detail["added"] = json!(res.added);
        detail["replaced"] = json!(res.replaced);
        detail["backupSets"] = json!(res.backup_sets);
        detail["siteStaged"] = json!(res.site_staged);
    }
    audit(
        &app,
        &actor,
        action,
        detail,
        r.as_ref().err().map(|e| e.message.clone()),
    )
    .await;
    respond(r)
}
