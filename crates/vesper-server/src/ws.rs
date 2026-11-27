//! The WebSocket at `/api/v1/ws`: live state, events, commands and previews.
//!
//! Per connection, a **reader** (the upgrade task) parses client messages and runs commands in
//! order (and answers `ping` with `pong` at once), and a **writer** task owns the socket's sending half. The writer forwards
//! - replies (`hello`, `result`, `error`) from the reader,
//! - the newest state from the show actor's `watch` channel (a slow client skips revisions),
//! - notices and document changes from the broadcast channel,
//! - preview frames (a 4-slot channel; a slow client drops frames, never stalls the server).
//!
//! Before a `result`, the writer sends any state it has not sent yet, so a client always has
//! the state a result's `revision` names when it reads the result.
//!
//! The writer pings every [`AppState::ws_ping`]; a client the reader hears nothing from (not
//! even a pong) for three of those has gone away (Wi-Fi lost, laptop asleep) and is dropped,
//! with its previews.

use crate::api::{AppState, dispatch};
use crate::auth::Access;
use crate::preview::run_subscription;
use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Request, State};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use model::ScreenId;
use protocol::{
    ApiError, ClientMessage, Command, CommandResult, ErrorCode, NoticeLevel, PROTOCOL_VERSION,
    ServerHello, ServerMessage,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

/// Close code for an incompatible protocol version.
pub const CLOSE_INCOMPATIBLE: u16 = 4001;
/// Close code for a client whose key was revoked (or credential dropped) while connected.
pub const CLOSE_UNAUTHORIZED: u16 = 4003;
/// How often a paired client's key is checked again while its WebSocket is open.
const RECHECK: std::time::Duration = std::time::Duration::from_secs(2);

enum Out {
    Msg(Box<ServerMessage>),
    Close(u16, String),
}

/// `GET /api/v1/ws`.
pub async fn handler(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    ws: WebSocketUpgrade,
    req: Request,
) -> Response {
    let declared = crate::api::query_param(req.uri().query(), "protocolVersion")
        .map(|v| v.parse::<u32>().unwrap_or(0));
    // A paired client's key, checked again while it stays connected.
    let token = matches!(access, Access::Paired(_))
        .then(|| {
            req.headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(|t| t.trim().to_string())
                .or_else(|| crate::api::query_param(req.uri().query(), "token").map(str::to_owned))
        })
        .flatten();
    // Who the client is, for presence: its key's name and the site user it signed in as.
    let peer = req
        .extensions()
        .get::<std::net::SocketAddr>()
        .map(|a| a.ip())
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    let actor = app.shared.site.actor(&access, peer);
    let who = crate::presence::Who {
        name: actor.client.clone(),
        user: actor.signed_in.then_some(actor.user).flatten(),
        computer: None,
    };
    ws.on_upgrade(move |socket| connection(app, socket, declared, access, token, who))
}

/// The paired client's key still holds (not revoked, not expired, its site the same).
fn still_paired(app: &AppState, token: &str) -> bool {
    // Not from this machine: only the key counts.
    let nowhere = std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);
    matches!(app.shared.classify(nowhere, Some(token)), Access::Paired(_))
}

fn text(m: &ServerMessage) -> Message {
    Message::Text(Utf8Bytes::from(
        serde_json::to_string(m).unwrap_or_else(|_| "{}".into()),
    ))
}

fn incompatible(v: u32) -> ApiError {
    ApiError::new(
        ErrorCode::IncompatibleProtocol,
        format!(
            "this server speaks protocol version {PROTOCOL_VERSION}, the client speaks {v}; \
             update the {} app",
            if v < PROTOCOL_VERSION {
                "control"
            } else {
                "server"
            }
        ),
    )
}

async fn connection(
    app: AppState,
    socket: WebSocket,
    declared: Option<u32>,
    access: Access,
    token: Option<String>,
    mut who: crate::presence::Who,
) {
    let (mut sink, mut stream) = socket.split();
    if let Some(v) = declared
        && v != PROTOCOL_VERSION
    {
        let _ = sink
            .send(text(&ServerMessage::Error {
                error: incompatible(v),
            }))
            .await;
        let _ = sink
            .send(Message::Close(Some(CloseFrame {
                code: CLOSE_INCOMPATIBLE,
                reason: Utf8Bytes::from_static("incompatible protocol version"),
            })))
            .await;
        return;
    }
    tracing::debug!(?access, "WebSocket connected");
    let (out_tx, out_rx) = mpsc::channel::<Out>(64);
    let (prev_tx, prev_rx) = mpsc::channel::<Bytes>(4);
    let writer = tokio::spawn(writer(app.clone(), sink, out_rx, prev_rx));
    let mut previews: HashMap<ScreenId, JoinHandle<()>> = HashMap::new();
    let mut shutdown = app.shutdown.clone();
    let mut recheck = tokio::time::interval(RECHECK);
    recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut recheck_due = false;
    // Liveness: anything from the client (pongs included) counts.
    let mut last_heard = Instant::now();
    let mut alive = tokio::time::interval(app.ws_ping);
    alive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // This connection's presence session (listed once it reports what it has open).
    let session = app.shared.presence.new_session();

    loop {
        let msg = tokio::select! {
            m = stream.next() => m,
            _ = shutdown.wait_for(|s| *s) => break,
            _ = alive.tick() => {
                if last_heard.elapsed() >= app.ws_ping * 3 {
                    tracing::info!(?access, "WebSocket client stopped answering: dropped");
                    break;
                }
                continue;
            }
            _ = recheck.tick(), if token.is_some() => {
                recheck_due = true;
                None
            }
        };
        if std::mem::take(&mut recheck_due) {
            let (a, t) = (app.clone(), token.clone().unwrap_or_default());
            let ok = tokio::task::spawn_blocking(move || still_paired(&a, &t))
                .await
                .unwrap_or(true);
            if !ok {
                tracing::info!(?access, "a key was revoked: its WebSocket closes");
                let _ = out_tx
                    .send(Out::Close(CLOSE_UNAUTHORIZED, "no longer paired".into()))
                    .await;
                break;
            }
            continue;
        }
        let Some(Ok(msg)) = msg else { break };
        last_heard = Instant::now();
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            Message::Binary(_) => {
                let _ = out_tx
                    .send(Out::Msg(Box::new(ServerMessage::Error {
                        error: ApiError::new(
                            ErrorCode::BadRequest,
                            "binary messages are not accepted",
                        ),
                    })))
                    .await;
                continue;
            }
            _ => continue,
        };
        let parsed: Result<ClientMessage, _> = serde_json::from_str(text.as_str());
        match parsed {
            Err(e) => {
                // A command that can't be read still gets its result (the client is waiting
                // for that id); anything else an id-less error.
                let reply = match unreadable_command(text.as_str(), &e) {
                    Some((id, error)) => ServerMessage::Result(CommandResult::err(id, error)),
                    None => ServerMessage::Error {
                        error: ApiError::new(
                            ErrorCode::BadRequest,
                            format!("invalid message: {e}"),
                        ),
                    },
                };
                let _ = out_tx.send(Out::Msg(Box::new(reply))).await;
            }
            Ok(ClientMessage::Hello {
                protocol_version,
                client_name,
            }) => {
                who.computer = client_name
                    .map(|n| n.trim().chars().take(80).collect::<String>())
                    .filter(|n| !n.is_empty());
                if protocol_version != PROTOCOL_VERSION {
                    let _ = out_tx
                        .send(Out::Msg(Box::new(ServerMessage::Error {
                            error: incompatible(protocol_version),
                        })))
                        .await;
                    let _ = out_tx
                        .send(Out::Close(
                            CLOSE_INCOMPATIBLE,
                            "incompatible protocol version".into(),
                        ))
                        .await;
                    break;
                }
            }
            Ok(ClientMessage::Ping { id }) => {
                if out_tx
                    .send(Out::Msg(Box::new(ServerMessage::Pong { id })))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(ClientMessage::Presence {
                client_id,
                computer,
                open,
            }) => {
                if let Err(error) = app
                    .shared
                    .presence
                    .report(session, &who, client_id, computer, open)
                {
                    let _ = out_tx
                        .send(Out::Msg(Box::new(ServerMessage::Error { error })))
                        .await;
                }
            }
            Ok(ClientMessage::Command { id, command }) => {
                let result = match command {
                    Command::SubscribePreview { screen, fps, width } => {
                        subscribe(&app, &mut previews, &prev_tx, screen, fps, width)
                    }
                    Command::UnsubscribePreview { screen } => {
                        previews.retain(|s, h| {
                            let stop = screen.is_none_or(|x| x == *s);
                            if stop {
                                h.abort();
                            }
                            !stop
                        });
                        Ok(app.show.snapshot().revision)
                    }
                    other => dispatch(&app, other).await,
                };
                let r = match result {
                    Ok(rev) => CommandResult::ok(id, rev),
                    Err(e) => CommandResult::err(id, e),
                };
                if out_tx
                    .send(Out::Msg(Box::new(ServerMessage::Result(r))))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }
    for (_, h) in previews {
        h.abort();
    }
    // Its presence and, with its client's last session, its edit locks go.
    app.shared.presence.disconnect(session);
    drop(out_tx);
    // Give the writer a moment to flush (e.g. the close frame), then stop it.
    let mut writer = writer;
    if tokio::time::timeout(Duration::from_secs(2), &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
    tracing::debug!("WebSocket closed");
}

/// A `command` message that did not parse: its id and the error to answer it with
/// (`notImplemented` for a command type this server does not know, as from a newer client;
/// `badRequest` for bad fields). `None`: not a command with an id.
fn unreadable_command(text: &str, e: &serde_json::Error) -> Option<(u32, ApiError)> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v.get("type")?.as_str()? != "command" {
        return None;
    }
    let id = u32::try_from(v.get("id")?.as_u64()?).ok()?;
    let kind = v
        .get("command")
        .and_then(|c| c.get("type"))
        .and_then(|t| t.as_str());
    // Whether the command's type alone is known: parse it with nothing else.
    let known = kind.is_some_and(|k| {
        serde_json::from_value::<Command>(serde_json::json!({ "type": k }))
            .err()
            .is_none_or(|e| !e.to_string().contains("unknown variant"))
    });
    let error = match kind {
        Some(k) if !known => ApiError::new(
            ErrorCode::NotImplemented,
            format!("this server does not know the command {k:?} (it may be older than the app)"),
        ),
        _ => ApiError::new(ErrorCode::BadRequest, format!("invalid command: {e}")),
    };
    Some((id, error))
}

fn subscribe(
    app: &AppState,
    previews: &mut HashMap<ScreenId, JoinHandle<()>>,
    out: &mpsc::Sender<Bytes>,
    screen: ScreenId,
    fps: f64,
    width: u32,
) -> Result<u64, ApiError> {
    if app.shared.screen(screen).is_none() {
        return Err(ApiError::new(
            ErrorCode::NotFound,
            format!("screen {} not found", screen.0),
        ));
    }
    if fps.is_nan() || fps <= 0.0 {
        return Err(ApiError::new(ErrorCode::BadRequest, "fps must be positive"));
    }
    let width = width.clamp(16, protocol::MAX_IMAGE_WIDTH);
    if let Some(old) = previews.remove(&screen) {
        old.abort();
    }
    previews.insert(
        screen,
        tokio::spawn(run_subscription(
            app.shared.clone(),
            app.previews.clone(),
            screen,
            fps,
            width,
            out.clone(),
        )),
    );
    Ok(app.show.snapshot().revision)
}

async fn writer(
    app: AppState,
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut out: mpsc::Receiver<Out>,
    mut previews: mpsc::Receiver<Bytes>,
) {
    let mut state = app.show.subscribe();
    let mut events = app.shared.events.subscribe();
    let id = app.shared.auth.identity();
    let hello = ServerMessage::Hello {
        server: ServerHello {
            id: id.id.to_string(),
            name: id.name,
            version: crate::VERSION.to_string(),
            protocol_version: PROTOCOL_VERSION,
        },
    };
    let snapshot = state.borrow_and_update().clone();
    let first = [
        hello,
        ServerMessage::State {
            revision: snapshot.revision,
            state: snapshot.state.clone(),
        },
    ];
    for m in first
        .iter()
        .chain(app.shared.recent_notices().iter().map(|m| m.as_ref()))
    {
        if sink.send(text(m)).await.is_err() {
            return;
        }
    }
    let mut previews_open = true;
    let mut ping = tokio::time::interval(app.ws_ping);
    ping.tick().await;
    let state_msg = |s: &protocol::StateSnapshot| ServerMessage::State {
        revision: s.revision,
        state: s.state.clone(),
    };
    loop {
        let msg = tokio::select! {
            biased;
            // Pings first: on a link slower than the previews, the preview channel is always
            // ready, and a ping polled after it would never go out (the client, busy
            // receiving, sends nothing either, and the reader would drop it as gone).
            _ = ping.tick() => Message::Ping(Bytes::new()),
            o = out.recv() => match o {
                None => break,
                Some(Out::Close(code, reason)) => {
                    let _ = sink.send(Message::Close(Some(CloseFrame { code, reason: reason.into() }))).await;
                    break;
                }
                Some(Out::Msg(m)) => {
                    let m = *m;
                    if matches!(m, ServerMessage::Result(_)) && state.has_changed().unwrap_or(false) {
                        let s = state.borrow_and_update().clone();
                        if sink.send(text(&state_msg(&s))).await.is_err() {
                            break;
                        }
                    }
                    text(&m)
                }
            },
            r = state.changed() => {
                if r.is_err() {
                    break;
                }
                let s = state.borrow_and_update().clone();
                text(&state_msg(&s))
            }
            e = events.recv() => match e {
                Ok(m) => text(&m),
                Err(broadcast::error::RecvError::Lagged(n)) => text(&ServerMessage::Notice {
                    level: NoticeLevel::Warning,
                    message: format!("{n} events were dropped because this client is slow; refetch documents"),
                    key: Some("lagged".into()),
                }),
                Err(broadcast::error::RecvError::Closed) => break,
            },
            p = previews.recv(), if previews_open => match p {
                Some(b) => Message::Binary(b),
                None => {
                    previews_open = false;
                    continue;
                }
            },
        };
        if sink.send(msg).await.is_err() {
            break;
        }
    }
    let _ = sink.close().await;
}
