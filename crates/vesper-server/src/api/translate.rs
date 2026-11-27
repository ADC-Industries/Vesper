//! Machine translation (see [`crate::translate`]): this box's translation models (reading
//! them: any client; downloading, stopping and deleting them: the site's admin) and
//! translation previews (any client: a preview reads a presentation and saves nothing; its
//! model is downloaded first when it is not on the box, as the owner decided: models come on
//! first use).

use super::*;
use crate::codex::api::{admin, audit};
use crate::translate::{MAX_SLIDES, PreviewInput, Refused};
use protocol::{TranslateJob, TranslatePreview, TranslationModel, TranslationModels};
use serde_json::json;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/translate/models", get(models))
        .route("/translate/models/{id}", delete(remove))
        .route("/translate/models/{id}/download", post(download))
        .route("/translate/models/{id}/stop", post(stop))
        .route("/translate/preview", post(preview))
        .route("/translate/jobs/{id}", get(job).delete(cancel))
}

/// `GET /translate/models`: the language pairs offered, their licences and where each stands.
async fn models(State(app): State<AppState>) -> ApiResult<TranslationModels> {
    ok(app.shared.translate.models())
}

#[derive(Clone, Copy)]
enum ModelAction {
    Download,
    Stop,
    Delete,
}

async fn model_action(
    app: AppState,
    access: Access,
    peer: SocketAddr,
    id: String,
    what: ModelAction,
) -> Result<axum::Json<TranslationModel>, Error> {
    let action = match what {
        ModelAction::Download => "translate.models.download",
        ModelAction::Stop => "translate.models.stop",
        ModelAction::Delete => "translate.models.delete",
    };
    let detail = json!({ "pair": id });
    let actor = admin(&app, &access, peer, action, &detail).await?;
    if app.shared.translate.model_of(&id).is_none() {
        let m = format!("no language pair {id:?}");
        audit(&app, &actor, action, detail, Some(m.clone())).await;
        return Err(err(ErrorCode::NotFound, m));
    }
    let shared = app.shared.clone();
    let r = tokio::task::spawn_blocking(move || match what {
        ModelAction::Download => shared.translate.download(&id),
        ModelAction::Stop => shared.translate.stop(&id),
        ModelAction::Delete => shared.translate.delete(&id),
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    audit(&app, &actor, action, detail, r.as_ref().err().cloned()).await;
    r.map(axum::Json).map_err(|e| err(ErrorCode::Internal, e))
}

/// `POST /translate/models/{id}/download` (admin): starts fetching a pair's model.
async fn download(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Result<axum::Json<TranslationModel>, Error> {
    model_action(app, access, peer, id, ModelAction::Download).await
}

/// `POST /translate/models/{id}/stop` (admin): stops fetching it (what was fetched is kept
/// only when complete and checked).
async fn stop(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Result<axum::Json<TranslationModel>, Error> {
    model_action(app, access, peer, id, ModelAction::Stop).await
}

/// `DELETE /translate/models/{id}` (admin): removes its files from this box.
async fn remove(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Result<axum::Json<TranslationModel>, Error> {
    model_action(app, access, peer, id, ModelAction::Delete).await
}

/// `POST /translate/preview`: translates a presentation's slides (`202`, the job). Nothing
/// is saved.
async fn preview(State(app): State<AppState>, body: Bytes) -> Result<Response, Error> {
    let req: TranslatePreview = parse_body(&body)?;
    let to = req.to.trim().to_string();
    let from = req
        .from
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .unwrap_or("en")
        .to_string();
    let valid = |l: &str| {
        !l.is_empty() && l.len() <= 35 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    if !valid(&to) || !valid(&from) {
        return Err(err(
            ErrorCode::BadRequest,
            "`to` and `from` are language codes (BCP 47, such as es or pt-BR)",
        ));
    }
    let d = app.shared.data();
    let p = d.content.presentation(req.presentation).ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("presentation {} not found", req.presentation.0),
        )
    })?;
    let slides: Vec<model::Slide> = match &req.slides {
        None => p.slides().cloned().collect(),
        Some(ids) => {
            if let Some(missing) = ids.iter().find(|id| !p.slides().any(|s| s.id == **id)) {
                return Err(err(
                    ErrorCode::NotFound,
                    format!("slide {} is not in this presentation", missing.0),
                ));
            }
            p.slides()
                .filter(|s| ids.contains(&s.id))
                .cloned()
                .collect()
        }
    };
    if slides.len() > MAX_SLIDES {
        return Err(err(
            ErrorCode::BadRequest,
            format!("a preview translates at most {MAX_SLIDES} slides"),
        ));
    }
    let input = PreviewInput {
        presentation: req.presentation,
        from,
        to,
        slides,
    };
    let shared = app.shared.clone();
    let job = tokio::task::spawn_blocking(move || shared.translate.preview(input))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    match job {
        Ok(job) => Ok((StatusCode::ACCEPTED, axum::Json(job)).into_response()),
        Err(Refused::NoPair(m)) => Err(err(ErrorCode::BadRequest, m)),
        Err(Refused::Busy) => Err(err(
            ErrorCode::Conflict,
            "too many translation previews: wait for one to end",
        )),
    }
}

/// `GET /translate/jobs/{id}`: a preview, with its proposals once done.
async fn job(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<TranslateJob> {
    app.shared
        .translate
        .job(&id)
        .map(axum::Json)
        .ok_or_else(|| err(ErrorCode::NotFound, "no such translation preview"))
}

/// `DELETE /translate/jobs/{id}`: cancels a preview (`204`).
async fn cancel(State(app): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, Error> {
    if app.shared.translate.cancel(&id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(err(ErrorCode::NotFound, "no such translation preview"))
    }
}
