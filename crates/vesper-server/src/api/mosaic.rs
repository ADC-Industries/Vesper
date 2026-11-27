//! `/mosaic/…`: the site's display boxes, their screens and the channels ([`crate::mosaic`],
//! `docs/MOSAIC-PLAN.md`). On a server that isn't the site's server, `GET /mosaic` answers
//! `managed: false` and changes `409`.
//!
//! | Route | |
//! |---|---|
//! | `GET  /mosaic` | `MosaicState`: boxes with their screens, channels |
//! | `PUT  /mosaic/boxes/{machine}` | `PutMosaicBox` (admins): add a machine of the site as a box, or set its address → `MosaicState` |
//! | `DELETE /mosaic/boxes/{machine}` | (admins) forget what is kept for it → `204` |
//! | `PUT  /mosaic/boxes/{machine}/screens/{n}/feed` | `MosaicFeed` (operators) → `MosaicState`; `503` kept but the box didn't take it |
//! | `PUT  /mosaic/boxes/{machine}/screens/{n}/settings` | `MosaicScreenSettings` (admins) → `MosaicState` |
//! | `POST /mosaic/boxes/{machine}/screens/{n}/identify`, `…/test-pattern` | `MosaicIdentify`, `MosaicTestPattern` (operators) → `204` |
//! | `GET  /mosaic/boxes/{machine}/screens/{n}/thumbnail?w=480` | JPEG of what it shows; `404` nothing |
//! | `POST /mosaic/identify`, `/mosaic/test-pattern` | every screen of every box (operators) → `204`; `503` names the boxes that didn't |
//! | `GET/POST /mosaic/channels` | `MosaicChannel[]`; `PutMosaicChannel` with a name (admins) → `201 MosaicChannel` |
//! | `PUT/DELETE /mosaic/channels/{id}` | `PutMosaicChannel` (a new name: admins; the source: operators) → `MosaicState`; delete (admins) `204`, `409` while screens or a wall follow it |
//! | `POST /mosaic/walls` | `PutMosaicWall` (admins) → `201 MosaicWall`; `503` made, but not every screen took it now |
//! | `PUT/DELETE /mosaic/walls/{id}` | `PutMosaicWall`, the wall whole (admins) → `MosaicState`; delete (admins) `204` |
//! | `PUT /mosaic/walls/{id}/feed` | `MosaicFeed` (operators): every tile → `MosaicState` |
//! | `POST /mosaic/walls/{id}/identify` | `MosaicIdentify` (operators): every tile's screen → `204` |
//! | `GET/PUT/DELETE /mosaic/logo` | the site's logo for the boxes' fallback: PNG; a PNG or JPEG body (admins, ≤ 8 MB) → `MosaicState`; delete (admins) → `204` |
//! | `PUT /mosaic/boxes/{machine}/service` | `PutMosaicService {mode: off/desktop/headless}` (admins): the machine's IDN display service through its Node → `MosaicState`; `400` headless on a non-Linux machine, `409` a Vesper machine, `503` its Node didn't take it |

use super::*;
use crate::site::Actor;
use idn_site_manager::users::Role;
use protocol::{
    MosaicFeed, MosaicIdentify, MosaicScreenSettings, MosaicState, MosaicTestPattern, PutMosaicBox,
    PutMosaicChannel, PutMosaicService, PutMosaicWall,
};
use serde_json::{Value, json};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/mosaic", get(state))
        .route("/mosaic/identify", post(identify_all))
        .route("/mosaic/test-pattern", post(test_pattern_all))
        .route(
            "/mosaic/boxes/{machine}",
            axum::routing::put(put_box).delete(forget_box),
        )
        .route(
            "/mosaic/boxes/{machine}/screens/{n}/feed",
            axum::routing::put(put_feed),
        )
        .route(
            "/mosaic/boxes/{machine}/screens/{n}/settings",
            axum::routing::put(put_settings),
        )
        .route(
            "/mosaic/boxes/{machine}/screens/{n}/identify",
            post(identify),
        )
        .route(
            "/mosaic/boxes/{machine}/screens/{n}/test-pattern",
            post(test_pattern),
        )
        .route(
            "/mosaic/boxes/{machine}/screens/{n}/thumbnail",
            get(thumbnail),
        )
        .route("/mosaic/channels", get(channels).post(create_channel))
        .route(
            "/mosaic/channels/{id}",
            axum::routing::put(put_channel).delete(delete_channel),
        )
        .route("/mosaic/walls", post(create_wall))
        .route(
            "/mosaic/walls/{id}",
            axum::routing::put(put_wall).delete(delete_wall),
        )
        .route("/mosaic/walls/{id}/feed", axum::routing::put(put_wall_feed))
        .route("/mosaic/walls/{id}/identify", post(identify_wall))
        .route(
            "/mosaic/logo",
            get(get_logo).put(put_logo).delete(delete_logo).layer(
                axum::extract::DefaultBodyLimit::max(crate::mosaic::LOGO_MAX + 1024),
            ),
        )
        .route(
            "/mosaic/boxes/{machine}/service",
            axum::routing::put(put_service),
        )
}

async fn get_logo(State(app): State<AppState>) -> Response {
    let sh = app.shared.clone();
    match tokio::task::spawn_blocking(move || sh.mosaic.logo_png()).await {
        Ok(Some(png)) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            png,
        )
            .into_response(),
        _ => Error(ApiError::new(ErrorCode::NotFound, "no logo")).into_response(),
    }
}

async fn put_logo(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> ApiResult<MosaicState> {
    let detail = json!({"bytes": body.len()});
    change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.logo",
        detail,
        move |sh| sh.mosaic.put_logo(&sh.site, &body),
    )
    .await?;
    ok(app.shared.mosaic.state())
}

async fn delete_logo(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
) -> Response {
    match change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.logo.delete",
        json!({}),
        move |sh| sh.mosaic.delete_logo(&sh.site),
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn put_service(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(machine): Path<String>,
    body: Bytes,
) -> ApiResult<MosaicState> {
    let b: PutMosaicService = parse_body(&body)?;
    let detail = json!({"machine": machine, "mode": b.mode});
    change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.service",
        detail,
        move |sh| {
            sh.mosaic
                .set_service(&sh.site, &own_name(sh), &machine, b.mode)
        },
    )
    .await?;
    ok(app.shared.mosaic.state())
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

/// Checks the role, runs `f` off the async threads with the site and audits the outcome.
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
    r.map_err(Error)
}

fn screen_no(s: &str) -> Result<u32, Error> {
    s.parse::<u32>()
        .ok()
        .filter(|n| *n >= 1)
        .ok_or_else(|| err(ErrorCode::BadRequest, "screens count from 1"))
}

fn own_name(sh: &Shared) -> String {
    sh.auth.identity().name.clone()
}

async fn state(State(app): State<AppState>) -> ApiResult<MosaicState> {
    ok(app.shared.mosaic.state())
}

async fn channels(State(app): State<AppState>) -> ApiResult<Vec<protocol::MosaicChannel>> {
    ok(app.shared.mosaic.state().channels)
}

async fn put_box(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(machine): Path<String>,
    body: Bytes,
) -> ApiResult<MosaicState> {
    let b: PutMosaicBox = if body.is_empty() {
        PutMosaicBox::default()
    } else {
        parse_body(&body)?
    };
    let detail = json!({"machine": machine, "address": b.address});
    change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.box",
        detail,
        move |sh| {
            sh.mosaic
                .put_box(&sh.site, &own_name(sh), &machine, b.address)
        },
    )
    .await?;
    ok(app.shared.mosaic.state())
}

async fn forget_box(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(machine): Path<String>,
) -> Response {
    let detail = json!({"machine": machine});
    match change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.box.forget",
        detail,
        move |sh| sh.mosaic.forget_box(&sh.site, &own_name(sh), &machine),
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn put_feed(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path((machine, n)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<MosaicState> {
    let n = screen_no(&n)?;
    let feed: MosaicFeed = parse_body(&body)?;
    let detail = json!({"machine": machine, "screen": n, "feed": feed});
    change(
        &app,
        &access,
        peer,
        Role::Operator,
        "mosaic.feed",
        detail,
        move |sh| sh.mosaic.set_feed(&sh.site, &machine, n, feed),
    )
    .await?;
    ok(app.shared.mosaic.state())
}

async fn put_settings(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path((machine, n)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<MosaicState> {
    let n = screen_no(&n)?;
    let settings: MosaicScreenSettings = parse_body(&body)?;
    let detail = json!({"machine": machine, "screen": n, "settings": settings});
    change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.settings",
        detail,
        move |sh| sh.mosaic.set_settings(&sh.site, &machine, n, settings),
    )
    .await?;
    ok(app.shared.mosaic.state())
}

fn identify_body(body: &Bytes) -> Result<Value, Error> {
    let b: MosaicIdentify = if body.is_empty() {
        MosaicIdentify::default()
    } else {
        parse_body(body)?
    };
    let secs = b.seconds.unwrap_or(5.0);
    if !(0.5..=600.0).contains(&secs) {
        return Err(err(ErrorCode::BadRequest, "seconds: 0.5 to 600"));
    }
    Ok(json!({"seconds": secs}))
}

fn test_pattern_body(body: &Bytes) -> Result<Value, Error> {
    let b: MosaicTestPattern = parse_body(body)?;
    let mut v = json!({"on": b.on});
    if let Some(s) = b.seconds {
        if !(1..=3600).contains(&s) {
            return Err(err(ErrorCode::BadRequest, "seconds: 1 to 3600"));
        }
        v["seconds"] = json!(s);
    }
    Ok(v)
}

async fn screen_command(
    app: &AppState,
    access: &Access,
    peer: SocketAddr,
    machine: String,
    n: Option<u32>,
    cmd: &'static str,
    body: Value,
) -> Response {
    let action = format!("mosaic.{cmd}");
    let detail = json!({"machine": machine, "screen": n, "body": body});
    let r = change(
        app,
        access,
        peer,
        Role::Operator,
        &action,
        detail,
        move |sh| {
            if machine.is_empty() {
                sh.mosaic.command_all(&sh.site, cmd, body)
            } else {
                sh.mosaic.command(&sh.site, &machine, n, cmd, body)
            }
        },
    )
    .await;
    match r {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn identify(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path((machine, n)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    match (screen_no(&n), identify_body(&body)) {
        (Ok(n), Ok(b)) => {
            screen_command(&app, &access, peer, machine, Some(n), "identify", b).await
        }
        (Err(e), _) | (_, Err(e)) => e.into_response(),
    }
}

async fn test_pattern(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path((machine, n)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    match (screen_no(&n), test_pattern_body(&body)) {
        (Ok(n), Ok(b)) => {
            screen_command(&app, &access, peer, machine, Some(n), "test_pattern", b).await
        }
        (Err(e), _) | (_, Err(e)) => e.into_response(),
    }
}

async fn identify_all(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    match identify_body(&body) {
        Ok(b) => screen_command(&app, &access, peer, String::new(), None, "identify", b).await,
        Err(e) => e.into_response(),
    }
}

async fn test_pattern_all(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    match test_pattern_body(&body) {
        Ok(b) => screen_command(&app, &access, peer, String::new(), None, "test_pattern", b).await,
        Err(e) => e.into_response(),
    }
}

async fn thumbnail(
    State(app): State<AppState>,
    Path((machine, n)): Path<(String, String)>,
    req: Request,
) -> Response {
    let n = match screen_no(&n) {
        Ok(n) => n,
        Err(e) => return e.into_response(),
    };
    let w = query_param(req.uri().query(), "w")
        .and_then(|w| w.parse::<u32>().ok())
        .unwrap_or(480);
    let sh = app.shared.clone();
    let r = tokio::task::spawn_blocking(move || sh.mosaic.thumbnail(&sh.site, &machine, n, w))
        .await
        .unwrap_or_else(|e| Err(ApiError::new(ErrorCode::Internal, e.to_string())));
    match r {
        Ok(jpeg) => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            jpeg,
        )
            .into_response(),
        Err(e) => Error(e).into_response(),
    }
}

async fn create_channel(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let b: PutMosaicChannel = match parse_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let detail = json!({"name": b.name, "source": b.source});
    let r = change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.channel.create",
        detail,
        move |sh| {
            sh.mosaic.create_channel(
                &sh.site,
                b.name.as_deref().unwrap_or(""),
                b.source.as_deref().unwrap_or(""),
            )
        },
    )
    .await;
    match r {
        Ok(c) => (StatusCode::CREATED, axum::Json(c)).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn put_channel(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<MosaicState> {
    let b: PutMosaicChannel = parse_body(&body)?;
    let least = if b.name.is_some() {
        Role::Admin
    } else {
        Role::Operator
    };
    let detail = json!({"channel": id, "name": b.name, "source": b.source});
    change(
        &app,
        &access,
        peer,
        least,
        "mosaic.channel",
        detail,
        move |sh| {
            sh.mosaic
                .put_channel(&sh.site, &id, b.name.as_deref(), b.source.as_deref())
        },
    )
    .await?;
    ok(app.shared.mosaic.state())
}

async fn delete_channel(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    let detail = json!({"channel": id});
    match change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.channel.delete",
        detail,
        move |sh| sh.mosaic.delete_channel(&sh.site, &id),
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn create_wall(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Response {
    let w: PutMosaicWall = match parse_body(&body) {
        Ok(w) => w,
        Err(e) => return e.into_response(),
    };
    let detail = json!({"wall": w});
    let r = change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.wall.create",
        detail,
        move |sh| sh.mosaic.create_wall(&sh.site, &w),
    )
    .await;
    match r {
        Ok(id) => match app
            .shared
            .mosaic
            .state()
            .walls
            .into_iter()
            .find(|w| w.id == id)
        {
            Some(w) => (StatusCode::CREATED, axum::Json(w)).into_response(),
            None => err(ErrorCode::Internal, "the wall went").into_response(),
        },
        Err(e) => e.into_response(),
    }
}

async fn put_wall(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<MosaicState> {
    let w: PutMosaicWall = parse_body(&body)?;
    let detail = json!({"id": id, "wall": w});
    change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.wall",
        detail,
        move |sh| sh.mosaic.put_wall(&sh.site, &id, &w),
    )
    .await?;
    ok(app.shared.mosaic.state())
}

async fn put_wall_feed(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<MosaicState> {
    let feed: MosaicFeed = parse_body(&body)?;
    let detail = json!({"id": id, "feed": feed});
    change(
        &app,
        &access,
        peer,
        Role::Operator,
        "mosaic.wall.feed",
        detail,
        move |sh| sh.mosaic.set_wall_feed(&sh.site, &id, feed),
    )
    .await?;
    ok(app.shared.mosaic.state())
}

async fn delete_wall(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    let detail = json!({"id": id});
    match change(
        &app,
        &access,
        peer,
        Role::Admin,
        "mosaic.wall.delete",
        detail,
        move |sh| sh.mosaic.delete_wall(&sh.site, &id),
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn identify_wall(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let b = match identify_body(&body) {
        Ok(b) => b,
        Err(e) => return e.into_response(),
    };
    let detail = json!({"id": id, "body": b});
    match change(
        &app,
        &access,
        peer,
        Role::Operator,
        "mosaic.wall.identify",
        detail,
        move |sh| sh.mosaic.identify_wall(&sh.site, &id, b),
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}
