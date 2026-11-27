//! Screen configuration: screens, output bindings, output transports, audio devices.

use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/screens", get(screens).post(create_screen))
        .route(
            "/screens/{id}",
            get(screen).put(update_screen).delete(delete_screen),
        )
        .route("/screens/{id}/color", axum::routing::put(put_screen_color))
        .route(
            "/screens/{id}/corner-pin",
            axum::routing::put(put_screen_corner_pin),
        )
        .route("/outputs", get(outputs_list).post(create_output))
        .route(
            "/outputs/{id}",
            get(output).put(update_output).delete(delete_output),
        )
        .route("/output-transports", get(output_transports))
        .route("/displays", get(displays))
        .route(
            "/displays/{id}/identify",
            axum::routing::post(identify_display),
        )
        .route("/audio-devices", get(audio_devices))
        .route("/audio-device", get(audio_device).put(put_audio_device))
}

fn config_response(r: ConfigResult, created: bool) -> Response {
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    match r {
        ConfigResult::Screen(e) => with_etag(status, &e.revision.clone(), e),
        ConfigResult::Output(e) => with_etag(status, &e.revision.clone(), e),
        ConfigResult::ScreenDeleted(d) => axum::Json(d).into_response(),
        ConfigResult::OutputDeleted => StatusCode::NO_CONTENT.into_response(),
        ConfigResult::Look(e) => with_etag(status, &e.revision.clone(), e),
        ConfigResult::LookDeleted => StatusCode::NO_CONTENT.into_response(),
        ConfigResult::Mask(e) => with_etag(status, &e.revision.clone(), *e),
        ConfigResult::MaskDeleted(d) => axum::Json(d).into_response(),
    }
}

pub(super) async fn configure(app: &AppState, op: ConfigOp, created: bool) -> Response {
    match app.show.configure(op).await {
        Ok(r) => config_response(r, created),
        Err(e) => e.into_response(),
    }
}

pub(super) fn json_body(body: &[u8]) -> Result<serde_json::Value, Error> {
    parse_body(body)
}

async fn screens(State(app): State<AppState>) -> ApiResult<Vec<protocol::ScreenEntry>> {
    ok(app
        .shared
        .data()
        .screens
        .iter()
        .map(crate::config::screen_entry)
        .collect())
}

async fn screen(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, Error> {
    let id: model::ScreenId = parse_id(&id, "screen id")?;
    let d = app.shared.data();
    let s = d
        .screens
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("screen {} not found", id.0)))?;
    let e = crate::config::screen_entry(s);
    Ok(with_etag(StatusCode::OK, &e.revision.clone(), e))
}

async fn create_screen(State(app): State<AppState>, body: Bytes) -> Response {
    match json_body(&body) {
        Ok(v) => configure(&app, ConfigOp::CreateScreen(v), true).await,
        Err(e) => e.into_response(),
    }
}

async fn update_screen(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = (|| {
        Ok::<_, Error>((
            parse_id::<model::ScreenId>(&id, "screen id")?,
            json_body(&body)?,
        ))
    })();
    match r {
        Ok((id, body)) => {
            let op = ConfigOp::UpdateScreen {
                id,
                body,
                if_match: if_match(&headers),
            };
            configure(&app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}

/// `PUT /screens/{id}/color`: the screen's colour correction, saved and applied live.
async fn put_screen_color(
    State(app): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    adjust_screen(&app, &id, &body, true).await
}

/// `PUT /screens/{id}/corner-pin`: the screen's corner pin, saved and applied live.
async fn put_screen_corner_pin(
    State(app): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    adjust_screen(&app, &id, &body, false).await
}

async fn adjust_screen(app: &AppState, id: &str, body: &[u8], color: bool) -> Response {
    let r = (|| {
        Ok::<_, Error>((
            parse_id::<model::ScreenId>(id, "screen id")?,
            json_body(body)?,
        ))
    })();
    match r {
        Ok((id, v)) => {
            let (color, corner_pin) = if color {
                (Some(v), None)
            } else {
                (None, Some(v))
            };
            let op = ConfigOp::AdjustScreen {
                id,
                color,
                corner_pin,
            };
            configure(app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}

async fn delete_screen(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match parse_id::<model::ScreenId>(&id, "screen id") {
        Ok(id) => {
            let op = ConfigOp::DeleteScreen {
                id,
                if_match: if_match(&headers),
            };
            configure(&app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}

async fn outputs_list(State(app): State<AppState>) -> ApiResult<Vec<protocol::OutputEntry>> {
    ok(app
        .shared
        .data()
        .outputs
        .iter()
        .map(crate::config::output_entry)
        .collect())
}

async fn output(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, Error> {
    let id: model::OutputId = parse_id(&id, "output id")?;
    let d = app.shared.data();
    let o = d
        .outputs
        .iter()
        .find(|o| o.id == id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("output {} not found", id.0)))?;
    let e = crate::config::output_entry(o);
    Ok(with_etag(StatusCode::OK, &e.revision.clone(), e))
}

async fn create_output(State(app): State<AppState>, body: Bytes) -> Response {
    match json_body(&body) {
        Ok(v) => configure(&app, ConfigOp::CreateOutput(v), true).await,
        Err(e) => e.into_response(),
    }
}

async fn update_output(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = (|| {
        Ok::<_, Error>((
            parse_id::<model::OutputId>(&id, "output id")?,
            json_body(&body)?,
        ))
    })();
    match r {
        Ok((id, body)) => {
            let op = ConfigOp::UpdateOutput {
                id,
                body,
                if_match: if_match(&headers),
            };
            configure(&app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}

async fn delete_output(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match parse_id::<model::OutputId>(&id, "output id") {
        Ok(id) => {
            let op = ConfigOp::DeleteOutput {
                id,
                if_match: if_match(&headers),
            };
            configure(&app, op, false).await
        }
        Err(e) => e.into_response(),
    }
}

/// `GET /audio-devices`: the playback devices of this machine (probing each takes a moment).
async fn audio_devices(State(app): State<AppState>) -> ApiResult<Vec<protocol::AudioDeviceInfo>> {
    let open = app.shared.sound.engine.status().open;
    let list = tokio::task::spawn_blocking(move || audio::device::list(Some(&open)))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    ok(list
        .into_iter()
        .map(|d| protocol::AudioDeviceInfo {
            name: d.name,
            description: d.description,
            available: d.available,
            problem: d.problem,
            channels: d.channels,
        })
        .collect())
}

/// `GET /audio-device`: the machine's audio device configuration.
async fn audio_device(State(app): State<AppState>) -> ApiResult<model::AudioDeviceConfig> {
    ok(app.shared.sound.config())
}

/// `PUT /audio-device`: stores and applies the audio device configuration (the device is
/// reopened when it changed; the master gain is set).
async fn put_audio_device(
    State(app): State<AppState>,
    body: Bytes,
) -> ApiResult<model::AudioDeviceConfig> {
    let mut cfg: model::AudioDeviceConfig = parse_body(&body)?;
    if !cfg.master_gain_db.is_finite()
        || !(crate::sound::MIN_DB..=crate::sound::MAX_DB).contains(&cfg.master_gain_db)
    {
        return Err(err(
            ErrorCode::BadRequest,
            "masterGainDb must be between -96 and +12",
        ));
    }
    if cfg
        .channel_map
        .iter()
        .any(|c| *c as usize >= audio::MAX_CHANNELS)
    {
        return Err(err(
            ErrorCode::BadRequest,
            "channelMap entries must be below 16",
        ));
    }
    if cfg.device.len() > 256 || cfg.device.contains('\0') {
        return Err(err(ErrorCode::BadRequest, "invalid device name"));
    }
    cfg.device = cfg.device.trim().to_string();
    cfg.schema_version = model::SCHEMA_VERSION;
    let root = app.shared.data().root.clone();
    let write =
        crate::store::DocWrite::put(&cfg).map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    tokio::task::spawn_blocking(move || crate::store::apply(&root, &[write]))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
        .map_err(|e| err(ErrorCode::Internal, format!("saving failed: {e}")))?;
    app.shared.sound.apply_config(&cfg);
    app.shared.status_changed.notify_one();
    ok(cfg)
}

/// `GET /displays`: the monitors and projectors attached to this box (also in the state).
async fn displays(State(app): State<AppState>) -> ApiResult<Vec<protocol::DisplayInfo>> {
    ok(app.shared.displays.infos(&app.shared.data().outputs))
}

/// `POST /displays/{id}/identify`: a display no output uses shows its name full screen for a
/// few seconds (`204`). `404` for an unknown display, `409` when it isn't connected, an
/// output uses it (identify its screen instead) or it is already showing its name.
async fn identify_display(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, Error> {
    let shared = app.shared.clone();
    let d = crate::displays::identify_check(&shared.displays, &shared.data().outputs, &id)
        .map_err(|(code, msg)| err(code, msg))?;
    let mode = d.resolve_mode(None).ok_or_else(|| {
        err(
            ErrorCode::Conflict,
            format!("{} reports no modes", d.connector),
        )
    })?;
    let (w, h) = (mode.width, mode.height);
    let lines = crate::displays::identify_lines(&d, mode);
    let (reply, rx) = tokio::sync::oneshot::channel();
    let job = crate::engine::ThumbJob {
        slide: render::patterns::identify_overlay(&lines, (w, h)).slide,
        canvas: model::Size::new(w as f64, h as f64),
        background: None,
        width: w,
        height: h,
        interactive: true,
        transparent: false,
        reply,
    };
    if !shared.send_render(crate::engine::Cmd::Thumbnail(Box::new(job))) {
        return Err(err(ErrorCode::Unavailable, "the renderer is not running"));
    }
    let frame = rx
        .await
        .map_err(|_| err(ErrorCode::Unavailable, "the renderer stopped"))?
        .map_err(|e| err(ErrorCode::Internal, format!("{e:#}")))?;
    tokio::task::spawn_blocking(move || crate::displays::identify_show(shared, d, mode, frame))
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
        .map_err(|e| err(ErrorCode::Conflict, format!("{e:#}")))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn output_transports(
    State(app): State<AppState>,
) -> ApiResult<Vec<protocol::OutputTransportInfo>> {
    let backend = app.shared.displays.backend();
    let problem = app.shared.displays.problem();
    let list =
        tokio::task::spawn_blocking(move || crate::config::transports(backend, problem.as_deref()))
            .await
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    ok(list)
}
