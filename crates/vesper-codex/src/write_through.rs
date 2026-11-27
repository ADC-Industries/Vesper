//! Write-through edits from show boxes (M10): a show box (Codex role `client`) forwards its
//! content writes to the site's Codex over the box-to-Codex link ([`super::link`]), and
//! answers the routes of Codex management and uploads with where the Codex box is.
//!
//! **What a show box does with a request** ([`route`]; the server's
//! `codex::api::write_through`, a middleware of the REST routes; every other request is
//! answered by the box itself):
//! - **Forwarded** ([`FORWARDED`]): content writes (libraries, presentations, playlists,
//!   timers, messages, props, stage layouts, themes, media bins, global groups), the item trash (its list and restoring from it)
//!   and the media relinks that need no disk search here. The box sends [`WRITE`] with the
//!   request as it came (method, path, query, the `If-Match`, `Content-Type` and `X-Client-Id`
//!   headers, the body), the client that made it (its key's id and name, or the box's own
//!   trusted machine), the box's server ID and the collection it shows. The Codex runs it
//!   through its own REST routes ([`execute`]) as that client, on that collection: the same
//!   validation, the same errors, `If-Match` against the Codex's revision (`409` with
//!   `current`), and the change log names the client and the box. The answer comes back to the
//!   client as the Codex gave it; after a write that went through the box brings its copy up
//!   to date at once ([`super::mirror::Mirror::refresh`]), so the client that edited reads its
//!   edit from this box.
//! - **Codex box** ([`CODEX_BOX`]): uploads, collection administration (collections, the
//!   collection trash, emptying the trash), the relink search under media roots (it reads
//!   the Codex box's disks for minutes), settings backups (lists, snapshots, restores, Oriel's
//!   preferences; the box's own snapshot is still offered with `POST
//!   /codex/backups/snapshots`) and export/import: `409 codexElsewhere` with `codex` (`CodexBox`: the
//!   Codex box's server ID, name, fingerprint and the addresses this box knows for it) so a
//!   client can send it there.
//! - The Codex can't be reached: `503 unavailable` ("edits need the site's server"); the show
//!   keeps playing from the box's copy. No offline queue (owner decision).
//!
//! Search ([`SEARCH`]): a show box searches its copy of its collection itself; `collection=all`
//! or another collection is forwarded (`503` while the Codex can't be reached).
//!
//! The Codex trusts the show box for who the client is: the box is a machine of the site (the
//! link refuses others) and checked the client's site key itself. A forwarded request runs
//! only the routes in [`FORWARDED`].

use crate::Shared;
use crate::auth::{Access, Client, Credential};
use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use model::CollectionId;
use protocol::{ApiError, ErrorCode, ErrorResponse};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use vesper_core::http::{Error, err};

/// `vesper.codex.write` (show box → Codex): a forwarded REST request.
pub const WRITE: &str = "vesper.codex.write";
/// `vesper.codex.search` (show box → Codex): a search beyond the box's own collection.
pub const SEARCH: &str = "vesper.codex.search";
/// Largest body forwarded (the channel's payload limit is 16 MiB; the REST routes take 2 MiB).
pub const MAX_BODY: usize = 8 << 20;
/// How long a forwarded request may take.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// How long the Codex waits for its writer before it runs a forwarded request: well before
/// the box stops waiting ([`TIMEOUT`]), so a write the box gave up on (its client told it
/// failed, and may try again) is never applied later. A long import or export holds the
/// writer for minutes.
pub const WRITER_WAIT: Duration = Duration::from_secs(20);
/// The request headers forwarded.
const HEADERS: &[&str] = &["if-match", "content-type", "x-client-id"];
/// The answer's headers passed back to the client.
const ANSWER_HEADERS: &[&str] = &["etag", "content-type"];

/// Routes a show box forwards to the Codex (method, path below `/api/v1`; `*` is one path
/// segment).
pub const FORWARDED: &[(&str, &str)] = &[
    ("POST", "/libraries"),
    ("PUT", "/libraries/*"),
    ("DELETE", "/libraries/*"),
    ("POST", "/presentations"),
    ("DELETE", "/presentations/*"),
    ("POST", "/presentations/*/edits"),
    ("POST", "/presentations/*/duplicate"),
    ("POST", "/presentations/*/move"),
    ("POST", "/playlists"),
    ("PUT", "/playlists/*"),
    ("DELETE", "/playlists/*"),
    ("POST", "/playlists/*/edits"),
    ("POST", "/playlists/*/duplicate"),
    ("POST", "/timers"),
    ("PUT", "/timers/*"),
    ("DELETE", "/timers/*"),
    ("POST", "/messages"),
    ("PUT", "/messages/*"),
    ("DELETE", "/messages/*"),
    ("POST", "/props"),
    ("PUT", "/props/*"),
    ("DELETE", "/props/*"),
    ("POST", "/props/*/edits"),
    ("POST", "/stage-layouts"),
    ("PUT", "/stage-layouts/*"),
    ("DELETE", "/stage-layouts/*"),
    ("POST", "/stage-layouts/*/edits"),
    ("POST", "/themes"),
    ("PUT", "/themes/*"),
    ("DELETE", "/themes/*"),
    ("POST", "/themes/*/edits"),
    ("POST", "/themes/*/duplicate"),
    ("POST", "/media-bins"),
    ("PUT", "/media-bins/*"),
    ("DELETE", "/media-bins/*"),
    ("POST", "/media-bins/*/edits"),
    ("POST", "/groups"),
    ("PUT", "/groups/*"),
    ("DELETE", "/groups/*"),
    ("GET", "/codex/trash"),
    ("POST", "/codex/trash/*/restore"),
    ("POST", "/codex/collections/*/media/*/relink"),
    ("POST", "/codex/collections/*/media/match"),
    ("POST", "/codex/collections/*/media/reroot"),
    ("POST", "/codex/collections/*/media/*/prepare"),
    ("PUT", "/codex/collections/*/media/*/trim"),
    ("POST", "/codex/collections/*/media/*/original"),
];

/// Routes a show box answers with where the Codex box is (`*` method: every method).
pub const CODEX_BOX: &[(&str, &str)] = &[
    ("*", "/codex/uploads"),
    ("*", "/codex/uploads/*"),
    ("*", "/codex/uploads/*/finish"),
    ("POST", "/codex/collections"),
    ("PUT", "/codex/collections/*"),
    ("DELETE", "/codex/collections/*"),
    ("POST", "/codex/collection-trash/*/restore"),
    ("DELETE", "/codex/collection-trash/*"),
    ("DELETE", "/codex/trash"),
    ("POST", "/codex/collections/*/media/search"),
    ("GET", "/codex/library"),
    // Settings backups (a show box offers its own snapshot with `POST
    // /codex/backups/snapshots`, answered here).
    ("GET", "/codex/backups"),
    ("*", "/codex/backups/oriel"),
    ("*", "/codex/backups/*/snapshots"),
    ("*", "/codex/backups/*/snapshots/*"),
    ("*", "/codex/backups/*/snapshots/*/preview"),
    ("*", "/codex/backups/*/snapshots/*/restore"),
    ("*", "/codex/backups/restores"),
    ("*", "/codex/backups/restores/*"),
    // Export and import.
    ("*", "/codex/export"),
    ("*", "/codex/export/*"),
    ("*", "/codex/export/imports/*"),
    ("*", "/codex/export/imports/*/*"),
    // Media preparation runs where the library is.
    ("*", "/codex/media-prep"),
    // PowerPoint import (the server's `codex::pptx`).
    ("*", "/codex/imports/pptx"),
    ("*", "/codex/imports/pptx/*"),
];

/// What a show box does with a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Answered by the box itself.
    Local,
    /// Forwarded to the Codex ([`FORWARDED`]).
    Forward,
    /// Answered with where the Codex box is ([`CODEX_BOX`]).
    CodexBox,
}

fn matches(pattern: &str, path: &str) -> bool {
    let (mut a, mut b) = (pattern.split('/'), path.split('/'));
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some("*"), Some(s)) => {
                if s.is_empty() || s == "." || s == ".." {
                    return false;
                }
            }
            (Some(p), Some(s)) if p == s => {}
            _ => return false,
        }
    }
}

/// What a show box does with `method` on `path` (with or without the `/api/v1` prefix).
pub fn route(method: &str, path: &str) -> Route {
    let path = path.strip_prefix(protocol::API_PREFIX).unwrap_or(path);
    let hit = |list: &[(&str, &str)]| {
        list.iter()
            .any(|(m, p)| (*m == "*" || *m == method) && matches(p, path))
    };
    if hit(FORWARDED) {
        Route::Forward
    } else if hit(CODEX_BOX) {
        Route::CodexBox
    } else {
        Route::Local
    }
}

fn v1() -> u32 {
    super::link::VERSION
}

/// How the client of a forwarded request reached the show box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientAccess {
    /// A paired client (its site key).
    Paired,
    /// The show box's own machine (trusted local requests).
    Local,
}

/// The client of a forwarded request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteClient {
    /// How it reached the box.
    pub access: ClientAccess,
    /// Its key's id (paired).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Its key's name (paired).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// [`WRITE`]'s request: a REST request as the show box got it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The collection the box shows (the content routes act on it).
    pub collection: CollectionId,
    /// The box's server ID (the change log names it).
    pub machine: String,
    /// Who made the request.
    pub client: WriteClient,
    /// `POST`, `PUT`, `DELETE`, `GET`.
    pub method: String,
    /// Below `/api/v1`.
    pub path: String,
    /// The query string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// `If-Match`, `Content-Type`, `X-Client-Id`.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// The body (JSON text).
    #[serde(default)]
    pub body: String,
}

/// [`WRITE`]'s answer: the Codex's REST answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteAnswer {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The HTTP status.
    pub status: u16,
    /// `ETag`, `Content-Type`.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// The body.
    #[serde(default)]
    pub body: String,
    /// The collection's change-log position after it.
    pub seq: u64,
}

/// [`SEARCH`]'s request: `GET /codex/search`'s query.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// `q`.
    #[serde(default)]
    pub q: Option<String>,
    /// `collection`.
    #[serde(default)]
    pub collection: Option<String>,
    /// `kinds`.
    #[serde(default)]
    pub kinds: Option<String>,
    /// `limit`.
    #[serde(default)]
    pub limit: Option<String>,
}

// ---- the show box -----------------------------------------------------------------------------

/// `409 codexElsewhere` naming the Codex box (`503` while this box doesn't know it).
pub fn elsewhere(sh: &Shared) -> Response {
    let codex = sh.mirror().codex_box(sh);
    let (status, error) = match &codex {
        Some(c) => {
            let name = c.name.clone().unwrap_or_else(|| c.server_id.clone());
            let at = if c.addresses.is_empty() {
                String::new()
            } else {
                format!(" at {}", c.addresses.join(", "))
            };
            (
                StatusCode::CONFLICT,
                ApiError::new(
                    ErrorCode::CodexElsewhere,
                    format!(
                        "this box shows a copy of the site's Codex: uploads and Codex management go to the box that holds it ({name}{at})"
                    ),
                ),
            )
        }
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiError::new(
                ErrorCode::Unavailable,
                "this box shows a copy of the site's Codex and doesn't know where it is now: uploads and Codex management go to the box that holds it",
            ),
        ),
    };
    (
        status,
        axum::Json(ErrorResponse {
            error,
            current: None,
            codex,
        }),
    )
        .into_response()
}

/// The answer when the Codex can't be reached (or refused the box).
fn unreachable(e: ApiError) -> Response {
    if e.code == ErrorCode::Unavailable {
        return Error(ApiError::new(
            ErrorCode::Unavailable,
            format!(
                "edits need the site's server (the box that holds the Codex), which this box can't reach now; the show keeps playing ({})",
                e.message
            ),
        ))
        .into_response();
    }
    Error(e).into_response()
}

/// A show box's REST write forwarded to the site's Codex, its answer passed back.
pub async fn forward(sh: &Arc<Shared>, req: Request) -> Response {
    let client = match req.extensions().get::<Access>() {
        Some(Access::Paired(c)) => WriteClient {
            access: ClientAccess::Paired,
            id: Some(c.id.clone()),
            name: Some(c.name.clone()),
        },
        Some(Access::Local) => WriteClient {
            access: ClientAccess::Local,
            id: None,
            name: None,
        },
        _ => {
            return err(ErrorCode::Unauthorized, "this client is not paired").into_response();
        }
    };
    let (parts, body) = req.into_parts();
    let path = parts.uri.path();
    let path = path
        .strip_prefix(protocol::API_PREFIX)
        .unwrap_or(path)
        .to_string();
    let Ok(body) = axum::body::to_bytes(body, MAX_BODY).await else {
        return err(
            ErrorCode::BadRequest,
            format!("the body is larger than {MAX_BODY} bytes"),
        )
        .into_response();
    };
    let Ok(body) = String::from_utf8(body.to_vec()) else {
        return err(ErrorCode::BadRequest, "the body is not UTF-8 text (JSON)").into_response();
    };
    let headers = HEADERS
        .iter()
        .filter_map(|h| {
            let v = parts.headers.get(*h)?.to_str().ok()?;
            Some((h.to_string(), v.to_string()))
        })
        .collect();
    let write = WriteRequest {
        v: super::link::VERSION,
        collection: sh.codex().active,
        machine: sh.auth().identity().id.to_string(),
        client,
        method: parts.method.to_string(),
        path,
        query: parts.uri.query().map(str::to_string),
        headers,
        body,
    };
    let changes = parts.method != Method::GET;
    let s = sh.clone();
    let answer = match tokio::task::spawn_blocking(move || s.mirror().forward(&write)).await {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => return unreachable(e),
        Err(e) => return err(ErrorCode::Internal, e.to_string()).into_response(),
    };
    if changes && (200..300).contains(&answer.status) {
        // The box's copy follows at once: the client reads its edit from this box.
        let s = sh.clone();
        let _ = tokio::task::spawn_blocking(move || s.mirror().refresh(&s)).await;
    }
    let mut r = Response::builder().status(answer.status);
    for (k, v) in &answer.headers {
        r = r.header(k.as_str(), v.as_str());
    }
    r.body(Body::from(answer.body))
        .unwrap_or_else(|e| err(ErrorCode::Internal, e.to_string()).into_response())
}

/// `GET /codex/search` on a show box for `collection=all` or another collection than its own:
/// asked of the Codex.
pub async fn search(sh: &Arc<Shared>, p: super::search::Params) -> Response {
    let req = SearchRequest {
        v: super::link::VERSION,
        q: p.q,
        collection: p.collection,
        kinds: p.kinds,
        limit: p.limit,
    };
    let s = sh.clone();
    match tokio::task::spawn_blocking(move || s.mirror().forward_search(&req)).await {
        Ok(Ok(r)) => axum::Json(r).into_response(),
        Ok(Err(e)) if e.code == ErrorCode::Unavailable => Error(ApiError::new(
            ErrorCode::Unavailable,
            format!(
                "searching beyond this box's collection needs the site's server, which this box can't reach now ({})",
                e.message
            ),
        ))
        .into_response(),
        Ok(Err(e)) => Error(e).into_response(),
        Err(e) => err(ErrorCode::Internal, e.to_string()).into_response(),
    }
}

// ---- the Codex ------------------------------------------------------------------------------

/// Runs a forwarded request through the Codex's own REST routes (`router`) as the show box's
/// client, on the collection the box shows: only the routes of [`FORWARDED`].
pub async fn execute(
    sh: Arc<Shared>,
    router: axum::Router,
    req: WriteRequest,
) -> Result<WriteAnswer, ApiError> {
    if route(&req.method, &req.path) != Route::Forward || !req.path.starts_with('/') {
        return Err(ApiError::new(
            ErrorCode::Forbidden,
            format!(
                "{} {} is not a request a show box forwards",
                req.method, req.path
            ),
        ));
    }
    let method = Method::from_bytes(req.method.as_bytes())
        .map_err(|_| ApiError::invalid("method", "not an HTTP method"))?;
    // The route must take the writer within WRITER_WAIT, or it refuses (`busy`): never
    // applied after the box gave up.
    let deadline = tokio::time::Instant::now() + WRITER_WAIT;
    let uri = match req.query.as_deref().filter(|q| !q.is_empty()) {
        Some(q) => format!("{}{}?{q}", protocol::API_PREFIX, req.path),
        None => format!("{}{}", protocol::API_PREFIX, req.path),
    };
    let mut b = axum::http::Request::builder().method(method).uri(uri);
    for (k, v) in &req.headers {
        if HEADERS.contains(&k.to_ascii_lowercase().as_str()) {
            b = b.header(k.as_str(), v.as_str());
        }
    }
    let mut r = b
        .body(Body::from(req.body))
        .map_err(|e| ApiError::invalid("path", e.to_string()))?;
    let access = match req.client.access {
        ClientAccess::Paired => {
            let id = req
                .client
                .id
                .filter(|i| !i.is_empty())
                .ok_or_else(|| ApiError::invalid("client", "a paired client has an id"))?;
            Access::Paired(Client {
                id,
                name: req.client.name.unwrap_or_default(),
                credential: Credential::Key,
            })
        }
        ClientAccess::Local => Access::Local,
    };
    r.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([0, 0, 0, 0], 0))));
    r.extensions_mut().insert(crate::Forwarded {
        access,
        machine: req.machine.clone(),
        collection: req.collection,
    });
    let resp = match super::writer::WRITER_DEADLINE
        .scope(deadline, router.oneshot(r))
        .await
    {
        Ok(r) => r,
        Err(e) => match e {},
    };
    let status = resp.status().as_u16();
    let headers = ANSWER_HEADERS
        .iter()
        .filter_map(|h| {
            let v = resp.headers().get(*h)?.to_str().ok()?;
            Some((h.to_string(), v.to_string()))
        })
        .collect();
    let body = axum::body::to_bytes(resp.into_body(), 15 << 20)
        .await
        .map_err(|e| ApiError::new(ErrorCode::Internal, format!("the answer: {e}")))?;
    let body = String::from_utf8(body.to_vec())
        .map_err(|_| ApiError::new(ErrorCode::Internal, "the answer is not UTF-8 text"))?;
    Ok(WriteAnswer {
        v: super::link::VERSION,
        status,
        headers,
        body,
        seq: sh.codex_writer().head(&sh, req.collection),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_of_a_show_box() {
        use Route::*;
        for (m, p, want) in [
            ("POST", "/api/v1/presentations", Forward),
            ("POST", "/presentations/abc/edits", Forward),
            ("DELETE", "/api/v1/timers/x", Forward),
            ("PUT", "/api/v1/stage-layouts/x", Forward),
            ("GET", "/api/v1/codex/trash", Forward),
            ("POST", "/api/v1/codex/trash/x/restore", Forward),
            (
                "POST",
                "/api/v1/codex/collections/c/media/a/relink",
                Forward,
            ),
            ("GET", "/api/v1/presentations/x", Local),
            ("GET", "/api/v1/presentations", Local),
            ("POST", "/api/v1/presentations/x/thumbnails", Local),
            ("PUT", "/api/v1/screens/x", Local),
            ("POST", "/api/v1/commands", Local),
            ("PUT", "/api/v1/codex/assignment", Local),
            ("GET", "/api/v1/codex/search", Local),
            ("GET", "/api/v1/codex/library", CodexBox),
            ("POST", "/api/v1/codex/uploads", CodexBox),
            ("PUT", "/api/v1/codex/uploads/u", CodexBox),
            ("GET", "/api/v1/codex/uploads", CodexBox),
            ("POST", "/api/v1/codex/uploads/u/finish", CodexBox),
            ("POST", "/api/v1/codex/collections", CodexBox),
            ("DELETE", "/api/v1/codex/trash", CodexBox),
            ("POST", "/api/v1/codex/collections/c/media/search", CodexBox),
            ("GET", "/api/v1/codex/collections", Local),
            ("GET", "/api/v1/codex/backups", CodexBox),
            ("POST", "/api/v1/codex/backups/snapshots", Local),
            ("GET", "/api/v1/codex/backups/s/snapshots", CodexBox),
            ("GET", "/api/v1/codex/backups/s/snapshots/n", CodexBox),
            (
                "GET",
                "/api/v1/codex/backups/s/snapshots/n/preview",
                CodexBox,
            ),
            (
                "POST",
                "/api/v1/codex/backups/s/snapshots/n/restore",
                CodexBox,
            ),
            ("GET", "/api/v1/codex/backups/restores", CodexBox),
            ("DELETE", "/api/v1/codex/backups/restores/r", CodexBox),
            ("PUT", "/api/v1/codex/backups/oriel", CodexBox),
            ("POST", "/api/v1/codex/export", CodexBox),
            ("PUT", "/api/v1/codex/export/settings", CodexBox),
            ("DELETE", "/api/v1/codex/export/passphrase", CodexBox),
            ("GET", "/api/v1/codex/export/scheduled", CodexBox),
            ("POST", "/api/v1/codex/export/imports", CodexBox),
            ("DELETE", "/api/v1/codex/export/imports/i", CodexBox),
            ("POST", "/api/v1/codex/export/imports/i/apply", CodexBox),
            ("POST", "/api/v1/codex/imports/pptx", CodexBox),
            ("GET", "/api/v1/codex/imports/pptx/j", CodexBox),
            // One segment per `*`: nothing else slips through.
            ("POST", "/api/v1/presentations/../clients", Local),
            ("DELETE", "/api/v1/timers/", Local),
            ("DELETE", "/api/v1/timers/a/b", Local),
            ("POST", "/api/v1/libraries/x", Local),
        ] {
            assert_eq!(route(m, p), want, "{m} {p}");
        }
    }
}
