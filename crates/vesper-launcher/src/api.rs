//! The Launcher's local API, on loopback only (`127.0.0.1:5556`), for the tray app and for
//! scripts on the machine. Same-machine programs are trusted (like the Vesper server's
//! `trust_localhost`), but nothing else:
//!
//! - a request from another address, or addressed to any other host name, is refused (a web
//!   page can't reach it through DNS rebinding);
//! - a request a browser makes for a web page of another origin (an `Origin` or `Referer`
//!   that isn't this API's own `http://127.0.0.1:5556`, or `Sec-Fetch-Site` other than `none`
//!   and `same-origin`) is refused: a page open on the machine can't update or roll back. The
//!   Launcher's own page (`GET /`: the tray window and the first-run wizard) is same-origin;
//!   it can't be framed (`X-Frame-Options: DENY`, CSP `frame-ancestors 'none'`);
//! - a `POST`, `PUT` or `DELETE` carries `Content-Type: application/json` (or the package
//!   signature header), which a page of another origin can't send without a CORS preflight,
//!   and none is ever granted.
//!
//! | Route | |
//! |---|---|
//! | `GET  /` | the Launcher's window (products, the first-run wizard): one HTML page |
//! | `GET  /launcher/v1/overview` | `Overview`: the status, newer versions in the site's channel, the channel, the setup |
//! | `GET  /launcher/v1/setup` | `SetupView`: first run, roles (chosen, offered), joining the site |
//! | `PUT  /launcher/v1/setup` | `{roles?, done?}` → `SetupView` |
//! | `POST /launcher/v1/setup/join` | `{server, code?}`: join the site (its code, or an admin's approval) → `SetupView` |
//! | `GET  /launcher/v1/setup/servers` | the Vesper servers on the network (mDNS, 1.5 s): `[{name, url, version}]` |
//! | `DELETE /launcher/v1/setup/join` | take a waiting join back, or leave the site here (forget the key) → `SetupView` |
//! | `POST /launcher/v1/products/{product}/start`, `…/stop` | Launch, Stop (body `{}`) → `204` |
//! | `GET  /launcher/v1/status` | `LauncherReport`: products, versions, states, health |
//! | `POST /launcher/v1/actions` | `LauncherAction` (update: from the store, else downloaded from the site's channel, else from GitHub; rollback; install: "Add app") → `UpdateOutcome` |
//! | `GET  /launcher/v1/catalogue` | `Catalogue`: the products this machine can add, from the site's channel and GitHub |
//! | `POST /launcher/v1/packages?file=NAME` | a package as the body, its `.minisig` base64 in `X-Vesper-Signature` → stored after verification: `{product, version, platform, keyId}` |
//! | `GET  /launcher/v1/products/{product}/logs?lines=200` | its log, as text |
//! | `GET  /launcher/v1/diagnostics` | `Diagnostics`: `idn doctor` run and explained, the Launcher's status, a report to copy (keys and tokens redacted) |
//! | `GET  /launcher/v1/version` | `{version}`: the Launcher's own health check (its self-update) |
//!
//! Errors are `{"error": {"code", "message"}}`, as on the Vesper server.

use crate::launcher::{Launcher, LauncherError};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use protocol::LauncherAction;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

/// The header carrying a package's signature (the `.minisig` file, base64).
pub const SIGNATURE_HEADER: &str = "x-vesper-signature";

/// The largest package accepted (4 GiB).
pub const MAX_PACKAGE: usize = 4 << 30;

/// The local API.
pub fn router(l: Arc<Launcher>) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/launcher/v1/overview", get(overview))
        .route("/launcher/v1/setup", get(setup).put(set_setup))
        .route("/launcher/v1/setup/join", post(join).delete(leave))
        .route("/launcher/v1/setup/servers", get(servers))
        .route("/launcher/v1/products/{product}/start", post(start))
        .route("/launcher/v1/products/{product}/stop", post(stop))
        .route("/launcher/v1/status", get(status))
        .route("/launcher/v1/actions", post(action))
        .route("/launcher/v1/catalogue", get(catalogue))
        .route(
            "/launcher/v1/packages",
            post(upload).layer(axum::extract::DefaultBodyLimit::max(MAX_PACKAGE)),
        )
        .route("/launcher/v1/products/{product}/logs", get(logs))
        .route("/launcher/v1/diagnostics", get(diagnostics))
        .route(
            "/launcher/v1/version",
            get(|| async { axum::Json(json!({"version": crate::VERSION})) }),
        )
        .fallback(|| async { error(StatusCode::NOT_FOUND, "notFound", "no such route") })
        .layer(middleware::from_fn(loopback_only))
        .with_state(l)
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(json!({"error": {"code": code, "message": message}})),
    )
        .into_response()
}

fn launcher_error(e: LauncherError) -> Response {
    let (s, c) = match &e {
        LauncherError::UnknownProduct(_) => (StatusCode::NOT_FOUND, "notFound"),
        LauncherError::Invalid(_) => (StatusCode::CONFLICT, "conflict"),
        LauncherError::Signature(_) => (StatusCode::UNPROCESSABLE_ENTITY, "badSignature"),
        LauncherError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    error(s, c, &e.to_string())
}

/// Only this machine's programs: from a loopback address (when the server records it, see
/// `into_make_service_with_connect_info`), addressed to a loopback name, not made by a
/// browser for a web page of another origin, and a change only with a JSON (or signed
/// package) body.
async fn loopback_only(req: Request, next: Next) -> Response {
    let forbidden = |message| error(StatusCode::FORBIDDEN, "forbidden", message);
    if let Some(ConnectInfo(peer)) = req.extensions().get::<ConnectInfo<SocketAddr>>()
        && !peer.ip().is_loopback()
        && !peer.ip().to_canonical().is_loopback()
    {
        return forbidden("the Launcher's API answers only on this machine (localhost)");
    }
    let h = req.headers();
    let host = h
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let name = match host.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    };
    if !matches!(name, "127.0.0.1" | "localhost" | "[::1]") {
        return forbidden("the Launcher's API answers only on this machine (localhost)");
    }
    // This API's own page: the same scheme, host and port as the request is addressed to.
    let own = format!("http://{host}");
    let text = |name| {
        h.get(name)
            .map(|v: &axum::http::HeaderValue| v.to_str().unwrap_or(""))
    };
    let fetch_site = text("sec-fetch-site").map(str::to_ascii_lowercase);
    let other_origin = text("origin").is_some_and(|o| o != own);
    let other_referer = text("referer").is_some_and(|r| {
        r.strip_prefix(own.as_str())
            .is_none_or(|rest| !(rest.is_empty() || rest.starts_with('/')))
    });
    if other_origin
        || other_referer
        || fetch_site.is_some_and(|s| s != "none" && s != "same-origin")
    {
        return forbidden("the Launcher's API doesn't answer web pages of other sites");
    }
    let change = matches!(
        *req.method(),
        axum::http::Method::POST | axum::http::Method::PUT | axum::http::Method::DELETE
    );
    if change && !h.contains_key(SIGNATURE_HEADER) {
        let json = h
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"));
        if !json {
            return error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "badRequest",
                "send the action as JSON (Content-Type: application/json)",
            );
        }
    }
    next.run(req).await
}

/// The Launcher's window: products and the first-run wizard (`ui/index.html`).
async fn page() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::X_FRAME_OPTIONS, "DENY"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
            ),
        ],
        PAGE,
    )
        .into_response()
}

/// The page's source.
const PAGE: &str = include_str!("../ui/index.html");

async fn overview(State(l): State<Arc<Launcher>>) -> Response {
    match tokio::task::spawn_blocking(move || l.overview()).await {
        Ok(o) => axum::Json(o).into_response(),
        Err(e) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &e.to_string(),
        ),
    }
}

async fn setup(State(l): State<Arc<Launcher>>) -> Response {
    axum::Json(l.setup_view()).into_response()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetSetup {
    #[serde(default)]
    roles: Option<Vec<String>>,
    #[serde(default)]
    done: Option<bool>,
}

async fn set_setup(State(l): State<Arc<Launcher>>, body: Bytes) -> Response {
    let s: SetSetup = match serde_json::from_slice(&body) {
        Ok(s) => s,
        Err(e) => return error(StatusCode::BAD_REQUEST, "badRequest", &e.to_string()),
    };
    let roles = s.roles.is_some();
    match l.set_setup(s.roles, s.done) {
        Ok(()) => {
            if roles {
                let l2 = l.clone();
                tokio::spawn(async move { l2.act_on_roles().await });
            }
            axum::Json(l.setup_view()).into_response()
        }
        Err(e) => launcher_error(e),
    }
}

async fn join(State(l): State<Arc<Launcher>>, body: Bytes) -> Response {
    let j: crate::setup::JoinRequest = match serde_json::from_slice(&body) {
        Ok(j) => j,
        Err(e) => return error(StatusCode::BAD_REQUEST, "badRequest", &e.to_string()),
    };
    let l2 = l.clone();
    match detached(async move { l2.join(&j.server, j.code).await }).await {
        Ok(_) => axum::Json(l.setup_view()).into_response(),
        Err(e) => launcher_error(e),
    }
}

/// The Vesper servers found on the network (mDNS), for the wizard to offer.
async fn servers() -> Response {
    let found = tokio::task::spawn_blocking(|| {
        protocol::discover::discover(std::time::Duration::from_millis(1500))
    })
    .await;
    match found {
        Ok(Ok(v)) => axum::Json(
            v.into_iter()
                .filter_map(|s| {
                    let url = s.base_url()?;
                    Some(json!({"name": s.name, "url": url, "version": s.version}))
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Ok(Err(e)) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            &format!("can't look for servers on the network: {e}"),
        ),
        Err(e) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &e.to_string(),
        ),
    }
}

async fn leave(State(l): State<Arc<Launcher>>) -> Response {
    let l2 = l.clone();
    match tokio::task::spawn_blocking(move || l2.leave()).await {
        Ok(Ok(())) => axum::Json(l.setup_view()).into_response(),
        Ok(Err(e)) => launcher_error(e),
        Err(e) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &e.to_string(),
        ),
    }
}

/// Runs `work` in a task of its own and waits for it: when the page that asked goes away (a
/// reload, a closed window), the request is dropped but an update, a rollback, a start or a
/// stop still runs to its end and records its outcome.
async fn detached<T: Send + 'static>(
    work: impl std::future::Future<Output = Result<T, LauncherError>> + Send + 'static,
) -> Result<T, LauncherError> {
    tokio::spawn(work)
        .await
        .map_err(|e| LauncherError::Io(e.to_string()))?
}

async fn start(State(l): State<Arc<Launcher>>, Path(product): Path<String>) -> Response {
    match detached(async move { l.start_product(&product).await }).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => launcher_error(e),
    }
}

async fn stop(State(l): State<Arc<Launcher>>, Path(product): Path<String>) -> Response {
    match detached(async move { l.stop_product(&product).await }).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => launcher_error(e),
    }
}

async fn status(State(l): State<Arc<Launcher>>) -> Response {
    match tokio::task::spawn_blocking(move || l.report()).await {
        Ok(r) => axum::Json(r).into_response(),
        Err(e) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &e.to_string(),
        ),
    }
}

async fn action(State(l): State<Arc<Launcher>>, body: Bytes) -> Response {
    let a: LauncherAction = match serde_json::from_slice(&body) {
        Ok(a) => a,
        Err(e) => return error(StatusCode::BAD_REQUEST, "badRequest", &e.to_string()),
    };
    match detached(async move { crate::channel::run_action(&l, &a).await }).await {
        Ok(o) => axum::Json(o).into_response(),
        Err(e) => launcher_error(e),
    }
}

async fn upload(
    State(l): State<Arc<Launcher>>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(sig) = headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            base64::engine::general_purpose::STANDARD
                .decode(v.trim())
                .ok()
        })
        .and_then(|b| String::from_utf8(b).ok())
    else {
        return error(
            StatusCode::BAD_REQUEST,
            "badRequest",
            "the package's signature (its .minisig file, base64) is missing from X-Vesper-Signature",
        );
    };
    let name = q.get("file").cloned().unwrap_or_else(|| "package".into());
    let r = tokio::task::spawn_blocking(move || {
        let tmp = l
            .config()
            .state_dir
            .join(format!(".upload-{}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, &body).map_err(|e| LauncherError::Io(e.to_string()))?;
        let r = l.add_package(&tmp, &name, &sig);
        let _ = std::fs::remove_file(&tmp);
        r
    })
    .await;
    match r {
        Ok(Ok(v)) => (
            StatusCode::CREATED,
            axum::Json(json!({
                "product": v.meta.product,
                "version": v.meta.version,
                "platform": v.meta.platform,
                "keyId": v.key_id,
            })),
        )
            .into_response(),
        Ok(Err(e)) => launcher_error(e),
        Err(e) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &e.to_string(),
        ),
    }
}

async fn catalogue(State(l): State<Arc<Launcher>>) -> Response {
    match tokio::task::spawn_blocking(move || l.catalogue()).await {
        Ok(Ok(products)) => axum::Json(protocol::Catalogue { products }).into_response(),
        Ok(Err(e)) => launcher_error(e),
        Err(e) => launcher_error(LauncherError::Io(e.to_string())),
    }
}

async fn diagnostics(State(l): State<Arc<Launcher>>) -> Response {
    axum::Json(crate::diagnostics::run(&l).await).into_response()
}

async fn logs(
    State(l): State<Arc<Launcher>>,
    Path(product): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let lines = q
        .get("lines")
        .and_then(|n| n.parse().ok())
        .unwrap_or(200usize)
        .min(5000);
    match l.logs(&product, lines).await {
        Ok(t) => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], t).into_response(),
        Err(e) => launcher_error(e),
    }
}
