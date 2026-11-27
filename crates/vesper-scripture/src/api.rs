//! The read-only scripture routes (`/api/v1/scripture/...`), over any router state that gives
//! the [`Scripture`]. The routes that write (generated presentations) and render (previews)
//! need the show and live in the server.
//!
//! | Route | |
//! |---|---|
//! | `GET /scripture/translations` | `ScriptureTranslation[]` |
//! | `GET /scripture/translations/{id}/books` | `ScriptureBook[]` (`404` unknown translation) |
//! | `GET /scripture/parse?translation=&q=` | `ScriptureParse` |
//! | `GET /scripture/passage?translation=&ref=` | `ScripturePassage` (`400` bad reference) |
//! | `GET /scripture/search?translation=&q=&limit=` | `ScriptureSearch` (limit default 50, at most 500) |

use crate::{LookupError, Scripture};
use axum::Router;
use axum::extract::{FromRef, Path, Query, State};
use axum::routing::get;
use protocol::{
    ErrorCode, ScriptureBook, ScriptureParse, ScripturePassage, ScriptureSearch,
    ScriptureTranslation,
};
use serde::Deserialize;
use std::sync::Arc;
use vesper_core::http::{ApiResult, Error, err, ok};

/// The state of the scripture handlers.
pub type ScriptureState = Arc<Scripture>;

/// The routes, to merge into `/api/v1`.
pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    ScriptureState: FromRef<S>,
{
    Router::new()
        .route("/scripture/translations", get(translations))
        .route("/scripture/translations/{id}/books", get(books))
        .route("/scripture/parse", get(parse))
        .route("/scripture/passage", get(passage))
        .route("/scripture/search", get(search))
}

/// A lookup error as an API error.
pub fn api_error(e: LookupError) -> Error {
    match e {
        LookupError::UnknownTranslation(_) => err(ErrorCode::NotFound, e.to_string()),
        LookupError::BadReference(_) => err(ErrorCode::BadRequest, e.to_string()),
        LookupError::Unreadable(_) => err(ErrorCode::Internal, e.to_string()),
    }
}

/// Query parameters of the routes.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Params {
    translation: String,
    q: String,
    #[serde(rename = "ref")]
    reference: String,
    limit: Option<usize>,
}

async fn translations(State(s): State<ScriptureState>) -> ApiResult<Vec<ScriptureTranslation>> {
    ok(s.translations())
}

async fn books(
    State(s): State<ScriptureState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<ScriptureBook>> {
    ok(s.get(&id).map_err(api_error)?.books())
}

async fn parse(
    State(s): State<ScriptureState>,
    Query(p): Query<Params>,
) -> ApiResult<ScriptureParse> {
    ok(s.get(&p.translation)
        .map_err(api_error)?
        .parse_feedback(&p.q))
}

async fn passage(
    State(s): State<ScriptureState>,
    Query(p): Query<Params>,
) -> ApiResult<ScripturePassage> {
    let t = s.get(&p.translation).map_err(api_error)?;
    ok(t.passage(&p.reference).map_err(api_error)?)
}

async fn search(
    State(s): State<ScriptureState>,
    Query(p): Query<Params>,
) -> ApiResult<ScriptureSearch> {
    let t = s.get(&p.translation).map_err(api_error)?;
    let limit = p.limit.unwrap_or(50).clamp(1, 500);
    ok(t.search(&p.q, limit).map_err(api_error)?)
}
