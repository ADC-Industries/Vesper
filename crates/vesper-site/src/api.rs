//! The site's REST routes (`/api/v1/site/...`). Every route needs a paired client or the
//! server machine, like the rest of the API; changes, the audit and logs also need a site role,
//! which the site's access level decides ([`super::Actor`]).
//!
//! | Route | |
//! |---|---|
//! | `GET /site` | `SiteStatus` (running, join mode, ports, who you are) |
//! | `GET/PUT /site/settings` | `SiteSettings` `{enabled, joinMode, access, pin?}` (admins; `If-Match`) |
//! | `GET/POST/DELETE /site/session` | `SiteSession`; sign in `{user, password}` (`users`); sign out and lock |
//! | `POST/DELETE /site/unlock` | unlock with the admin PIN `{pin}` (`pin`); lock again |
//! | `GET /site/join-requests` | `JoinRequestInfo[]` |
//! | `POST /site/join-requests/{id}/approve`, `…/deny` | `204` |
//! | `POST/DELETE /site/pairing` | open `{seconds?}` / close the approval window |
//! | `GET/POST /site/join-code` | the site code (`404` none) / a new one (earlier ones taken back) |
//! | `GET /site/join-codes`, `DELETE /site/join-codes/{id}` | codes waiting; take one back |
//! | `GET /site/devices`, `GET/DELETE /site/devices/{fp}` | `SiteDevice[]`; one; Remove (revoke) |
//! | `POST /site/devices/{fp}/allow` | let a removed or denied machine ask again |
//! | `POST /site/devices/{fp}/identify`, `…/restart`, `…/logs` | `{seconds?}`, `{service}`, fetch logs |
//! | `POST /site/devices/{fp}/relay` | `SetDeviceRelay {on}`: mark a pairing relay, or not (admins) |
//! | `GET /site/devices/{fp}/logs`, `…/logs/{snapshot}/{file}` | `LogSnapshot[]`; a file (text) |
//! | `GET /site/users`, `PUT/DELETE /site/users/{name}` | `SiteUser[]`; add / change (`If-Match`); remove |
//! | `PUT /site/password` | `{password}`: the site password (admins) |
//! | `POST /site/tokens`, `DELETE /site/tokens/{user}` | `IssuedToken`; withdraw a user's or a client's (`vesper:<client id>`) tokens |
//! | `GET /site/pairings` | `PairingRequest[]`: clients waiting for approval (here, and relayed) |
//! | `POST /site/pairings/{id}/approve`, `…/deny` | `204` (admins; deny takes `{reason?}`) |
//! | `GET /site/api-keys` | `SiteApiKey[]`: the site's named keys |
//! | `DELETE /site/api-keys/{id}` | revoke a named site key (a paired client's): it stops on every machine (admins) |
//! | `GET /site/membership` | `SiteMembership`: this server in another server's site |
//! | `POST /site/join` | `JoinSite {server?, code?}` → `SiteMembership` (admins; `409` while this site has other machines) |
//! | `POST /site/leave` | `204`: leave it; this server runs its own site again (admins) |
//! | `GET /site/audit?n=` | `SiteAuditEntry[]`, newest first |
//! | `GET /site/events?n=` | `SiteEvent[]`, newest first |

use super::{Actor, Site};
use crate::auth::Access;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{FromRef, Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use protocol::{ApiError, ErrorCode};
use std::net::SocketAddr;
use std::sync::Arc;
use vesper_core::change::ConfigError;
use vesper_core::http::{ApiResult, Error, err, if_match, ok, parse_body, with_etag};

/// The state of the site's handlers: the site.
type AppState = Arc<Site>;

/// The routes, to merge into `/api/v1`: over any router state that gives the site.
pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    AppState: FromRef<S>,
{
    Router::new()
        .route("/site", get(status))
        .route("/site/settings", get(settings).put(put_settings))
        .route("/site/session", get(session).post(sign_in).delete(sign_out))
        .route("/site/unlock", post(unlock).delete(lock_again))
        .route("/site/join-requests", get(join_requests))
        .route("/site/join-requests/{id}/approve", post(approve))
        .route("/site/join-requests/{id}/deny", post(deny))
        .route("/site/pairing", post(open_pairing).delete(close_pairing))
        .route("/site/join-code", get(current_code).post(new_code))
        .route("/site/join-codes", get(join_codes))
        .route("/site/join-codes/{id}", delete(cancel_code))
        .route("/site/devices", get(devices))
        .route("/site/devices/{fp}", get(device).delete(remove_device))
        .route("/site/devices/{fp}/allow", post(allow_device))
        .route("/site/devices/{fp}/identify", post(identify))
        .route("/site/devices/{fp}/restart", post(restart))
        .route("/site/devices/{fp}/relay", post(set_relay))
        .route(
            "/site/devices/{fp}/logs",
            get(log_snapshots).post(collect_logs),
        )
        .route("/site/devices/{fp}/logs/{snapshot}/{file}", get(log_file))
        .route("/site/users", get(users))
        .route(
            "/site/users/{name}",
            axum::routing::put(put_user).delete(delete_user),
        )
        .route("/site/password", axum::routing::put(set_password))
        .route("/site/tokens", post(issue_token))
        .route("/site/tokens/{user}", delete(withdraw_tokens))
        .route("/site/api-keys", get(api_keys))
        .route("/site/api-keys/{id}", delete(revoke_key))
        .route("/site/pairings", get(pairings))
        .route("/site/pairings/{id}/approve", post(approve_pairing))
        .route("/site/pairings/{id}/deny", post(deny_pairing))
        .route("/site/membership", get(membership))
        .route("/site/join", post(join))
        .route("/site/leave", post(leave))
        .route("/site/audit", get(audit))
        .route("/site/events", get(events))
}

/// Who makes the request.
fn actor(app: &AppState, access: Access, peer: SocketAddr) -> Actor {
    app.actor(&access, peer.ip())
}

/// Runs `f` on the blocking pool with the site.
async fn blocking<T: Send + 'static>(
    app: &AppState,
    f: impl FnOnce(&Site) -> T + Send + 'static,
) -> Result<T, Error> {
    let site = app.clone();
    tokio::task::spawn_blocking(move || f(&site))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))
}

fn no_content(r: Result<(), ApiError>) -> Result<StatusCode, Error> {
    r.map(|_| StatusCode::NO_CONTENT).map_err(Error)
}

/// A conflict with the current document, or a plain error.
fn conflict<T: serde::Serialize>(e: ApiError, current: Option<T>) -> Response {
    ConfigError {
        error: e,
        current: current.map(|c| serde_json::to_value(c).unwrap_or_default()),
    }
    .into_response()
}

/// An optional JSON body (empty: the default).
fn optional_body<T: serde::de::DeserializeOwned + Default>(body: &[u8]) -> Result<T, Error> {
    if body.iter().all(u8::is_ascii_whitespace) {
        Ok(T::default())
    } else {
        parse_body(body)
    }
}

fn parse_u64(s: &str, what: &str) -> Result<u64, Error> {
    s.parse().map_err(|_| {
        err(
            ErrorCode::BadRequest,
            format!("{what} {s:?} is not a number"),
        )
    })
}

fn count(query: Option<&str>) -> usize {
    vesper_core::http::query_param(query, "n")
        .and_then(|n| n.parse().ok())
        .unwrap_or(200usize)
        .clamp(1, 5000)
}

type Who = (
    State<AppState>,
    axum::Extension<Access>,
    axum::Extension<SocketAddr>,
);

// ---- status, settings, session ----------------------------------------------------------------

async fn status(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
) -> ApiResult<protocol::SiteStatus> {
    let a = actor(&app, access, peer);
    ok(blocking(&app, move |s| s.status(&a)).await?)
}

async fn settings(State(app): State<AppState>) -> Result<Response, Error> {
    let s = blocking(&app, |s| s.settings()).await?;
    let rev = s.revision.clone().unwrap_or_default();
    Ok(with_etag(StatusCode::OK, &rev, s))
}

async fn put_settings(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let want: protocol::SiteSettings = parse_body(&body)?;
    let a = actor(&app, access, peer);
    let m = if_match(&headers);
    match blocking(&app, move |s| s.put_settings(&a, want, m)).await? {
        Ok(s) => {
            let rev = s.revision.clone().unwrap_or_default();
            Ok(with_etag(StatusCode::OK, &rev, s))
        }
        Err((e, cur)) => Ok(conflict(e, cur)),
    }
}

async fn session(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
) -> ApiResult<protocol::SiteSession> {
    let a = actor(&app, access, peer);
    ok(a.session())
}

async fn sign_in(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> ApiResult<protocol::SiteSession> {
    let req: protocol::SiteSignIn = parse_body(&body)?;
    let a = actor(&app, access, peer);
    let r = blocking(&app, move |s| {
        s.sign_in(&a, peer.ip(), &req.user, &req.password)
    })
    .await?;
    ok(r?)
}

async fn sign_out(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
) -> Result<StatusCode, Error> {
    let a = actor(&app, access, peer);
    blocking(&app, move |s| s.sign_out(&a)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unlock(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    body: Bytes,
) -> ApiResult<protocol::SiteSession> {
    let req: protocol::SiteUnlock = parse_body(&body)?;
    let a = actor(&app, access, peer);
    ok(blocking(&app, move |s| s.unlock(&a, &req.pin)).await??)
}

async fn lock_again(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
) -> Result<StatusCode, Error> {
    let a = actor(&app, access, peer);
    blocking(&app, move |s| s.lock_client(&a)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- joining ----------------------------------------------------------------------------------

async fn join_requests(State(app): State<AppState>) -> ApiResult<Vec<protocol::JoinRequestInfo>> {
    ok(blocking(&app, |s| s.join_requests()).await?)
}

async fn approve(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<StatusCode, Error> {
    let id = parse_u64(&id, "join request")?;
    let req: protocol::ApproveJoin = optional_body(&body)?;
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.approve(&a, id, req.name)).await?)
}

async fn deny(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(id): Path<String>,
) -> Result<StatusCode, Error> {
    let id = parse_u64(&id, "join request")?;
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.deny(&a, id)).await?)
}

async fn open_pairing(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    body: Bytes,
) -> Result<StatusCode, Error> {
    let req: protocol::OpenPairing = optional_body(&body)?;
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.pairing(&a, Some(req.seconds))).await?)
}

async fn close_pairing(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
) -> Result<StatusCode, Error> {
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.pairing(&a, None)).await?)
}

async fn current_code(State(app): State<AppState>) -> ApiResult<protocol::JoinCodeInfo> {
    match blocking(&app, |s| s.current_code()).await? {
        Some(c) => ok(c),
        None => Err(err(
            ErrorCode::NotFound,
            "no site code is waiting: make one first",
        )),
    }
}

async fn new_code(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
) -> Result<Response, Error> {
    let a = actor(&app, access, peer);
    let c = blocking(&app, move |s| s.new_code(&a)).await??;
    Ok((StatusCode::CREATED, axum::Json(c)).into_response())
}

async fn join_codes(State(app): State<AppState>) -> ApiResult<Vec<protocol::JoinCodeInfo>> {
    ok(blocking(&app, |s| s.join_codes()).await?)
}

async fn cancel_code(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(id): Path<String>,
) -> Result<StatusCode, Error> {
    let id = parse_u64(&id, "join code")?;
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.cancel_code(&a, id)).await?)
}

// ---- devices ----------------------------------------------------------------------------------

async fn devices(State(app): State<AppState>) -> ApiResult<Vec<protocol::SiteDevice>> {
    ok(blocking(&app, |s| s.devices()).await?)
}

async fn device(
    State(app): State<AppState>,
    Path(fp): Path<String>,
) -> ApiResult<protocol::SiteDevice> {
    ok(blocking(&app, move |s| s.device(&fp)).await??)
}

async fn remove_device(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(fp): Path<String>,
) -> Result<StatusCode, Error> {
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.remove(&a, &fp)).await?)
}

async fn allow_device(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(fp): Path<String>,
) -> Result<StatusCode, Error> {
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.allow(&a, &fp)).await?)
}

async fn identify(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(fp): Path<String>,
    body: Bytes,
) -> Result<StatusCode, Error> {
    let req: protocol::IdentifyDevice = optional_body(&body)?;
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.identify(&a, &fp, req.seconds)).await?)
}

async fn restart(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(fp): Path<String>,
    body: Bytes,
) -> Result<StatusCode, Error> {
    let req: protocol::RestartDevice = parse_body(&body)?;
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.restart(&a, &fp, &req.service)).await?)
}

async fn set_relay(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(fp): Path<String>,
    body: Bytes,
) -> Result<StatusCode, Error> {
    let req: protocol::SetDeviceRelay = parse_body(&body)?;
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.set_relay(&a, &fp, req.on)).await?)
}

async fn collect_logs(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(fp): Path<String>,
) -> Result<Response, Error> {
    let a = actor(&app, access, peer);
    let snapshot = blocking(&app, move |s| s.collect_logs(&a, &fp)).await??;
    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({ "snapshot": snapshot })),
    )
        .into_response())
}

async fn log_snapshots(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(fp): Path<String>,
) -> ApiResult<Vec<protocol::LogSnapshot>> {
    let a = actor(&app, access, peer);
    ok(blocking(&app, move |s| {
        s.require_viewer(&a)?;
        s.log_snapshots(&fp)
    })
    .await??)
}

async fn log_file(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path((fp, snapshot, file)): Path<(String, String, String)>,
) -> Result<Response, Error> {
    let a = actor(&app, access, peer);
    let text = blocking(&app, move |s| {
        s.require_viewer(&a)?;
        s.log_file(&fp, &snapshot, &file)
    })
    .await??;
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        text,
    )
        .into_response())
}

// ---- users, password, tokens --------------------------------------------------------------------

async fn users(State(app): State<AppState>) -> ApiResult<Vec<protocol::SiteUser>> {
    ok(blocking(&app, |s| s.user_list()).await?)
}

async fn put_user(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Error> {
    let req: protocol::PutSiteUser = parse_body(&body)?;
    let a = actor(&app, access, peer);
    let m = if_match(&headers);
    match blocking(&app, move |s| {
        s.put_user(&a, &name, req.role, req.password, m)
    })
    .await?
    {
        Ok(u) => {
            let rev = u.revision.clone();
            Ok(with_etag(StatusCode::OK, &rev, u))
        }
        Err((e, cur)) => Ok(conflict(e, cur)),
    }
}

async fn delete_user(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, Error> {
    let a = actor(&app, access, peer);
    let m = if_match(&headers);
    match blocking(&app, move |s| s.delete_user(&a, &name, m)).await? {
        Ok(()) => Ok(StatusCode::NO_CONTENT.into_response()),
        Err((e, cur)) => Ok(conflict(e, cur)),
    }
}

async fn set_password(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    body: Bytes,
) -> Result<StatusCode, Error> {
    let req: protocol::SetSitePassword = parse_body(&body)?;
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.set_password(&a, &req.password)).await?)
}

async fn issue_token(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    body: Bytes,
) -> Result<Response, Error> {
    let req: protocol::IssueToken = parse_body(&body)?;
    let a = actor(&app, access, peer);
    let t = blocking(&app, move |s| s.issue_token(&a, &req)).await??;
    Ok((StatusCode::CREATED, axum::Json(t)).into_response())
}

async fn withdraw_tokens(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(user): Path<String>,
) -> Result<StatusCode, Error> {
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.withdraw_tokens(&a, &user)).await?)
}

async fn revoke_key(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(id): Path<String>,
) -> Result<StatusCode, Error> {
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.revoke_key(&a, &id)).await?)
}

async fn api_keys(State(app): State<AppState>) -> ApiResult<Vec<protocol::SiteApiKey>> {
    ok(blocking(&app, |s| s.api_keys()).await?)
}

async fn pairings(State(app): State<AppState>) -> ApiResult<Vec<protocol::PairingRequest>> {
    ok(blocking(&app, |s| s.pairing_requests()).await?)
}

async fn approve_pairing(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(id): Path<String>,
) -> Result<StatusCode, Error> {
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.approve_pairing(&a, &id)).await?)
}

async fn deny_pairing(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<StatusCode, Error> {
    let req: protocol::DenyPairing = optional_body(&body)?;
    let a = actor(&app, access, peer);
    no_content(
        blocking(&app, move |s| {
            s.deny_pairing(&a, &id, req.reason.as_deref())
        })
        .await?,
    )
}

// ---- membership of another site ---------------------------------------------------------------

async fn membership(State(app): State<AppState>) -> ApiResult<protocol::SiteMembership> {
    ok(blocking(&app, |s| s.membership()).await?)
}

async fn join(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    body: Bytes,
) -> ApiResult<protocol::SiteMembership> {
    let req: protocol::JoinSite = parse_body(&body)?;
    let a = actor(&app, access, peer);
    ok(blocking(&app, move |s| s.join(&a, &req)).await??)
}

async fn leave(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
) -> Result<StatusCode, Error> {
    let a = actor(&app, access, peer);
    no_content(blocking(&app, move |s| s.leave(&a)).await?)
}

// ---- audit, events ----------------------------------------------------------------------------

async fn audit(
    (State(app), axum::Extension(access), axum::Extension(peer)): Who,
    RawQuery(q): RawQuery,
) -> ApiResult<Vec<protocol::SiteAuditEntry>> {
    let n = count(q.as_deref());
    let a = actor(&app, access, peer);
    ok(blocking(&app, move |s| {
        s.require_viewer(&a)?;
        Ok::<_, ApiError>(s.audit_log(n))
    })
    .await??)
}

async fn events(
    State(app): State<AppState>,
    RawQuery(q): RawQuery,
) -> ApiResult<Vec<protocol::SiteEvent>> {
    let n = count(q.as_deref());
    ok(blocking(&app, move |s| s.recent_events(n)).await?)
}
