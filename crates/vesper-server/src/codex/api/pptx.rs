//! The PowerPoint import routes (`/api/v1/codex/imports/pptx…`), merged into the Codex's
//! routes: a content edit (any paired client, like uploads). On a show box they answer
//! `409 codexElsewhere` ([`crate::codex::write_through`]).
//!
//! | Route | |
//! |---|---|
//! | `POST /codex/imports/pptx?collection=&library=&name=&fileName=` | the `.pptx` as the body → `202 PptxImportJob` |
//! | `GET /codex/imports/pptx` | `PptxImportJob[]`, newest first |
//! | `GET /codex/imports/pptx/{id}` | `PptxImportJob` |

use crate::api::{AppState, err};
use crate::auth::Access;
use crate::codex::pptx::Request;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use protocol::{ApiError, ErrorCode};
use std::collections::HashMap;

/// Largest `.pptx` taken, bytes.
pub const MAX_PPTX: usize = 1 << 30;

/// The routes, merged into the Codex's.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/codex/imports/pptx",
            get(list)
                .post(start)
                .layer(axum::extract::DefaultBodyLimit::max(MAX_PPTX)),
        )
        .route("/codex/imports/pptx/{id}", get(one))
}

fn invalid(field: &str, message: impl Into<String>) -> Response {
    crate::api::Error(ApiError::invalid(field, message)).into_response()
}

async fn start(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    Query(q): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    let file_name = q
        .get("fileName")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "presentation.pptx".into());
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pptx" | "ppsx" | "potx" | "pptm" | "ppsm" => {}
        "key" => {
            return invalid(
                "fileName",
                "a Keynote file: in Keynote choose File ▸ Export To ▸ PowerPoint, then import the .pptx",
            );
        }
        "ppt" | "pps" | "pot" => {
            return invalid(
                "fileName",
                "an old PowerPoint file (.ppt): open it in PowerPoint and save it as .pptx first",
            );
        }
        _ => {
            return invalid(
                "fileName",
                "not a .pptx file (Keynote and Google Slides: export as .pptx first)",
            );
        }
    }
    if body.len() < 4 || &body[..2] != b"PK" {
        return invalid("body", "the file isn't a .pptx (not a zip package)");
    }
    let collection = match q.get("collection").filter(|s| !s.is_empty()) {
        Some(c) => match uuid::Uuid::parse_str(c) {
            Ok(u) => model::CollectionId(u),
            Err(_) => return invalid("collection", "not a collection ID"),
        },
        None => app.shared.codex().active,
    };
    let Some(content) = app.shared.collection_content(collection) else {
        return invalid(
            "collection",
            format!("collection {} is not in the Codex", collection.0),
        );
    };
    let library = match q.get("library").filter(|s| !s.is_empty()) {
        Some(l) => match uuid::Uuid::parse_str(l).map(model::LibraryId) {
            Ok(l) if content.libraries.contains_key(&l) => Some(l),
            _ => return invalid("library", "not a library of the collection"),
        },
        None => None,
    };
    let name = q
        .get("name")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if name.as_ref().is_some_and(|n| n.chars().count() > 200) {
        return invalid("name", "a name of at most 200 characters");
    }
    if let Err(e) = crate::codex::writer::writable(&app.codex) {
        return crate::api::Error(e).into_response();
    }
    // The body goes to the job as it is (no copy of up to 1 GiB).
    match app.pptx.start(
        &app,
        access,
        Request {
            collection,
            library,
            name,
            file_name,
        },
        body,
    ) {
        Some(job) => (StatusCode::ACCEPTED, axum::Json(job)).into_response(),
        None => err(
            ErrorCode::RateLimited,
            "another PowerPoint import is already waiting: try again when one has finished",
        )
        .into_response(),
    }
}

async fn list(State(app): State<AppState>) -> Response {
    axum::Json(app.pptx.list()).into_response()
}

async fn one(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    match app.pptx.get(&id) {
        Some(j) => axum::Json(j).into_response(),
        None => err(ErrorCode::NotFound, format!("no import {id}")).into_response(),
    }
}
