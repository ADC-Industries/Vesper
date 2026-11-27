//! Live captions: the box's configuration, lines from another captioner, the models
//! (see [`crate::captions`]).

use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/captions/config", get(config).put(put_config))
        .route("/captions/lines", axum::routing::post(push_line))
        .route("/captions/models", get(models))
}

/// `GET /captions/config`: this box's captions configuration.
async fn config(State(app): State<AppState>) -> ApiResult<model::CaptionsConfig> {
    ok(app.shared.captions.config())
}

/// `PUT /captions/config`: stores and applies the configuration (the engine restarts only
/// when its source, model, language or words changed).
async fn put_config(State(app): State<AppState>, body: Bytes) -> ApiResult<model::CaptionsConfig> {
    let cfg: model::CaptionsConfig = parse_body(&body)?;
    let shared = app.shared.clone();
    let saved = tokio::task::spawn_blocking(move || shared.captions.save(cfg))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
        .map_err(|e| err(ErrorCode::BadRequest, e))?;
    ok(saved)
}

/// `POST /captions/lines`: a caption line from another captioner (`204`). `409` while
/// captions are off.
async fn push_line(State(app): State<AppState>, body: Bytes) -> Result<StatusCode, Error> {
    let line: protocol::PushCaption = parse_body(&body)?;
    if line.text.chars().count() > 500 {
        return Err(err(
            ErrorCode::BadRequest,
            "a caption line is at most 500 characters",
        ));
    }
    if !app.shared.captions.config().enabled {
        return Err(err(ErrorCode::Conflict, "captions are off"));
    }
    app.shared.captions.line(&line.text, line.is_final);
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /captions/models`: the speech models offered, and which are on this box.
async fn models(State(app): State<AppState>) -> ApiResult<Vec<protocol::CaptionModel>> {
    ok(app.shared.captions.models())
}
