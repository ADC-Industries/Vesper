//! Server info, pairing, the clients from before site keys, diagnostics and fonts.

use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/server", get(server_info))
        .route("/pair", post(pair))
        .route("/pair/upgrade", post(upgrade))
        .route(
            "/pair/{pairing}",
            get(pairing_status).delete(withdraw_pairing),
        )
        .route("/pairing/code", get(pairing_code))
        .route("/clients", get(clients))
        .route("/clients/{id}", delete(revoke_client))
        .route("/diagnostics", get(diagnostics))
        .route("/fonts", get(fonts))
}

async fn server_info(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
) -> ApiResult<ServerInfo> {
    let id = app.shared.auth.identity();
    ok(ServerInfo {
        id: id.id.to_string(),
        name: id.name,
        version: crate::VERSION.to_string(),
        protocol_version: protocol::PROTOCOL_VERSION,
        capabilities: CAPABILITIES
            .iter()
            // `mdns` only while the server is announced (off with `--no-mdns`, or when the
            // announcement failed).
            .filter(|c| {
                **c != "mdns"
                    || app
                        .shared
                        .mdns_on
                        .load(std::sync::atomic::Ordering::Acquire)
            })
            .map(|c| c.to_string())
            .collect(),
        auth: access.status(),
        pairing_code: access
            .is_local()
            .then(|| app.shared.pairing_code().map(|c| c.code))
            .flatten(),
        upgrade_credential: matches!(
            &access,
            Access::Paired(c) if c.credential == crate::auth::Credential::Legacy
        )
        .then_some(true),
    })
}

/// A pairing's outcome as a response: `200` with the key, or `202` waiting for approval.
fn pair_outcome(o: crate::site::pairing::PairOutcome) -> Response {
    match o {
        crate::site::pairing::PairOutcome::Paired(r) => axum::Json(r).into_response(),
        crate::site::pairing::PairOutcome::Waiting(p) => {
            (StatusCode::ACCEPTED, axum::Json(p)).into_response()
        }
    }
}

/// `POST /api/v1/pair`: the code (if any) is checked here; the site makes the client's key,
/// or the pairing waits for a site admin's approval ([`crate::site::pairing`]).
async fn pair(
    State(app): State<AppState>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Result<Response, Error> {
    let req: PairRequest = parse_body(&body)?;
    if let Some(m) = &req.launcher
        && !crate::site::launchers::valid_machine_id(m)
    {
        return Err(err(
            ErrorCode::BadRequest,
            "launcher: a machine ID is 1 to 64 letters, digits, '-', '_' or '.'",
        ));
    }
    let sh = app.shared.clone();
    let r = tokio::task::spawn_blocking(move || {
        let ip = peer.ip();
        // Without a site no key can be made: say so before the code is used up.
        sh.site.can_pair()?;
        let typed = req.code.as_deref().map(str::trim).filter(|c| !c.is_empty());
        // At a relay the site's server checks the code (the site's own, which it gave this
        // server): it is passed on, never judged here.
        let relay = sh.site.relays_pairings();
        let code_ok = if relay {
            sh.auth.pair_attempt(ip)?;
            false
        } else {
            // A right code is used up here, at once (it pairs one client).
            sh.auth.check_code(ip, typed)?
        };
        let name = match &req.launcher {
            Some(_) => crate::site::launchers::launcher_key_name(&req.client_name, ip),
            None => crate::auth::key_name(&req.client_name, ip),
        };
        sh.site.pair(
            &name,
            ip,
            code_ok,
            typed.filter(|_| relay),
            req.browser.unwrap_or(false),
            req.launcher.as_deref(),
        )
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))??;
    Ok(pair_outcome(r))
}

/// `GET /api/v1/pair/{pairingId}`: a waiting pairing's answer (the key is given once).
async fn pairing_status(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let sh = app.shared.clone();
    let r = tokio::task::spawn_blocking(move || sh.site.pairing_status(&id))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))??;
    Ok(pair_outcome(r))
}

/// `DELETE /api/v1/pair/{pairingId}`: the client takes its waiting pairing back (it cancels,
/// or types its code again): `204`; `404` for one unknown or already answered. No token: the
/// pairing's id is its secret.
async fn withdraw_pairing(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let sh = app.shared.clone();
    tokio::task::spawn_blocking(move || sh.site.withdraw_pairing(&id))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))??;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /api/v1/pair/upgrade`: a client on a credential from before site keys gets its
/// named site key, once (no one pairs again). The old credential keeps working until the
/// key is first used, then it is dropped; asked again before that, the key handed over
/// before is revoked and a new one made (the answer may have been lost).
async fn upgrade(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    axum::Extension(peer): axum::Extension<SocketAddr>,
    body: Bytes,
) -> Result<Response, Error> {
    let req: protocol::UpgradeCredential = if body.iter().all(u8::is_ascii_whitespace) {
        Default::default()
    } else {
        parse_body(&body)?
    };
    let client = match access {
        Access::Paired(c) if c.credential == crate::auth::Credential::Legacy => c,
        _ => {
            return Err(err(
                ErrorCode::Conflict,
                "only a client's credential from before site keys is swapped for a key",
            ));
        }
    };
    let sh = app.shared.clone();
    let r = tokio::task::spawn_blocking(move || {
        let name = crate::auth::key_name(&client.name, peer.ip());
        let caller = idn_site_manager::host::Caller {
            address: peer.ip().to_string(),
            client: name.clone(),
        };
        let (token, id) =
            sh.site
                .create_client_key(&name, &caller, req.browser.unwrap_or(false))?;
        if let Some(prev) = sh.auth.upgraded(&client.id, &id)
            && let Err(e) = sh.site.revoke_key_as_server(&prev)
        {
            tracing::warn!(key = %prev, "revoking a key handed over but never used: {}", e.message);
        }
        tracing::info!(client = %client.name, key = %id, "site key handed over for a credential from before site keys");
        Ok::<_, ApiError>(PairResponse {
            token,
            client_id: id,
        })
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))??;
    Ok(axum::Json(r).into_response())
}

async fn pairing_code(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
) -> ApiResult<protocol::PairingCode> {
    require_local(&access)?;
    match app.shared.pairing_code() {
        Some(c) => ok(c),
        None => Err(err(
            ErrorCode::Unavailable,
            "the site's server hasn't given this server its pairing code yet; try again in a moment",
        )),
    }
}

async fn clients(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
) -> ApiResult<Vec<protocol::ClientInfo>> {
    require_local(&access)?;
    ok(app.shared.auth.legacy_clients())
}

async fn revoke_client(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    Path(id): Path<String>,
) -> Result<StatusCode, Error> {
    require_local(&access)?;
    let sh = app.shared.clone();
    tokio::task::spawn_blocking(move || {
        if let Some(handed_over) = sh.auth.revoke_legacy(&id) {
            // The key it was swapped for (not used yet) goes with it (unless revoked already).
            match handed_over.map(|key| sh.site.revoke_key_as_server(&key)) {
                Some(Err(e)) if e.code != ErrorCode::NotFound => Err(e),
                _ => Ok(StatusCode::NO_CONTENT),
            }
        } else {
            // A site key, by its id.
            sh.site
                .revoke_key_as_server(&id)
                .map(|_| StatusCode::NO_CONTENT)
        }
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
    .map_err(Error)
}

async fn diagnostics(State(app): State<AppState>) -> ApiResult<protocol::DiagnosticsReport> {
    let r = crate::lock(&app.shared.diagnostics).report(&app.shared);
    ok(r)
}

async fn fonts(State(app): State<AppState>) -> ApiResult<protocol::FontList> {
    let (families, fallback) = crate::lock(&app.shared.fonts).clone();
    let installed = |name: &str| families.iter().any(|f| f.family.eq_ignore_ascii_case(name));
    let substitutions = app
        .shared
        .data()
        .font_substitutions
        .rules
        .iter()
        .map(|r| protocol::FontSubstitutionInfo {
            family: r.family.clone(),
            postscript_name: r.postscript_name.clone(),
            replacement_family: r.replacement_family.clone(),
            installed: installed(&r.replacement_family),
        })
        .collect();
    ok(protocol::FontList {
        families: families
            .iter()
            .map(|f| protocol::FontFamilyInfo {
                family: f.family.clone(),
                faces: f
                    .faces
                    .iter()
                    .map(|x| protocol::FontFaceInfo {
                        weight: x.weight,
                        italic: x.italic,
                        postscript_name: x.postscript_name.clone(),
                    })
                    .collect(),
            })
            .collect(),
        substitutions,
        fallback_family: fallback,
    })
}
