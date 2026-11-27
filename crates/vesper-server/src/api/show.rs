//! The show state and commands.

use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/ws", get(ws::handler))
        .route("/state", get(state))
        .route("/commands", post(command))
}

async fn state(State(app): State<AppState>) -> axum::Json<protocol::StateSnapshot> {
    axum::Json((*app.show.snapshot()).clone())
}

/// A client opened a presentation (it prefetches its thumbnails): announce its missing media.
pub(super) fn announce_opened(app: &AppState, id: PresentationId) {
    let sh = app.shared.clone();
    tokio::task::spawn_blocking(move || sh.announce_presentation_media(id));
}

/// Runs a command that is not tied to a WebSocket. Returns the state revision that includes
/// its effect.
pub async fn dispatch(app: &AppState, command: Command) -> Result<u64, ApiError> {
    match command {
        Command::PrefetchThumbnails {
            presentation,
            width,
        } => {
            announce_opened(app, presentation);
            app.thumbs
                .prefetch(presentation, Thumbnails::clamp_width(width))
                .await?;
            Ok(app.show.snapshot().revision)
        }
        Command::SubscribePreview { .. } | Command::UnsubscribePreview { .. } => {
            Err(ApiError::new(
                ErrorCode::WsOnly,
                "preview subscriptions need the WebSocket",
            ))
        }
        Command::ReloadData => {
            // A reload replaces the collections from disk: through the Codex writer, never in
            // the middle of a write, and what changed goes into the change logs.
            app.shared.codex_writer.reload(&app.codex, &app.show).await
        }
        other => app.show.apply(other).await,
    }
}

async fn command(State(app): State<AppState>, body: Bytes) -> Response {
    let cmd: Command = match parse_body(&body) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    match dispatch(&app, cmd).await {
        Ok(revision) => axum::Json(CommandResponse {
            ok: true,
            revision: Some(revision),
            error: None,
        })
        .into_response(),
        Err(e) => {
            let status = StatusCode::from_u16(e.code.http_status())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (
                status,
                axum::Json(CommandResponse {
                    ok: false,
                    revision: None,
                    error: Some(e),
                }),
            )
                .into_response()
        }
    }
}
