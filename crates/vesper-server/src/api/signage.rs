//! `/signage/…`: signage channels, playlists, feeds, settings and the override
//! ([`crate::signage`], `docs/SIGNAGE-PLAN.md`). On a server that isn't the site's server,
//! `GET /signage` answers `managed: false` and changes `409`. Every change is audited.
//!
//! | Route | |
//! |---|---|
//! | `GET  /signage` | `SignageState`: channels (with now/next/health), playlists, feeds, settings, weather, override |
//! | `POST /signage/channels` | `PutSignageChannel` with a name (admins) → `201 SignageChannel`; its screen and IDN output follow |
//! | `PUT/DELETE /signage/channels/{id}` | `PutSignageChannel` (admins) → `SignageChannel` (a rename moves Mosaic's channels, walls and screens showing its source to the new name); delete (admins) `204` (its screen and output go), `409` while Mosaic shows its source |
//! | `POST /signage/playlists`, `PUT/DELETE …/{id}` | `SignagePlaylist` whole (admins) → `201`/`200 SignagePlaylist`; delete `204`, `409` while a channel plays it |
//! | `POST /signage/feeds`, `PUT/DELETE …/{id}` | `SignageFeed` (admins) → `201`/`200 SignageFeed`; delete `204`, `409` while used |
//! | `POST /signage/feeds/preview` | `SignageFeed` (admins): read once → `SignageFeedPreview`; `400` it can't be read |
//! | `PUT  /signage/settings` | `SignageSettings` (admins) → `SignageSettings` |
//! | `GET  /signage/places?q=` | Open-Meteo's geocoding (admins) → `SignagePlaces`; `503` unreachable |
//! | `POST /signage/override` | `SignageOverride` (operators): shows at once → `SignageOverride` |
//! | `DELETE /signage/override` | (operators) → `204` |

use super::*;
use crate::site::Actor;
use axum::Json;
use axum::extract::Query;
use idn_site_manager::users::Role;
use protocol::{
    PutSignageChannel, SignageChannel, SignageFeed, SignageFeedPreview, SignageOverride,
    SignagePlaces, SignagePlaylist, SignageSettings, SignageState,
};
use serde_json::{Value, json};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/signage", get(state))
        .route("/signage/channels", post(create_channel))
        .route(
            "/signage/channels/{id}",
            axum::routing::put(put_channel).delete(delete_channel),
        )
        .route("/signage/playlists", post(create_playlist))
        .route(
            "/signage/playlists/{id}",
            axum::routing::put(put_playlist).delete(delete_playlist),
        )
        .route("/signage/feeds", post(create_feed))
        .route("/signage/feeds/preview", post(preview_feed))
        .route(
            "/signage/feeds/{id}",
            axum::routing::put(put_feed).delete(delete_feed),
        )
        .route("/signage/settings", axum::routing::put(put_settings))
        .route("/signage/places", get(places))
        .route(
            "/signage/override",
            post(push_override).delete(clear_override),
        )
}

/// The actor with at least `least`, or `403` (audited).
async fn allowed(
    app: &AppState,
    access: &Access,
    peer: SocketAddr,
    least: Role,
    action: &str,
    detail: &Value,
) -> Result<Actor, Error> {
    let actor = app.shared.site.actor(access, peer.ip());
    let (site, a) = (app.shared.site.clone(), actor.clone());
    let checked = tokio::task::spawn_blocking(move || site.require(&a, least))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    match checked {
        Ok(_) => Ok(actor),
        Err(e) => {
            crate::codex::api::audit(app, &actor, action, detail.clone(), Some(e.message.clone()))
                .await;
            Err(Error(e))
        }
    }
}

/// Checks the role, runs `f` off the async threads, audits the outcome and tells clients.
#[allow(clippy::too_many_arguments)]
async fn change<T: Send + 'static>(
    app: &AppState,
    access: &Access,
    peer: SocketAddr,
    least: Role,
    action: &str,
    detail: Value,
    f: impl FnOnce(&Shared) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, Error> {
    let actor = allowed(app, access, peer, least, action, &detail).await?;
    let sh = app.shared.clone();
    let r = tokio::task::spawn_blocking(move || f(&sh))
        .await
        .unwrap_or_else(|e| Err(ApiError::new(ErrorCode::Internal, e.to_string())));
    crate::codex::api::audit(
        app,
        &actor,
        action,
        detail,
        r.as_ref().err().map(|e| e.message.clone()),
    )
    .await;
    if r.is_ok() {
        // Channels' screens and outputs follow; the next tick draws the change.
        app.shared.signage.kick.notify_one();
        let sh = app.shared.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let data = sh.data();
            if sh
                .signage
                .tick(&data, chrono::Local::now(), std::time::Instant::now())
            {
                sh.wake_render();
            }
            sh.signage.announce(&sh.site);
        })
        .await;
    }
    r.map_err(Error)
}

async fn state(State(app): State<AppState>) -> ApiResult<SignageState> {
    ok(app.shared.signage.state())
}

async fn create_channel(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let put: PutSignageChannel = match parse_body(&body) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let detail = json!({"name": put.name});
    match change(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.channel.create",
        detail,
        move |sh| sh.signage.create_channel(put),
    )
    .await
    {
        Ok(c) => {
            // Its screen and output are made before the answer.
            crate::signage::ensure(&app.shared, &app.show).await;
            let c = app.shared.signage.channel(&c.id).unwrap_or(c);
            (StatusCode::CREATED, Json(c)).into_response()
        }
        Err(e) => e.into_response(),
    }
}

async fn put_channel(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<SignageChannel> {
    let put: PutSignageChannel = parse_body(&body)?;
    let detail = json!({"channel": id, "change": put});
    let c = change(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.channel",
        detail,
        move |sh| sh.signage.update_channel(&id, put),
    )
    .await?;
    crate::signage::ensure(&app.shared, &app.show).await;
    ok(app.shared.signage.channel(&c.id).unwrap_or(c))
}

async fn delete_channel(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    let detail = json!({"channel": id});
    // Refused while Mosaic shows its source: those screens would lose their picture (X3-4).
    if let Some(user) = app
        .shared
        .signage
        .channel(&id)
        .and_then(|c| mosaic_user(&app.shared.mosaic.state(), &c.source))
    {
        return Error(ApiError::new(
            ErrorCode::Conflict,
            format!("{user} shows this channel: point it at another source first"),
        ))
        .into_response();
    }
    match change(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.channel.delete",
        detail,
        move |sh| sh.signage.delete_channel(&id),
    )
    .await
    {
        Ok(()) => {
            crate::signage::ensure(&app.shared, &app.show).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => e.into_response(),
    }
}

/// What in Mosaic shows `source` (a channel, a wall or a screen), in words.
fn mosaic_user(m: &protocol::MosaicState, source: &str) -> Option<String> {
    if source.is_empty() {
        return None;
    }
    let feed = protocol::MosaicFeed::Source {
        source: source.to_string(),
    };
    m.channels
        .iter()
        .find(|c| c.source == source)
        .map(|c| format!("Mosaic's channel \u{201c}{}\u{201d}", c.name))
        .or_else(|| {
            m.walls
                .iter()
                .find(|w| w.feed == feed)
                .map(|w| format!("the wall \u{201c}{}\u{201d}", w.name))
        })
        .or_else(|| {
            m.boxes.iter().find_map(|b| {
                b.screens
                    .iter()
                    .find(|s| s.feed == feed)
                    .map(|s| format!("{} screen {}", b.name, s.number))
            })
        })
}

async fn create_playlist(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let p: SignagePlaylist = match parse_body(&body) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let detail = json!({"name": p.name});
    match change(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.playlist.create",
        detail,
        move |sh| sh.signage.put_playlist(None, p),
    )
    .await
    {
        Ok(p) => (StatusCode::CREATED, Json(p)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn put_playlist(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<SignagePlaylist> {
    let p: SignagePlaylist = parse_body(&body)?;
    let detail = json!({"playlist": id, "name": p.name, "items": p.items.len()});
    let p = change(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.playlist",
        detail,
        move |sh| sh.signage.put_playlist(Some(&id), p),
    )
    .await?;
    ok(p)
}

async fn delete_playlist(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    let detail = json!({"playlist": id});
    match change(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.playlist.delete",
        detail,
        move |sh| sh.signage.delete_playlist(&id),
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn create_feed(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let f: SignageFeed = match parse_body(&body) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let detail = json!({"name": f.name, "url": f.url});
    match change(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.feed.create",
        detail,
        move |sh| sh.signage.put_feed(None, f),
    )
    .await
    {
        Ok(f) => (StatusCode::CREATED, Json(f)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn put_feed(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<SignageFeed> {
    let f: SignageFeed = parse_body(&body)?;
    let detail = json!({"feed": id, "name": f.name, "url": f.url});
    let f = change(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.feed",
        detail,
        move |sh| sh.signage.put_feed(Some(&id), f),
    )
    .await?;
    ok(f)
}

async fn delete_feed(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    let detail = json!({"feed": id});
    match change(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.feed.delete",
        detail,
        move |sh| sh.signage.delete_feed(&id),
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn preview_feed(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> ApiResult<SignageFeedPreview> {
    let f: SignageFeed = parse_body(&body)?;
    let detail = json!({"url": f.url});
    allowed(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.feed.preview",
        &detail,
    )
    .await?;
    app.shared.signage.require_managed().map_err(Error)?;
    let sh = app.shared.clone();
    let r = tokio::task::spawn_blocking(move || sh.signage.preview(&f))
        .await
        .unwrap_or_else(|e| Err(ApiError::new(ErrorCode::Internal, e.to_string())));
    ok(r.map_err(Error)?)
}

async fn put_settings(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> ApiResult<SignageSettings> {
    let st: SignageSettings = parse_body(&body)?;
    let detail = json!({"settings": st});
    let st = change(
        &app,
        &access,
        peer,
        Role::Admin,
        "signage.settings",
        detail,
        move |sh| sh.signage.put_settings(st),
    )
    .await?;
    ok(st)
}

#[derive(serde::Deserialize)]
struct PlacesQuery {
    #[serde(default)]
    q: String,
}

async fn places(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Query(q): Query<PlacesQuery>,
) -> ApiResult<SignagePlaces> {
    let detail = json!({"q": q.q});
    allowed(&app, &access, peer, Role::Admin, "signage.places", &detail).await?;
    let sh = app.shared.clone();
    let r = tokio::task::spawn_blocking(move || sh.signage.places(&q.q))
        .await
        .unwrap_or_else(|e| Err(ApiError::new(ErrorCode::Internal, e.to_string())));
    ok(SignagePlaces {
        places: r.map_err(Error)?,
    })
}

async fn push_override(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> ApiResult<SignageOverride> {
    let o: SignageOverride = parse_body(&body)?;
    let detail = json!({"text": o.text, "channels": o.channels, "seconds": o.seconds});
    let o = change(
        &app,
        &access,
        peer,
        Role::Operator,
        "signage.override",
        detail,
        move |sh| sh.signage.push_override(o),
    )
    .await?;
    ok(o)
}

async fn clear_override(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
) -> Response {
    match change(
        &app,
        &access,
        peer,
        Role::Operator,
        "signage.override.clear",
        json!({}),
        move |sh| sh.signage.clear_override(),
    )
    .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}
