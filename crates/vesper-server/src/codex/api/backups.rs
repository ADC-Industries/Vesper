//! The settings backup routes (`/api/v1/codex/backups…`), merged into the Codex's routes. Lists
//! and the queued restores need a paired client; a snapshot's contents, its preview, a restore
//! and a snapshot on demand are the site's admin actions (audited: `codex.backupNow`,
//! `codex.restore`); Oriel's preferences are each client's own, and any paired client may take
//! over another computer's (`?set=`).
//!
//! | Route | |
//! |---|---|
//! | `GET /codex/backups` | `BackupSetSummary[]` |
//! | `POST /codex/backups/snapshots` | this box's settings now (admins) → `201 SnapshotTaken` (`200` with the newest when nothing changed) |
//! | `GET /codex/backups/{set}/snapshots` | `SnapshotSummary[]`, newest first |
//! | `GET /codex/backups/{set}/snapshots/{snapshot}` | `SnapshotContents` (admins) |
//! | `GET /codex/backups/{set}/snapshots/{snapshot}/preview` | `RestorePreview` (`?target=` the box restored onto, by server ID) (admins) |
//! | `POST /codex/backups/{set}/snapshots/{snapshot}/restore` | `RestoreRequest` → `RestoreResult` (this box), or `202 QueuedRestore` (a show box takes it at its next poll) (admins) |
//! | `GET /codex/backups/restores` | `QueuedRestore[]`, newest first |
//! | `GET`, `DELETE /codex/backups/restores/{id}` | one; cancel while it waits → `QueuedRestore` (cancel: admins) |
//! | `GET`, `PUT /codex/backups/oriel` | this client's preferences: `OrielPreferencesBackup` (`?snapshot=`; `?set=` another computer's Oriel set); `OrielPreferences` → `SnapshotTaken` |

use crate::api::{AppState, Error, err, parse_body};
use crate::auth::Access;
use crate::codex::api::{admin, audit};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use model::{BackupSetId, SnapshotId, SnapshotReason};
use protocol::ErrorCode;
use serde_json::json;
use std::net::SocketAddr;

/// The routes, merged into the Codex's.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/codex/backups", get(list))
        .route("/codex/backups/snapshots", post(snapshot_now))
        .route(
            "/codex/backups/oriel",
            get(preferences).put(put_preferences),
        )
        .route("/codex/backups/{set}/snapshots", get(snapshots))
        .route("/codex/backups/{set}/snapshots/{snapshot}", get(contents))
        .route(
            "/codex/backups/{set}/snapshots/{snapshot}/preview",
            get(preview),
        )
        .route(
            "/codex/backups/{set}/snapshots/{snapshot}/restore",
            post(restore),
        )
        .route("/codex/backups/restores", get(restores))
        .route(
            "/codex/backups/restores/{id}",
            get(queued_restore).delete(cancel_restore),
        )
}

fn parse_set(s: &str) -> Result<BackupSetId, Error> {
    s.parse().map_err(|_| {
        err(
            ErrorCode::BadRequest,
            format!("backup set id {s:?} is not a UUID"),
        )
    })
}

fn parse_snapshot(s: &str) -> Result<SnapshotId, Error> {
    s.parse().map_err(|_| {
        err(
            ErrorCode::BadRequest,
            format!("snapshot id {s:?} is not a UUID"),
        )
    })
}

/// The asking client's Oriel set: its key's id and name (the server machine's own clients
/// share one).
fn client_of(access: &Access) -> (String, String) {
    match access {
        Access::Paired(c) => (c.id.clone(), c.name.clone()),
        _ => (
            crate::codex::backups::LOCAL_CLIENT.to_string(),
            crate::site::SERVER_MACHINE.to_string(),
        ),
    }
}

fn respond<T: serde::Serialize>(r: Result<T, protocol::ApiError>) -> Response {
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn list(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
) -> Response {
    let client = client_of(&access).0;
    respond(app.shared.backups.list(&app.codex, Some(client)).await)
}

async fn snapshot_now(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
) -> Response {
    let action = "codex.backupNow";
    let actor = match admin(&app, &access, peer, action, &json!({})).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let by = crate::api::change_by(&app.shared);
    let r = app
        .shared
        .backups
        .snapshot_now(&app.codex, SnapshotReason::Manual, by)
        .await;
    let (detail, error) = match &r {
        Ok(t) => (
            json!({"snapshot": t.snapshot.id, "created": t.created}),
            None,
        ),
        Err(e) => (json!({}), Some(e.message.clone())),
    };
    audit(&app, &actor, action, detail, error).await;
    match r {
        Ok(t) if t.created => (StatusCode::CREATED, axum::Json(t)).into_response(),
        r => respond(r),
    }
}

async fn snapshots(State(app): State<AppState>, Path(set): Path<String>) -> Response {
    let set = match parse_set(&set) {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    respond(app.shared.backups.snapshots(&app.codex, set).await)
}

/// The set and snapshot of a path, and the admin check.
async fn admin_path(
    app: &AppState,
    access: &Access,
    peer: SocketAddr,
    action: &str,
    set: &str,
    snapshot: &str,
) -> Result<(crate::site::Actor, BackupSetId, SnapshotId), Error> {
    let (set, snapshot) = (parse_set(set)?, parse_snapshot(snapshot)?);
    let detail = json!({"set": set, "snapshot": snapshot});
    let actor = admin(app, access, peer, action, &detail).await?;
    Ok((actor, set, snapshot))
}

async fn contents(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path((set, snapshot)): Path<(String, String)>,
) -> Response {
    let (_, set, snapshot) =
        match admin_path(&app, &access, peer, "codex.readSnapshot", &set, &snapshot).await {
            Ok(x) => x,
            Err(e) => return e.into_response(),
        };
    respond(app.shared.backups.contents(&app.codex, set, snapshot).await)
}

async fn preview(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path((set, snapshot)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    let (_, set, snapshot) =
        match admin_path(&app, &access, peer, "codex.previewRestore", &set, &snapshot).await {
            Ok(x) => x,
            Err(e) => return e.into_response(),
        };
    // (A server ID: nothing to percent-decode.)
    let target = crate::api::query_param(query.as_deref(), "target")
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    respond(
        app.shared
            .backups
            .preview(&app.codex, set, snapshot, target)
            .await,
    )
}

async fn restore(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path((set, snapshot)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let req: protocol::RestoreRequest = if body.is_empty() {
        Default::default()
    } else {
        match parse_body(&body) {
            Ok(b) => b,
            Err(e) => return e.into_response(),
        }
    };
    let action = "codex.restore";
    let (actor, set, snapshot) =
        match admin_path(&app, &access, peer, action, &set, &snapshot).await {
            Ok(x) => x,
            Err(e) => return e.into_response(),
        };
    let by = crate::api::change_by(&app.shared);
    let r = app
        .shared
        .backups
        .restore(&app.codex, &app.show, set, snapshot, req.clone(), by)
        .await;
    let mut detail =
        json!({"set": set, "snapshot": snapshot, "items": req.items, "target": req.target});
    let error = match &r {
        Ok(crate::codex::backups::RestoreOutcome::Done(res)) => {
            detail["summary"] = json!(res.summary);
            detail["beforeRestore"] = json!(res.before_restore.id);
            None
        }
        Ok(crate::codex::backups::RestoreOutcome::Queued(q)) => {
            detail["queued"] = json!(q.id);
            detail["target"] = json!(q.target);
            None
        }
        Err(e) => Some(e.message.clone()),
    };
    audit(&app, &actor, action, detail, error).await;
    match r {
        Ok(crate::codex::backups::RestoreOutcome::Done(res)) => axum::Json(res).into_response(),
        Ok(crate::codex::backups::RestoreOutcome::Queued(q)) => {
            (StatusCode::ACCEPTED, axum::Json(q)).into_response()
        }
        Err(e) => Error(e).into_response(),
    }
}

/// The admin check of a queued restore's routes.
async fn restore_admin(
    app: &AppState,
    access: &Access,
    peer: SocketAddr,
    action: &str,
    id: Option<&str>,
) -> Result<crate::site::Actor, Error> {
    admin(app, access, peer, action, &json!({"restore": id})).await
}

/// (Reads: any paired client, like the site's other reads.)
async fn restores(State(app): State<AppState>) -> Response {
    respond(app.shared.backups.restores(&app.codex).await)
}

async fn queued_restore(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    respond(app.shared.backups.queued_restore(&app.codex, &id).await)
}

async fn cancel_restore(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    let action = "codex.cancelRestore";
    let actor = match restore_admin(&app, &access, peer, action, Some(&id)).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app.shared.backups.cancel_restore(&app.codex, &id).await;
    let error = r.as_ref().err().map(|e| e.message.clone());
    audit(&app, &actor, action, json!({"restore": id}), error).await;
    respond(r)
}

async fn put_preferences(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    body: Bytes,
) -> Response {
    let prefs: protocol::OrielPreferences = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let by = crate::api::change_by(&app.shared);
    let r = app
        .shared
        .backups
        .put_preferences(&app.codex, client_of(&access), prefs, by)
        .await;
    match r {
        Ok(t) if t.created => (StatusCode::CREATED, axum::Json(t)).into_response(),
        r => respond(r),
    }
}

async fn preferences(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    RawQuery(query): RawQuery,
) -> Response {
    let snapshot = match crate::api::query_param(query.as_deref(), "snapshot") {
        None => None,
        Some(s) => match parse_snapshot(s) {
            Ok(id) => Some(id),
            Err(e) => return e.into_response(),
        },
    };
    let set = match crate::api::query_param(query.as_deref(), "set") {
        None => None,
        Some(s) => match parse_set(s) {
            Ok(id) => Some(id),
            Err(e) => return e.into_response(),
        },
    };
    respond(
        app.shared
            .backups
            .preferences(&app.codex, client_of(&access).0, set, snapshot)
            .await,
    )
}
