//! Recording and streaming (see [`crate::gateways`], `docs/RECORD-STREAM-PLAN.md`).

use super::*;
use crate::gateways::GwError;
use protocol::{RecordingSettings, StartGateway, StreamTarget, StreamingSettings, StreamingStatus};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/streaming", get(settings))
        .route("/streaming/status", get(status))
        .route("/streaming/recording", axum::routing::put(set_recording))
        .route("/streaming/targets", post(add_target))
        .route(
            "/streaming/targets/{id}",
            axum::routing::put(update_target).delete(delete_target),
        )
        .route("/streaming/targets/{id}/start", post(start_stream))
        .route("/streaming/targets/{id}/stop", post(stop_stream))
        .route("/recording/start", post(start_recording))
        .route("/recording/stop", post(stop_recording))
        .route("/recording/split", post(split_recording))
}

fn gw_err((code, msg): GwError) -> Error {
    err(code, msg)
}

/// Runs `f` on a blocking thread (settings files, encoders opening, files closing).
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, GwError> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
        .map_err(gw_err)
}

/// An optional JSON body (empty means none).
fn optional_body<T: serde::de::DeserializeOwned + Default>(body: &[u8]) -> Result<T, Error> {
    if body.iter().all(u8::is_ascii_whitespace) {
        Ok(T::default())
    } else {
        parse_body(body)
    }
}

async fn settings(State(app): State<AppState>) -> ApiResult<StreamingSettings> {
    let sh = app.shared.clone();
    ok(blocking(move || Ok(sh.gateways.settings())).await?)
}

async fn status(State(app): State<AppState>) -> ApiResult<StreamingStatus> {
    let sh = app.shared.clone();
    ok(blocking(move || Ok(sh.gateways.status(&sh))).await?)
}

async fn set_recording(State(app): State<AppState>, body: Bytes) -> ApiResult<StreamingSettings> {
    let r: RecordingSettings = parse_body(&body)?;
    let sh = app.shared.clone();
    ok(blocking(move || sh.gateways.set_recording(r)).await?)
}

async fn add_target(State(app): State<AppState>, body: Bytes) -> ApiResult<StreamTarget> {
    let t: StreamTarget = parse_body(&body)?;
    let sh = app.shared.clone();
    ok(blocking(move || sh.gateways.add_target(t)).await?)
}

async fn update_target(
    State(app): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<StreamTarget> {
    let t: StreamTarget = parse_body(&body)?;
    let sh = app.shared.clone();
    ok(blocking(move || sh.gateways.update_target(&id, t)).await?)
}

async fn delete_target(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, Error> {
    let sh = app.shared.clone();
    blocking(move || sh.gateways.delete_target(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn start_stream(
    State(app): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<StreamingStatus> {
    let b: StartGateway = optional_body(&body)?;
    let sh = app.shared.clone();
    ok(blocking(move || sh.gateways.start_stream(&sh, &id, b.screen)).await?)
}

async fn stop_stream(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StreamingStatus> {
    let sh = app.shared.clone();
    ok(blocking(move || {
        sh.gateways.stop_stream(&id)?;
        Ok(sh.gateways.status(&sh))
    })
    .await?)
}

async fn start_recording(State(app): State<AppState>, body: Bytes) -> ApiResult<StreamingStatus> {
    let b: StartGateway = optional_body(&body)?;
    let sh = app.shared.clone();
    ok(blocking(move || sh.gateways.start_recording(&sh, b.screen)).await?)
}

async fn stop_recording(State(app): State<AppState>) -> ApiResult<StreamingStatus> {
    let sh = app.shared.clone();
    ok(blocking(move || {
        sh.gateways.stop_recording()?;
        Ok(sh.gateways.status(&sh))
    })
    .await?)
}

async fn split_recording(State(app): State<AppState>) -> ApiResult<StreamingStatus> {
    let sh = app.shared.clone();
    ok(blocking(move || sh.gateways.split_recording(&sh)).await?)
}
