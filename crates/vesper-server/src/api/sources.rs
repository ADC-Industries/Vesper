//! Live network video sources (see [`crate::live_sources`]): the list and bin thumbnails.

use super::*;
use protocol::LiveSourceList;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/sources", get(sources))
        .route("/sources/thumbnail", get(source_thumbnail))
}

async fn sources(State(app): State<AppState>) -> ApiResult<LiveSourceList> {
    let sh = app.shared.clone();
    let sources = tokio::task::spawn_blocking(move || sh.live_sources.list())
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    ok(LiveSourceList { sources })
}

#[derive(serde::Deserialize)]
struct ThumbQuery {
    source: String,
    #[serde(default)]
    w: Option<u32>,
}

/// `GET /sources/thumbnail?source=…&w=…`: the newest picture of a source as JPEG (`204` while
/// a thumbnail receiver is still connecting).
async fn source_thumbnail(
    State(app): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<ThumbQuery>,
) -> Result<Response, Error> {
    if q.source.trim().is_empty() {
        return Err(err(ErrorCode::BadRequest, "give `source`"));
    }
    let width = q.w.unwrap_or(320).clamp(16, 640);
    let sh = app.shared.clone();
    let jpeg = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, Error> {
        let Some(f) = sh.live_sources.thumbnail(q.source.trim()) else {
            return Ok(None);
        };
        let rgb = f.to_rgb(width);
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, crate::thumbs::QUALITY)
            .encode_image(&rgb)
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
        Ok(Some(out))
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))??;
    Ok(match jpeg {
        Some(j) => (
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg")),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            ],
            Body::from(j),
        )
            .into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}
