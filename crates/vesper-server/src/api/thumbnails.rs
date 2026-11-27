//! Slide thumbnails and editor renders.

use super::show::announce_opened;
use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/presentations/{id}/slides/{slide}/thumbnail",
            get(thumbnail),
        )
        .route(
            "/presentations/{id}/slides/{slide}/render",
            get(editor_render),
        )
        .route("/presentations/{id}/thumbnails", post(prefetch))
}

async fn thumbnail(
    State(app): State<AppState>,
    Path((id, slide)): Path<(String, String)>,
    headers: HeaderMap,
    req: Request,
) -> Result<Response, Error> {
    let pid: PresentationId = parse_id(&id, "presentation id")?;
    let sid: SlideId = parse_id(&slide, "slide id")?;
    let w = query_param(req.uri().query(), "w").and_then(|w| w.parse().ok());
    let width = Thumbnails::clamp_width(w);
    let spec = app
        .thumbs
        .specs(pid, Some(sid), width)?
        .pop()
        .ok_or_else(|| err(ErrorCode::NotFound, "slide not found"))?;
    let cache = [
        (
            header::ETAG,
            HeaderValue::from_str(&spec.etag).unwrap_or(HeaderValue::from_static("\"\"")),
        ),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
    ];
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|t| t.trim() == spec.etag || t.trim() == "*")
        });
    if matches {
        return Ok((StatusCode::NOT_MODIFIED, cache).into_response());
    }
    let t = app.thumbs.get(spec, true).await?;
    Ok((
        cache,
        [(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))],
        Body::from(t.jpeg),
    )
        .into_response())
}

/// Largest width of an editor render.
pub(super) const EDITOR_MAX_WIDTH: u32 = 3840;

async fn editor_render(
    State(app): State<AppState>,
    Path((id, slide)): Path<(String, String)>,
    req: Request,
) -> Result<Response, Error> {
    let pid: PresentationId = parse_id(&id, "presentation id")?;
    let sid: SlideId = parse_id(&slide, "slide id")?;
    let q = req.uri().query();
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
    let chords = query_param(q, "chords").is_some_and(|v| v == "1" || v == "true");
    let png = app
        .thumbs
        .editor_png(pid, sid, width, &hide, chords)
        .await?;
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        Body::from(png),
    )
        .into_response())
}

async fn prefetch(
    State(app): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Result<Response, Error> {
    let pid: PresentationId = parse_id(&id, "presentation id")?;
    let w = query_param(req.uri().query(), "w").and_then(|w| w.parse().ok());
    announce_opened(&app, pid);
    let r = app.thumbs.prefetch(pid, Thumbnails::clamp_width(w)).await?;
    Ok((StatusCode::ACCEPTED, axum::Json(r)).into_response())
}
