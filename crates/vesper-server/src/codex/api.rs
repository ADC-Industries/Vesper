//! The Codex's REST routes (`/api/v1/codex/...`). Reading needs a paired client (as every
//! route); collection administration and this box's assignment are the site's admin actions
//! (the site's access level decides who: [`crate::site::Actor`]) and are audited
//! ([`crate::site::Site::audit`]). Every change goes through the [`super::writer::Writer`].
//!
//! | Route | |
//! |---|---|
//! | `GET /codex` | `CodexStatus`: role, this box's active collection, counts |
//! | `GET/POST /codex/collections` | `CollectionSummary[]`; create (admins) → `201` |
//! | `GET/PUT/DELETE /codex/collections/{id}` | one (`ETag`); change (admins, `If-Match`); delete: into the collection trash (admins, `If-Match`) → `204` |
//! | `GET /codex/collection-trash` | `DeletedCollection[]`, newest first |
//! | `POST /codex/collection-trash/{id}/restore` | back (admins) → `CollectionSummary` |
//! | `DELETE /codex/collection-trash/{id}` | for good, with its item trash entries (admins, `If-Match`) → `204` |
//! | `PUT /codex/assignment` | `{collection, confirm?}`: the collection this box shows (admins; `409 liveShow` while something is live, unless confirmed) → `CodexStatus` |
//! | `GET /codex/collections/{id}/changes?since=&limit=` | `CollectionChanges`: the change log after `since` |
//! | `GET /codex/trash`, `DELETE /codex/trash` | `TrashItem[]`, newest first; purge (admins) → `TrashPurged` |
//! | `POST /codex/trash/{id}/restore` | `TrashRestored`: back with their IDs (a content edit: any paired client) |
//! | `GET/POST /codex/uploads` | `UploadInfo[]`; start one (`CreateUpload`) → `201 UploadInfo` |
//! | `GET/PUT/DELETE /codex/uploads/{id}` | where it stands; a chunk (`Content-Range`, no body limit; `409 {offset}`); cancel → `204` |
//! | `POST /codex/uploads/{id}/finish` | `FinishUpload {sha256?}` → `201 UploadFinished` |
//! | `GET /codex/blobs/{sha}.{ext}` | a media file of the Codex by its SHA-256: the library's item, else a managed file of the old store (`Range`, `ETag`) |
//! | `GET /codex/collections/{id}/media?status=` | `CollectionMedia[]`: assets with their availability now |
//! | `POST /codex/collections/{id}/media/{asset}/relink` | `RelinkTarget` (a library item, a media file by its SHA-256, a file under a root) → `RelinkedAsset` |
//! | `POST /codex/collections/{id}/media/search` | `RelinkSearch` → `RelinkReport`: under the roots bound here |
//! | `POST /codex/collections/{id}/media/match` | `MediaMatchRequest` → `MediaMatchReport`: a client folder's files |
//! | `POST /codex/collections/{id}/media/reroot` | `RerootRequest` → `RelinkReport` |
//! | `GET /codex/storage` | `CodexStorage`: disk, per collection, shared and unused media, trashes, thumbnails, uploads |
//! | `GET /codex/blobs/{sha}.{ext}/thumbnail` | its 320 px JPEG (`ETag`) |
//! | `GET /codex/library?q=&kind=&limit=` | `LibraryItems`: the media library's items by path, for a relink to any of them (on a show box: `409 codexElsewhere`) |
//! | `GET /codex/search?q=&collection=&kinds=&limit=` | `SearchResults`: this box's active collection, one collection or `all` ([`super::search`]; on a show box beyond its collection: asked of the Codex) |
//! | `GET /codex/backups`, `POST /codex/backups/snapshots`, `…/backups/{set}/snapshots[/{snapshot}[/preview, /restore]]`, `…/backups/restores[/{id}]`, `GET`, `PUT /codex/backups/oriel` | settings backups and restores ([`super::backups::api`]) |
//! | `POST /codex/export`, `GET`, `PUT /codex/export/settings`, `PUT`, `DELETE /codex/export/passphrase`, `GET /codex/export/scheduled`, `POST /codex/export/imports`, `DELETE …/imports/{id}`, `POST …/imports/{id}/preview`, `POST …/imports/{id}/apply` | encrypted export and import (admins; the two `GET`s: any paired client; [`super::export::api`]) |
//! | `GET`, `PUT /codex/media-prep`, `POST …/media/{asset}/prepare`, `PUT …/media/{asset}/trim`, `POST …/media/{asset}/original` | media preparation: status and settings (admins), prepare, trim, play the original ([`media_prep`]) |
//! | `POST /codex/imports/pptx?collection=&library=&name=&fileName=`, `GET /codex/imports/pptx[/{id}]` | PowerPoint import: the `.pptx` as the body → `202 PptxImportJob`; the jobs ([`super::pptx`]) |
//! | `GET`, `POST`, `DELETE /codex/dormant/upload` | a show box's own, dormant collection: the offer and progress (`DormantUpload`); upload it to the site's Codex (`StartDormantUpload`, admins) → `202`; cancel (admins) → `204` ([`super::adopt`]) |
//!
//! On a show box, uploads, Codex management, settings backups (but offering the box's own
//! snapshot) and export/import answer `409 codexElsewhere` naming the Codex box
//! ([`super::write_through`]).

mod adopt;
mod backups;
mod export;
mod media_prep;
mod pptx;

use super::writer;
use crate::api::{AppState, Error, err, if_match, parse_body, with_etag};
use crate::auth::Access;
use crate::config::ConfigError;
use crate::site::Actor;
use axum::Router;
use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use idn_site_manager::users::Role;
use model::CollectionId;
use protocol::ErrorCode;
use serde_json::{Value, json};
use std::net::SocketAddr;

/// The REST middleware of a show box (see [`super::write_through`]): forwards, answers where the Codex
/// box is, or lets the box answer.
pub async fn write_through(State(app): State<AppState>, req: Request, next: Next) -> Response {
    if app.shared.codex().config.role != model::CodexRole::Client {
        return next.run(req).await;
    }
    match super::write_through::route(req.method().as_str(), req.uri().path()) {
        super::write_through::Route::Local => next.run(req).await,
        super::write_through::Route::CodexBox => super::write_through::elsewhere(&*app.shared),
        super::write_through::Route::Forward => {
            super::write_through::forward(&app.codex, req).await
        }
    }
}

/// The routes, to merge into `/api/v1`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/codex", get(status))
        .route(
            "/codex/collections",
            get(collections).post(create_collection),
        )
        .route(
            "/codex/collections/{id}",
            get(collection)
                .put(update_collection)
                .delete(delete_collection),
        )
        .route("/codex/collection-trash", get(collection_trash))
        .route(
            "/codex/collection-trash/{id}",
            axum::routing::delete(purge_collection),
        )
        .route(
            "/codex/collection-trash/{id}/restore",
            post(restore_collection),
        )
        .route("/codex/uploads", get(uploads).post(create_upload))
        .route(
            "/codex/uploads/{id}",
            get(upload)
                .put(upload_chunk)
                .delete(cancel_upload)
                .layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route("/codex/uploads/{id}/finish", post(finish_upload))
        .route("/codex/blobs/{file}", get(blob))
        .route("/codex/blobs/{file}/thumbnail", get(blob_thumbnail))
        .route("/codex/storage", get(storage))
        .route("/codex/collections/{id}/media", get(collection_media))
        .route("/codex/collections/{id}/media/search", post(search_media))
        .route("/codex/collections/{id}/media/match", post(match_media))
        .route("/codex/collections/{id}/media/reroot", post(reroot_media))
        .route(
            "/codex/collections/{id}/media/{asset}/relink",
            post(relink_media),
        )
        .route("/codex/assignment", put(assign))
        .route("/codex/collections/{id}/changes", get(changes))
        .route("/codex/trash", get(trash).delete(purge))
        .route("/codex/trash/{id}/restore", post(restore))
        .route("/codex/search", get(search))
        .route("/codex/library", get(library_items))
        .merge(backups::routes())
        .merge(export::routes())
        .merge(adopt::routes())
        .merge(media_prep::routes())
        .merge(pptx::routes())
}

/// `GET /codex/library?q=&kind=&limit=` (any paired client): the media library's items, for
/// a relink to any of them.
async fn library_items(
    State(app): State<AppState>,
    query: Result<
        axum::extract::Query<std::collections::HashMap<String, String>>,
        axum::extract::rejection::QueryRejection,
    >,
) -> Response {
    let Ok(axum::extract::Query(mut q)) = query else {
        return err(ErrorCode::BadRequest, "the query string can't be read").into_response();
    };
    let text = q.remove("q").unwrap_or_default();
    let kind = q.remove("kind").filter(|k| !k.is_empty());
    if kind
        .as_deref()
        .is_some_and(|k| !["video", "picture", "sound"].contains(&k))
    {
        return Error(protocol::ApiError::invalid(
            "kind",
            "the kind is video, picture or sound",
        ))
        .into_response();
    }
    let limit = match q.remove("limit").map(|l| l.parse::<usize>()) {
        None => 500,
        Some(Ok(n)) if (1..=5000).contains(&n) => n,
        Some(_) => {
            return Error(protocol::ApiError::invalid(
                "limit",
                "the limit is 1 to 5000",
            ))
            .into_response();
        }
    };
    let lib = app.shared.library.clone();
    let r = tokio::task::spawn_blocking(move || lib.list(&text, kind.as_deref(), limit)).await;
    match r {
        Ok(Some(items)) => axum::Json(items).into_response(),
        Ok(None) => err(
            ErrorCode::Unavailable,
            "the media library isn't open on this box",
        )
        .into_response(),
        Err(e) => err(ErrorCode::Internal, e.to_string()).into_response(),
    }
}

/// `GET /codex/search?q=&collection=&kinds=&limit=` (any paired client).
async fn search(
    State(app): State<AppState>,
    query: Result<
        axum::extract::Query<std::collections::HashMap<String, String>>,
        axum::extract::rejection::QueryRejection,
    >,
) -> Response {
    let Ok(axum::extract::Query(mut q)) = query else {
        return err(ErrorCode::BadRequest, "the query string can't be read").into_response();
    };
    let params = super::search::Params {
        q: q.remove("q"),
        collection: q.remove("collection"),
        kinds: q.remove("kinds"),
        limit: q.remove("limit"),
    };
    // A show box searches its copy of its collection; beyond it, the site's Codex.
    let codex = app.shared.codex();
    if codex.config.role == model::CodexRole::Client {
        let beyond = match params.collection.as_deref().map(str::trim) {
            None | Some("") => false,
            Some("all") => true,
            Some(c) => c
                .parse::<CollectionId>()
                .map_or(true, |c| c != codex.active),
        };
        if beyond {
            return super::write_through::search(&app.codex, params).await;
        }
    }
    drop(codex);
    let sh = app.codex.clone();
    match tokio::task::spawn_blocking(move || super::search::run(&sh, &params)).await {
        Ok(Ok(r)) => axum::Json(r).into_response(),
        Ok(Err(e)) => Error(e).into_response(),
        Err(e) => err(ErrorCode::Internal, e.to_string()).into_response(),
    }
}

fn parse_collection(s: &str) -> Result<CollectionId, Error> {
    s.parse::<CollectionId>().map_err(|_| {
        err(
            ErrorCode::BadRequest,
            format!("collection id {s:?} is not a UUID"),
        )
    })
}

/// Writes an audit entry off the async threads.
pub(crate) async fn audit(
    app: &AppState,
    actor: &Actor,
    action: &str,
    detail: Value,
    error: Option<String>,
) {
    let (site, actor, action) = (app.shared.site.clone(), actor.clone(), action.to_string());
    let _ =
        tokio::task::spawn_blocking(move || site.audit(&actor, &action, &detail, error.as_deref()))
            .await;
}

/// The site's admin, or `403` (audited).
pub(crate) async fn admin(
    app: &AppState,
    access: &Access,
    peer: SocketAddr,
    action: &str,
    detail: &Value,
) -> Result<Actor, Error> {
    let actor = app.shared.site.actor(access, peer.ip());
    let site = app.shared.site.clone();
    let a = actor.clone();
    let checked = tokio::task::spawn_blocking(move || site.require(&a, Role::Admin))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    match checked {
        Ok(_) => Ok(actor),
        Err(e) => {
            audit(app, &actor, action, detail.clone(), Some(e.message.clone())).await;
            Err(Error(e))
        }
    }
}

/// Runs an admin change and audits its outcome.
async fn audited<T>(
    app: &AppState,
    actor: &Actor,
    action: &str,
    detail: Value,
    r: Result<T, ConfigError>,
) -> Result<T, ConfigError> {
    let error = r.as_ref().err().map(|e| e.error.message.clone());
    audit(app, actor, action, detail, error).await;
    r
}

async fn status(State(app): State<AppState>) -> Response {
    axum::Json(writer::status(&app.codex)).into_response()
}

async fn collections(State(app): State<AppState>) -> Response {
    match writer::summaries(&app.codex).await {
        Ok(s) => axum::Json(s).into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn collection(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    let r = async {
        let id = parse_collection(&id)?;
        writer::summary_of(&app.codex, id).await.map_err(Error)
    }
    .await;
    match r {
        Ok(s) => with_etag(StatusCode::OK, &s.revision.clone(), s),
        Err(e) => e.into_response(),
    }
}

async fn create_collection(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let body: protocol::CreateCollection = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let action = "codex.createCollection";
    let detail = json!({"name": body.name});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app
        .shared
        .codex_writer
        .create_collection(&app.codex, body)
        .await;
    let detail = match &r {
        Ok(s) => json!({"collection": s.collection.id, "name": s.collection.name}),
        Err(_) => detail,
    };
    match audited(&app, &actor, action, detail, r).await {
        Ok(s) => with_etag(StatusCode::CREATED, &s.revision.clone(), s),
        Err(e) => e.into_response(),
    }
}

async fn update_collection(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let parsed = (|| {
        Ok::<_, Error>((
            parse_collection(&id)?,
            parse_body::<protocol::UpdateCollection>(&body)?,
        ))
    })();
    let (id, body) = match parsed {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let action = "codex.updateCollection";
    let detail = json!({"collection": id, "name": body.name});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app
        .shared
        .codex_writer
        .update_collection(&app.codex, id, body, if_match(&headers))
        .await;
    match audited(&app, &actor, action, detail, r).await {
        Ok(s) => with_etag(StatusCode::OK, &s.revision.clone(), s),
        Err(e) => e.into_response(),
    }
}

async fn delete_collection(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let id = match parse_collection(&id) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let action = "codex.deleteCollection";
    let name = app
        .shared
        .codex()
        .collection(id)
        .map(|c| c.collection.name.clone());
    let detail = json!({"collection": id, "name": name});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let by = crate::api::change_by(&app.shared);
    let r = app
        .shared
        .codex_writer
        .delete_collection(&app.codex, id, if_match(&headers), by)
        .await;
    match audited(&app, &actor, action, detail, r).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn collection_trash(State(app): State<AppState>) -> Response {
    match writer::deleted_collections(&app.codex).await {
        Ok(list) => axum::Json(list).into_response(),
        Err(e) => Error(e).into_response(),
    }
}

/// The name of a collection in the collection trash, for the audit.
fn deleted_name(app: &AppState, id: CollectionId) -> Option<String> {
    app.shared
        .codex()
        .deleted(id)
        .map(|d| d.collection.name.clone())
}

async fn restore_collection(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    let id = match parse_collection(&id) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let action = "codex.restoreCollection";
    let detail = json!({"collection": id, "name": deleted_name(&app, id)});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app
        .shared
        .codex_writer
        .restore_collection(&app.codex, id)
        .await;
    match audited(&app, &actor, action, detail, r).await {
        Ok(s) => with_etag(StatusCode::OK, &s.revision.clone(), s),
        Err(e) => e.into_response(),
    }
}

async fn purge_collection(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let id = match parse_collection(&id) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let action = "codex.purgeCollection";
    let detail = json!({"collection": id, "name": deleted_name(&app, id)});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app
        .shared
        .codex_writer
        .purge_collection(&app.codex, id, if_match(&headers))
        .await;
    let detail = match &r {
        Ok(items) => json!({"collection": id, "name": detail["name"], "trashItems": items}),
        Err(_) => detail,
    };
    match audited(&app, &actor, action, detail, r).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn assign(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let body: protocol::SetAssignment = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let action = "codex.assign";
    let detail = json!({"collection": body.collection, "from": app.shared.codex().active, "confirm": body.confirm});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app
        .shared
        .codex_writer
        .assign(&app.codex, &app.show, body.collection, body.confirm)
        .await;
    match audited(&app, &actor, action, detail, r).await {
        Ok(s) => axum::Json(s).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn changes(
    State(app): State<AppState>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let r = async {
        let id = parse_collection(&id)?;
        let number = |name: &str, default: u64| -> Result<u64, Error> {
            match crate::api::query_param(query.as_deref(), name) {
                None => Ok(default),
                Some(v) => v.parse().map_err(|_| {
                    err(
                        ErrorCode::BadRequest,
                        format!("{name} must be a whole number, not {v:?}"),
                    )
                }),
            }
        };
        let since = number("since", 0)?;
        let limit = number("limit", super::log::MAX_LIMIT as u64)? as usize;
        writer::changes(&app.codex, id, since, limit)
            .await
            .map_err(Error)
    }
    .await;
    match r {
        Ok(c) => axum::Json(c).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn trash(State(app): State<AppState>) -> Response {
    let sh = app.shared.clone();
    match tokio::task::spawn_blocking(move || sh.codex_writer.trash_items(&*sh)).await {
        Ok(items) => axum::Json(items).into_response(),
        Err(e) => err(ErrorCode::Internal, e.to_string()).into_response(),
    }
}

async fn restore(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Ok(id) = id.parse::<uuid::Uuid>() else {
        return err(
            ErrorCode::BadRequest,
            format!("trash entry id {id:?} is not a UUID"),
        )
        .into_response();
    };
    let client = headers
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().chars().take(80).collect::<String>())
        .filter(|s| !s.is_empty());
    let by = crate::api::change_by(&app.shared);
    match app
        .shared
        .codex_writer
        .restore(&app.codex, &app.show, id, client, by)
        .await
    {
        Ok(r) => axum::Json(r).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn purge(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
) -> Response {
    let action = "codex.purgeTrash";
    if let Err(e) = writer::writable(&app.codex) {
        return Error(e).into_response();
    }
    let actor = match admin(&app, &access, peer, action, &json!({})).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app.shared.codex_writer.purge(&app.codex).await;
    audit(&app, &actor, action, json!({"purged": r.purged}), None).await;
    axum::Json(r).into_response()
}

// ---- uploads and blobs ----------------------------------------------------------------------

fn parse_upload(s: &str) -> Result<uuid::Uuid, Error> {
    s.parse().map_err(|_| {
        err(
            ErrorCode::BadRequest,
            format!("upload id {s:?} is not a UUID"),
        )
    })
}

fn client_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().chars().take(80).collect::<String>())
        .filter(|s| !s.is_empty())
}

fn upload_error(e: super::uploads::UploadError) -> Response {
    match e {
        super::uploads::UploadError::Api(e) => e.into_response(),
        super::uploads::UploadError::Offset(error, offset) => (
            StatusCode::CONFLICT,
            axum::Json(protocol::UploadConflict { error, offset }),
        )
            .into_response(),
    }
}

async fn storage(State(app): State<AppState>) -> Response {
    match app.shared.codex_writer.storage(&app.codex).await {
        Ok(s) => axum::Json(s).into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn uploads(State(app): State<AppState>) -> Response {
    axum::Json(app.shared.uploads.list(&app.codex).await).into_response()
}

async fn create_upload(State(app): State<AppState>, body: Bytes) -> Response {
    let body: protocol::CreateUpload = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let by = crate::api::change_by(&app.shared);
    match app.shared.uploads.create(&app.codex, body, by).await {
        Ok(info) => (StatusCode::CREATED, axum::Json(info)).into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn upload(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    let id = match parse_upload(&id) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match app.shared.uploads.get(&app.codex, id).await {
        Ok(info) => axum::Json(info).into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn upload_chunk(
    State(app): State<AppState>,
    Path(id): Path<String>,
    req: axum::extract::Request,
) -> Response {
    let id = match parse_upload(&id) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let (parts, body) = req.into_parts();
    let range = parts
        .headers
        .get(axum::http::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok());
    match app.shared.uploads.chunk(&app.codex, id, range, body).await {
        Ok(info) => axum::Json(info).into_response(),
        Err(e) => upload_error(e),
    }
}

async fn cancel_upload(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    let id = match parse_upload(&id) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match app.shared.uploads.cancel(&app.codex, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn finish_upload(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let id = match parse_upload(&id) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let body: protocol::FinishUpload = if body.iter().all(u8::is_ascii_whitespace) {
        protocol::FinishUpload::default()
    } else {
        match parse_body(&body) {
            Ok(b) => b,
            Err(e) => return e.into_response(),
        }
    };
    let by = crate::api::change_by(&app.shared);
    match app
        .shared
        .uploads
        .finish(
            &app.codex,
            &app.show,
            &app.media_jobs,
            id,
            body,
            client_header(&headers),
            by,
        )
        .await
    {
        Ok(f) => (StatusCode::CREATED, axum::Json(f)).into_response(),
        Err(e) => upload_error(e),
    }
}

/// `"<sha>"`: blobs never change.
fn blob_etag(sha: &str, suffix: &str) -> String {
    format!("\"{sha}{suffix}\"")
}

fn not_modified(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"))
}

fn cache_headers(etag: &str) -> [(axum::http::HeaderName, axum::http::HeaderValue); 2] {
    use axum::http::{HeaderValue, header};
    [
        (
            header::ETAG,
            HeaderValue::from_str(etag).unwrap_or(HeaderValue::from_static("\"\"")),
        ),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, max-age=31536000, immutable"),
        ),
    ]
}

async fn blob(
    State(app): State<AppState>,
    Path(file): Path<String>,
    req: axum::extract::Request,
) -> Response {
    let Some((sha, _)) = super::blobs::parse_blob_name(&file) else {
        return err(
            ErrorCode::BadRequest,
            format!("{file:?} is not a blob name"),
        )
        .into_response();
    };
    let sh = app.codex.clone();
    let f = file.clone();
    let path = match tokio::task::spawn_blocking(move || super::blobs::find_blob(&sh, &f)).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return err(ErrorCode::NotFound, format!("no media file {file}")).into_response();
        }
        Err(e) => return err(ErrorCode::Internal, e.to_string()).into_response(),
    };
    let etag = blob_etag(&sha, "");
    if not_modified(req.headers(), &etag) {
        return (StatusCode::NOT_MODIFIED, cache_headers(&etag)).into_response();
    }
    // ServeFile answers Range requests and sets the content type.
    use tower::ServiceExt;
    match tower_http::services::ServeFile::new(path)
        .oneshot(req)
        .await
    {
        Ok(r) => {
            let mut r = r.map(axum::body::Body::new);
            for (k, v) in cache_headers(&etag) {
                r.headers_mut().insert(k, v);
            }
            r
        }
        Err(e) => err(ErrorCode::Internal, e.to_string()).into_response(),
    }
}

async fn blob_thumbnail(
    State(app): State<AppState>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Response {
    use axum::http::{HeaderValue, header};
    let Some((sha, ext)) = super::blobs::parse_blob_name(&file) else {
        return err(
            ErrorCode::BadRequest,
            format!("{file:?} is not a blob name"),
        )
        .into_response();
    };
    let etag = blob_etag(&sha, &format!("-{}", super::blobs::THUMB_WIDTH));
    if not_modified(&headers, &etag) {
        return (StatusCode::NOT_MODIFIED, cache_headers(&etag)).into_response();
    }
    let sh = app.codex.clone();
    let f = file.clone();
    let path = match tokio::task::spawn_blocking(move || super::blobs::find_blob(&sh, &f)).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return err(ErrorCode::NotFound, format!("no media file {file}")).into_response();
        }
        Err(e) => return err(ErrorCode::Internal, e.to_string()).into_response(),
    };
    let kind = super::blobs::kind_for(&ext).unwrap_or(model::MediaKind::Unknown);
    let jpeg = match app.media_jobs.thumbnail(&file, path).await {
        Ok(j) => j,
        Err(e) if kind == model::MediaKind::Image => {
            return err(ErrorCode::NotFound, format!("cannot decode {file}: {e}")).into_response();
        }
        // Audio, and videos without a decodable frame: the generic picture (not stored).
        Err(_) => {
            let img = crate::api::placeholder_thumbnail(kind, super::blobs::THUMB_WIDTH);
            let mut out = Vec::new();
            if let Err(e) =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, crate::thumbs::QUALITY)
                    .encode_image(&img)
            {
                return err(ErrorCode::Internal, e.to_string()).into_response();
            }
            if kind != model::MediaKind::Audio {
                // A video's frame may decode later (the media worker back, another FFmpeg):
                // the stand-in is kept only briefly, under no ETag of the real thumbnail.
                return (
                    [
                        (
                            header::CACHE_CONTROL,
                            HeaderValue::from_static("private, max-age=60"),
                        ),
                        (header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg")),
                    ],
                    axum::body::Body::from(out),
                )
                    .into_response();
            }
            // Audio has no picture, ever: the generic one is its thumbnail.
            out
        }
    };
    (
        cache_headers(&etag),
        [(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))],
        axum::body::Body::from(jpeg),
    )
        .into_response()
}

// ---- missing media and relinking ------------------------------------------------------------

fn content_of(
    app: &AppState,
    id: CollectionId,
) -> Result<std::sync::Arc<crate::data::CollectionContent>, Error> {
    app.shared
        .collection_content(id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("collection {id} not found")))
}

async fn collection_media(
    State(app): State<AppState>,
    Path(id): Path<String>,
    query: Result<
        axum::extract::Query<std::collections::HashMap<String, String>>,
        axum::extract::rejection::QueryRejection,
    >,
) -> Response {
    let Ok(axum::extract::Query(mut query)) = query else {
        return err(ErrorCode::BadRequest, "the query string can't be read").into_response();
    };
    let r = async {
        let content = content_of(&app, parse_collection(&id)?)?;
        // Percent-decoded: `missing,unsupported` arrives as `missing%2Cunsupported` from a
        // client that encodes its query.
        let status = query.remove("status");
        tokio::task::spawn_blocking(move || super::relink::list(&content, status.as_deref()))
            .await
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
            .map_err(Error)
    }
    .await;
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Applies a search's or a re-root's relinks and status changes through the writer, then
/// queues probes of the relinked assets.
async fn apply_relinks(
    app: &AppState,
    collection: CollectionId,
    outcome: &super::relink::Outcome,
    headers: &HeaderMap,
) -> Result<(), crate::config::ConfigError> {
    if outcome.changes.is_empty() && outcome.statuses.is_empty() {
        return Ok(());
    }
    let by = crate::api::change_by(&app.shared);
    app.shared
        .codex_writer
        .media(
            &app.codex,
            &app.show,
            collection,
            |c| {
                Ok(super::writer::MediaPlan {
                    assets: super::relink::apply(c, outcome),
                    bins: Vec::new(),
                })
            },
            client_header(headers),
            by,
        )
        .await?;
    for ch in &outcome.changes {
        app.media_jobs.probe(collection, ch.asset);
    }
    Ok(())
}

async fn search_media(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = async {
        let collection = parse_collection(&id)?;
        let body: protocol::RelinkSearch = if body.iter().all(u8::is_ascii_whitespace) {
            protocol::RelinkSearch::default()
        } else {
            parse_body(&body)?
        };
        let content = content_of(&app, collection)?;
        let assets = body.assets.clone();
        let outcome =
            super::relink::low_priority(move || super::relink::search(&content, assets.as_deref()))
                .await
                .ok_or_else(|| err(ErrorCode::Internal, "the relink search stopped"))?
                .map_err(Error)?;
        if !body.dry_run {
            apply_relinks(&app, collection, &outcome, &headers)
                .await
                .map_err(|e| Error(e.error))?;
        }
        Ok::<_, Error>(outcome.report)
    }
    .await;
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn reroot_media(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = async {
        let collection = parse_collection(&id)?;
        let body: protocol::RerootRequest = parse_body(&body)?;
        let content = content_of(&app, collection)?;
        let req = body.clone();
        let outcome = super::relink::low_priority(move || super::relink::reroot(&content, &req))
            .await
            .ok_or_else(|| err(ErrorCode::Internal, "the re-root stopped"))?
            .map_err(Error)?;
        if !body.dry_run {
            apply_relinks(&app, collection, &outcome, &headers)
                .await
                .map_err(|e| Error(e.error))?;
        }
        Ok::<_, Error>(outcome.report)
    }
    .await;
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn match_media(State(app): State<AppState>, Path(id): Path<String>, body: Bytes) -> Response {
    let r = async {
        let content = content_of(&app, parse_collection(&id)?)?;
        let body: protocol::MediaMatchRequest = parse_body(&body)?;
        tokio::task::spawn_blocking(move || {
            super::relink::match_files(&content, &body.files, body.assets.as_deref())
        })
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
        .map_err(Error)
    }
    .await;
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn relink_media(
    State(app): State<AppState>,
    Path((id, asset)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    use model::AssetLocation;
    use protocol::{ApiError, RelinkTarget};
    let r = async {
        let collection = parse_collection(&id)?;
        let asset: model::MediaAssetId =
            asset.parse::<uuid::Uuid>().map(Into::into).map_err(|_| {
                err(
                    ErrorCode::BadRequest,
                    format!("media id {asset:?} is not a UUID"),
                )
            })?;
        let target: RelinkTarget = parse_body(&body)?;
        let content = content_of(&app, collection)?;
        let location = match target {
            RelinkTarget::Library { id } => {
                let sha = app
                    .shared
                    .library
                    .item(&id)
                    .ok_or_else(|| {
                        Error(ApiError::invalid(
                            "id",
                            format!("the media library has no item {id}"),
                        ))
                    })?
                    .sha256
                    .ok_or_else(|| {
                        Error(ApiError::new(
                            ErrorCode::Conflict,
                            format!(
                                "the media library is still reading item {id} (its SHA-256): try again shortly"
                            ),
                        ))
                    })?;
                AssetLocation::Library { id, sha256: sha }
            }
            RelinkTarget::Blob { sha256, .. } if app.shared.library.by_sha(&sha256).is_some() => {
                let (id, _) = app.shared.library.by_sha(&sha256).unwrap_or_default();
                AssetLocation::Library { id, sha256 }
            }
            RelinkTarget::Blob { sha256, extension } => {
                let extension = extension.to_ascii_lowercase();
                let name = super::blobs::blob_name(&sha256, &extension);
                if super::blobs::parse_blob_name(&name).is_none() {
                    return Err(Error(ApiError::invalid("sha256", "not a media file name")));
                }
                let sh = app.codex.clone();
                let n = name.clone();
                let found = tokio::task::spawn_blocking(move || super::blobs::find_blob(&sh, &n))
                    .await
                    .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
                if found.is_none() {
                    return Err(Error(ApiError::invalid(
                        "sha256",
                        format!("the Codex has no media file {name}"),
                    )));
                }
                AssetLocation::Managed { sha256, extension }
            }
            RelinkTarget::Linked { root, path } => {
                let path = super::relink::safe_relative(&path)
                    .ok_or_else(|| Error(ApiError::invalid("path", "a path below the root")))?;
                if !super::relink::root_known(&content, root) {
                    return Err(Error(ApiError::invalid(
                        "root",
                        format!(
                            "media root {} is neither bound on this box nor in the collection",
                            root.0
                        ),
                    )));
                }
                AssetLocation::Linked { root, path }
            }
        };
        let by = crate::api::change_by(&app.shared);
        let mut changed = false;
        app.shared
            .codex_writer
            .media(
                &app.codex,
                &app.show,
                collection,
                |c| {
                    let cur = c.assets.get(&asset).ok_or_else(|| {
                        crate::config::ConfigError::from(ApiError::new(
                            ErrorCode::NotFound,
                            format!("media asset {} not found", asset.0),
                        ))
                    })?;
                    let mut plan = super::writer::MediaPlan::default();
                    if let Some(a) = super::relink::manual(c, cur, location.clone()) {
                        changed = true;
                        plan.assets.push(a);
                    }
                    Ok(plan)
                },
                client_header(&headers),
                by,
            )
            .await
            .map_err(|e| Error(e.error))?;
        if changed {
            if let AssetLocation::Library { id, .. } = &location {
                app.shared.library.add_marks(std::slice::from_ref(id));
            }
            app.media_jobs.probe(collection, asset);
        }
        let a = content_of(&app, collection)?
            .assets
            .get(&asset)
            .cloned()
            .ok_or_else(|| {
                err(
                    ErrorCode::NotFound,
                    format!("media asset {} not found", asset.0),
                )
            })?;
        Ok::<_, Error>(protocol::RelinkedAsset {
            revision: crate::config::revision(&a),
            asset: a,
        })
    }
    .await;
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}
