//! Control surfaces: the macro list (`triggerMacro` runs one) and the Remote API compatibility
//! layer's setting ([`crate::remote_compat`]).

use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/macros", get(macros))
        .route("/remote-compat", get(remote_compat).put(put_remote_compat))
}

/// `GET /macros`: the active collection's macros, in their list order.
async fn macros(State(app): State<AppState>) -> ApiResult<Vec<model::Macro>> {
    ok(
        crate::remote_compat::sorted(app.shared.data().content.macros.iter(), |m| {
            (m.sort_key.clone(), m.name.to_lowercase())
        })
        .into_iter()
        .cloned()
        .collect(),
    )
}

/// `GET /remote-compat`: the setting and whether the layer listens.
async fn remote_compat(State(app): State<AppState>) -> ApiResult<protocol::RemoteCompatStatus> {
    ok(app.remote_compat.status().await)
}

/// `PUT /remote-compat`: saves the setting and applies it at once.
async fn put_remote_compat(
    State(app): State<AppState>,
    body: Bytes,
) -> ApiResult<protocol::RemoteCompatStatus> {
    let change: protocol::UpdateRemoteCompat = parse_body(&body)?;
    let root = app.shared.data().root.clone();
    match app.remote_compat.change(&app, root, change).await {
        Ok(status) => ok(status),
        Err(e) => Err(err(ErrorCode::Internal, e)),
    }
}
