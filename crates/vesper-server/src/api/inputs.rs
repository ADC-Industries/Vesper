//! Logical video inputs and this box's bindings of them (see [`crate::inputs`]):
//! `GET/POST /video-inputs`, `GET/PUT/DELETE /video-inputs/{id}`.

use super::*;
use crate::inputs::{InputOp, InputResult};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/video-inputs", get(list).post(create))
        .route("/video-inputs/{id}", get(one).put(update).delete(remove))
}

async fn run(app: &AppState, op: InputOp, created: bool) -> Response {
    match app.show.inputs(op).await {
        Ok(InputResult::Input(e)) => {
            let status = if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            with_etag(status, &e.revision.clone(), *e)
        }
        Ok(InputResult::Deleted) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn list(State(app): State<AppState>) -> ApiResult<Vec<protocol::VideoInputEntry>> {
    ok(crate::inputs::entries(&app.shared.data()))
}

async fn one(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, Error> {
    let id: model::VideoInputId = parse_id(&id, "video input id")?;
    let d = app.shared.data();
    let v = d.video_inputs.iter().find(|v| v.id == id).ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("video input {} not found", id.0),
        )
    })?;
    let e = crate::inputs::entry(v, crate::inputs::binding_of(&d, id));
    Ok(with_etag(StatusCode::OK, &e.revision.clone(), e))
}

async fn create(State(app): State<AppState>, body: Bytes) -> Response {
    let body = if body.is_empty() {
        Ok(serde_json::json!({}))
    } else {
        parse_body::<serde_json::Value>(&body)
    };
    match body {
        Ok(v) => run(&app, InputOp::Create(v), true).await,
        Err(e) => e.into_response(),
    }
}

async fn update(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = (|| {
        Ok::<_, Error>((
            parse_id::<model::VideoInputId>(&id, "video input id")?,
            parse_body::<serde_json::Value>(&body)?,
        ))
    })();
    match r {
        Ok((id, body)) => {
            let op = InputOp::Update {
                id,
                body,
                if_match: if_match(&headers),
            };
            run(&app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}

async fn remove(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match parse_id::<model::VideoInputId>(&id, "video input id") {
        Ok(id) => {
            let op = InputOp::Delete {
                id,
                if_match: if_match(&headers),
            };
            run(&app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}
