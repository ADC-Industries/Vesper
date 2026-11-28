//! The show actor: the single owner of the show state.
//!
//! Commands arrive over a channel from every WebSocket and REST request and are applied one at
//! a time, in arrival order. After each change the actor
//! 1. hands a copy of the state to the render thread ([`Shared::publish`]),
//! 2. bumps the revision and publishes the full [`protocol::ShowState`] on a `watch` channel
//!    (every WebSocket forwards the newest value; a slow client skips revisions, never sees
//!    stale ones),
//! 3. answers the command with that revision.
//!
//! Screen / output / diagnostics status lives in the same `ShowState`: the actor rebuilds it
//! when the render thread signals a change (and once a second) and broadcasts a new revision
//! when something an operator would notice changed (a render time, an output's health, an fps
//! change of more than 1, a new diagnostic). Frame counters alone trigger a broadcast at most
//! every 10 s, so an idle server with running outputs sends one small state per 10 s.

use crate::config::{ConfigError, ConfigOp, ConfigResult};
use crate::data::ShowData;
use crate::show::{HeldSlide, LiveSlide, ShowError, ShowState, TimerOp};
use crate::timers::Clock;
use crate::{Shared, lock};
use model::{DocumentKind, Layer, LookId, MediaKind};
use protocol::{
    ApiError, Command, DocumentChange, ErrorCode, MediaLayerState, MediaPlayback, NoticeLevel,
    OutputStatus, PlaybackState, ScreenState, ServerMessage, SlidePosition, StateSnapshot,
    TimerRunState, VolumeTarget,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, watch};

/// A change of this box's active collection, already planned and written by the Codex writer
/// ([`crate::codex::writer`]): the actor swaps the new content in and lets the show follow.
pub use crate::codex::CollectionUpdate;

enum Request {
    Command {
        command: Command,
        reply: oneshot::Sender<Result<u64, ApiError>>,
    },
    Config {
        op: ConfigOp,
        reply: oneshot::Sender<Result<ConfigResult, ConfigError>>,
    },
    Collection {
        update: Box<CollectionUpdate>,
        reply: oneshot::Sender<Result<(), ApiError>>,
    },
    Inputs {
        op: crate::inputs::InputOp,
        reply: oneshot::Sender<Result<crate::inputs::InputResult, ConfigError>>,
    },
    Settings {
        settings: Box<model::ShowSettings>,
        reply: oneshot::Sender<Result<(), ApiError>>,
    },
}

/// Longest stage message, characters.
pub const MAX_STAGE_MESSAGE: usize = 500;

/// How long `identifyScreens` stays on.
pub const IDENTIFY_FOR: Duration = Duration::from_secs(60);

/// Handle to the show actor (cheap to clone).
#[derive(Clone)]
pub struct ShowHandle {
    tx: mpsc::Sender<Request>,
    state: watch::Receiver<Arc<StateSnapshot>>,
}

impl ShowHandle {
    /// Applies a show command; returns the revision of the state that includes its effect.
    pub async fn apply(&self, command: Command) -> Result<u64, ApiError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Request::Command { command, reply })
            .await
            .map_err(|_| ApiError::new(ErrorCode::Unavailable, "the server is shutting down"))?;
        rx.await
            .map_err(|_| ApiError::new(ErrorCode::Internal, "the show actor stopped"))?
    }

    /// Applies a configuration change (validate, persist, reconfigure live; see
    /// [`crate::config`]). Runs in order with the show commands.
    pub async fn configure(&self, op: ConfigOp) -> Result<ConfigResult, ConfigError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Request::Config { op, reply })
            .await
            .map_err(|_| ApiError::new(ErrorCode::Unavailable, "the server is shutting down"))?;
        rx.await
            .map_err(|_| ApiError::new(ErrorCode::Internal, "the show actor stopped"))?
    }

    /// Applies a change of the logical video inputs or their bindings (see
    /// [`crate::inputs`]). Runs in order with the show commands.
    pub async fn inputs(
        &self,
        op: crate::inputs::InputOp,
    ) -> Result<crate::inputs::InputResult, ConfigError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Request::Inputs { op, reply })
            .await
            .map_err(|_| ApiError::new(ErrorCode::Unavailable, "the server is shutting down"))?;
        rx.await
            .map_err(|_| ApiError::new(ErrorCode::Internal, "the show actor stopped"))?
    }

    /// Saves new show settings and swaps them in (every screen re-renders: the CCLI line).
    /// Runs in order with the show commands.
    pub async fn settings(&self, settings: model::ShowSettings) -> Result<(), ApiError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Request::Settings {
                settings: Box::new(settings),
                reply,
            })
            .await
            .map_err(|_| ApiError::new(ErrorCode::Unavailable, "the server is shutting down"))?;
        rx.await
            .map_err(|_| ApiError::new(ErrorCode::Internal, "the show actor stopped"))?
    }

    /// Swaps in a change of the active collection that the Codex writer has written (see
    /// [`CollectionUpdate`]). Runs in order with the show commands.
    pub async fn collection(&self, update: CollectionUpdate) -> Result<(), ApiError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(Request::Collection {
                update: Box::new(update),
                reply,
            })
            .await
            .map_err(|_| ApiError::new(ErrorCode::Unavailable, "the server is shutting down"))?;
        rx.await
            .map_err(|_| ApiError::new(ErrorCode::Internal, "the show actor stopped"))?
    }

    /// The newest state.
    pub fn snapshot(&self) -> Arc<StateSnapshot> {
        self.state.borrow().clone()
    }

    /// A receiver of every new state.
    pub fn subscribe(&self) -> watch::Receiver<Arc<StateSnapshot>> {
        self.state.clone()
    }
}

/// Starts the actor.
pub fn spawn(shared: Arc<Shared>, initial: ShowState) -> ShowHandle {
    let (tx, rx) = mpsc::channel(256);
    let mut actor = Actor {
        status: StatusKey::default(),
        shared,
        revision: 1,
        state_tx: None,
        counters: Vec::new(),
        last_broadcast: Instant::now(),
        identify_until: None,
        transport_changed: false,
        last_position: None,
        timer_key: Vec::new(),
        last_tick: Instant::now(),
        saved_look: initial.look,
        state: initial,
        retired_videos: Vec::new(),
        captions_key: Default::default(),
        sound_recheck: None,
        step0: Default::default(),
    };
    let first = Arc::new(StateSnapshot {
        revision: actor.revision,
        state: actor.build(),
    });
    actor.status = StatusKey::of(&first.state);
    let (state_tx, state_rx) = watch::channel(first);
    actor.state_tx = Some(state_tx);
    tokio::spawn(actor.run(rx));
    ShowHandle {
        tx,
        state: state_rx,
    }
}

/// (status, detail, has errors) of one output.
type OutputKey = (OutputStatus, Option<String>, u64);
/// (last render ms × 10, rounded fps, outputs) of one screen.
type ScreenKey = (i64, i64, Vec<OutputKey>);

/// The parts of the status an operator would notice (frame counters excluded).
#[derive(Default, PartialEq)]
struct StatusKey {
    screens: Vec<ScreenKey>,
    diagnostics: protocol::DiagnosticsSummary,
    displays: Vec<protocol::DisplayInfo>,
    /// The sound card's state and why (lost, back again: the status bar shows it, H05-1).
    audio: (protocol::AudioOutputState, Option<String>),
}

impl StatusKey {
    fn of(s: &protocol::ShowState) -> StatusKey {
        StatusKey {
            screens: s
                .screens
                .iter()
                .map(|sc| {
                    (
                        sc.last_render_ms.map(|m| (m * 10.0) as i64).unwrap_or(-1),
                        sc.measured_fps.round() as i64,
                        sc.outputs
                            .iter()
                            .map(|o| (o.status, o.detail.clone(), o.errors.min(1)))
                            .collect(),
                    )
                })
                .collect(),
            diagnostics: s.diagnostics.clone(),
            displays: s.displays.clone(),
            audio: (s.audio.output_state, s.audio.detail.clone()),
        }
    }

    /// Changed enough to broadcast: anything but small fps jitter.
    fn differs(&self, new: &StatusKey) -> bool {
        if self.diagnostics != new.diagnostics
            || self.displays != new.displays
            || self.audio != new.audio
            || self.screens.len() != new.screens.len()
        {
            return true;
        }
        self.screens
            .iter()
            .zip(&new.screens)
            .any(|(a, b)| a.0 != b.0 || a.2 != b.2 || (a.1 - b.1).abs() > 1)
    }
}

struct Actor {
    shared: Arc<Shared>,
    state: ShowState,
    revision: u64,
    state_tx: Option<watch::Sender<Arc<StateSnapshot>>>,
    status: StatusKey,
    /// Frame counters of the last broadcast, and when it happened.
    counters: Vec<u64>,
    last_broadcast: Instant,
    /// When `identify` turns itself off.
    identify_until: Option<Instant>,
    /// A transport command changed a player (not the show state): publish anyway.
    transport_changed: bool,
    /// The transport target's (state, position) in the last broadcast.
    last_position: Option<(u64, PlaybackState, i64)>,
    /// Each timer's run state in the last broadcast (a change broadcasts at once).
    timer_key: Vec<TimerRunState>,
    /// When timer values were last sent (state or `timerTick`).
    last_tick: Instant,
    /// The live look as last saved for the next start (see [`crate::show::LIVE_LOOK_FILE`]).
    saved_look: Option<LookId>,
    /// Video plays the state no longer references: they stay in the registry until a state
    /// without them is published ([`Actor::retire_videos`]), so the render thread, still
    /// drawing the previous state while a new video opens, never finds its play missing.
    retired_videos: Vec<u64>,
    /// The captions state in the last broadcast.
    captions_key: protocol::CaptionsState,
    /// When a timed build of the live slide starts or ends: element sounds are checked again
    /// then (see [`crate::element_video::sounding`]).
    sound_recheck: Option<Instant>,
    /// When the screens started the live slide's first build step (H04-2).
    step0: crate::element_video::Step0,
}

/// How often timer states are checked (a timer expiring is broadcast within this).
const TIMER_CHECK: Duration = Duration::from_millis(200);

/// Timer values are sent at most this often while timers run.
const TIMER_TICK: Duration = Duration::from_secs(1);

/// How often positions of playing videos are refreshed (at most 4 broadcasts a second).
const POSITION_REFRESH: Duration = Duration::from_millis(250);

/// Cross-fade of the sound when a play continues another one (`mediaSetLoop`), s.
const CONTINUE_FADE: f64 = 0.05;

/// Frame counters and measured fps change every frame: they are refreshed with every
/// broadcast, and on their own at most this often (only while they change).
const COUNTER_REFRESH: Duration = Duration::from_secs(10);

/// Sleeps until `at` (never returns without one).
async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at.into()).await,
        None => std::future::pending().await,
    }
}

fn counters(s: &protocol::ShowState) -> Vec<u64> {
    s.screens
        .iter()
        .flat_map(|sc| sc.outputs.iter().map(|o| o.frames_sent + o.frames_dropped))
        .collect()
}

impl Actor {
    async fn run(mut self, mut rx: mpsc::Receiver<Request>) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut media_tick = tokio::time::interval(POSITION_REFRESH);
        media_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut timer_tick = tokio::time::interval(TIMER_CHECK);
        timer_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                req = rx.recv() => {
                    match req {
                        None => break,
                        Some(Request::Command { command, reply }) => {
                            let name = command.name();
                            let res = self.apply(command).await;
                            if let Err(e) = &res {
                                tracing::debug!(command = %name, "refused: {e}");
                            }
                            self.save_look().await;
                            let _ = reply.send(res);
                        }
                        Some(Request::Collection { update, reply }) => {
                            let res = self.collection(*update).await;
                            if let Err(e) = &res {
                                tracing::warn!("collection change not applied: {e}");
                            }
                            let _ = reply.send(res);
                        }
                        Some(Request::Config { op, reply }) => {
                            let res = self.configure(op).await;
                            if let Err(e) = &res {
                                tracing::info!("configuration change refused: {}", e.error);
                            }
                            let _ = reply.send(res);
                        }
                        Some(Request::Inputs { op, reply }) => {
                            let res = self.change_inputs(op).await;
                            if let Err(e) = &res {
                                tracing::info!("video input change refused: {}", e.error);
                            }
                            let _ = reply.send(res);
                        }
                        Some(Request::Settings { settings, reply }) => {
                            let _ = reply.send(self.change_settings(*settings).await);
                        }
                    }
                }
                _ = self.shared.status_changed.notified() => self.refresh_status(),
                _ = self.shared.captions.changed.notified() => self.sync_captions(),
                _ = tick.tick() => {
                    self.expire_identify();
                    self.sync_captions();
                    self.refresh_status();
                }
                _ = media_tick.tick() => self.media_tick().await,
                _ = timer_tick.tick() => self.timer_tick(),
                _ = sleep_until(self.sound_recheck), if self.sound_recheck.is_some() => {
                    self.sound_recheck = None;
                    self.sync_element_sounds();
                }
            }
        }
        tracing::debug!("show actor stopped");
    }

    async fn apply(&mut self, command: Command) -> Result<u64, ApiError> {
        let data = self.shared.data();
        let before = self.state.clone();
        let d: &ShowData = &data;
        let r: Result<(), ShowError> = match command.clone() {
            Command::TriggerSlide {
                presentation,
                slide,
                index,
                playlist_item,
                step,
                transition,
            } => self.state.trigger(
                d,
                presentation,
                slide,
                index.map(|i| i as usize),
                playlist_item,
                step.map(|k| k as usize),
                transition.as_ref(),
            ),
            Command::Next => self.state.step(d, 1).map(|_| ()),
            Command::Prev => self.state.step(d, -1).map(|_| ()),
            Command::TriggerPlaylistItem { playlist, item } => {
                self.state.trigger_playlist_item(d, playlist, item)
            }
            Command::ClearLayer { layer } => {
                if layer == Layer::Captions {
                    self.shared.captions.clear();
                }
                self.state.clear_layer(d, layer)
            }
            Command::RestoreLayer { layer } => self.state.restore_layer(d, layer),
            Command::ClearAll => {
                self.state.clear_all(d);
                Ok(())
            }
            Command::ClearGroup { id } => self.state.clear_group(d, id),
            Command::SetLook { look } => self.state.set_look(d, look),
            Command::SetClockFormat {
                use_24_hour,
                seconds,
            } => {
                let mut f = self.state.clock;
                f.use_24_hour = use_24_hour.unwrap_or(f.use_24_hour);
                f.seconds = seconds.unwrap_or(f.seconds);
                if f != self.state.clock {
                    self.state.clock = f;
                    let root = d.root.clone();
                    let saved = tokio::task::spawn_blocking(move || {
                        crate::show::save_clock_format(&root, f)
                    })
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(|r| r);
                    if let Err(e) = saved {
                        tracing::warn!("the clock format is not saved for the next start: {e:#}");
                    }
                }
                Ok(())
            }
            Command::TriggerMedia {
                asset,
                layer,
                transition,
            } => self
                .state
                .trigger_media(d, asset, layer, transition.as_ref()),
            Command::TriggerLiveSource { source, transition } => {
                self.state
                    .trigger_live_source(d, &source, transition.as_ref())
            }
            Command::SetScreensEnabled { role, enabled } => {
                self.state.screens_enabled.set(role, enabled);
                Ok(())
            }
            Command::SetStageLayout { screen, layout } => {
                let Some(sc) = d.screens.iter().find(|s| s.id == screen) else {
                    return Err(ApiError::new(
                        ErrorCode::NotFound,
                        format!("screen {} not found", screen.0),
                    ));
                };
                if !matches!(sc.kind, model::ScreenKind::Stage { .. }) {
                    return Err(ApiError::new(
                        ErrorCode::BadRequest,
                        format!("screen {:?} is not a stage screen", sc.name),
                    ));
                }
                if let Some(l) = layout
                    && !d.content.stage_layouts.iter().any(|x| x.id == l)
                {
                    return Err(ApiError::new(
                        ErrorCode::NotFound,
                        format!("stage layout {} not found", l.0),
                    ));
                }
                let previous = self.state.stage_layouts.clone();
                self.state.stage_layouts.retain(|(s, _)| *s != Some(screen));
                if let Some(l) = layout {
                    self.state.stage_layouts.push((Some(screen), l));
                }
                // Stored with the screen, so the choice survives a restart.
                let kind = model::ScreenKind::Stage { layout };
                if sc.kind != kind {
                    let mut doc = sc.clone();
                    doc.kind = kind;
                    let body = serde_json::to_value(&doc)
                        .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?;
                    let op = crate::config::ConfigOp::UpdateScreen {
                        id: screen,
                        body,
                        if_match: None,
                    };
                    if let Err(e) = self.configure(op).await {
                        // Not saved: the choice is not made (a later broadcast must not
                        // show it).
                        self.state.stage_layouts = previous;
                        return Err(e.error);
                    }
                }
                Ok(())
            }
            Command::StageMessage { text, flash } => {
                if text.trim().is_empty() {
                    return Err(ApiError::new(
                        ErrorCode::BadRequest,
                        "the stage message is empty",
                    ));
                }
                if text.chars().count() > MAX_STAGE_MESSAGE {
                    return Err(ApiError::new(
                        ErrorCode::BadRequest,
                        format!("the stage message is longer than {MAX_STAGE_MESSAGE} characters"),
                    ));
                }
                self.state.stage_message = Some(crate::stage::StageMessage {
                    text,
                    flash,
                    shown_at: Instant::now(),
                });
                Ok(())
            }
            Command::StageMessageClear => {
                self.state.stage_message = None;
                Ok(())
            }
            Command::IdentifyScreens { enabled } => {
                self.state.identify = enabled;
                self.identify_until = enabled.then(|| Instant::now() + IDENTIFY_FOR);
                Ok(())
            }
            Command::SetTestPattern { screen, pattern } => {
                if !d.screens.iter().any(|s| s.id == screen) {
                    return Err(ApiError::new(
                        ErrorCode::NotFound,
                        format!("screen {} not found", screen.0),
                    ));
                }
                match pattern {
                    Some(p) => {
                        self.state.test_patterns.insert(screen, p);
                    }
                    None => {
                        self.state.test_patterns.remove(&screen);
                    }
                }
                Ok(())
            }
            Command::SetOption {
                continuous_playlist,
            } => {
                if let Some(c) = continuous_playlist {
                    self.state.options.continuous_playlist = c;
                }
                Ok(())
            }
            Command::ReloadData => return self.reload().await,
            Command::TimerStart { timer } => {
                self.state.timer(d, timer, TimerOp::Start, Clock::now())
            }
            Command::TimerStop { timer } => self.state.timer(d, timer, TimerOp::Stop, Clock::now()),
            Command::TimerReset { timer } => {
                self.state.timer(d, timer, TimerOp::Reset, Clock::now())
            }
            Command::TimerSet {
                timer,
                seconds,
                time_of_day,
            } => match (seconds, time_of_day) {
                (Some(s), None) => self.state.timer(d, timer, TimerOp::Set(s), Clock::now()),
                (None, Some(t)) => self
                    .state
                    .timer(d, timer, TimerOp::TimeOfDay(t), Clock::now()),
                _ => Err(ShowError::BadRequest(
                    "give exactly one of `seconds` and `timeOfDay`".into(),
                )),
            },
            Command::TimerAdjust {
                timer,
                delta_seconds,
            } => self
                .state
                .timer(d, timer, TimerOp::Adjust(delta_seconds), Clock::now()),
            Command::PropShow { prop } => self.state.show_prop(d, prop),
            Command::PropHide { prop } => {
                if !d.content.props.iter().any(|p| p.id == prop) {
                    return Err(ApiError::new(
                        ErrorCode::NotFound,
                        format!("prop {} not found", prop.0),
                    ));
                }
                self.state.hide_prop(d, prop);
                Ok(())
            }
            Command::PropToggle { prop } => self.state.toggle_prop(d, prop),
            Command::TriggerMacro { macro_id } => {
                self.state.trigger_macro(d, macro_id, Clock::now())
            }
            Command::MessageShow { message, tokens } => self.state.show_message(d, message, tokens),
            Command::MessageHide { message } => {
                if let Some(id) = message
                    && !d.content.messages.iter().any(|m| m.id == id)
                {
                    return Err(ApiError::new(
                        ErrorCode::NotFound,
                        format!("message {} not found", id.0),
                    ));
                }
                self.state.hide_message(d, message);
                Ok(())
            }
            Command::MediaPlay { layer }
            | Command::MediaPause { layer }
            | Command::MediaToggle { layer }
            | Command::MediaSeek { layer, .. }
            | Command::MediaSkip { layer, .. }
            | Command::MediaRestart { layer }
            | Command::MediaStop { layer } => self.transport(d, &command, layer),
            Command::MediaSetLoop { looping, layer } => self.set_loop(layer, looping),
            Command::AudioNext => self.state.audio_step(d, 1).map(|_| ()),
            Command::AudioPrev => {
                let now = Instant::now();
                let restart = self
                    .state
                    .audio
                    .as_ref()
                    .and_then(|m| m.play)
                    .and_then(|p| self.shared.live_audio(p))
                    .filter(|a| a.position_at(now) > 3.0);
                match restart {
                    Some(a) => {
                        a.restart(now);
                        self.transport_changed = true;
                        Ok(())
                    }
                    None => self.state.audio_step(d, -1).map(|_| ()),
                }
            }
            Command::SetAudioOptions { shuffle, repeat } => {
                self.state.set_audio_options(shuffle, repeat);
                Ok(())
            }
            Command::SetVolume {
                target,
                layer,
                output,
                db,
            } => {
                if !db.is_finite() {
                    return Err(ApiError::new(ErrorCode::BadRequest, "db must be a number"));
                }
                self.set_level(target, layer, output, Some(db), None)
            }
            Command::SetMute {
                target,
                layer,
                output,
                muted,
            } => self.set_level(target, layer, output, None, Some(muted)),
            Command::PlayTestTone { frequency, seconds } => {
                let f = frequency.unwrap_or(1000.0);
                let secs = seconds.unwrap_or(2.0);
                if !f.is_finite() || !(20.0..=20_000.0).contains(&f) {
                    return Err(ApiError::new(
                        ErrorCode::BadRequest,
                        "frequency must be 20 – 20000 Hz",
                    ));
                }
                if !secs.is_finite() || !(0.0..=30.0).contains(&secs) {
                    return Err(ApiError::new(
                        ErrorCode::BadRequest,
                        "seconds must be 0 – 30",
                    ));
                }
                self.shared.sound.test_tone(f, secs);
                Ok(())
            }
            Command::TriggerVideoInput { input } => {
                self.state.trigger_video_input(d, input, None, None, None)
            }
            Command::SetTransitionOverride { layer, transition } => {
                if !layer.is_visual() {
                    return Err(ApiError::new(
                        ErrorCode::BadRequest,
                        format!("{layer:?} has no transitions"),
                    ));
                }
                match transition {
                    Some(t) => {
                        self.state.transition_overrides.insert(layer, t);
                    }
                    None => {
                        self.state.transition_overrides.remove(&layer);
                    }
                }
                Ok(())
            }
            Command::SubscribePreview { .. } | Command::UnsubscribePreview { .. } => {
                return Err(ApiError::new(
                    ErrorCode::WsOnly,
                    "preview subscriptions need the WebSocket",
                ));
            }
            Command::PrefetchThumbnails { .. } => {
                return Err(ApiError::new(
                    ErrorCode::Internal,
                    "prefetchThumbnails is handled by the API layer",
                ));
            }
        };
        r?;
        self.want_media(&before);
        self.sync_videos().await;
        self.sync_audio().await;
        self.sync_element_sounds();
        self.sync_live_sound();
        if self.state != before || self.transport_changed {
            self.transport_changed = false;
            self.shared.publish(self.state.clone());
            self.broadcast();
        } else if self.position_key(Instant::now()) != self.last_position {
            // A command answers with a fresh transport position.
            self.broadcast();
        }
        self.retire_videos();
        // A presentation that just became the navigation position: tell the operator about
        // its missing media (once per presentation; file checks off the actor).
        if let Some(id) = self.state.position.map(|p| p.presentation)
            && Some(id) != before.position.map(|p| p.presentation)
        {
            let sh = self.shared.clone();
            tokio::task::spawn_blocking(move || sh.announce_presentation_media(id));
        }
        Ok(self.revision)
    }

    /// On a show box: media just triggered that isn't on this box yet is copied now (before
    /// the scheduled media still missing); until it is here the cue behaves like missing media,
    /// and the operator is told it is still copying.
    fn want_media(&self, before: &ShowState) {
        if !self.shared.library.is_remote() {
            return;
        }
        let new = |now: &Option<crate::show::LiveMedia>, was: &Option<crate::show::LiveMedia>| {
            now.as_ref()
                .filter(|m| was.as_ref().map(|w| (w.asset, w.play)) != Some((m.asset, m.play)))
                .map(|m| m.asset)
        };
        let assets: Vec<model::MediaAssetId> = [
            new(&self.state.media, &before.media),
            new(&self.state.slide_media, &before.slide_media),
            new(&self.state.audio, &before.audio),
        ]
        .into_iter()
        .flatten()
        .collect();
        if assets.is_empty() {
            return;
        }
        for name in crate::codex::mirror::want(&*self.shared, &assets) {
            self.shared.notice_keyed(
                NoticeLevel::Warning,
                format!("“{name}” is still copying to this box: it plays once it is here (trigger it again then)"),
                Some(format!("copying:{name}")),
            );
        }
    }

    /// Opens players for the plays the state references and drops the others (see
    /// [`crate::playback`]). A video that cannot be opened is announced and its layer treated
    /// as missing media.
    async fn sync_videos(&mut self) {
        let data = self.shared.data();
        let mut wanted: Vec<(Layer, crate::show::LiveMedia)> = Vec::new();
        for layer in [Layer::Media, Layer::Slide] {
            if let Some(m) = self.state.layer_media(layer)
                && m.play.is_some()
            {
                wanted.push((layer, m.clone()));
            }
        }
        // The others go once the state without them is published (see `retired_videos`).
        let keep = self.kept_videos();
        {
            let reg = lock(&self.shared.videos);
            for k in reg.keys() {
                if !keep.contains(k) && !self.retired_videos.contains(k) {
                    self.retired_videos.push(*k);
                }
            }
        }
        for (layer, m) in wanted {
            let Some(play) = m.play else { continue };
            let cont = self.state.continues.get(&play).copied();
            if cont.is_none() && self.shared.video(play).is_some() {
                continue;
            }
            let start = cont.map_or(0.0, |c| c.at);
            let name = data.content.asset_name(m.asset);
            let opened = match data.content.asset_path(m.asset) {
                Ok(path) => {
                    // The asset's trim is the use's default range (media preparation).
                    let pb = match data.content.assets.get(&m.asset) {
                        Some(a) => a.trimmed(&m.playback),
                        None => m.playback.clone(),
                    };
                    let (asset, fade) = (m.asset, data.settings.default_media_fade);
                    let n = name.clone();
                    let ch = self.shared.sound.channels();
                    tokio::task::spawn_blocking(move || {
                        crate::playback::LiveVideo::open(
                            play,
                            asset,
                            n,
                            layer,
                            &path,
                            &pb,
                            fade,
                            Some(ch),
                            start,
                        )
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))
                    .and_then(|r| r)
                }
                Err(why) => Err(anyhow::anyhow!(why)),
            };
            // The play it continues (still in the registry under both IDs) goes now.
            let old = cont.and_then(|c| {
                self.state.continues.remove(&play);
                lock(&self.shared.videos).remove(&c.from)
            });
            if let Some(o) = &old {
                lock(&self.shared.videos).remove(&play);
                self.shared
                    .sound
                    .engine
                    .remove_source(o.play, CONTINUE_FADE);
            }
            if let Some(o) = old {
                tokio::task::spawn_blocking(move || drop(o));
            }
            match opened {
                Ok(v) => {
                    let now = Instant::now();
                    match v.take_audio() {
                        Some((tap, sync)) => {
                            // Starts when its first sound is heard (the output's latency
                            // from now), so the sound starts at its first sample.
                            v.start_at(now + self.shared.sound.latency(), start);
                            let src = crate::sound::PlaySource::new(
                                tap,
                                sync,
                                v.sound_fade_in(),
                                v.sound_fade_out(),
                            );
                            self.shared.sound.engine.add_source(
                                play,
                                Box::new(src),
                                crate::sound::params_of(layer, &levelled(&data, &m)),
                                if cont.is_some() {
                                    CONTINUE_FADE
                                } else {
                                    self.layer_fade(layer)
                                },
                            );
                        }
                        None => v.start_at(now, start),
                    }
                    if cont.is_some_and(|c| c.paused) {
                        v.pause(now);
                    }
                    lock(&self.shared.videos).insert(play, Arc::new(v));
                    // The layer's transition starts when its video can show frames, not when
                    // the trigger arrived (opening took a moment). A continued play is no
                    // change of the layer.
                    if cont.is_none()
                        && let Some(c) = self.state.changes.get_mut(&layer)
                    {
                        c.at = now;
                    }
                }
                Err(e) => {
                    self.shared.notice_keyed(
                        NoticeLevel::Warning,
                        format!("cannot play {name}: {e:#}"),
                        Some(format!("video:{}", m.asset.0)),
                    );
                    if let Some(slot) = self.state.layer_media_mut(layer) {
                        slot.play = None;
                    }
                }
            }
        }
    }

    /// The plays the state references: its layers' videos, and plays being continued (they
    /// keep playing until their successor is open).
    fn kept_videos(&mut self) -> Vec<u64> {
        self.state.prune_continues();
        let mut keep: Vec<u64> = [Layer::Media, Layer::Slide]
            .into_iter()
            .filter_map(|l| self.state.layer_media(l).and_then(|m| m.play))
            .collect();
        keep.extend(self.state.continues.values().map(|c| c.from));
        keep
    }

    /// Drops the video plays [`sync_videos`](Self::sync_videos) retired, once the state
    /// without them is published (call after `publish`, or when nothing was published).
    fn retire_videos(&mut self) {
        if self.retired_videos.is_empty() {
            return;
        }
        let keep = self.kept_videos();
        let dropped: Vec<Arc<crate::playback::LiveVideo>> = {
            let mut reg = lock(&self.shared.videos);
            std::mem::take(&mut self.retired_videos)
                .into_iter()
                .filter(|k| !keep.contains(k))
                .filter_map(|k| reg.remove(&k))
                .collect()
        };
        // Their sound fades out with the layer's change (a cut: a short de-click ramp).
        for v in &dropped {
            let fade = self.layer_fade(v.layer);
            self.shared.sound.engine.remove_source(v.play, fade);
        }
        // Stopping a decode thread joins it: off the async threads.
        if !dropped.is_empty() {
            tokio::task::spawn_blocking(move || drop(dropped));
        }
    }

    /// Keeps the live slide's video elements with sound in step with the state (see
    /// [`crate::element_video`]); before the state is published, so the render thread finds
    /// their plays.
    fn sync_element_sounds(&mut self) {
        let data = self.shared.data();
        let step0 = self.step0.follow(&data, &self.state);
        let (wanted, recheck) =
            crate::element_video::sounding(&self.shared, &data, &self.state, step0, Instant::now());
        self.sound_recheck = recheck;
        let fade = self.layer_fade(Layer::Slide);
        let change = self.state.changes.get(&Layer::Slide);
        let run = change.map_or(0, |c| c.seq);
        let since = change.map(|c| c.at);
        crate::element_video::sync_plays(&self.shared, wanted, fade, run, since);
    }

    /// Keeps the video-input layer's live sound in step with the state (see
    /// [`crate::live_sound`]).
    fn sync_live_sound(&mut self) {
        let want = self
            .state
            .video_input
            .as_ref()
            .map(|v| (v.source.clone(), v.audio.clone()));
        crate::live_sound::sync(&self.shared, want, self.layer_fade(Layer::VideoInput));
    }

    /// The fade of a layer's sound for its last change: the dissolve's length (0 for cuts).
    fn layer_fade(&self, layer: Layer) -> f64 {
        self.state
            .changes
            .get(&layer)
            .map(|c| c.transition.renderable())
            .filter(|t| t.kind != model::TransitionKind::Cut)
            .map(|t| t.duration)
            .unwrap_or(0.0)
    }

    /// Opens the audio layer's play and drops the others (see [`crate::sound`]): the new
    /// file cross-fades with the old one (its fade-in, else the show's default media fade);
    /// a cleared layer fades out (the play's fade-out, else the default).
    async fn sync_audio(&mut self) {
        let mut skipped = 0;
        while self.sync_audio_once(skipped).await {
            skipped += 1;
        }
    }

    /// One pass of [`sync_audio`](Self::sync_audio). `true` when the audio playlist's item
    /// could not be opened and the playlist moved on to its next item (after `skipped` such
    /// skips, at most one round of the playlist): that one is opened next.
    async fn sync_audio_once(&mut self, skipped: usize) -> bool {
        let data = self.shared.data();
        let default = data.settings.default_media_fade;
        let want = self.state.audio.clone().filter(|m| m.play.is_some());
        let keep = want.as_ref().and_then(|m| m.play);
        // A play being continued (`mediaSetLoop`) keeps playing until its successor is open.
        self.state.prune_continues();
        let cont = keep.and_then(|p| self.state.continues.get(&p).copied());
        let dropped: Vec<Arc<crate::sound::LiveAudio>> = {
            let mut reg = lock(&self.shared.audios);
            let gone: Vec<u64> = reg
                .keys()
                .filter(|k| Some(**k) != keep && Some(**k) != cont.map(|c| c.from))
                .copied()
                .collect();
            gone.iter().filter_map(|k| reg.remove(k)).collect()
        };
        let replacing =
            !dropped.is_empty() && keep.is_some_and(|p| self.shared.live_audio(p).is_none());
        let cross = want
            .as_ref()
            .map(|m| m.playback.fade_in.effective(default).unwrap_or(default))
            .unwrap_or(default);
        for a in &dropped {
            let fade = if replacing {
                cross
            } else {
                a.playback.fade_out.effective(default).unwrap_or(default)
            };
            self.shared.sound.engine.remove_source(a.play, fade);
        }
        if !dropped.is_empty() {
            tokio::task::spawn_blocking(move || drop(dropped));
        }
        let Some(m) = want else { return false };
        let Some(play) = m.play else { return false };
        if cont.is_none() && self.shared.live_audio(play).is_some() {
            return false;
        }
        let start = cont.map_or(0.0, |c| c.at);
        let name = data.content.asset_name(m.asset);
        let opened = match data.content.asset_path(m.asset) {
            Ok(path) => {
                let pb = match data.content.assets.get(&m.asset) {
                    Some(a) => a.trimmed(&m.playback),
                    None => m.playback.clone(),
                };
                let (asset, au, n) = (m.asset, levelled(&data, &m), name.clone());
                let ch = self.shared.sound.channels();
                tokio::task::spawn_blocking(move || {
                    crate::sound::LiveAudio::open(play, asset, n, &path, &pb, &au, ch, start)
                })
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))
                .and_then(|r| r)
            }
            Err(why) => Err(anyhow::anyhow!(why)),
        };
        if let Some(c) = cont {
            self.state.continues.remove(&play);
            let old = {
                let mut reg = lock(&self.shared.audios);
                reg.remove(&play);
                reg.remove(&c.from)
            };
            if let Some(o) = old {
                self.shared
                    .sound
                    .engine
                    .remove_source(o.play, CONTINUE_FADE);
                tokio::task::spawn_blocking(move || drop(o));
            }
        }
        match opened {
            Ok((a, tap)) => {
                let now = Instant::now();
                a.start_at(now + self.shared.sound.latency(), start);
                if cont.is_some_and(|c| c.paused) {
                    a.pause(now);
                }
                let fade_in = if cont.is_some() {
                    CONTINUE_FADE
                } else if replacing {
                    cross
                } else {
                    m.playback.fade_in.effective(default).unwrap_or(0.0)
                };
                let src = crate::sound::PlaySource::new(tap, a.sync(), 0.0, None);
                self.shared.sound.engine.add_source(
                    play,
                    Box::new(src),
                    crate::sound::params_of(Layer::Audio, &levelled(&data, &m)),
                    fade_in,
                );
                lock(&self.shared.audios).insert(play, Arc::new(a));
                false
            }
            Err(e) => {
                self.shared.notice_keyed(
                    NoticeLevel::Warning,
                    format!("cannot play {name}: {e:#}"),
                    Some(format!("audio:{}", m.asset.0)),
                );
                if let Some(slot) = &mut self.state.audio {
                    slot.play = None;
                }
                // An audio playlist goes on with its next item.
                self.state.audio_playlist.is_some()
                    && skipped < self.state.audio_items(&data).len()
                    && matches!(self.state.audio_step(&data, 1), Ok(true))
            }
        }
    }

    /// `setVolume` / `setMute`.
    fn set_level(
        &mut self,
        target: VolumeTarget,
        layer: Option<Layer>,
        output: Option<model::OutputId>,
        db: Option<f64>,
        muted: Option<bool>,
    ) -> Result<(), ShowError> {
        let hub = &self.shared.sound;
        match target {
            VolumeTarget::Master => {
                if let Some(d) = db {
                    hub.engine
                        .set_master(d.clamp(crate::sound::MIN_DB, crate::sound::MAX_DB));
                }
                if let Some(m) = muted {
                    hub.engine.set_mute(m);
                }
            }
            VolumeTarget::Layer => {
                let l = layer.ok_or_else(|| ShowError::BadRequest("give `layer`".into()))?;
                if !hub.set_layer(l, db, muted) {
                    return Err(ShowError::BadRequest(format!(
                        "the {l:?} layer has no sound (audio, media, slide or videoInput)"
                    )));
                }
            }
            VolumeTarget::Output => {
                let id = output.ok_or_else(|| ShowError::BadRequest("give `output`".into()))?;
                if !self.shared.data().outputs.iter().any(|b| b.id == id) {
                    return Err(ShowError::NotFound(format!("output {} not found", id.0)));
                }
                hub.set_output(id, db, muted);
            }
        }
        self.transport_changed = true;
        Ok(())
    }

    /// The layer a transport command acts on: the given one, else the transport target.
    fn transport_target(&self, layer: Option<Layer>) -> Option<Arc<crate::playback::LiveVideo>> {
        let layers = match layer {
            Some(l) => vec![l],
            None => vec![Layer::Slide, Layer::Media],
        };
        layers.into_iter().find_map(|l| {
            self.state
                .layer_media(l)
                .and_then(|m| m.play)
                .and_then(|p| self.shared.video(p))
        })
    }

    /// Applies a transport command (`mediaPlay`, `mediaSeek`, ...).
    fn transport(
        &mut self,
        data: &ShowData,
        command: &Command,
        layer: Option<Layer>,
    ) -> Result<(), ShowError> {
        if let Some(l) = layer
            && !matches!(l, Layer::Media | Layer::Slide | Layer::Audio)
        {
            return Err(ShowError::BadRequest(format!(
                "no transport on the {l:?} layer (media, slide or audio)"
            )));
        }
        let video = if layer == Some(Layer::Audio) {
            None
        } else {
            self.transport_target(layer)
        };
        let Some(video) = video else {
            if layer.is_none() || layer == Some(Layer::Audio) {
                return self.audio_transport(data, command);
            }
            return Err(ShowError::Conflict("no video is live".into()));
        };
        let now = Instant::now();
        match command {
            Command::MediaPlay { .. } => video.play(now),
            Command::MediaPause { .. } => video.pause(now),
            Command::MediaToggle { .. } => {
                if video.state_at(now) == PlaybackState::Playing {
                    video.pause(now)
                } else {
                    video.play(now)
                }
            }
            Command::MediaSeek { seconds, .. } => {
                if !seconds.is_finite() {
                    return Err(ShowError::BadRequest("seconds must be a number".into()));
                }
                video.seek_position(*seconds, now)
            }
            Command::MediaSkip { seconds, .. } => {
                if !seconds.is_finite() {
                    return Err(ShowError::BadRequest("seconds must be a number".into()));
                }
                video.skip(*seconds, now)
            }
            Command::MediaRestart { .. } => video.restart(now),
            Command::MediaStop { .. } => return self.state.clear_layer(data, video.layer),
            _ => {}
        }
        // A play that had ended runs again.
        for l in [Layer::Media, Layer::Slide] {
            if let Some(m) = self.state.layer_media_mut(l)
                && m.play == Some(video.play)
            {
                m.ended = video.state_at(now) == PlaybackState::Stopped;
            }
        }
        self.transport_changed = true;
        Ok(())
    }

    /// `mediaSetLoop`: the play continues under a new play ID with the new repeat; until that
    /// one is open, its ID names the old player (see [`crate::playback`]).
    fn set_loop(&mut self, layer: Option<Layer>, looping: bool) -> Result<(), ShowError> {
        if let Some(l) = layer
            && !matches!(l, Layer::Media | Layer::Slide | Layer::Audio)
        {
            return Err(ShowError::BadRequest(format!(
                "no transport on the {l:?} layer (media, slide or audio)"
            )));
        }
        let now = Instant::now();
        let video = if layer == Some(Layer::Audio) {
            None
        } else {
            self.transport_target(layer)
        };
        if let Some(v) = video {
            let paused = v.state_at(now) == PlaybackState::Paused;
            if let Some((_, new)) =
                self.state
                    .set_loop(v.layer, looping, v.position_at(now), paused)?
            {
                lock(&self.shared.videos).insert(new, v);
            }
        } else if layer.is_none() || layer == Some(Layer::Audio) {
            let a = self
                .state
                .audio
                .as_ref()
                .and_then(|m| m.play)
                .and_then(|p| self.shared.live_audio(p))
                .ok_or_else(|| ShowError::Conflict("nothing is playing".into()))?;
            let paused = a.state_at(now) == PlaybackState::Paused;
            if let Some((_, new)) =
                self.state
                    .set_loop(Layer::Audio, looping, a.position_at(now), paused)?
            {
                lock(&self.shared.audios).insert(new, a);
            }
        } else {
            return Err(ShowError::Conflict("no video is live".into()));
        }
        self.transport_changed = true;
        Ok(())
    }

    /// A transport command on the audio layer.
    fn audio_transport(&mut self, data: &ShowData, command: &Command) -> Result<(), ShowError> {
        let a = self
            .state
            .audio
            .as_ref()
            .and_then(|m| m.play)
            .and_then(|p| self.shared.live_audio(p))
            .ok_or_else(|| ShowError::Conflict("nothing is playing".into()))?;
        let now = Instant::now();
        match command {
            Command::MediaPlay { .. } => a.play(now),
            Command::MediaPause { .. } => a.pause(now),
            Command::MediaToggle { .. } => {
                if a.playing(now) {
                    a.pause(now)
                } else {
                    a.play(now)
                }
            }
            Command::MediaSeek { seconds, .. } => {
                if !seconds.is_finite() {
                    return Err(ShowError::BadRequest("seconds must be a number".into()));
                }
                a.seek_position(*seconds, now)
            }
            Command::MediaSkip { seconds, .. } => {
                if !seconds.is_finite() {
                    return Err(ShowError::BadRequest("seconds must be a number".into()));
                }
                a.skip(*seconds, now)
            }
            Command::MediaRestart { .. } => a.restart(now),
            Command::MediaStop { .. } => return self.state.clear_layer(data, Layer::Audio),
            _ => {}
        }
        if let Some(m) = &mut self.state.audio {
            m.ended = a.state_at(now) == PlaybackState::Stopped;
        }
        self.transport_changed = true;
        Ok(())
    }

    /// While videos are live: applies end behaviours (clear the layer, mark ended) and
    /// broadcasts the transport position (at most every [`POSITION_REFRESH`]).
    async fn media_tick(&mut self) {
        if lock(&self.shared.videos).is_empty() && lock(&self.shared.audios).is_empty() {
            if self.last_position.take().is_some() {
                self.broadcast();
            }
            return;
        }
        let now = Instant::now();
        let before = self.state.clone();
        // The audio layer: at the end, the next playlist item (or the layer clears).
        if let Some(m) = self.state.audio.clone()
            && let Some(a) = m.play.and_then(|p| self.shared.live_audio(p))
            && a.state_at(now) == PlaybackState::Stopped
        {
            let data = self.shared.data();
            let advanced = self.state.audio_playlist.is_some()
                && matches!(self.state.audio_step(&data, 1), Ok(true));
            if !advanced {
                self.state.audio = None;
                self.state.audio_playlist = None;
                self.state
                    .record_change(Layer::Audio, model::Transition::CUT);
            }
        }
        for layer in [Layer::Media, Layer::Slide] {
            let Some(m) = self.state.layer_media(layer).cloned() else {
                continue;
            };
            let Some(v) = m.play.and_then(|p| self.shared.video(p)) else {
                continue;
            };
            let ended = v.state_at(now) == PlaybackState::Stopped;
            if ended && !m.ended {
                match m.playback.end_behavior {
                    model::EndBehavior::Clear | model::EndBehavior::FadeToClear => {
                        // The screens already show nothing there: no transition.
                        match layer {
                            Layer::Media => self.state.media = None,
                            _ => self.state.slide_media = None,
                        }
                        self.state.record_change(layer, model::Transition::CUT);
                    }
                    _ => {
                        if let Some(s) = self.state.layer_media_mut(layer) {
                            s.ended = true;
                        }
                    }
                }
            } else if !ended
                && m.ended
                && let Some(s) = self.state.layer_media_mut(layer)
            {
                s.ended = false;
            }
        }
        if self.state != before {
            self.sync_videos().await;
            self.sync_audio().await;
            self.sync_element_sounds();
            self.sync_live_sound();
            self.shared.publish(self.state.clone());
            self.broadcast();
            self.retire_videos();
            return;
        }
        // Video and audio states (playing, ended, looping) show and hide video elements with
        // media conditions: so are their sounds.
        if !lock(&self.shared.element_plays).is_empty() {
            self.sync_element_sounds();
        }
        // Position refresh: only while it moves (or its state changed).
        let key = self.position_key(now);
        if key != self.last_position {
            self.broadcast();
        }
    }

    /// (play, state, position in tenths) of the transport target and the audio layer.
    fn position_key(&self, now: Instant) -> Option<(u64, PlaybackState, i64)> {
        let v = self.transport_target(None).map(|v| {
            (
                v.play,
                v.state_at(now),
                (v.position_at(now) * 10.0).round() as i64,
            )
        });
        let a = self
            .state
            .audio
            .as_ref()
            .and_then(|m| m.play)
            .and_then(|p| self.shared.live_audio(p))
            .map(|a| {
                (
                    a.play,
                    a.state_at(now),
                    (a.position_at(now) * 10.0).round() as i64,
                )
            });
        match (v, a) {
            (Some(v), Some(a)) => Some((
                v.0 ^ a.0.rotate_left(32),
                if v.1 == PlaybackState::Playing {
                    v.1
                } else {
                    a.1
                },
                v.2 * 100_000 + a.2,
            )),
            (v, a) => v.or(a),
        }
    }

    /// Turns `identify` off once its time is up.
    fn expire_identify(&mut self) {
        if let Some(t) = self.identify_until
            && Instant::now() >= t
        {
            self.identify_until = None;
            if self.state.identify {
                self.state.identify = false;
                self.shared.publish(self.state.clone());
                self.broadcast();
            }
        }
    }

    /// Saves the live look for the next start when a command changed it (`setLook`, a slide's
    /// look action). A failed save is logged (once per change); the look stays live.
    async fn save_look(&mut self) {
        let look = self.state.look;
        if look == self.saved_look {
            return;
        }
        self.saved_look = look;
        let root = self.shared.data().root.clone();
        let saved = tokio::task::spawn_blocking(move || crate::show::save_live_look(&root, look))
            .await
            .map_err(anyhow::Error::from)
            .and_then(|r| r);
        if let Err(e) = saved {
            tracing::warn!("the live look is not saved for the next start: {e:#}");
        }
    }

    /// Validates, persists and applies a configuration change (see [`crate::config`]).
    async fn configure(&mut self, op: ConfigOp) -> Result<ConfigResult, ConfigError> {
        let data = self.shared.data();
        if let ConfigOp::DeleteLook { id, .. } = &op
            && self.state.look == Some(*id)
        {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "this look is live; make another look live first",
            )
            .into());
        }
        let plan = crate::config::plan(&data, &self.shared.displays.list(), op)?;
        if plan.writes.is_empty() {
            return Ok(plan.result);
        }
        let root = data.root.clone();
        let writes = plan.writes.clone();
        tokio::task::spawn_blocking(move || crate::store::apply(&root, &writes))
            .await
            .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?
            .map_err(|e| match e {
                crate::store::StoreError::TooNew(m) => ApiError::new(ErrorCode::Conflict, m),
                crate::store::StoreError::Io(e) => ApiError::new(
                    ErrorCode::Internal,
                    format!("saving the configuration failed: {e:#}"),
                ),
            })?;
        let new = Arc::new(data.with_config(plan.screens, plan.looks, plan.masks, plan.outputs));
        // Only the configuration changed: a held copy of a deleted live slide stays on screen.
        self.state.revalidate_config(&new);
        // A saved edit of the live look plays the look's transition, like making it live. The
        // state carrying it goes out with the new data (before the engine reconfigures, which
        // wakes the render thread), so the render that shows the edit plays it.
        let look_edited = if let Some(live) = self.state.look
            && plan
                .changes
                .iter()
                .any(|(k, id, _)| *k == DocumentKind::Look && *id == live.0)
        {
            let t = new
                .looks
                .iter()
                .find(|l| l.id == live)
                .and_then(|l| l.transition.clone())
                .unwrap_or(model::Transition::CUT);
            self.state.record_look_change(t);
            true
        } else {
            false
        };
        self.shared.set_data(new.clone());
        // R2-1: the looks and screens decide which element videos the screens draw.
        self.sync_element_sounds();
        if look_edited {
            self.shared.publish(self.state.clone());
        }
        let (sh, d) = (self.shared.clone(), new.clone());
        tokio::task::spawn_blocking(move || crate::engine::apply_config(&sh, &d))
            .await
            .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?;
        self.shared.publish(self.state.clone());
        self.broadcast();
        for (kind, id, change) in &plan.changes {
            let _ = self
                .shared
                .events
                .send(Arc::new(ServerMessage::DocumentChanged {
                    kind: *kind,
                    id: id.to_string(),
                    change: *change,
                    revision: None,
                    client_id: None,
                    ops: None,
                    collection: None,
                }));
        }
        Ok(plan.result)
    }

    /// Validates, persists and applies a change of the video inputs (see [`crate::inputs`]):
    /// a live input follows its new binding at once.
    async fn change_inputs(
        &mut self,
        op: crate::inputs::InputOp,
    ) -> Result<crate::inputs::InputResult, ConfigError> {
        let data = self.shared.data();
        let plan = crate::inputs::plan(&data, op)?;
        if plan.writes.is_empty() {
            return Ok(plan.result);
        }
        let root = data.root.clone();
        let writes = plan.writes.clone();
        tokio::task::spawn_blocking(move || crate::store::apply(&root, &writes))
            .await
            .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?
            .map_err(|e| match e {
                crate::store::StoreError::TooNew(m) => ApiError::new(ErrorCode::Conflict, m),
                crate::store::StoreError::Io(e) => ApiError::new(
                    ErrorCode::Internal,
                    format!("saving the video input failed: {e:#}"),
                ),
            })?;
        let new = Arc::new(data.with_inputs(plan.video_inputs, plan.input_bindings));
        self.state.follow_inputs(&new);
        self.shared.set_data(new);
        self.sync_live_sound();
        // F01-3: a binding can flip a slide element's video-input condition.
        self.sync_element_sounds();
        self.shared.publish(self.state.clone());
        self.broadcast();
        for (kind, id, change) in &plan.changes {
            let _ = self
                .shared
                .events
                .send(Arc::new(ServerMessage::DocumentChanged {
                    kind: *kind,
                    id: id.to_string(),
                    change: *change,
                    revision: None,
                    client_id: None,
                    ops: None,
                    collection: None,
                }));
        }
        Ok(plan.result)
    }

    /// Saves the show settings (this box's document) and swaps them in.
    async fn change_settings(&mut self, mut settings: model::ShowSettings) -> Result<(), ApiError> {
        let data = self.shared.data();
        settings.schema_version = model::SCHEMA_VERSION;
        let write = crate::store::DocWrite::put(&settings)
            .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?;
        let root = data.root.clone();
        tokio::task::spawn_blocking(move || crate::store::apply(&root, &[write]))
            .await
            .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?
            .map_err(|e| match e {
                crate::store::StoreError::TooNew(m) => ApiError::new(ErrorCode::Conflict, m),
                crate::store::StoreError::Io(e) => ApiError::new(
                    ErrorCode::Internal,
                    format!("saving the settings failed: {e:#}"),
                ),
            })?;
        self.shared.set_data(Arc::new(data.with_settings(settings)));
        self.shared.publish(self.state.clone());
        self.broadcast();
        let _ = self
            .shared
            .events
            .send(Arc::new(ServerMessage::DocumentChanged {
                kind: DocumentKind::ShowSettings,
                id: uuid::Uuid::nil().to_string(),
                change: DocumentChange::Changed,
                revision: None,
                client_id: None,
                ops: None,
                collection: None,
            }));
        Ok(())
    }

    /// Swaps in a change of the active collection (see [`CollectionUpdate`]); the Codex
    /// writer has written it and sends the events.
    async fn collection(&mut self, update: CollectionUpdate) -> Result<(), ApiError> {
        let data = self.shared.data();
        match update {
            CollectionUpdate::Content(change)
                if change.media_only()
                    && !crate::show::uses_assets(&self.state, &data, &change.asset_ids()) =>
            {
                // Media nothing live uses (an upload, a probe, a relink): the documents are
                // swapped in without a new content generation, so no screen re-renders (frame
                // pacing during a show) and the state, unchanged, isn't sent again.
                self.shared.set_data(Arc::new(data.with_media(&change)));
                return Ok(());
            }
            CollectionUpdate::Content(change) => {
                let new = Arc::new(data.with_content(&change));
                self.shared.set_data(new.clone());
                let before = self.state.clone();
                self.state.revalidate_edit(&data, &new);
                if self.state != before {
                    tracing::debug!("show state followed a content edit");
                }
            }
            CollectionUpdate::ShowDocs {
                timers,
                props,
                messages,
                stage_layouts,
            } => {
                let new = Arc::new(data.with_show_docs(timers, props, messages, stage_layouts));
                self.shared.set_data(new.clone());
                self.state.revalidate_show_docs(&new);
            }
            CollectionUpdate::Mirror(content) => {
                let new = Arc::new(data.with_collection(content));
                self.shared.set_data(new.clone());
                self.state.revalidate_edit(&data, &new);
            }
            CollectionUpdate::Switch(to) => {
                let codex = self.shared.codex();
                let (codex, content) =
                    codex.switched(to, data.content.clone()).ok_or_else(|| {
                        ApiError::new(
                            ErrorCode::NotFound,
                            format!("collection {to} is not in the Codex"),
                        )
                    })?;
                let new = Arc::new(data.with_collection(content));
                self.shared.set_both(new.clone(), Arc::new(codex));
                self.shared.reset_announced();
                self.state.revalidate(&new);
            }
        }
        // Media that left the show (its asset is not in the new content) stops playing.
        self.sync_videos().await;
        self.sync_audio().await;
        self.sync_element_sounds();
        self.sync_live_sound();
        // Re-render: the live slide's content may have changed (its fingerprint includes the
        // content generation, which every change but the media-only one above renews).
        self.shared.publish(self.state.clone());
        self.broadcast();
        self.retire_videos();
        Ok(())
    }

    /// Timers: a change of any timer's run state (one expired) broadcasts the state at once;
    /// while timers run, their values go out as `timerTick` at most once a second.
    fn timer_tick(&mut self) {
        let data = self.shared.data();
        if data.content.timers.is_empty() {
            return;
        }
        let states = self.state.timers.states(&data.content.timers, Clock::now());
        let key: Vec<TimerRunState> = states.iter().map(|t| t.state).collect();
        if key != self.timer_key {
            // A timer that ran out shows or hides elements with a timer condition: so are
            // their video elements' sounds (a start or stop came through `apply`).
            self.sync_element_sounds();
            self.broadcast();
            return;
        }
        // Counting, or a stopped countdown-to-time timer (it shows the time left from now).
        let moving: Vec<bool> = data
            .content
            .timers
            .iter()
            .zip(&states)
            .map(|(t, s)| {
                matches!(s.state, TimerRunState::Running | TimerRunState::Overrun)
                    || (s.state == TimerRunState::Stopped && self.state.timers.moves(t))
            })
            .collect();
        if !moving.iter().any(|m| *m) || self.last_tick.elapsed() < TIMER_TICK {
            return;
        }
        self.last_tick = Instant::now();
        let ticks = states
            .into_iter()
            .zip(moving)
            .filter_map(|(t, m)| m.then_some(t))
            .map(|t| protocol::TimerTick {
                id: t.id,
                state: t.state,
                current_seconds: t.current_seconds,
                display: t.display,
            })
            .collect();
        let _ = self
            .shared
            .events
            .send(Arc::new(ServerMessage::TimerTick { timers: ticks }));
    }

    /// Caption lines or the captions engine's status changed (or lines went stale): the
    /// screens get the lines, clients the state.
    fn sync_captions(&mut self) {
        let shown = self.shared.captions.shown(Instant::now());
        let screens = shown != self.state.captions;
        if screens {
            self.state.captions = shown;
            self.shared.publish(self.state.clone());
        }
        if screens || self.shared.captions.state() != self.captions_key {
            self.broadcast();
        }
    }

    /// Builds and sends a new revision of the full state.
    fn broadcast(&mut self) {
        self.note_song_use();
        self.revision += 1;
        let state = self.build();
        self.captions_key = state.captions.clone();
        self.timer_key = state.timers.iter().map(|t| t.state).collect();
        self.last_position = self.position_key(Instant::now());
        self.status = StatusKey::of(&state);
        self.counters = counters(&state);
        self.last_broadcast = Instant::now();
        if let Some(tx) = &self.state_tx {
            tx.send_replace(Arc::new(StateSnapshot {
                revision: self.revision,
                state,
            }));
        }
    }

    /// Logs a song on the slide layer as used (one use per service: [`crate::ccli`]).
    fn note_song_use(&self) {
        let Some(s) = &self.state.slide else { return };
        let data = self.shared.data();
        let Some(p) = data.content.presentation(s.presentation) else {
            return;
        };
        let service = s
            .playlist
            .as_ref()
            .and_then(|c| data.content.playlists.get(&c.playlist))
            .map(|n| n.name.clone());
        let g = &self.shared.gateways;
        self.shared.ccli.observe(
            Some(crate::ccli::Live {
                presentation: p,
                service,
                projected: self.state.screens_enabled.audience,
                streamed: g.streaming(),
                recorded: g.recording(),
            }),
            chrono::Local::now(),
        );
    }

    fn refresh_status(&mut self) {
        let state = self.build();
        let key = StatusKey::of(&state);
        // Output errors become notices once per new error text.
        for (old, new) in self.status.screens.iter().zip(&key.screens) {
            for ((_, od, _), (ns, nd, _)) in old.2.iter().zip(&new.2) {
                if *ns == OutputStatus::Error && nd != od {
                    self.shared.notice(
                        NoticeLevel::Error,
                        format!("output error: {}", nd.as_deref().unwrap_or("?")),
                    );
                }
            }
        }
        let stale =
            self.last_broadcast.elapsed() >= COUNTER_REFRESH && counters(&state) != self.counters;
        if self.status.differs(&key) || stale {
            self.broadcast();
        }
    }

    async fn reload(&mut self) -> Result<u64, ApiError> {
        let old = self.shared.data();
        let root = old.root.clone();
        let name = self.shared.auth.identity().name;
        // Through the Codex: old-layout documents written into the data folder since (an
        // import) move into the active collection first.
        let lib = self.shared.library.clone();
        let (new, codex) =
            tokio::task::spawn_blocking(move || crate::codex::load(&root, &name, &lib))
                .await
                .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?
                .map_err(|e| ApiError::new(ErrorCode::Internal, format!("reload failed: {e:#}")))?;
        let active = codex.active;
        let codex = Arc::new(codex);
        let mut changes = Vec::new();
        let (old_hashes, new_hashes) = (old.doc_hashes(), new.doc_hashes());
        for (key, h) in &new_hashes {
            match old_hashes.get(key) {
                None => changes.push((key.0, key.1, DocumentChange::Added)),
                Some(o) if o != h => changes.push((key.0, key.1, DocumentChange::Changed)),
                _ => {}
            }
        }
        for key in old_hashes.keys() {
            if !new_hashes.contains_key(key) {
                changes.push((key.0, key.1, DocumentChange::Removed));
            }
        }
        let screens_changed = changes.iter().any(|c| {
            matches!(
                c.0,
                DocumentKind::Screen
                    | DocumentKind::OutputBinding
                    | DocumentKind::FontSubstitutionTable
            )
        });
        let new = Arc::new(new);
        self.shared.set_both(new.clone(), codex);
        self.shared.reset_announced();
        if new.audio_device != self.shared.sound.config() {
            self.shared.sound.apply_config(&new.audio_device);
        }
        self.state.revalidate(&new);
        // Media that left the show (its asset is gone) stops playing.
        self.sync_videos().await;
        self.sync_audio().await;
        self.sync_element_sounds();
        self.sync_live_sound();
        if screens_changed {
            let (sh, d) = (self.shared.clone(), new.clone());
            let _ = tokio::task::spawn_blocking(move || crate::engine::apply_config(&sh, &d)).await;
        }
        // Always re-render: documents behind the live slide may have changed.
        self.shared.publish(self.state.clone());
        self.broadcast();
        self.retire_videos();
        for (kind, id, change) in &changes {
            let _ = self
                .shared
                .events
                .send(Arc::new(ServerMessage::DocumentChanged {
                    kind: *kind,
                    id: id.to_string(),
                    change: *change,
                    revision: None,
                    client_id: None,
                    ops: None,
                    collection: crate::data::COLLECTION_KINDS
                        .contains(kind)
                        .then_some(active),
                }));
        }
        let _ = self
            .shared
            .events
            .send(Arc::new(ServerMessage::CodexChanged {
                parts: vec![
                    protocol::CodexPart::Status,
                    protocol::CodexPart::Collections,
                ],
                collection: None,
            }));
        self.shared.notice(
            NoticeLevel::Info,
            format!("data reloaded: {} documents changed", changes.len()),
        );
        if changes
            .iter()
            .any(|c| c.0 == DocumentKind::FontSubstitutionTable)
        {
            self.shared.notice(
                NoticeLevel::Warning,
                "font substitutions changed: restart the server to apply them",
            );
        }
        Ok(self.revision)
    }

    /// The protocol view of the current state.
    fn build(&self) -> protocol::ShowState {
        let data = self.shared.data();
        let d: &ShowData = &data;
        let position = |l: &LiveSlide, held: Option<&Arc<HeldSlide>>| -> Option<SlidePosition> {
            if l.detached {
                let h = held?;
                return Some(SlidePosition {
                    presentation: l.presentation,
                    presentation_name: h.presentation_name.clone(),
                    slide: h.slide.id,
                    slide_index: l.index as u32,
                    section: h.section,
                    section_name: h.section_name.clone(),
                    playlist: l.playlist,
                    step: None,
                });
            }
            let p = d.content.presentation(l.presentation)?;
            let (section, slide) = p
                .sections
                .iter()
                .flat_map(|sec| sec.slides.iter().map(move |s| (sec, s)))
                .nth(l.index)?;
            Some(SlidePosition {
                presentation: p.id,
                presentation_name: p.name.clone(),
                slide: slide.id,
                slide_index: l.index as u32,
                section: section.id,
                section_name: section.name.clone(),
                playlist: l.playlist,
                step: crate::show::step_in(&crate::show::play_order(d, p, l.playlist), l)
                    .map(|k| k as u32),
            })
        };
        let now = Instant::now();
        let layer_state = |m: &crate::show::LiveMedia| {
            let video = m.play.and_then(|p| self.shared.video(p));
            MediaLayerState {
                asset: m.asset,
                name: d.content.asset_name(m.asset),
                kind: d
                    .content
                    .assets
                    .get(&m.asset)
                    .map(|a| a.kind)
                    .unwrap_or(MediaKind::Image),
                online: d.content.asset_path(m.asset).is_ok()
                    && (m.kind != MediaKind::Video || video.is_some()),
                playback: match &video {
                    Some(v) => MediaPlayback {
                        state: v.state_at(now),
                        position: round_ms(v.position_at(now)),
                        duration: v.duration().map(round_ms),
                        // The layer's setting (a play being continued still loops the old way).
                        looping: crate::show::loops(&m.playback),
                        rate: v.clock().rate(),
                        hw: v.player.info().hw,
                    },
                    None => MediaPlayback {
                        state: PlaybackState::Playing,
                        position: 0.0,
                        duration: None,
                        looping: false,
                        rate: 1.0,
                        hw: false,
                    },
                },
            }
        };
        let media = self.state.media.as_ref().map(layer_state);
        let foreground = self.state.slide_media.as_ref().map(layer_state);
        let audio_layer = self.state.audio.as_ref().map(|m| {
            let a = m.play.and_then(|p| self.shared.live_audio(p));
            MediaLayerState {
                asset: m.asset,
                name: d.content.asset_name(m.asset),
                kind: MediaKind::Audio,
                online: a.is_some(),
                playback: match &a {
                    Some(a) => MediaPlayback {
                        state: a.state_at(now),
                        position: round_ms(a.position_at(now)),
                        duration: a.region_len().map(round_ms),
                        looping: crate::show::loops(&m.playback),
                        rate: a.rate(),
                        hw: false,
                    },
                    None => MediaPlayback {
                        state: PlaybackState::Stopped,
                        position: 0.0,
                        duration: None,
                        looping: false,
                        rate: 1.0,
                        hw: false,
                    },
                },
            }
        });
        let transport = [
            (Layer::Slide, foreground.as_ref()),
            (Layer::Media, media.as_ref()),
            (Layer::Audio, audio_layer.as_ref()),
        ]
        .into_iter()
        .find_map(|(layer, m)| {
            let m = m?;
            (matches!(m.kind, MediaKind::Video | MediaKind::Audio) && m.online).then(|| {
                protocol::MediaTransport {
                    layer,
                    asset: m.asset,
                    name: m.name.clone(),
                    state: m.playback.state,
                    position: m.playback.position,
                    duration: m.playback.duration,
                    looping: m.playback.looping,
                    rate: m.playback.rate,
                    hw: m.playback.hw,
                }
            })
        });
        let transitions = self
            .state
            .changes
            .iter()
            .filter_map(|(layer, c)| {
                let t = c.transition.renderable();
                if t.kind == model::TransitionKind::Cut || t.duration <= 0.0 {
                    return None;
                }
                let p = now.saturating_duration_since(c.at).as_secs_f64() / t.duration;
                (p < 1.0).then(|| protocol::TransitionState {
                    layer: *layer,
                    kind: t.kind.clone(),
                    progress: (p * 100.0).round() / 100.0,
                    duration: t.duration,
                })
            })
            .collect();
        let clock = Clock::now();
        let timers = self.state.timers.states(&d.content.timers, clock);
        // Message text with timer / clock tokens as of now.
        let tctx = render::live::DataContext {
            timers: self.state.timers.values(&d.content.timers, clock),
            last_started: self.state.timers.last_started,
            wall: Some(crate::timers::wall_time()),
            ..Default::default()
        };
        let props = self
            .state
            .props
            .iter()
            .filter_map(|id| d.content.props.iter().find(|p| p.id == *id))
            .map(|p| protocol::PropLayerState {
                prop: p.id,
                name: p.name.clone(),
            })
            .collect();
        let n = self.state.messages.len();
        let messages = self
            .state
            .messages
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let doc = d.content.messages.iter().find(|x| x.id == m.message)?;
                Some(protocol::MessageLayerState {
                    message: m.message,
                    name: doc.name.clone(),
                    text: crate::live::message_text(doc, &m.tokens, &tctx),
                    tokens: m.tokens.clone(),
                    visible: i + 1 == n,
                })
            })
            .collect();
        let stage_layouts = self
            .state
            .stage_layouts
            .iter()
            .map(|(screen, layout)| protocol::StageLayoutState {
                screen: *screen,
                layout: *layout,
            })
            .collect();
        let stage_screens = d
            .screens
            .iter()
            .filter(|s| matches!(s.kind, model::ScreenKind::Stage { .. }))
            .map(|s| {
                let layout = crate::stage::layout_id_for(d, &self.state, s);
                protocol::StageScreenState {
                    screen: s.id,
                    name: s.name.clone(),
                    layout,
                    layout_name: layout
                        .and_then(|id| d.content.stage_layouts.iter().find(|l| l.id == id))
                        .map(|l| l.name.clone())
                        .unwrap_or_else(|| crate::stage::Template::CurrentNext.name().into()),
                }
            })
            .collect();
        let stage_message =
            self.state
                .stage_message
                .as_ref()
                .map(|m| protocol::StageMessageState {
                    text: m.text.clone(),
                    flash: m.flash,
                });
        let screens = self
            .shared
            .screens()
            .iter()
            .map(|rt| {
                let st = lock(&rt.stats).clone();
                let screen = rt.screen();
                let role = crate::show::screen_role(&screen);
                ScreenState {
                    id: screen.id,
                    name: screen.name.clone(),
                    role,
                    enabled: self.state.screens_enabled.get(role),
                    width: screen.size.width.round().max(1.0) as u32,
                    height: screen.size.height.round().max(1.0) as u32,
                    frame_rate: screen.frame_rate,
                    measured_fps: (st.fps * 10.0).round() / 10.0,
                    last_render_ms: st.last_render_ms.map(|m| (m * 10.0).round() / 10.0),
                    test_pattern: self.state.test_patterns.get(&screen.id).cloned(),
                    outputs: rt.outputs().iter().map(|o| o.state(&screen)).collect(),
                    signage: match &screen.kind {
                        model::ScreenKind::Signage { channel } => Some(channel.clone()),
                        _ => None,
                    },
                }
            })
            .collect();
        let mut diagnostics = lock(&self.shared.diagnostics).summary(d.report.issues.len());
        diagnostics.skipped_actions = self.state.skipped_actions.min(u32::MAX as u64) as u32;
        protocol::ShowState {
            restorable_layers: self.state.restorable(),
            clock_format: self.state.clock,
            displays: self.shared.displays.infos(&d.outputs),
            layers: protocol::LayerStates {
                video_input: self.state.video_input.as_ref().map(|v| {
                    protocol::VideoInputLayerState {
                        input: v.input,
                        name: v.name.clone(),
                        source: v.source.clone(),
                    }
                }),
                media,
                foreground,
                slide: self
                    .state
                    .slide
                    .as_ref()
                    .and_then(|l| position(l, self.state.held_slide.as_ref())),
                announcements: self
                    .state
                    .announcements
                    .as_ref()
                    .and_then(|l| position(l, self.state.held_announcement.as_ref())),
                props,
                messages,
                audio: audio_layer,
            },
            active_look: self.state.look,
            transitions,
            transport,
            timers,
            stage_layouts,
            stage_screens,
            stage_message,
            screens,
            screens_enabled: self.state.screens_enabled,
            identify: self.state.identify,
            options: self.state.options,
            cursor: self
                .state
                .cursor(d)
                .as_ref()
                .and_then(|l| position(l, None)),
            diagnostics,
            build: self
                .state
                .build_progress(d)
                .map(|(b, steps)| protocol::BuildProgress {
                    layer: b.layer,
                    presentation: b.presentation,
                    slide: b.slide,
                    step: b.step as u32,
                    steps: steps as u32,
                }),
            audio: self.shared.sound.state(
                self.state.audio_playlist,
                self.state.audio_shuffle,
                self.state.audio_repeat,
            ),
            captions: self.shared.captions.state(),
        }
    }
}

/// Seconds rounded to milliseconds (keeps the JSON short).
fn round_ms(s: f64) -> f64 {
    (s * 1000.0).round() / 1000.0
}

/// A use's level with its asset's loudness gain (media preparation) added.
fn levelled(data: &ShowData, m: &crate::show::LiveMedia) -> model::AudioSettings {
    match data.content.assets.get(&m.asset) {
        Some(a) => a.levelled(&m.audio),
        None => m.audio.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H05-1: a sound card lost or back again changes the status, so the per-second refresh
    /// broadcasts it at once.
    #[test]
    fn the_sound_card_state_is_part_of_the_status() {
        let mut s = protocol::ShowState::default();
        s.audio.output_state = protocol::AudioOutputState::Ok;
        let ok = StatusKey::of(&s);
        assert!(!ok.differs(&StatusKey::of(&s)));
        s.audio.output_state = protocol::AudioOutputState::Error;
        s.audio.detail = Some("output lost".into());
        let lost = StatusKey::of(&s);
        assert!(ok.differs(&lost));
        s.audio.detail = Some("reconnecting".into());
        assert!(lost.differs(&StatusKey::of(&s)), "a new reason shows too");
    }
}
