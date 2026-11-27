//! Timers, messages and props.

use super::edit::client_header;
use super::screens::json_body;
use super::thumbnails::EDITOR_MAX_WIDTH;
use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/timers",
            get(timers_list).post(
                |State(app): State<AppState>, headers: HeaderMap, body: Bytes| async move {
                    create_doc(app, headers, body, DocKind::Timer).await
                },
            ),
        )
        .route(
            "/timers/{id}",
            get(timer_get)
                .put(
                    |State(app): State<AppState>,
                     Path(id): Path<String>,
                     headers: HeaderMap,
                     body: Bytes| async move {
                        update_doc(app, id, headers, body, DocKind::Timer).await
                    },
                )
                .delete(
                    |State(app): State<AppState>,
                     Path(id): Path<String>,
                     headers: HeaderMap| async move {
                        delete_doc(app, id, headers, DocKind::Timer).await
                    },
                ),
        )
        .route(
            "/messages",
            get(messages_list).post(
                |State(app): State<AppState>, headers: HeaderMap, body: Bytes| async move {
                    create_doc(app, headers, body, DocKind::Message).await
                },
            ),
        )
        .route(
            "/messages/{id}",
            get(message_get)
                .put(
                    |State(app): State<AppState>,
                     Path(id): Path<String>,
                     headers: HeaderMap,
                     body: Bytes| async move {
                        update_doc(app, id, headers, body, DocKind::Message).await
                    },
                )
                .delete(
                    |State(app): State<AppState>,
                     Path(id): Path<String>,
                     headers: HeaderMap| async move {
                        delete_doc(app, id, headers, DocKind::Message).await
                    },
                ),
        )
        .route(
            "/props",
            get(props_list).post(
                |State(app): State<AppState>, headers: HeaderMap, body: Bytes| async move {
                    create_doc(app, headers, body, DocKind::Prop).await
                },
            ),
        )
        .route(
            "/props/{id}",
            get(prop_get)
                .put(
                    |State(app): State<AppState>,
                     Path(id): Path<String>,
                     headers: HeaderMap,
                     body: Bytes| async move {
                        update_doc(app, id, headers, body, DocKind::Prop).await
                    },
                )
                .delete(
                    |State(app): State<AppState>,
                     Path(id): Path<String>,
                     headers: HeaderMap| async move {
                        delete_doc(app, id, headers, DocKind::Prop).await
                    },
                ),
        )
        .route("/props/{id}/document", get(prop_document))
        .route("/props/{id}/edits", post(edit_prop))
        .route("/props/{id}/thumbnail", get(prop_thumbnail))
        .route("/props/{id}/slides/{slide}/render", get(prop_render))
}

fn doc_response(r: DocResult, created: bool) -> Response {
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    match r {
        DocResult::Timer(e) => with_etag(status, &e.revision.clone(), e),
        DocResult::Message(e) => with_etag(status, &e.revision.clone(), e),
        DocResult::Prop(e) => with_etag(status, &e.revision.clone(), e),
        DocResult::StageLayout(e) => with_etag(status, &e.revision.clone(), e),
        DocResult::PropEdit(r) => with_etag(StatusCode::OK, &r.revision.clone(), r),
        DocResult::Deleted => StatusCode::NO_CONTENT.into_response(),
    }
}

pub(super) async fn run_docs(
    app: &AppState,
    op: DocOp,
    created: bool,
    client: Option<String>,
) -> Response {
    match crate::codex::server::docs(&app.shared, &app.show, op, client, change_by(&app.shared))
        .await
    {
        Ok(r) => doc_response(r, created),
        Err(e) => e.into_response(),
    }
}

pub(super) async fn create_doc(
    app: AppState,
    headers: HeaderMap,
    body: Bytes,
    kind: DocKind,
) -> Response {
    match json_body(&body) {
        Ok(v) => run_docs(&app, DocOp::Create(kind, v), true, client_header(&headers)).await,
        Err(e) => e.into_response(),
    }
}

pub(super) async fn update_doc(
    app: AppState,
    id: String,
    headers: HeaderMap,
    body: Bytes,
    kind: DocKind,
) -> Response {
    let r = (|| Ok::<_, Error>((parse_id::<uuid::Uuid>(&id, "id")?, json_body(&body)?)))();
    match r {
        Ok((id, body)) => {
            let op = DocOp::Update {
                kind,
                id,
                body,
                if_match: if_match(&headers),
            };
            run_docs(&app, op, false, client_header(&headers)).await
        }
        Err(e) => e.into_response(),
    }
}

pub(super) async fn delete_doc(
    app: AppState,
    id: String,
    headers: HeaderMap,
    kind: DocKind,
) -> Response {
    match parse_id::<uuid::Uuid>(&id, "id") {
        Ok(id) => {
            let op = DocOp::Delete {
                kind,
                id,
                if_match: if_match(&headers),
            };
            run_docs(&app, op, false, client_header(&headers)).await
        }
        Err(e) => e.into_response(),
    }
}

async fn timers_list(State(app): State<AppState>) -> ApiResult<Vec<protocol::TimerEntry>> {
    ok(app
        .shared
        .data()
        .content
        .timers
        .iter()
        .map(crate::showdocs::timer_entry)
        .collect())
}

async fn messages_list(State(app): State<AppState>) -> ApiResult<Vec<protocol::MessageEntry>> {
    ok(app
        .shared
        .data()
        .content
        .messages
        .iter()
        .map(crate::showdocs::message_entry)
        .collect())
}

async fn props_list(State(app): State<AppState>) -> ApiResult<Vec<protocol::PropEntry>> {
    ok(app
        .shared
        .data()
        .content
        .props
        .iter()
        .map(crate::showdocs::prop_entry)
        .collect())
}

async fn timer_get(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, Error> {
    let id: uuid::Uuid = parse_id(&id, "timer id")?;
    let t = crate::showdocs::find(&app.shared.data().content.timers, id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("timer {id} not found")))?;
    let e = crate::showdocs::timer_entry(&t);
    Ok(with_etag(StatusCode::OK, &e.revision.clone(), e))
}

async fn message_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let id: uuid::Uuid = parse_id(&id, "message id")?;
    let m = crate::showdocs::find(&app.shared.data().content.messages, id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("message {id} not found")))?;
    let e = crate::showdocs::message_entry(&m);
    Ok(with_etag(StatusCode::OK, &e.revision.clone(), e))
}

fn find_prop(app: &AppState, id: &str) -> Result<model::Prop, Error> {
    let id: uuid::Uuid = parse_id(id, "prop id")?;
    crate::showdocs::find(&app.shared.data().content.props, id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("prop {id} not found")))
}

async fn prop_get(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, Error> {
    let e = crate::showdocs::prop_entry(&find_prop(&app, &id)?);
    Ok(with_etag(StatusCode::OK, &e.revision.clone(), e))
}

/// `GET /props/{id}/document`: the prop as a one-slide presentation (the Edit view's document).
async fn prop_document(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let p = find_prop(&app, &id)?;
    let rev = crate::config::revision(&p);
    Ok(with_etag(
        StatusCode::OK,
        &rev,
        crate::showdocs::prop_document(&p),
    ))
}

async fn edit_prop(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = (|| {
        Ok::<_, Error>((
            parse_id::<model::PropId>(&id, "prop id")?,
            parse_body::<protocol::PresentationEdit>(&body)?,
        ))
    })();
    match r {
        Ok((id, edit)) => {
            let client = edit.client_id.clone().or(client_header(&headers));
            run_docs(&app, DocOp::EditProp(id, edit), false, client).await
        }
        Err(e) => e.into_response(),
    }
}

/// `GET /props/{id}/thumbnail?w=160`: the prop alone as a PNG with alpha (`ETag` = revision).
async fn prop_thumbnail(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Request,
) -> Result<Response, Error> {
    let p = find_prop(&app, &id)?;
    let rev = crate::config::revision(&p);
    slide_thumbnail(
        &app,
        p.slide,
        p.size,
        &rev,
        &headers,
        req.uri().query().map(str::to_owned),
    )
    .await
}

/// A one-slide document's picture as a PNG with alpha, cached by `rev` (`ETag`).
pub(super) async fn slide_thumbnail(
    app: &AppState,
    slide: model::Slide,
    size: model::Size,
    rev: &str,
    headers: &HeaderMap,
    query: Option<String>,
) -> Result<Response, Error> {
    let width = query_param(query.as_deref(), "w")
        .and_then(|w| w.parse::<u32>().ok())
        .unwrap_or(320)
        .clamp(16, 1920);
    let etag = format!("\"{rev}-{width}\"");
    let cache = [
        (
            header::ETAG,
            HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("\"\"")),
        ),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
    ];
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag))
    {
        return Ok((StatusCode::NOT_MODIFIED, cache).into_response());
    }
    let png = app.thumbs.slide_png(slide, size, width, &[]).await?;
    Ok((
        cache,
        [(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))],
        Body::from(png),
    )
        .into_response())
}

/// `GET /props/{id}/slides/{slide}/render?w=960&hide=…`: the Edit view's picture of a prop.
async fn prop_render(
    State(app): State<AppState>,
    Path((id, slide)): Path<(String, String)>,
    req: Request,
) -> Result<Response, Error> {
    let p = find_prop(&app, &id)?;
    let sid: SlideId = parse_id(&slide, "slide id")?;
    if p.slide.id != sid {
        return Err(err(ErrorCode::NotFound, "slide not in prop"));
    }
    one_slide_render(&app, p.slide, p.size, req.uri().query().map(str::to_owned)).await
}

/// The Edit view's picture of a one-slide document (PNG, never cached).
pub(super) async fn one_slide_render(
    app: &AppState,
    slide: model::Slide,
    size: model::Size,
    query: Option<String>,
) -> Result<Response, Error> {
    let q = query.as_deref();
    let width = query_param(q, "w")
        .and_then(|w| w.parse::<u32>().ok())
        .unwrap_or(960)
        .clamp(16, EDITOR_MAX_WIDTH);
    let mut hide = Vec::new();
    if let Some(h) = query_param(q, "hide") {
        let h = h.replace("%2C", ",").replace("%2c", ",");
        for part in h.split(',').filter(|p| !p.is_empty()) {
            hide.push(parse_id::<model::ElementId>(part, "hide")?);
        }
    }
    let png = app.thumbs.slide_png(slide, size, width, &hide).await?;
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        Body::from(png),
    )
        .into_response())
}
