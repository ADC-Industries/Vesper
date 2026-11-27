//! Looks: create, read, update, duplicate, delete (`GET /looks` lists them, in `library.rs`);
//! masks likewise (`/masks`). Edits go through the screen configuration path
//! ([`crate::config`]).

use super::screens::{configure, json_body};
use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/looks", axum::routing::post(create_look))
        .route(
            "/looks/{id}",
            axum::routing::get(look)
                .put(update_look)
                .delete(delete_look),
        )
        .route("/looks/{id}/duplicate", axum::routing::post(duplicate_look))
        .route("/masks", axum::routing::get(masks_list).post(create_mask))
        .route(
            "/masks/{id}",
            axum::routing::get(mask)
                .put(update_mask)
                .delete(delete_mask),
        )
        .route("/masks/{id}/duplicate", axum::routing::post(duplicate_mask))
}

async fn masks_list(State(app): State<AppState>) -> ApiResult<Vec<protocol::MaskEntry>> {
    ok(app
        .shared
        .data()
        .masks
        .iter()
        .map(crate::config::mask_entry)
        .collect())
}

async fn mask(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, Error> {
    let id: model::MaskId = parse_id(&id, "mask id")?;
    let d = app.shared.data();
    let m = d
        .masks
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("mask {} not found", id.0)))?;
    let e = crate::config::mask_entry(m);
    Ok(with_etag(StatusCode::OK, &e.revision.clone(), e))
}

async fn create_mask(State(app): State<AppState>, body: Bytes) -> Response {
    let body = if body.is_empty() {
        Ok(serde_json::json!({}))
    } else {
        json_body(&body)
    };
    match body {
        Ok(v) => configure(&app, ConfigOp::CreateMask(v), true).await,
        Err(e) => e.into_response(),
    }
}

async fn update_mask(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = (|| {
        Ok::<_, Error>((
            parse_id::<model::MaskId>(&id, "mask id")?,
            json_body(&body)?,
        ))
    })();
    match r {
        Ok((id, body)) => {
            let op = ConfigOp::UpdateMask {
                id,
                body,
                if_match: if_match(&headers),
            };
            configure(&app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}

async fn duplicate_mask(
    State(app): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let r = (|| {
        let id = parse_id::<model::MaskId>(&id, "mask id")?;
        let req: protocol::DuplicateLook = if body.is_empty() {
            Default::default()
        } else {
            parse_body(&body)?
        };
        Ok::<_, Error>((id, req))
    })();
    match r {
        Ok((id, req)) => {
            let op = ConfigOp::DuplicateMask { id, name: req.name };
            configure(&app, op, true).await
        }
        Err(e) => e.into_response(),
    }
}

async fn delete_mask(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::extract::RawQuery(q): axum::extract::RawQuery,
) -> Response {
    let detach = q.as_deref().is_some_and(|q| {
        q.split('&')
            .any(|kv| kv == "detach=true" || kv == "detach=1")
    });
    match parse_id::<model::MaskId>(&id, "mask id") {
        Ok(id) => {
            let op = ConfigOp::DeleteMask {
                id,
                if_match: if_match(&headers),
                detach,
            };
            configure(&app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}

async fn look(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, Error> {
    let id: model::LookId = parse_id(&id, "look id")?;
    let d = app.shared.data();
    let l = d
        .looks
        .iter()
        .find(|l| l.id == id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("look {} not found", id.0)))?;
    let e = crate::config::look_entry(l);
    Ok(with_etag(StatusCode::OK, &e.revision.clone(), e))
}

async fn create_look(State(app): State<AppState>, body: Bytes) -> Response {
    let body = if body.is_empty() {
        Ok(serde_json::json!({}))
    } else {
        json_body(&body)
    };
    match body {
        Ok(v) => configure(&app, ConfigOp::CreateLook(v), true).await,
        Err(e) => e.into_response(),
    }
}

async fn update_look(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = (|| {
        Ok::<_, Error>((
            parse_id::<model::LookId>(&id, "look id")?,
            json_body(&body)?,
        ))
    })();
    match r {
        Ok((id, body)) => {
            let op = ConfigOp::UpdateLook {
                id,
                body,
                if_match: if_match(&headers),
            };
            configure(&app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}

async fn duplicate_look(
    State(app): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let r = (|| {
        let id = parse_id::<model::LookId>(&id, "look id")?;
        let req: protocol::DuplicateLook = if body.is_empty() {
            Default::default()
        } else {
            parse_body(&body)?
        };
        Ok::<_, Error>((id, req))
    })();
    match r {
        Ok((id, req)) => {
            let op = ConfigOp::DuplicateLook { id, name: req.name };
            configure(&app, op, true).await
        }
        Err(e) => e.into_response(),
    }
}

async fn delete_look(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match parse_id::<model::LookId>(&id, "look id") {
        Ok(id) => {
            let op = ConfigOp::DeleteLook {
                id,
                if_match: if_match(&headers),
            };
            configure(&app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}
