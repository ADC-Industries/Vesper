//! `/updates/…`: the site's update channel and its Vesper Launchers ([`crate::updates`], G10).
//!
//! | Route | |
//! |---|---|
//! | `GET  /updates/packages` | `UpdatePackage[]`, by product and platform, newest first |
//! | `POST /updates/packages?file=NAME` | a signed package as the body, its `.minisig` file base64 in `X-Vesper-Signature` (admins) → `201 UpdatePackage`; `400` bad signature, `409` already there |
//! | `GET  /updates/packages/{id}/file`, `…/signature` | the package (Range) and its `.minisig` text |
//! | `DELETE /updates/packages/{id}` | (admins) → `204` |
//! | `PUT  /updates/launchers/{machineId}` | a Launcher's `LauncherReport` (its machine's Launcher key, the server machine, or an admin's key: see [`report`]) → `LauncherActions` queued for it |
//! | `GET  /updates/launchers` | `LauncherEntry[]`: every Launcher's last report, online, pending actions, newer versions in the channel |
//! | `POST /updates/launchers/{machineId}/actions` | a `LauncherAction` (admins) → `202 LauncherEntry` |
//! | `POST /updates/launchers/{machineId}/idn-join` | (that machine's Launcher key only) → `LauncherJoinCode`: an IDN join code for its display box's IDN Node ([`idn_join`]) |
//!
//! A machine's **Launcher key** ([`crate::site::launchers`]) is taken by the package list and
//! downloads and by its own machine's report, and by no other route ([`launcher_route`]).

use super::*;
use crate::codex::api::{admin, audit};
use base64::Engine;
use futures_util::StreamExt;
use protocol::{LauncherAction, LauncherActions, LauncherReport};
use serde_json::json;

/// The largest package accepted (4 GiB).
const MAX_PACKAGE: u64 = 4 << 30;
/// The largest Launcher report accepted (a real one is a few KB).
const MAX_REPORT: usize = 64 * 1024;

/// The routes a Launcher key reaches (`/api/v1/…`): the package list and downloads, and a
/// Launcher's report (each route checks the key's machine).
pub(super) fn launcher_route(method: &Method, path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/api/v1/updates/") else {
        return false;
    };
    let parts: Vec<&str> = rest.split('/').collect();
    match (method, parts.as_slice()) {
        (&Method::GET, ["packages"]) => true,
        (&Method::GET, ["packages", _, "file" | "signature"]) => true,
        (&Method::PUT, ["launchers", m]) => !m.is_empty(),
        (&Method::POST, ["launchers", m, "idn-join"]) => !m.is_empty(),
        _ => false,
    }
}

/// The Launcher key a request was let in with (by `authenticate`), if any.
type LauncherCaller = Option<axum::Extension<crate::site::launchers::LauncherKey>>;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/updates/packages", get(list).post(upload))
        .route("/updates/packages/{id}", delete(remove))
        .route("/updates/packages/{id}/file", get(file))
        .route("/updates/packages/{id}/signature", get(signature))
        .route("/updates/launchers", get(launchers))
        .route(
            "/updates/launchers/{machine}",
            axum::routing::put(report).layer(axum::extract::DefaultBodyLimit::max(MAX_REPORT)),
        )
        .route("/updates/launchers/{machine}/actions", post(queue))
        .route("/updates/launchers/{machine}/idn-join", post(idn_join))
}

async fn list(State(app): State<AppState>) -> ApiResult<Vec<protocol::UpdatePackage>> {
    ok(app.shared.updates.list())
}

async fn upload(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    headers: HeaderMap,
    req: Request,
) -> Response {
    let name = query_param(req.uri().query(), "file")
        .map(|s| s.to_string())
        .unwrap_or_default();
    let action = "updates.upload";
    let detail = json!({"file": name});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let Some(sig) = headers
        .get(vesper_launcher::api::SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            base64::engine::general_purpose::STANDARD
                .decode(v.trim())
                .ok()
        })
        .and_then(|b| String::from_utf8(b).ok())
    else {
        return err(
            ErrorCode::BadRequest,
            "the package's signature (its .minisig file, base64) is missing from X-Vesper-Signature",
        )
        .into_response();
    };
    let r = receive(&app, req.into_body()).await;
    let r = match r {
        Ok((tmp, digests)) => {
            let updates = app.shared.clone();
            let (tmp2, name2) = (tmp.clone(), name.clone());
            let r = tokio::task::spawn_blocking(move || {
                updates.updates.add(&tmp2, &digests, &name2, &sig)
            })
            .await
            .unwrap_or_else(|e| Err(protocol::ApiError::new(ErrorCode::Internal, e.to_string())));
            let _ = std::fs::remove_file(&tmp);
            r.map_err(Error)
        }
        Err(e) => Err(e),
    };
    let detail = match &r {
        Ok(p) => {
            json!({"file": name, "product": p.product, "version": p.version, "platform": p.platform})
        }
        Err(_) => detail,
    };
    audit(
        &app,
        &actor,
        action,
        detail,
        r.as_ref().err().map(|e| e.0.message.clone()),
    )
    .await;
    match r {
        Ok(p) => (StatusCode::CREATED, axum::Json(p)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Streams the body to a staging file, hashing it on the way.
async fn receive(
    app: &AppState,
    body: Body,
) -> Result<(std::path::PathBuf, vesper_launcher::sign::Digests), Error> {
    use tokio::io::AsyncWriteExt;
    let internal = |e: std::io::Error| err(ErrorCode::Internal, e.to_string());
    let dir = app.shared.updates.staging().map_err(internal)?;
    let tmp = dir.join(uuid::Uuid::new_v4().to_string());
    let mut f = tokio::fs::File::create(&tmp).await.map_err(internal)?;
    let mut h = vesper_launcher::sign::Hasher::new();
    let mut stream = body.into_data_stream();
    let mut size = 0u64;
    let r: Result<(), Error> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| err(ErrorCode::BadRequest, e.to_string()))?;
            size += chunk.len() as u64;
            if size > MAX_PACKAGE {
                return Err(err(
                    ErrorCode::BadRequest,
                    "the package is larger than 4 GiB",
                ));
            }
            h.update(&chunk);
            f.write_all(&chunk).await.map_err(internal)?;
        }
        f.flush().await.map_err(internal)?;
        Ok(())
    }
    .await;
    if let Err(e) = r {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    if size == 0 {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(err(ErrorCode::BadRequest, "the package is empty"));
    }
    Ok((tmp, h.finish()))
}

async fn file(State(app): State<AppState>, Path(id): Path<String>, req: Request) -> Response {
    let Some((p, path, _)) = app.shared.updates.get(&id) else {
        return err(ErrorCode::NotFound, "no such package").into_response();
    };
    match tower_http::services::ServeFile::new(path)
        .oneshot(req)
        .await
    {
        Ok(mut r) => {
            if let Ok(v) = HeaderValue::from_str(&format!(
                "attachment; filename=\"{}\"",
                p.file_name.replace('"', "")
            )) {
                r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
            }
            r.map(Body::new)
        }
        Err(e) => err(ErrorCode::Internal, e.to_string()).into_response(),
    }
}

async fn signature(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    let Some((_, _, sig)) = app.shared.updates.get(&id) else {
        return err(ErrorCode::NotFound, "no such package").into_response();
    };
    match tokio::fs::read_to_string(sig).await {
        Ok(t) => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], t).into_response(),
        Err(e) => err(ErrorCode::Internal, e.to_string()).into_response(),
    }
}

async fn remove(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    let action = "updates.delete";
    let detail = json!({"package": id});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let shared = app.shared.clone();
    let id2 = id.clone();
    let found = tokio::task::spawn_blocking(move || shared.updates.delete(&id2))
        .await
        .unwrap_or(false);
    let e = (!found).then(|| "no such package".to_string());
    audit(&app, &actor, action, detail, e.clone()).await;
    match e {
        None => StatusCode::NO_CONTENT.into_response(),
        Some(m) => err(ErrorCode::NotFound, m).into_response(),
    }
}

/// A Launcher's report, and the actions queued for it (given out once). A report takes the
/// machine's queued update or rollback and sets what Site ▸ Updates shows and offers for it,
/// so it is taken only with **that machine's own Launcher key** (made as the Launcher joined
/// the site, bound to its machine ID: [`crate::site::launchers`]), from the server machine
/// itself, or with a key that may administer the site. A Launcher key of another machine is
/// refused, as is any other client's key. A refusal isn't audited: a Launcher reports every
/// 30 seconds.
async fn report(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    launcher: LauncherCaller,
    Path(machine): Path<String>,
    body: Bytes,
) -> ApiResult<LauncherActions> {
    match launcher {
        Some(axum::Extension(k)) => {
            if k.machine_id != machine {
                tracing::warn!(key = %k.key_id, bound = %k.machine_id, %machine, "a Launcher key reported as another machine: refused");
                return Err(err(
                    ErrorCode::Forbidden,
                    "this Launcher key belongs to another machine: a Launcher reports only as its own machine",
                ));
            }
        }
        None => {
            let actor = app.shared.site.actor(&access, peer.ip());
            let site = app.shared.site.clone();
            tokio::task::spawn_blocking(move || {
                site.require(&actor, idn_site_manager::users::Role::Admin)
            })
            .await
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
            .map_err(|e| {
                err(
                    ErrorCode::Forbidden,
                    format!(
                        "a Launcher reports with its machine's own Launcher key (join the site in the Launcher), from the server machine, or with a key that may administer the site ({})",
                        e.message
                    ),
                )
            })?;
        }
    }
    let r: LauncherReport = parse_body(&body)?;
    if r.machine_id != machine {
        return Err(err(
            ErrorCode::BadRequest,
            "the report's machineId isn't the one in the path",
        ));
    }
    app.shared.mosaic.note_launcher(&r.machine_name, &r.roles);
    let actions = app.shared.updates.report(r).map_err(Error)?;
    ok(LauncherActions { actions })
}

/// An IDN join code for a display box (`docs/LAUNCHER-PLAN.md`, round 3): the machine's
/// Launcher asks with its own Launcher key, gets a code the site's server makes, and runs
/// `idn-node join CODE`, so the volunteer setting the box up types the server's pairing code
/// once (to join the Launcher) and nothing else. Only that machine's Launcher key: another
/// machine's is `403`, and a client's key or an admin gets `403` too (admins make codes in
/// Site ▸ Machines).
async fn idn_join(
    State(app): State<AppState>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    launcher: LauncherCaller,
    Path(machine): Path<String>,
) -> ApiResult<protocol::LauncherJoinCode> {
    let Some(axum::Extension(k)) = launcher else {
        return Err(err(
            ErrorCode::Forbidden,
            "only a machine's own Vesper Launcher asks for its IDN join code; make a code in Site ▸ Machines",
        ));
    };
    if k.machine_id != machine {
        tracing::warn!(key = %k.key_id, bound = %k.machine_id, %machine, "a Launcher key asked for another machine's join code: refused");
        return Err(err(
            ErrorCode::Forbidden,
            "this Launcher key belongs to another machine",
        ));
    }
    let name = app
        .shared
        .updates
        .launchers()
        .into_iter()
        .find(|e| e.report.machine_id == machine)
        .map(|e| e.report.machine_name)
        .unwrap_or_else(|| format!("machine {machine}"));
    let client = format!("{}{name}", crate::site::launchers::LAUNCHER_KEY_PREFIX);
    let site = app.shared.site.clone();
    let code = tokio::task::spawn_blocking(move || site.launcher_join_code(&k, &client, peer.ip()))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
        .map_err(Error)?;
    ok(code)
}

async fn launchers(State(app): State<AppState>) -> ApiResult<Vec<protocol::LauncherEntry>> {
    ok(app.shared.updates.launchers())
}

async fn queue(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    Path(machine): Path<String>,
    body: Bytes,
) -> Response {
    let a: LauncherAction = match parse_body(&body) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let action = "updates.queue";
    let detail = json!({"machine": machine, "action": a});
    let actor = match admin(&app, &access, peer, action, &detail).await {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let r = app.shared.updates.queue(&machine, a);
    audit(
        &app,
        &actor,
        action,
        detail,
        r.as_ref().err().map(|e| e.message.clone()),
    )
    .await;
    match r {
        Ok(e) => (StatusCode::ACCEPTED, axum::Json(e)).into_response(),
        Err(e) => Error(e).into_response(),
    }
}
