//! Media preparation's routes (`docs/MEDIA-PREP-PLAN.md`, [`crate::codex::prep`]):
//!
//! | Route | Does |
//! |---|---|
//! | `GET /codex/media-prep` | `MediaPrepStatus`: settings, jobs, the converter and encoder, live |
//! | `PUT /codex/media-prep` | `MediaPrepSettings` (admin) → the saved settings |
//! | `POST /codex/collections/{id}/media/{asset}/prepare` | `PrepareMedia {again}` → `202` (queued) |
//! | `PUT /codex/collections/{id}/media/{asset}/trim` | `SetMediaTrim {trimIn, trimOut}` → the asset |
//! | `POST /codex/collections/{id}/media/{asset}/original` | plays the original again → the asset |

use super::*;
use model::{MediaAsset, MediaAssetId, MediaPrep};
use protocol::{ApiError, MediaPrepSettings, PrepareMedia, SetMediaTrim};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/codex/media-prep", get(status).put(set_settings))
        .route(
            "/codex/collections/{id}/media/{asset}/prepare",
            post(prepare),
        )
        .route("/codex/collections/{id}/media/{asset}/trim", put(trim))
        .route(
            "/codex/collections/{id}/media/{asset}/original",
            post(original),
        )
}

async fn status(State(app): State<AppState>) -> Response {
    let (prep, sh) = (app.media_jobs.prep().clone(), app.codex.clone());
    match tokio::task::spawn_blocking(move || prep.status(&*sh)).await {
        Ok(s) => axum::Json(s).into_response(),
        Err(e) => err(ErrorCode::Internal, e.to_string()).into_response(),
    }
}

async fn set_settings(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let body: MediaPrepSettings = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let action = "codex.mediaPrepSettings";
    let detail = serde_json::to_value(&body).unwrap_or_default();
    if let Err(e) = admin(&app, &access, peer, action, &detail).await {
        return e.into_response();
    }
    let (prep, sh) = (app.media_jobs.prep().clone(), app.codex.clone());
    match tokio::task::spawn_blocking(move || prep.set_settings(&*sh, body)).await {
        Ok(Ok(s)) => axum::Json(s).into_response(),
        Ok(Err(e)) => err(ErrorCode::Internal, format!("saving the settings: {e}")).into_response(),
        Err(e) => err(ErrorCode::Internal, e.to_string()).into_response(),
    }
}

fn ids(id: &str, asset: &str) -> Result<(CollectionId, MediaAssetId), Error> {
    let collection = parse_collection(id)?;
    let asset = asset.parse::<uuid::Uuid>().map(Into::into).map_err(|_| {
        err(
            ErrorCode::BadRequest,
            format!("media id {asset:?} is not a UUID"),
        )
    })?;
    Ok((collection, asset))
}

fn the_asset(app: &AppState, c: CollectionId, a: MediaAssetId) -> Result<MediaAsset, Error> {
    content_of(app, c)?.assets.get(&a).cloned().ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("media asset {} not found", a.0),
        )
    })
}

async fn prepare(
    State(app): State<AppState>,
    Path((id, asset)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let r = async {
        let (collection, asset) = ids(&id, &asset)?;
        let body: PrepareMedia = if body.is_empty() {
            PrepareMedia::default()
        } else {
            parse_body(&body)?
        };
        let a = the_asset(&app, collection, asset)?;
        if !matches!(a.kind, model::MediaKind::Video | model::MediaKind::Audio) {
            return Err(Error(ApiError::invalid(
                "asset",
                "only videos and sounds are prepared",
            )));
        }
        app.media_jobs.prep().queue(collection, &a, body.again);
        Ok::<_, Error>(())
    }
    .await;
    match r {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(e) => e.into_response(),
    }
}

/// Saves `change` into the asset through the Codex writer and answers the asset.
async fn change_asset(
    app: &AppState,
    headers: &HeaderMap,
    collection: CollectionId,
    asset: MediaAssetId,
    change: impl FnOnce(&MediaAsset) -> Result<MediaAsset, ApiError>,
) -> Result<protocol::RelinkedAsset, Error> {
    let by = crate::api::change_by(&app.shared);
    app.shared
        .codex_writer
        .media(
            &app.codex,
            &app.show,
            collection,
            |c| {
                let cur = c.assets.get(&asset).ok_or_else(|| {
                    crate::config::ConfigError::from(ApiError::new(
                        ErrorCode::NotFound,
                        format!("media asset {} not found", asset.0),
                    ))
                })?;
                let mut plan = super::writer::MediaPlan::default();
                plan.assets
                    .push(change(cur).map_err(crate::config::ConfigError::from)?);
                Ok(plan)
            },
            client_header(headers),
            by,
        )
        .await
        .map_err(|e| Error(e.error))?;
    let a = the_asset(app, collection, asset)?;
    Ok(protocol::RelinkedAsset {
        revision: crate::config::revision(&a),
        asset: a,
    })
}

async fn trim(
    State(app): State<AppState>,
    Path((id, asset)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = async {
        let (collection, asset) = ids(&id, &asset)?;
        let t: SetMediaTrim = parse_body(&body)?;
        let r = change_asset(&app, &headers, collection, asset, |cur| {
            let duration = cur.probe.as_ref().and_then(|p| p.duration);
            let check = |v: Option<f64>, field: &str| -> Result<Option<f64>, ApiError> {
                match v {
                    None => Ok(None),
                    Some(v) if !v.is_finite() || v < 0.0 => {
                        Err(ApiError::invalid(field, "a time in seconds, 0 or more"))
                    }
                    Some(v) if duration.is_some_and(|d| v > d + 0.05) => Err(ApiError::invalid(
                        field,
                        format!(
                            "after the end of the media ({:.2} s)",
                            duration.unwrap_or(0.0)
                        ),
                    )),
                    Some(v) => Ok(Some((v * 1000.0).round() / 1000.0)),
                }
            };
            let (i, o) = (check(t.trim_in, "trimIn")?, check(t.trim_out, "trimOut")?);
            if let (Some(i), Some(o)) = (i, o)
                && o <= i
            {
                return Err(ApiError::invalid("trimOut", "after the in point"));
            }
            let mut a = cur.clone();
            let p = a.prep.get_or_insert_with(MediaPrep::default);
            p.trim_in = i.filter(|v| *v > 0.0);
            p.trim_out = o;
            if p.is_empty() {
                a.prep = None;
            }
            Ok(a)
        })
        .await?;
        Ok::<_, Error>(r)
    }
    .await;
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn original(
    State(app): State<AppState>,
    Path((id, asset)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let r = async {
        let (collection, asset) = ids(&id, &asset)?;
        let r = change_asset(&app, &headers, collection, asset, |cur| {
            let mut a = cur.clone();
            let p = a.prep.get_or_insert_with(MediaPrep::default);
            let o = p.original.take().ok_or_else(|| {
                ApiError::new(ErrorCode::Conflict, "this media plays its original already")
            })?;
            // Kept as the original is from now on: not converted again by itself.
            p.analysed = crate::codex::prep::file_key(&o.location);
            p.notes = vec!["Plays the original (chosen by hand)".into()];
            a.location = o.location;
            if o.probe.is_some() {
                a.probe = o.probe;
            }
            Ok(a)
        })
        .await?;
        app.media_jobs.probe(collection, asset);
        Ok::<_, Error>(r)
    }
    .await;
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}
