//! Remote API compatibility: the commonly used endpoints of the presentation remote REST API
//! that existing Stream Deck / Companion button profiles call (`/version`, `/v1/...`), on a
//! port of its own, off by default (`machine/remote-compat.json`, set with
//! `PUT /api/v1/remote-compat`), mapped onto Vesper's commands.
//!
//! - **Auth** is the API's ([`crate::api::request_access`]): the server machine is trusted,
//!   every other client sends a paired site key (`Authorization: Bearer <key>` or
//!   `?token=<key>`), otherwise `401`. Its triggers are plain `GET`s, which any web page can
//!   send (an `<img>`), so a browser-made request (`Sec-Fetch-Site` other than `none`, or any
//!   `Origin` or `Referer`) is never the server machine here: it needs a key too.
//! - Commands run through [`crate::api::dispatch`], like `POST /api/v1/commands`. Triggers
//!   answer `204`; errors answer the API's JSON error with its status.
//! - `{id}` is a UUID, a name (case-insensitive) or a 0-based index in the list's order
//!   (documents by their sort key, then name). Lists answer `{"id": {"uuid", "name",
//!   "index"}, …}` objects.
//!
//! The endpoints covered are listed in `docs/CONTROL-SURFACES-PLAN.md` and the protocol README.

use crate::api::{AppState, Error, Pages, dispatch, err, request_access};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use model::{Layer, PlaylistItemKind, PlaylistNodeKind};
use protocol::{
    Command, ErrorCode, PlaylistItemRef, RemoteCompatSettings, RemoteCompatStatus, TimerRunState,
};
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};
use std::path::Path as FsPath;

/// Where the setting is kept, under the data folder (machine-local).
pub const SETTINGS_FILE: &str = "machine/remote-compat.json";

/// The saved setting (the default, off, when none was saved or it can't be read).
pub fn load_settings(root: &FsPath) -> RemoteCompatSettings {
    let path = root.join(SETTINGS_FILE);
    let Ok(bytes) = std::fs::read(&path) else {
        return Default::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        tracing::warn!("{} is not readable, ignored: {e}", path.display());
        Default::default()
    })
}

/// Saves the setting (atomically).
pub fn save_settings(root: &FsPath, s: RemoteCompatSettings) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(&s)?;
    crate::store::write_atomic(&root.join(SETTINGS_FILE), &bytes)
}

/// `items` in list order: by `key` (sort key, then name).
pub(crate) fn sorted<'a, T: 'a, K: Ord>(
    items: impl Iterator<Item = &'a T>,
    key: impl Fn(&T) -> K,
) -> Vec<&'a T> {
    let mut v: Vec<&T> = items.collect();
    v.sort_by_key(|t| key(t));
    v
}

/// The layer's listener: started, moved and stopped by its setting.
pub struct RemoteCompat {
    ip: IpAddr,
    inner: tokio::sync::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    settings: RemoteCompatSettings,
    running: Option<Running>,
    problem: Option<String>,
}

struct Running {
    /// The configured port it was started for.
    configured: u16,
    /// The port it listens on.
    port: u16,
    stop: tokio::sync::watch::Sender<bool>,
}

impl RemoteCompat {
    /// A layer that listens on `ip` (the API's interface), not started yet.
    pub fn new(ip: IpAddr) -> Self {
        RemoteCompat {
            ip,
            inner: Default::default(),
        }
    }

    /// The setting in force.
    pub async fn settings(&self) -> RemoteCompatSettings {
        self.inner.lock().await.settings
    }

    /// The setting and whether it listens.
    pub async fn status(&self) -> RemoteCompatStatus {
        let i = self.inner.lock().await;
        RemoteCompatStatus {
            enabled: i.settings.enabled,
            port: i.settings.port,
            listening: i.running.is_some(),
            listening_port: i.running.as_ref().map(|r| r.port),
            problem: i.problem.clone(),
        }
    }

    /// Applies `settings`: starts, moves or stops the listener. A port that can't be opened
    /// is reported in [`Self::status`] (`problem`).
    pub async fn apply(&self, app: &AppState, settings: RemoteCompatSettings) {
        let mut i = self.inner.lock().await;
        self.apply_locked(&mut i, app, settings).await;
    }

    /// `PUT /remote-compat`: merges `change` into the setting, saves it under the data folder
    /// `root` and applies it, all in one go (two changes at once can't save one setting and
    /// apply the other). The status, or why it couldn't be saved.
    pub async fn change(
        &self,
        app: &AppState,
        root: std::path::PathBuf,
        change: protocol::UpdateRemoteCompat,
    ) -> Result<RemoteCompatStatus, String> {
        let mut i = self.inner.lock().await;
        let mut s = i.settings;
        if let Some(e) = change.enabled {
            s.enabled = e;
        }
        if let Some(p) = change.port {
            s.port = p;
        }
        tokio::task::spawn_blocking(move || save_settings(&root, s))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("saving the setting: {e}"))?;
        self.apply_locked(&mut i, app, s).await;
        drop(i);
        Ok(self.status().await)
    }

    async fn apply_locked(&self, i: &mut Inner, app: &AppState, settings: RemoteCompatSettings) {
        i.settings = settings;
        let keep = i
            .running
            .as_ref()
            .is_some_and(|r| settings.enabled && r.configured == settings.port);
        if keep {
            return;
        }
        let mut stopped_port = None;
        if let Some(r) = i.running.take() {
            let _ = r.stop.send(true);
            stopped_port = Some(r.port);
            tracing::info!(port = r.port, "Remote API compatibility stopped");
        }
        i.problem = None;
        if !settings.enabled {
            return;
        }
        let addr = SocketAddr::new(self.ip, settings.port);
        // The listener just stopped closes its socket once its task sees the stop: moving
        // to the port it had (after port 0, the one it was given) waits for that.
        let mut tries = if stopped_port == Some(settings.port) {
            40
        } else {
            0
        };
        let listener = loop {
            match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => break l,
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && tries > 0 => {
                    tries -= 1;
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                Err(e) => {
                    tracing::warn!("Remote API compatibility can't listen on {addr}: {e}");
                    i.problem = Some(format!("port {} can't be opened: {e}", settings.port));
                    return;
                }
            }
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let mut shutdown = app.shutdown.clone();
        let router = router(app.clone());
        tokio::spawn(async move {
            let served = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                tokio::select! {
                    _ = stopped.wait_for(|s| *s) => {}
                    _ = shutdown.wait_for(|s| *s) => {}
                }
            })
            .await;
            if let Err(e) = served {
                tracing::warn!("Remote API compatibility: {e}");
            }
        });
        tracing::info!(
            "Remote API compatibility listening on http://{}:{port}/v1/",
            self.ip
        );
        i.running = Some(Running {
            configured: settings.port,
            port,
            stop,
        });
    }
}

/// The layer's routes, behind the API's authentication.
fn router(app: AppState) -> Router {
    Router::new()
        .route("/version", get(version))
        .route("/v1/version", get(version))
        .route("/v1/trigger/next", get(next))
        .route("/v1/trigger/previous", get(previous))
        .route("/v1/presentation/active", get(presentation_active))
        .route("/v1/presentation/slide_index", get(slide_index))
        .route("/v1/presentation/{id}/trigger", get(presentation_trigger))
        .route(
            "/v1/presentation/{id}/{index}/trigger",
            get(presentation_index_trigger),
        )
        .route("/v1/status/slide", get(status_slide))
        .route("/v1/playlists", get(playlists))
        .route("/v1/playlist/{id}/trigger", get(playlist_trigger))
        .route(
            "/v1/playlist/{id}/{index}/trigger",
            get(playlist_index_trigger),
        )
        .route("/v1/clear/layer/{layer}", get(clear_layer))
        .route("/v1/clear/groups", get(clear_groups))
        .route("/v1/clear/group/{id}/trigger", get(clear_group))
        .route("/v1/timers", get(timers))
        .route("/v1/timers/{op}", get(timers_op))
        .route("/v1/timer/system_time", get(system_time))
        .route("/v1/timer/{id}/{op}", get(timer_op).put(timer_put))
        .route("/v1/messages", get(messages))
        .route(
            "/v1/message/{id}/trigger",
            get(message_trigger_get).post(message_trigger_post),
        )
        .route("/v1/message/{id}/clear", get(message_clear))
        .route("/v1/looks", get(looks))
        .route("/v1/look/current", get(look_current))
        .route("/v1/look/{id}/trigger", get(look_trigger))
        .route("/v1/props", get(props))
        .route("/v1/prop/{id}/trigger", get(prop_trigger))
        .route("/v1/prop/{id}/clear", get(prop_clear))
        .route("/v1/macros", get(macros))
        .route("/v1/macro/{id}/trigger", get(macro_trigger))
        .route(
            "/v1/stage/message",
            get(stage_message)
                .put(stage_message_put)
                .delete(stage_message_delete),
        )
        .fallback(not_covered)
        .layer(middleware::from_fn_with_state(app.clone(), authenticate))
        .with_state(app)
}

/// The API's authentication: the server machine (not a browser page on it), or a paired
/// site key.
async fn authenticate(
    State(app): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if !request_access(&app, peer, &req, Pages::None).allowed() {
        return err(
            ErrorCode::Unauthorized,
            "this client is not paired with the server: send a site key (Authorization: Bearer <key>, or ?token=<key>)",
        )
        .into_response();
    }
    next.run(req).await
}

async fn not_covered(req: Request) -> Error {
    err(
        ErrorCode::NotFound,
        format!(
            "{} {} is not covered by Vesper's Remote API compatibility",
            req.method(),
            req.uri().path()
        ),
    )
}

// ---- helpers ------------------------------------------------------------------------------

/// Runs `cmd`: `204`, or the API error.
async fn run(app: &AppState, cmd: Command) -> Response {
    match dispatch(app, cmd).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => Error(e).into_response(),
    }
}

/// Runs several commands; the first error stops them.
async fn run_all(app: &AppState, cmds: Vec<Command>) -> Response {
    for c in cmds {
        if let Err(e) = dispatch(app, c).await {
            return Error(e).into_response();
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

/// An id object: `{uuid, name, index}`.
fn id_obj(uuid: impl ToString, name: &str, index: usize) -> Value {
    json!({"uuid": uuid.to_string(), "name": name, "index": index})
}

/// Finds `id` (UUID, name, or index) in `list` (in list order): its index and item.
fn find<'a, T>(
    list: &[&'a T],
    id: &str,
    uuid_of: impl Fn(&T) -> uuid::Uuid,
    name_of: impl Fn(&T) -> &str,
    what: &str,
) -> Result<(usize, &'a T), Error> {
    let id = id.trim();
    let hit = id
        .parse::<uuid::Uuid>()
        .ok()
        .and_then(|u| list.iter().position(|t| uuid_of(t) == u))
        .or_else(|| {
            list.iter()
                .position(|t| name_of(t).trim().eq_ignore_ascii_case(id))
        })
        .or_else(|| id.parse::<usize>().ok().filter(|i| *i < list.len()));
    hit.map(|i| (i, list[i]))
        .ok_or_else(|| err(ErrorCode::NotFound, format!("no {what} {id:?}")))
}

fn data(app: &AppState) -> std::sync::Arc<crate::data::ShowData> {
    app.shared.data()
}

fn state(app: &AppState) -> protocol::ShowState {
    app.show.snapshot().state.clone()
}

// ---- version and slides -------------------------------------------------------------------

async fn version(State(app): State<AppState>) -> axum::Json<Value> {
    let id = app.shared.auth.identity();
    axum::Json(json!({
        "name": id.name,
        "platform": std::env::consts::OS,
        "os_version": std::env::consts::OS,
        "host_description": format!("Vesper {}", crate::VERSION),
        "api_version": "v1",
    }))
}

async fn next(State(app): State<AppState>) -> Response {
    run(&app, Command::Next).await
}

async fn previous(State(app): State<AppState>) -> Response {
    run(&app, Command::Prev).await
}

/// `active` / `focused`: the live presentation (its slide position); else a presentation id.
fn live_or(
    app: &AppState,
    id: &str,
) -> Result<(model::PresentationId, Option<PlaylistItemRef>), Error> {
    if id == "active" || id == "focused" {
        let s = state(app);
        let pos = s
            .layers
            .slide
            .or(s.cursor)
            .ok_or_else(|| err(ErrorCode::Conflict, "no presentation is live"))?;
        let item = pos.playlist.map(|p| PlaylistItemRef {
            playlist: p.playlist,
            item: p.item,
        });
        return Ok((pos.presentation, item));
    }
    let d = data(app);
    let list = sorted(d.content.presentations.iter(), |p| p.name.to_lowercase());
    let (_, p) = find(&list, id, |p| p.id.0, |p| &p.name, "presentation")?;
    Ok((p.id, None))
}

async fn presentation_trigger(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    trigger_index(&app, &id, 0).await
}

async fn trigger_index(app: &AppState, id: &str, index: u32) -> Response {
    match live_or(app, id) {
        Ok((presentation, playlist_item)) => {
            run(
                app,
                Command::TriggerSlide {
                    presentation,
                    slide: None,
                    index: Some(index),
                    playlist_item,
                    step: None,
                    transition: None,
                },
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

async fn presentation_index_trigger(
    State(app): State<AppState>,
    Path((id, index)): Path<(String, String)>,
) -> Response {
    match index.as_str() {
        "next" => run(&app, Command::Next).await,
        "previous" => run(&app, Command::Prev).await,
        i => match i.parse::<u32>() {
            Ok(i) => trigger_index(&app, &id, i).await,
            Err(_) => {
                err(ErrorCode::BadRequest, format!("{i:?} is not a slide index")).into_response()
            }
        },
    }
}

async fn presentation_active(State(app): State<AppState>) -> Response {
    let s = state(&app);
    let Some(pos) = s.layers.slide else {
        return axum::Json(json!({"presentation": null})).into_response();
    };
    axum::Json(json!({"presentation": {
        "id": id_obj(pos.presentation.0, &pos.presentation_name, 0),
        "has_timeline": false,
    }}))
    .into_response()
}

async fn slide_index(State(app): State<AppState>) -> Response {
    let s = state(&app);
    let v = s.layers.slide.map(|pos| {
        json!({
            "index": pos.slide_index,
            "presentation_id": id_obj(pos.presentation.0, &pos.presentation_name, 0),
        })
    });
    axum::Json(json!({ "presentation_index": v })).into_response()
}

/// A slide's `{uuid, text, notes}` (empty text when it is gone).
fn slide_json(app: &AppState, pos: Option<&protocol::SlidePosition>) -> Value {
    let Some(pos) = pos else {
        return json!({"uuid": "", "text": "", "notes": ""});
    };
    let d = data(app);
    let slide = d.content.presentation(pos.presentation).and_then(|p| {
        crate::show::doc_slides(p)
            .into_iter()
            .find(|s| s.id == pos.slide)
            .cloned()
    });
    json!({
        "uuid": pos.slide.0.to_string(),
        "text": slide.as_ref().map(|s| s.plain_text()).unwrap_or_default(),
        "notes": slide.as_ref().map(|s| s.notes.plain_text()).unwrap_or_default(),
    })
}

async fn status_slide(State(app): State<AppState>) -> Response {
    let s = state(&app);
    axum::Json(json!({
        "current": slide_json(&app, s.layers.slide.as_ref()),
        "next": slide_json(&app, s.cursor.as_ref()),
    }))
    .into_response()
}

// ---- playlists ----------------------------------------------------------------------------

fn playlist_list(d: &crate::data::ShowData) -> Vec<&model::PlaylistNode> {
    sorted(
        d.content
            .playlists
            .values()
            .filter(|p| matches!(p.kind, PlaylistNodeKind::Playlist { .. })),
        |p| (p.sort_key.clone(), p.name.to_lowercase()),
    )
}

async fn playlists(State(app): State<AppState>) -> Response {
    let d = data(&app);
    let v: Vec<Value> = playlist_list(&d)
        .iter()
        .enumerate()
        .map(|(i, p)| json!({"id": id_obj(p.id.0, &p.name, i), "field_type": "playlist", "children": []}))
        .collect();
    axum::Json(Value::Array(v)).into_response()
}

async fn playlist_trigger(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    trigger_item(&app, &id, None).await
}

async fn playlist_index_trigger(
    State(app): State<AppState>,
    Path((id, index)): Path<(String, String)>,
) -> Response {
    match index.parse::<usize>() {
        Ok(i) => trigger_item(&app, &id, Some(i)).await,
        Err(_) => err(
            ErrorCode::BadRequest,
            format!("{index:?} is not an item index"),
        )
        .into_response(),
    }
}

/// Triggers a playlist's `index`-th item (`None`: its first enabled presentation or media).
async fn trigger_item(app: &AppState, id: &str, index: Option<usize>) -> Response {
    let item = {
        let d = data(app);
        let list = playlist_list(&d);
        let p = match find(&list, id, |p| p.id.0, |p| &p.name, "playlist") {
            Ok((_, p)) => p,
            Err(e) => return e.into_response(),
        };
        let PlaylistNodeKind::Playlist { items, .. } = &p.kind else {
            return err(ErrorCode::NotFound, "not a playlist").into_response();
        };
        let it = match index {
            Some(i) => items.get(i),
            None => items
                .iter()
                .find(|it| it.enabled && !matches!(it.kind, PlaylistItemKind::Header { .. })),
        };
        match it {
            Some(it) => (p.id, it.id),
            None => {
                return err(ErrorCode::NotFound, "the playlist has no such item").into_response();
            }
        }
    };
    run(
        app,
        Command::TriggerPlaylistItem {
            playlist: item.0,
            item: item.1,
        },
    )
    .await
}

// ---- clear --------------------------------------------------------------------------------

async fn clear_layer(State(app): State<AppState>, Path(layer): Path<String>) -> Response {
    let layer = match layer.as_str() {
        "audio" => Layer::Audio,
        "props" => Layer::Props,
        "messages" => Layer::Messages,
        "announcements" => Layer::Announcements,
        "slide" => Layer::Slide,
        "media" => Layer::Media,
        "video_input" => Layer::VideoInput,
        other => {
            return err(ErrorCode::BadRequest, format!("unknown layer {other:?}")).into_response();
        }
    };
    run(&app, Command::ClearLayer { layer }).await
}

fn group_list(d: &crate::data::ShowData) -> Vec<&model::ClearGroup> {
    sorted(d.content.clear_groups.iter(), |g| {
        (g.sort_key.clone(), g.name.to_lowercase())
    })
}

async fn clear_groups(State(app): State<AppState>) -> Response {
    let d = data(&app);
    let v: Vec<Value> = group_list(&d)
        .iter()
        .enumerate()
        .map(|(i, g)| json!({"id": id_obj(g.id.0, &g.name, i)}))
        .collect();
    axum::Json(Value::Array(v)).into_response()
}

async fn clear_group(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    let group = {
        let d = data(&app);
        let list = group_list(&d);
        match find(&list, &id, |g| g.id.0, |g| &g.name, "clear group") {
            Ok((_, g)) => g.id,
            Err(e) => return e.into_response(),
        }
    };
    run(&app, Command::ClearGroup { id: group }).await
}

// ---- timers -------------------------------------------------------------------------------

fn timer_list(d: &crate::data::ShowData) -> Vec<&model::Timer> {
    sorted(d.content.timers.iter(), |t| {
        (t.sort_key.clone(), t.name.to_lowercase())
    })
}

fn run_state(s: TimerRunState) -> &'static str {
    match s {
        TimerRunState::Running => "running",
        TimerRunState::Paused => "stopped",
        TimerRunState::Overrun => "overrunning",
        TimerRunState::Expired => "complete",
        _ => "stopped",
    }
}

async fn timers(State(app): State<AppState>) -> Response {
    let d = data(&app);
    let v: Vec<Value> = timer_list(&d)
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut o =
                json!({"id": id_obj(t.id.0, &t.name, i), "allows_overrun": t.allow_overrun});
            match &t.kind {
                model::TimerKind::Countdown { seconds } => {
                    o["countdown"] = json!({"duration": seconds});
                }
                model::TimerKind::CountdownToTime {
                    seconds_after_midnight,
                    ..
                } => o["count_down_to_time"] = json!({"time_of_day": seconds_after_midnight}),
                _ => o["elapsed"] = json!({}),
            }
            o
        })
        .collect();
    axum::Json(Value::Array(v)).into_response()
}

fn timers_current(app: &AppState) -> Value {
    let d = data(app);
    let s = state(app);
    let v: Vec<Value> = timer_list(&d)
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let live = s.timers.iter().find(|x| x.id == t.id);
            json!({
                "id": id_obj(t.id.0, &t.name, i),
                "time": live.map(|l| l.display.clone()).unwrap_or_default(),
                "state": live.map_or("stopped", |l| run_state(l.state)),
            })
        })
        .collect();
    Value::Array(v)
}

fn timer_command(op: &str, timer: model::TimerId) -> Result<Command, Error> {
    Ok(match op {
        "start" => Command::TimerStart { timer },
        "stop" => Command::TimerStop { timer },
        "reset" => Command::TimerReset { timer },
        other => {
            return Err(err(
                ErrorCode::BadRequest,
                format!("unknown timer operation {other:?} (start, stop, reset)"),
            ));
        }
    })
}

/// `GET /v1/timers/current`, or `start` / `stop` / `reset` every timer.
async fn timers_op(State(app): State<AppState>, Path(op): Path<String>) -> Response {
    if op == "current" {
        return axum::Json(timers_current(&app)).into_response();
    }
    let ids: Vec<model::TimerId> = timer_list(&data(&app)).iter().map(|t| t.id).collect();
    let mut cmds = Vec::new();
    for id in ids {
        match timer_command(&op, id) {
            Ok(c) => cmds.push(c),
            Err(e) => return e.into_response(),
        }
    }
    run_all(&app, cmds).await
}

fn timer_id(app: &AppState, id: &str) -> Result<model::TimerId, Error> {
    let d = data(app);
    let list = timer_list(&d);
    find(&list, id, |t| t.id.0, |t| &t.name, "timer").map(|(_, t)| t.id)
}

async fn timer_op(State(app): State<AppState>, Path((id, op)): Path<(String, String)>) -> Response {
    let cmd = timer_id(&app, &id).and_then(|t| timer_command(&op, t));
    match cmd {
        Ok(c) => run(&app, c).await,
        Err(e) => e.into_response(),
    }
}

/// `PUT /v1/timer/{id}/increment` (body: seconds, a JSON number).
async fn timer_put(
    State(app): State<AppState>,
    Path((id, op)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    if op != "increment" {
        return not_covered_put(&op);
    }
    let seconds = match serde_json::from_slice::<Value>(&body) {
        Ok(v) => v
            .as_f64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())),
        Err(_) => std::str::from_utf8(&body)
            .ok()
            .and_then(|s| s.trim().parse().ok()),
    };
    let Some(delta_seconds) = seconds.filter(|s: &f64| s.is_finite()) else {
        return err(
            ErrorCode::BadRequest,
            "the body must be a number of seconds",
        )
        .into_response();
    };
    match timer_id(&app, &id) {
        Ok(timer) => {
            run(
                &app,
                Command::TimerAdjust {
                    timer,
                    delta_seconds,
                },
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

fn not_covered_put(op: &str) -> Response {
    err(
        ErrorCode::NotFound,
        format!("timer operation {op:?} is not covered by Vesper's Remote API compatibility"),
    )
    .into_response()
}

async fn system_time() -> axum::Json<Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    axum::Json(json!(now))
}

// ---- messages -----------------------------------------------------------------------------

fn message_list(d: &crate::data::ShowData) -> Vec<&model::Message> {
    sorted(d.content.messages.iter(), |m| {
        (m.sort_key.clone(), m.name.to_lowercase())
    })
}

async fn messages(State(app): State<AppState>) -> Response {
    let d = data(&app);
    let v: Vec<Value> = message_list(&d)
        .iter()
        .enumerate()
        .map(|(i, m)| {
            json!({
                "id": id_obj(m.id.0, &m.name, i),
                "message": m.text,
                "tokens": m.tokens.iter().map(|t| json!({"name": t.name})).collect::<Vec<_>>(),
            })
        })
        .collect();
    axum::Json(Value::Array(v)).into_response()
}

fn message_id(app: &AppState, id: &str) -> Result<model::MessageId, Error> {
    let d = data(app);
    let list = message_list(&d);
    find(&list, id, |m| m.id.0, |m| &m.name, "message").map(|(_, m)| m.id)
}

async fn message_trigger_get(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    show_message(&app, &id, Vec::new()).await
}

/// `POST /v1/message/{id}/trigger` with `[{name, text: {text}}]` token values.
async fn message_trigger_post(
    State(app): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let mut tokens = Vec::new();
    if !body.iter().all(u8::is_ascii_whitespace) {
        let Ok(Value::Array(list)) = serde_json::from_slice::<Value>(&body) else {
            return err(
                ErrorCode::BadRequest,
                "the body must be a list of tokens: [{\"name\", \"text\": {\"text\"}}]",
            )
            .into_response();
        };
        for t in list {
            let Some(name) = t["name"].as_str() else {
                continue;
            };
            let value = t["text"]["text"]
                .as_str()
                .or_else(|| t["text"].as_str())
                .or_else(|| t["value"].as_str())
                .unwrap_or_default();
            tokens.push(model::TokenValue {
                name: name.to_string(),
                value: value.to_string(),
            });
        }
    }
    show_message(&app, &id, tokens).await
}

async fn show_message(app: &AppState, id: &str, tokens: Vec<model::TokenValue>) -> Response {
    match message_id(app, id) {
        Ok(message) => run(app, Command::MessageShow { message, tokens }).await,
        Err(e) => e.into_response(),
    }
}

async fn message_clear(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    match message_id(&app, &id) {
        Ok(m) => run(&app, Command::MessageHide { message: Some(m) }).await,
        Err(e) => e.into_response(),
    }
}

// ---- looks --------------------------------------------------------------------------------

fn look_list(d: &crate::data::ShowData) -> Vec<&model::Look> {
    sorted(d.looks.iter(), |l| {
        (l.sort_key.clone(), l.name.to_lowercase())
    })
}

async fn looks(State(app): State<AppState>) -> Response {
    let d = data(&app);
    let v: Vec<Value> = look_list(&d)
        .iter()
        .enumerate()
        .map(|(i, l)| json!({"id": id_obj(l.id.0, &l.name, i)}))
        .collect();
    axum::Json(Value::Array(v)).into_response()
}

async fn look_current(State(app): State<AppState>) -> Response {
    let d = data(&app);
    let s = state(&app);
    let list = look_list(&d);
    let v = s
        .active_look
        .and_then(|id| list.iter().position(|l| l.id == id).map(|i| (i, list[i])))
        .map(|(i, l)| json!({"id": id_obj(l.id.0, &l.name, i)}))
        .unwrap_or(Value::Null);
    axum::Json(v).into_response()
}

async fn look_trigger(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    let look = {
        let d = data(&app);
        let list = look_list(&d);
        match find(&list, &id, |l| l.id.0, |l| &l.name, "look") {
            Ok((_, l)) => l.id,
            Err(e) => return e.into_response(),
        }
    };
    run(&app, Command::SetLook { look: Some(look) }).await
}

// ---- props and macros ---------------------------------------------------------------------

fn prop_list(d: &crate::data::ShowData) -> Vec<&model::Prop> {
    sorted(d.content.props.iter(), |p| {
        (p.sort_key.clone(), p.name.to_lowercase())
    })
}

async fn props(State(app): State<AppState>) -> Response {
    let d = data(&app);
    let s = state(&app);
    let v: Vec<Value> = prop_list(&d)
        .iter()
        .enumerate()
        .map(|(i, p)| {
            json!({
                "id": id_obj(p.id.0, &p.name, i),
                "is_active": s.layers.props.iter().any(|x| x.prop == p.id),
            })
        })
        .collect();
    axum::Json(Value::Array(v)).into_response()
}

fn prop_id(app: &AppState, id: &str) -> Result<model::PropId, Error> {
    let d = data(app);
    let list = prop_list(&d);
    find(&list, id, |p| p.id.0, |p| &p.name, "prop").map(|(_, p)| p.id)
}

async fn prop_trigger(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    match prop_id(&app, &id) {
        Ok(prop) => run(&app, Command::PropShow { prop }).await,
        Err(e) => e.into_response(),
    }
}

async fn prop_clear(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    match prop_id(&app, &id) {
        Ok(prop) => run(&app, Command::PropHide { prop }).await,
        Err(e) => e.into_response(),
    }
}

fn macro_list(d: &crate::data::ShowData) -> Vec<&model::Macro> {
    sorted(d.content.macros.iter(), |m| {
        (m.sort_key.clone(), m.name.to_lowercase())
    })
}

async fn macros(State(app): State<AppState>) -> Response {
    let d = data(&app);
    let v: Vec<Value> = macro_list(&d)
        .iter()
        .enumerate()
        .map(|(i, m)| json!({"id": id_obj(m.id.0, &m.name, i)}))
        .collect();
    axum::Json(Value::Array(v)).into_response()
}

async fn macro_trigger(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    let macro_id = {
        let d = data(&app);
        let list = macro_list(&d);
        match find(&list, &id, |m| m.id.0, |m| &m.name, "macro") {
            Ok((_, m)) => m.id,
            Err(e) => return e.into_response(),
        }
    };
    run(&app, Command::TriggerMacro { macro_id }).await
}

// ---- stage message ------------------------------------------------------------------------

async fn stage_message(State(app): State<AppState>) -> axum::Json<Value> {
    let s = state(&app);
    axum::Json(json!(s.stage_message.map(|m| m.text).unwrap_or_default()))
}

/// `PUT /v1/stage/message` (body: the text as a JSON string).
async fn stage_message_put(State(app): State<AppState>, body: Bytes) -> Response {
    let text = match serde_json::from_slice::<Value>(&body) {
        Ok(Value::String(s)) => s,
        _ => String::from_utf8_lossy(&body).trim().to_string(),
    };
    if text.trim().is_empty() {
        return run(&app, Command::StageMessageClear).await;
    }
    run(&app, Command::StageMessage { text, flash: false }).await
}

async fn stage_message_delete(State(app): State<AppState>) -> Response {
    run(&app, Command::StageMessageClear).await
}
