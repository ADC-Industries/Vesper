//! The control API: REST routes under `/api/v1/` and the WebSocket (see [`crate::ws`]).
//! `crates/protocol/README.md` documents it for client authors.
//!
//! | Route | |
//! |---|---|
//! | `GET  /api/v1/server` | identity, version, capabilities (no token needed) |
//! | `POST /api/v1/pair` | `{code?, clientName, browser?}` → `{token, clientId}`: a named site API key, or `202 {pairingId}` waiting for approval (no token needed) |
//! | `GET  /api/v1/pair/{pairingId}` | a waiting pairing's answer: `200` key, `202` still waiting, `403` refused, `404` ended (no token needed) |
//! | `DELETE /api/v1/pair/{pairingId}` | the client takes a waiting pairing back: `204`; `404` unknown or answered (no token needed) |
//! | `POST /api/v1/pair/upgrade` | a credential from before site keys → its site key, once |
//! | `GET  /api/v1/pairing/code` | current pairing code (server machine only) |
//! | `GET  /api/v1/clients`, `DELETE /api/v1/clients/{id}` | clients still on a credential from before site keys; revoke one, or a site key by its id (server machine only) |
//! | `GET  /api/v1/ws` | WebSocket: state, events, commands, previews |
//! | `GET  /api/v1/state` | current `{revision, state}` |
//! | `POST /api/v1/commands` | any command (same JSON as over the WebSocket) |
//! | `GET  /api/v1/libraries` | library summaries |
//! | `GET  /api/v1/libraries/{id}/presentations` | presentation summaries of a library |
//! | `GET  /api/v1/presentations` | every presentation summary |
//! | `GET  /api/v1/presentations/{id}` | the full presentation document |
//! | `GET  /api/v1/presentations/{id}/slides/{slideId}/thumbnail?w=320` | JPEG (ETag) |
//! | `POST /api/v1/presentations/{id}/thumbnails?w=320` | prefetch every slide thumbnail |
//! | `GET  /api/v1/playlists` | folder / playlist tree |
//! | `GET  /api/v1/playlists/{id}` | a playlist with its items resolved |
//! | `GET  /api/v1/looks`, `/clear-groups` | documents |
//! | `POST /api/v1/looks`, `GET/PUT/DELETE /api/v1/looks/{id}`, `…/duplicate` | looks (with revisions; edits use `If-Match`; the live look can't be deleted) |
//! | `GET/POST /api/v1/masks`, `GET/PUT/DELETE /api/v1/masks/{id}`, `…/duplicate` | masks (same rules; a mask that looks use is deleted only with `?detach=true`) |
//! | `GET/POST /api/v1/video-inputs`, `GET/PUT/DELETE /api/v1/video-inputs/{id}` | logical video inputs with this box's binding (`source`; revisions, `If-Match`) |
//! | `GET/POST /api/v1/screens`, `GET/PUT/DELETE /api/v1/screens/{id}` | screens (with revisions; edits use `If-Match`) |
//! | `PUT  /api/v1/screens/{id}/color`, `/screens/{id}/corner-pin` | a screen's colour correction / corner pin, saved and applied live (no `If-Match`) |
//! | `GET/POST /api/v1/outputs`, `GET/PUT/DELETE /api/v1/outputs/{id}` | output bindings (same rules) |
//! | `GET  /api/v1/output-transports` | which output transports this server can run |
//! | `GET/POST /api/v1/themes`, `GET/PUT/DELETE /api/v1/themes/{id}`, `…/duplicate`, `…/document`, `…/edits`, `…/used-by`, `…/slides/{slideId}/render`, `…/thumbnail` | themes (linked themes, see [`crate::themes`]) |
//! | `GET  /api/v1/displays` | the monitors and projectors attached to this box (`DisplayInfo[]`) |
//! | `POST /api/v1/displays/{id}/identify` | a display no output uses shows its name for 5 s |
//! | `GET  /api/v1/media/{id}` | asset metadata; `/media/{id}/file` streams it (Range) |
//! | `GET  /api/v1/media/{id}/thumbnail?w=` | JPEG thumbnail: an image, the frame of a video at about 10 % in (placeholder for audio and missing videos) |
//! | `GET  /api/v1/media-bins`, `/media-bins/{id}` | media bin tree; a bin's items |
//! | `GET  /api/v1/media/{id}/used-by`, `/props/{id}/used-by` | `DocumentUse[]`: the documents that use it |
//! | `GET  /api/v1/diagnostics` | render and load issues |
//! | `GET/PUT /api/v1/captions/config`, `POST /api/v1/captions/lines`, `GET /api/v1/captions/models` | live captions ([`crate::captions`]) |
//! | `GET  /api/v1/server/details` | this Vesper's version, OS, CPU, GPU, uptime and products (G9) |
//! | `GET  /api/v1/server/stats` | health counters: frame pacing per screen, video decoders, process memory, threads, files (soak) |
//! | `GET  /api/v1/macros` | the active collection's macros (`triggerMacro` runs one) |
//! | `GET/PUT /api/v1/remote-compat` | the Remote API compatibility layer: on/off, port, listening ([`crate::remote_compat`]) |
//! | `GET  /api/v1/presence`, `POST /api/v1/presence/locks`, `DELETE /api/v1/presence/locks/{kind}/{id}` | who has what open, edit locks ([`crate::presence`]) |
//! | `/api/v1/site/...` | the site: joins, join codes, devices, users, tokens, audit (see [`crate::site::api`]) |
//! | `/api/v1/updates/...` | the site's update channel: signed packages, the Vesper Launchers' reports and queued actions ([`updates`], G10) |
//! | `/api/v1/mosaic/...` | the site's display boxes, their screens and channels (Vesper Mosaic, see [`crate::mosaic`]) |
//! | `/api/v1/signage/...` | the site's signage channels, playlists, feeds and override (see [`crate::signage`]) |
//! | `GET /api/v1/reports/ccli`, `GET /api/v1/reports/ccli.csv` | CCLI song usage in a date range, as JSON or CSV ([`crate::ccli`]) |
//! | `/api/v1/codex/...` | Vesper Codex: status, collections, this box's assignment (see [`crate::codex::api`]) |
//! | `GET /api/v1/streaming`, `PUT …/recording`, `POST …/targets`, `PUT/DELETE …/targets/{id}`, `POST …/targets/{id}/start`, `…/stop`; `GET /api/v1/streaming/status`; `POST /api/v1/recording/start`, `/stop`, `/split` | recording and streaming a screen ([`crate::gateways`]) |
//! | `POST /api/v1/chordpro/parse`, `GET /api/v1/presentations/{id}/chordpro` | ChordPro → a new presentation; a presentation as ChordPro ([`chordpro`]) |
//! | `POST /api/v1/songselect/parse` | a SongSelect file (lyrics `.txt`, `.usr`, ChordPro) or pasted SongSelect lyrics → a new presentation with its song metadata ([`songselect`]) |
//! | `/api/v1/scripture/...` | Bible translations, references, passages, search ([`vesper_scripture::api`]); slides, previews and presentations from a passage ([`scripture`]) |
//!
//! Every route except the first two needs a trusted local request or a paired token
//! ([`crate::auth`]). Errors are `{"error": {"code", "message"}}` with the code's HTTP status.
//!
//! `/api/v1` answers CORS requests from any origin (the packaged desktop app's pages come from
//! `tauri://localhost`); preflight `OPTIONS` requests need no token. Everything outside `/api`
//! is the built control UI ([`crate::ServerConfig::ui_dir`]) with a single-page-app fallback
//! to `index.html`; it needs no token either (the UI pairs itself).

// Each area keeps its own routes (`routes()`), merged by [`router`].
mod captions;
mod chordpro;
mod details;
mod docs;
mod edit;
mod inputs;
mod library;
mod looks;
mod media;
mod mosaic;
mod presence;
mod remote;
mod reports;
mod screens;
mod scripture;
mod server;
mod show;
mod signage;
mod songselect;
mod sources;
mod stage;
mod streaming;
mod themes;
mod thumbnails;
mod translate;
mod updates;
mod usage;

pub(crate) use media::placeholder_thumbnail;
pub use show::dispatch;
pub(crate) use vesper_core::http::{ApiResult, err, if_match, ok, parse_body, with_etag};
pub use vesper_core::http::{Error, query_param};

use crate::actor::ShowHandle;
use crate::auth::Access;
use crate::config::{ConfigOp, ConfigResult};
use crate::content::{ContentOp, ContentResult};
use crate::data::ShowData;
use crate::preview::PreviewHub;
use crate::showdocs::{DocKind, DocOp, DocResult};
use crate::thumbs::Thumbnails;
use crate::{Shared, lock, ws};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use model::{
    BinBrowser, LibraryId, MediaAssetId, MediaBin, MediaBinId, MediaBinKind, MediaKind, PlaylistId,
    PlaylistItemKind, PlaylistNodeKind, Presentation, PresentationId, SlideId,
};
use protocol::{
    ApiError, Command, CommandResponse, ErrorCode, LibrarySummary, MediaAssetInfo, MediaBinDetail,
    MediaBinItemInfo, MediaBinNode, MediaBinNodeType, PairRequest, PairResponse, PlaylistDetail,
    PlaylistItemDetail, PlaylistNodeType, PlaylistTreeNode, PresentationSummary, ServerInfo,
};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceExt;

/// State of every handler.
#[derive(Clone)]
pub struct AppState {
    /// Shared server state.
    pub shared: Arc<Shared>,
    /// The same, as the Codex's [`crate::codex::Server`].
    pub codex: Arc<dyn crate::codex::Server>,
    /// The show actor.
    pub show: ShowHandle,
    /// Thumbnails.
    pub thumbs: Arc<Thumbnails>,
    /// Preview encoder.
    pub previews: Arc<PreviewHub>,
    /// Becomes `true` when the server shuts down (WebSockets close).
    pub shutdown: tokio::sync::watch::Receiver<bool>,
    /// Built control UI served at `/` (`None` = not served).
    pub ui_dir: Option<std::path::PathBuf>,
    /// Limits concurrent media thumbnail decodes.
    pub media_thumbs: Arc<tokio::sync::Semaphore>,
    /// The Codex's media worker (probes, stored thumbnails).
    pub media_jobs: crate::codex::worker::MediaWorker,
    /// The Bible translations ([`vesper_scripture`]).
    pub scripture: Arc<vesper_scripture::Scripture>,
    /// How often WebSocket clients are pinged ([`crate::ServerConfig::ws_ping`]).
    pub ws_ping: std::time::Duration,
    /// Generated passages waiting to be split again after a theme change.
    pub(crate) resplits: Arc<scripture::Resplits>,
    /// The Remote API compatibility layer's listener ([`crate::remote_compat`]).
    pub remote_compat: Arc<crate::remote_compat::RemoteCompat>,
    /// PowerPoint import jobs ([`crate::codex::pptx`]).
    pub pptx: Arc<crate::codex::pptx::PptxJobs>,
}

/// The scripture routes ([`vesper_scripture::api`]) take the translations as their state.
impl axum::extract::FromRef<AppState> for Arc<vesper_scripture::Scripture> {
    fn from_ref(app: &AppState) -> Self {
        app.scripture.clone()
    }
}

/// The site's routes ([`crate::site::api`]) take the site as their state.
impl axum::extract::FromRef<AppState> for Arc<crate::site::Site> {
    fn from_ref(app: &AppState) -> Self {
        app.shared.site.clone()
    }
}

/// Capabilities announced in `GET /api/v1/server` (the protocol README lists what each means).
pub const CAPABILITIES: &[&str] = &[
    "ws",
    "commands",
    "thumbnails",
    "preview",
    "pairing",
    "mdns",
    "mediaFiles",
    "reload",
    "mediaBins",
    "screenToggles",
    "screenAdjust",
    "fillKey",
    "ping",
    "screenConfig",
    "identifyScreens",
    "testPatterns",
    "editing",
    "site",
    "codex",
    "codexExport",
    "themeEditing",
    "scripture",
    "displays",
    "presence",
    "presenceBrowsing",
    "lookEditing",
    "masks",
    "screenLayouts",
    "macros",
    "remoteCompat",
    "streaming",
    "updates",
    "mosaic",
    "signage",
    "translate",
];

/// Builds the router.
pub fn router(app: AppState) -> Router {
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::IF_MATCH,
            header::IF_NONE_MATCH,
            header::RANGE,
            header::CONTENT_RANGE,
            header::HeaderName::from_static("x-client-id"),
            // Site ▸ Updates: a package's signature (`POST /updates/packages`).
            header::HeaderName::from_static(vesper_launcher::api::SIGNATURE_HEADER),
        ])
        // (Content-Disposition and the note: an export's file name and what it holds.)
        .expose_headers([
            header::ETAG,
            header::CONTENT_RANGE,
            header::ACCEPT_RANGES,
            header::CONTENT_DISPOSITION,
            header::HeaderName::from_static("x-codex-export-note"),
            header::HeaderName::from_static("x-codex-media-included"),
        ])
        .max_age(std::time::Duration::from_secs(600));
    let v1 = Router::new()
        .merge(server::routes())
        .merge(details::routes())
        .merge(show::routes())
        .merge(library::routes())
        .merge(thumbnails::routes())
        .merge(docs::routes())
        .merge(stage::routes())
        .merge(screens::routes())
        .merge(looks::routes())
        .merge(media::routes())
        .merge(usage::routes())
        .merge(sources::routes())
        .merge(streaming::routes())
        .merge(inputs::routes())
        .merge(themes::routes())
        .merge(presence::routes())
        .merge(remote::routes())
        .merge(scripture::routes())
        .merge(captions::routes())
        .merge(vesper_scripture::api::routes::<AppState>())
        .merge(crate::site::api::routes::<AppState>())
        .merge(crate::codex::api::routes())
        .merge(updates::routes())
        .merge(mosaic::routes())
        .merge(signage::routes())
        .merge(reports::routes())
        .merge(chordpro::routes())
        .merge(translate::routes())
        .merge(songselect::routes())
        .fallback(not_found_route)
        // A show box forwards content writes to the site's Codex (write-through).
        .layer(middleware::from_fn_with_state(
            app.clone(),
            crate::codex::api::write_through,
        ))
        // Edit locks: checked (and taken) before a show box forwards a write.
        .layer(middleware::from_fn_with_state(
            app.clone(),
            crate::presence::layer,
        ))
        .layer(cors);
    Router::new()
        .nest(protocol::API_PREFIX, v1)
        .fallback(ui_or_not_found)
        .layer(middleware::from_fn_with_state(app.clone(), authenticate))
        .with_state(app)
}

/// Everything outside `/api/v1`: the control UI (single-page app: unknown paths get
/// `index.html`), or a JSON 404 for `/api/...` and when no UI is configured.
async fn ui_or_not_found(State(app): State<AppState>, req: Request) -> Response {
    let path = req.uri().path();
    let api = path == "/api" || path.starts_with("/api/");
    match &app.ui_dir {
        Some(dir) if !api && matches!(*req.method(), Method::GET | Method::HEAD) => {
            let serve = tower_http::services::ServeDir::new(dir)
                .fallback(tower_http::services::ServeFile::new(dir.join("index.html")));
            match serve.oneshot(req).await {
                Ok(r) => r.map(Body::new),
                Err(e) => err(ErrorCode::Internal, e.to_string()).into_response(),
            }
        }
        _ => not_found_route(req).await.into_response(),
    }
}

// ---- errors and helpers -------------------------------------------------------------------

fn parse_id<T: From<uuid::Uuid>>(s: &str, what: &str) -> Result<T, Error> {
    s.parse::<uuid::Uuid>()
        .map(T::from)
        .map_err(|_| err(ErrorCode::BadRequest, format!("{what} {s:?} is not a UUID")))
}

fn require_local(access: &Access) -> Result<(), Error> {
    match access {
        Access::Local => Ok(()),
        _ => Err(err(
            ErrorCode::Forbidden,
            "only available on the server machine itself",
        )),
    }
}

async fn not_found_route(req: Request) -> Error {
    err(
        ErrorCode::NotFound,
        format!("no route {} {}", req.method(), req.uri().path()),
    )
}

/// Which requests made by a browser page may still count as the server machine (see
/// [`request_access`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Pages {
    /// The API: a page of this machine (its `Origin` is local: Oriel served here, the desktop
    /// app, a dev server on localhost). A page's plain reads (`GET`/`HEAD`, which change
    /// nothing) keep the machine's trust too: the desktop app's `<img>` loads carry neither
    /// `Origin` nor `Referer`.
    Local,
    /// The Remote API compatibility layer, whose triggers are plain `GET`s: no browser page
    /// at all (only a key counts for one). Its clients are button panels and scripts.
    None,
}

/// Who sends `req` (from `peer`): the server machine, a paired client (its key in
/// `Authorization: Bearer` or `?token=`), or neither. A web page from elsewhere, open in a
/// browser on the server machine, isn't the server machine: only a key counts for it. Nor is
/// a request addressed to another host name that resolves here (a DNS-rebinding page reads its
/// "own" origin without `Origin`). Which browser-made requests may still be the machine's is
/// `pages`'s choice: a browser marks them with `Sec-Fetch-Site` (anything but `none`, a
/// typed URL or bookmark), `Origin` or `Referer`. Also used by the Remote API compatibility
/// layer ([`crate::remote_compat`]) with [`Pages::None`].
pub(crate) fn request_access(
    app: &AppState,
    peer: SocketAddr,
    req: &Request,
    pages: Pages,
) -> Access {
    let token = bearer(req);
    let page_elsewhere =
        !request_host(req).is_some_and(crate::auth::local_host) || browser_page(req, pages);
    let ip = if page_elsewhere {
        std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
    } else {
        peer.ip()
    };
    app.shared.classify(ip, token.as_deref())
}

/// The key a request shows: `Authorization: Bearer`, or `?token=`.
fn bearer(req: &Request) -> Option<String> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .map(str::to_owned)
        .or_else(|| query_param(req.uri().query(), "token").map(str::to_owned))
}

/// Whether `req` comes from a browser page that `pages` doesn't let act as the server
/// machine.
fn browser_page(req: &Request, pages: Pages) -> bool {
    let h = req.headers();
    let text = |name: &str| h.get(name).map(|v| v.to_str().unwrap_or(""));
    let origin = text("origin");
    let referer = text("referer");
    // `none`: the user typed the URL (or used a bookmark); `same-origin`: a page this server
    // served. A page elsewhere is `same-site` or `cross-site`.
    let fetch_site = text("sec-fetch-site").map(|s| s.to_ascii_lowercase());
    match pages {
        Pages::None => {
            origin.is_some() || referer.is_some() || fetch_site.is_some_and(|s| s != "none")
        }
        Pages::Local => {
            let local_origin = origin.is_some_and(crate::auth::local_origin);
            let from_other_page = fetch_site.is_some_and(|s| s != "none" && s != "same-origin");
            let read = matches!(*req.method(), Method::GET | Method::HEAD);
            (origin.is_some() && !local_origin)
                || referer.is_some_and(|r| !local_referer(r))
                || (from_other_page && !read && !local_origin)
        }
    }
}

/// A `Referer` naming a page of this machine (see [`crate::auth::local_origin`]).
fn local_referer(referer: &str) -> bool {
    let Some((scheme, rest)) = referer.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    crate::auth::local_origin(&format!("{scheme}://{authority}"))
}

/// Authenticates every request and stores its [`Access`] as a request extension.
async fn authenticate(
    State(app): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut req: Request,
    next: Next,
) -> Response {
    // A show box's request run here by the Codex (write-through): its client, as the show
    // box authenticated it. (Only in-process code sets this extension.)
    let forwarded = req.extensions().get::<Forwarded>().cloned();
    let mut access = match &forwarded {
        Some(f) => f.access.clone(),
        None => request_access(&app, peer, &req, Pages::Local),
    };
    // A Vesper Launcher's own key (`crate::site::launchers`): the request is that machine's
    // Launcher (not the server machine, even from it, as a client's key is that client), and
    // reaches only the update channel's package and report routes, which check its machine.
    let launcher = (forwarded.is_none() && !matches!(access, Access::Paired(_)))
        .then(|| bearer(&req))
        .flatten()
        .filter(|t| t.starts_with(idn_open_protocol::trust::token::PREFIX))
        .and_then(|t| app.shared.site.verify_launcher(&t));
    if launcher.is_some() {
        access = Access::Unpaired;
    }
    let path = req.uri().path();
    // The UI's static files, CORS preflights, server info and pairing need no token.
    let public = !(path == "/api" || path.starts_with("/api/"))
        || req.method() == Method::OPTIONS
        || (req.method() == Method::GET && path == "/api/v1/server")
        || (req.method() == Method::POST && path == "/api/v1/pair")
        || ((req.method() == Method::GET || req.method() == Method::DELETE)
            && path.starts_with("/api/v1/pair/")
            && path != "/api/v1/pair/upgrade");
    let launcher = launcher.filter(|_| updates::launcher_route(req.method(), path));
    if !public && !access.allowed() && launcher.is_none() {
        return err(
            ErrorCode::Unauthorized,
            "this client is not paired with the server: pair it first",
        )
        .into_response();
    }
    if let Some(l) = launcher {
        req.extensions_mut().insert(l);
    }
    req.extensions_mut().insert(access.clone());
    req.extensions_mut().insert(peer);
    let caller = Caller { access, forwarded };
    CALLER.scope(caller, next.run(req)).await
}

/// The host a request is addressed to: its `Host` header, else the URI's authority (HTTP/2).
fn request_host(req: &Request) -> Option<&str> {
    match req.headers().get(header::HOST) {
        Some(h) => h.to_str().ok(),
        None => req.uri().authority().map(|a| a.as_str()),
    }
}

/// A show box's request run by the Codex through its own routes (see
/// [`crate::codex::Forwarded`]).
pub use crate::codex::Forwarded;

/// Who makes the request being handled.
#[derive(Clone)]
struct Caller {
    access: Access,
    forwarded: Option<Forwarded>,
}

tokio::task_local! {
    /// Who makes the request being handled (set by [`authenticate`]): the change logs name
    /// them.
    static CALLER: Caller;
}

/// The collection a show box's forwarded request acts on (the content routes act on it
/// instead of this box's active collection).
pub(crate) fn forwarded_collection() -> Option<model::CollectionId> {
    CALLER
        .try_with(|c| c.forwarded.as_ref().map(|f| f.collection))
        .ok()
        .flatten()
}

/// The show box's forwarded request being handled, if any (for work it starts in the
/// background, see [`as_forwarded`]).
pub(crate) fn forwarded() -> Option<Forwarded> {
    CALLER.try_with(|c| c.forwarded.clone()).ok().flatten()
}

/// Runs `f` as part of forwarded request `fwd` (its client, its collection): background work
/// that a show box's request started (a spawned task doesn't inherit the request's caller).
pub(crate) async fn as_forwarded<F: std::future::Future>(fwd: Forwarded, f: F) -> F::Output {
    let caller = Caller {
        access: fwd.access.clone(),
        forwarded: Some(fwd),
    };
    CALLER.scope(caller, f).await
}

/// Who makes the request being handled, for the Codex's change log and trash: the paired
/// client's key name and id, or the server machine; the machine is this server, or the show
/// box a forwarded request came through.
pub(crate) fn change_by(sh: &Shared) -> model::ChangeBy {
    let (client, client_id, machine) = CALLER
        .try_with(|c| {
            let (client, id) = match &c.access {
                Access::Paired(c) => (c.name.clone(), Some(c.id.clone())),
                Access::Local => (crate::site::SERVER_MACHINE.to_string(), None),
                Access::Unpaired => (String::new(), None),
            };
            (client, id, c.forwarded.as_ref().map(|f| f.machine.clone()))
        })
        .unwrap_or_default();
    let machine = machine.unwrap_or_else(|| sh.auth.identity().id.to_string());
    model::ChangeBy {
        client,
        client_id,
        machine,
    }
}
