//! # `vesper-server` — Vesper, the headless render/playout server (binary `vesper`)
//!
//! ```text
//!  data dir ──load / reload──► ShowData (immutable; a reload swaps in a new one)
//!  (codex/: the active collection's content; portable/, machine/: this box's documents)
//!
//!  clients (control app, browser, scripts)
//!     │ REST /api/v1/*   WebSocket /api/v1/ws        (auth: site API key or same machine)
//!     ▼
//!  api / ws ──commands (in order)──► show actor ──state + revision──► every WebSocket
//!     │                                  │ publishes ShowState
//!     │ thumbnails (low priority)        ▼
//!     └──────────────────────────► render thread (Renderer, wgpu) ◄── wake
//!                                        │ renders changed screens first, then thumbnails
//!        preview JPEGs ◄── latest frame per screen ──► one clock thread per screen
//!                                                       │ offers the latest frame, never waits
//!                                        one worker thread per output (IDN, PNG, ffplay)
//! ```
//!
//! - [`data`] (from `vesper-codex`): loads a `pro-import`-style data directory, migrating every
//!   document: this box's documents ([`ShowData`]) and the collection content it shows
//!   ([`data::CollectionContent`], shared as an `Arc` and swapped by content edits).
//! - [`show`]: what is live and the commands that change it (playlist-aware `next` / `prev`,
//!   clear groups, looks), and how a screen turns the state into layers.
//! - [`actor`]: the single owner of the show state. Applies commands one at a time and
//!   broadcasts the full [`protocol::ShowState`] with a monotonic revision after every change
//!   (no locks are held across awaits; the render thread gets a copy).
//! - [`engine`]: the render thread (live screens first, then queued thumbnails) and one clock
//!   thread per screen (re-sends the latest frame at the screen's frame rate).
//! - [`outputs`]: sink worker threads built from the machine's output bindings and CLI flags
//!   (mode / alpha overrides, video delay).
//! - [`config`] / [`store`]: editing screens and output bindings (validation, revisions,
//!   cascades) and atomic document writes; [`engine::apply_config`] applies them live.
//! - [`edit`] / [`content`]: library content editing: id-addressed presentation and playlist
//!   edit ops with inverses (undo), library / presentation / playlist CRUD with revisions,
//!   live re-render of edited live slides (a deleted live slide stays on screen until the next
//!   trigger), `documentChanged` with the applied ops.
//! - [`api`] / [`ws`]: the control protocol (see `crates/protocol/README.md`).
//! - [`thumbs`]: slide thumbnails (render on demand, memory LRU + disk cache, prefetch).
//! - [`preview`]: live screen previews as JPEG over the WebSocket.
//! - [`heap`]: the heap's numbers for `/server/stats` and the trimmer that gives freed memory
//!   back to the system (glibc).
//! - [`auth`]: server identity, pairing codes, who a request comes from (site keys checked by
//!   [`site`], credentials from before keys until swapped).
//! - [`codex`]: Vesper Codex (its own crate, `crates/vesper-codex`, re-exported with the
//!   server's side of it: its routes, [`codex::Server`] for the server's state, presence over
//!   the link), the site's content store: collections of content documents
//!   (`codex/`), this box's active collection (`machine/codex.json`), the resumable migration
//!   of a standalone install's data folder into its own Codex, the one serialized writer of
//!   every collection (content edits included), collection CRUD and this box's assignment,
//!   the change log and trash, the media library (IDN's) and uploads, search, settings
//!   backups, and show boxes (a mirror of one collection, write-through edits)
//!   (`/api/v1/codex`).
//! - [`site`]: the site this server is in: its own IDN Site Manager as a module (unless it is a
//!   machine of another server's site), joins, join codes, devices, users, paired clients'
//!   keys, engine tokens, audit, typed site events; joining and leaving another site.
//!
//! Operator notices are summarised: rolling notices with stable keys (fonts, unsupported
//! features, render notes) and one missing-media summary per presentation opened or
//! triggered, instead of one notice per file ([`Diagnostics`]). Audience / stage screens can be
//! switched off per role: they keep sending, black (see [`show`]). The built control UI can be
//! served at `/` ([`ServerConfig::ui_dir`]).
//!
//! - [`playback`]: video playback (players decoding ahead with VA-API, media clocks, end
//!   behaviour, fades, transport); the engine renders video screens every tick and runs layer
//!   transitions (dissolves in linear light).
//! - [`sound`]: the audio bus (video soundtracks, the audio layer, audio playlists, levels),
//!   the sound as the clock of videos with sound, embedded audio on IDN outputs.
//! - [`timers`] / [`live`] / [`showdocs`]: timers (pure engine with an injectable clock), live
//!   data in rendered slides (data-link text, visibility conditions, colour triggers; screens
//!   re-render when a value they show changes, at most once a second for clocks and timers),
//!   messages (token substitution, template or built-in banner) and props; their REST CRUD.
//!   Slide actions run when a slide is triggered (see [`show`]).
//! - [`stage`]: stage display. Stage screens draw their stage layout (data links, slide
//!   pictures) and the stage message, never the audience layers; the layout per screen,
//!   built-in templates. Stage layouts are edited through [`showdocs`].
//!
//! ## Not yet (later milestones)
//! - Soft loops, sound at playback rates other than 1, masks, video inputs.
//!   `triggerVideoInput` answers `notImplemented`.
//! - Output regions, display and SDI outputs, OMT outputs and inputs (`docs/OMT-PLAN.md`).
//! - Navigation follows document order, not arrangements.
//! - Editing looks, global groups and media bins (read-only; `reloadData` picks up changes on
//!   disk). Presentations, libraries, playlists and themes are editable ([`content`],
//!   [`themes`]: linked themes), screens and output bindings too ([`config`]).
//!
//! [`start`] wires everything together and returns a [`RunningServer`]; `vesper` (the
//! binary) and the integration tests both use it.

pub mod actor;
pub mod api;
pub mod captions;
pub mod ccli;
pub mod codex;
pub mod config;
pub mod content;
pub mod displays;
pub mod edit;
pub mod element_video;
pub mod engine;
pub mod gateways;
pub mod heap;
pub mod inputs;
pub mod live;
pub mod live_sound;
pub mod live_sources;
pub mod mosaic;
pub mod outputs;
pub mod playback;
pub mod presence;
pub mod preview;
pub mod releases;
pub mod remote_compat;
pub mod show;
pub mod showdocs;
pub mod signage;
pub mod sound;
pub mod stage;
pub mod themes;
pub mod thumbs;
pub mod timers;
pub mod translate;
pub mod updates;
pub mod ws;

// The site and who a request comes from live in their own crate (`crates/vesper-site`); the
// helpers every part shares in `crates/vesper-core`. Re-exported under their old paths.
pub use vesper_core::lock;
pub use vesper_site as site;
pub use vesper_site::auth;
// The Codex's crate (`crates/vesper-codex`) holds the documents' loading and atomic writes too.
pub use vesper_codex::{data, store};

use data::ShowData;
use engine::{Cmd, Engine, ScreenStats};
use model::Screen;
use outputs::DevOutputs;
use protocol::{
    DiagnosticsReport, DiagnosticsSummary, FontIssueInfo, IssueCount, LoadIssueInfo, NoticeLevel,
    ServerMessage,
};
use render::{RenderReport, RenderedFrame, RendererOptions};
use show::ShowState;
use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tokio::sync::{Notify, broadcast};

/// Server software version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Server settings.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Data directory (`portable/`, `machine/`).
    pub data_dir: PathBuf,
    /// HTTP listen address (port 0 = any free port).
    pub listen: SocketAddr,
    /// Development outputs from the command line.
    pub dev: DevOutputs,
    /// Renderer options. `wait_for_images` is turned off (images decode on the renderer's
    /// worker pool, see [`engine`]). `font_substitutions` is replaced by the data directory's
    /// table and `<data>/fonts` is added to `font_dirs` when it exists.
    pub renderer: RendererOptions,
    /// Requests from 127.0.0.1 / ::1 need no token (the same-machine control app).
    pub trust_localhost: bool,
    /// Advertise the server over mDNS.
    pub mdns: bool,
    /// Built control UI to serve at `/` (`None` = no UI; `/` answers 404).
    pub ui_dir: Option<PathBuf>,
    /// Audio device to use instead of the machine's audio device configuration (`""` = none).
    pub audio_device: Option<String>,
    /// The site's Site Manager module (see [`site`]).
    pub site: site::SiteConfig,
    /// The Codex's media limits (uploads, free space, media workers).
    pub codex: codex::CodexOptions,
    /// The bundled Bible translations (`*.vbible`, read-only; `None` = none). The data
    /// folder's `scripture/` is read too (see [`vesper_scripture`]).
    pub scripture_dir: Option<PathBuf>,
    /// Where display outputs come from (`None` = this platform's, see [`displays`]; tests
    /// pass a [`displays::FakeDisplays`]).
    pub displays: Option<Arc<dyn displays::DisplaySource>>,
    /// How often WebSocket clients are pinged; one that sends nothing (not even the pong
    /// every WebSocket client answers a ping with) for three of these is dropped.
    pub ws_ping: std::time::Duration,
    /// More update signing keys (minisign public key text) for the site's update channel,
    /// besides the compiled-in release keys (see [`updates`]).
    pub update_keys: Vec<String>,
    /// GitHub's API, where the server checks Vesper's releases for the site's update channel
    /// (see [`releases`]; `None`: never, as in tests; the `vesper` binary passes GitHub's).
    pub github_api: Option<String>,
    /// How often the release checker looks whether a check is due (60 s; tests shorten it).
    pub github_tick: std::time::Duration,
    /// How often the site's display boxes are read (see [`mosaic`]; 5 s).
    pub mosaic_poll: std::time::Duration,
    /// The IDN Nodes Mosaic works with (`None`: the Site Manager's only, never this machine's
    /// own Node, as in tests; the `vesper` binary passes [`mosaic::IdnNodes`] with it).
    pub mosaic_nodes: Option<Arc<dyn mosaic::Nodes>>,
    /// How long after a display box accepted the site's logo its screens are given it
    /// ([`mosaic::LOGO_SETTLE`]; tests shorten it).
    pub mosaic_logo_settle: std::time::Duration,
    /// How PowerPoint imports draw slide pictures for what isn't editable (default: LibreOffice
    /// when installed, see [`pptx_import::render::Renderer::Auto`]).
    pub slide_pictures: pptx_import::render::Renderer,
    /// Machine translation's engine, pairs and model source (tests replace them; see
    /// [`translate::TranslateOptions`]).
    pub translation: translate::TranslateOptions,
    /// How live captions' speech engine runs: the program whisper runs in (see
    /// [`captions::CaptionsOptions`]; the `vesper` binary passes itself).
    pub captions: captions::CaptionsOptions,
}

impl ServerConfig {
    /// Defaults for a data directory: listen on `0.0.0.0:5555`, trust localhost, mDNS on.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        ServerConfig {
            data_dir: data_dir.into(),
            listen: SocketAddr::from(([0, 0, 0, 0], protocol::DEFAULT_PORT)),
            dev: DevOutputs::default(),
            renderer: RendererOptions::default(),
            trust_localhost: true,
            mdns: true,
            ui_dir: None,
            audio_device: None,
            site: site::SiteConfig::default(),
            codex: codex::CodexOptions::default(),
            scripture_dir: None,
            displays: None,
            ws_ping: std::time::Duration::from_secs(20),
            update_keys: Vec::new(),
            github_api: None,
            github_tick: std::time::Duration::from_secs(60),
            mosaic_poll: mosaic::DEFAULT_POLL,
            mosaic_nodes: None,
            mosaic_logo_settle: mosaic::LOGO_SETTLE,
            slide_pictures: pptx_import::render::Renderer::Auto,
            translation: translate::TranslateOptions::default(),
            captions: captions::CaptionsOptions::default(),
        }
    }
}

/// Per-screen runtime shared between the render thread, the screen's clock thread and the
/// API. Lives as long as the screen exists and is enabled; its configuration can change
/// underneath (see [`engine::apply_config`]).
pub struct ScreenRuntime {
    /// The screen's ID.
    pub id: model::ScreenId,
    config: RwLock<Screen>,
    /// Latest rendered frame.
    pub latest: Mutex<Option<Arc<RenderedFrame>>>,
    /// Live numbers.
    pub stats: Mutex<ScreenStats>,
    /// Its outputs (the clock offers frames to each one that has no clock of its own).
    pub outputs: Mutex<Vec<Arc<outputs::OutputRuntime>>>,
    /// Tells the clock thread to stop.
    pub(crate) stop: std::sync::atomic::AtomicBool,
    /// The clock thread.
    pub(crate) clock: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The screen shows an animated test pattern: its clock asks for a render every tick.
    pub(crate) animated: std::sync::atomic::AtomicBool,
}

impl ScreenRuntime {
    /// A runtime for `screen` (no clock, no outputs yet).
    pub fn new(screen: Screen) -> Self {
        ScreenRuntime {
            id: screen.id,
            config: RwLock::new(screen),
            latest: Mutex::new(None),
            stats: Mutex::new(ScreenStats::default()),
            outputs: Mutex::new(Vec::new()),
            stop: std::sync::atomic::AtomicBool::new(false),
            clock: Mutex::new(None),
            animated: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The screen's current configuration.
    pub fn screen(&self) -> Screen {
        self.config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The screen's frame rate.
    pub fn frame_rate(&self) -> model::FrameRate {
        self.config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .frame_rate
    }

    pub(crate) fn set_screen(&self, screen: Screen) {
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = screen;
    }

    /// Snapshot of the outputs.
    pub fn outputs(&self) -> Vec<Arc<outputs::OutputRuntime>> {
        lock(&self.outputs).clone()
    }
}

/// Notices kept for clients that connect later.
const RECENT_NOTICES: usize = 20;

/// State shared by the API, the show actor and the render thread.
pub struct Shared {
    data: RwLock<Arc<ShowData>>,
    codex: RwLock<Arc<codex::CodexState>>,
    /// The one serialized writer of every Codex collection (see [`codex::writer`]).
    pub codex_writer: codex::writer::Writer,
    /// Media uploads in progress (see [`codex::uploads`]).
    pub uploads: codex::uploads::Uploads,
    /// The Codex's own membership of its site (see [`codex::member`]).
    pub codex_member: codex::member::OwnMembership,
    /// The Codex's media library (see [`codex::library`]).
    pub library: Arc<codex::library::MediaLibrary>,
    /// The Codex's side of the box-to-Codex link: its channel in the site (see
    /// [`codex::link`]).
    pub codex_link: Arc<codex::link::Link>,
    /// A show box's mirror of the site's Codex (see [`codex::mirror`]).
    pub mirror: Arc<codex::mirror::Mirror>,
    /// The search index of the collections (see [`codex::search`]); told of every change of
    /// the documents by the setters below.
    pub search: codex::search::SearchIndex,
    /// Settings backups (see [`codex::backups`]).
    pub backups: codex::backups::Backups,
    /// The Codex's encrypted exports and imports (see [`codex::export`]).
    pub exports: codex::export::Exports,
    /// A show box's upload of its dormant collection to the site's Codex (G15).
    pub adopt: codex::adopt::Adopter,
    /// The server is announced over mDNS (the `mdns` capability).
    pub mdns_on: std::sync::atomic::AtomicBool,
    show: Mutex<ShowState>,
    version: AtomicU64,
    /// Running screens in sort order (replaced as a whole when the configuration changes).
    screens: RwLock<Arc<Vec<Arc<ScreenRuntime>>>>,
    /// Command-line development outputs (added to every screen, including new ones).
    pub dev: DevOutputs,
    /// Serializes configuration changes of the engine.
    pub(crate) config_lock: Mutex<()>,
    /// Deduplicated render diagnostics.
    pub diagnostics: Mutex<Diagnostics>,
    /// GPU adapter description.
    pub adapter: Mutex<Option<String>>,
    /// The renderer's cache sizes, published by the render thread every few seconds
    /// (`/server/stats`).
    pub render_caches: Mutex<Vec<render::CacheSize>>,
    /// Installed font families and the fallback family (set once the renderer is up).
    pub fonts: Mutex<(Vec<render::FontFamily>, String)>,
    /// The renderer's image decoder (set once the renderer is up; counters for diagnostics
    /// and tests).
    pub decoder: std::sync::OnceLock<Arc<render::ImageDecoder>>,
    /// The renderer's font options (fonts, fallback, substitutions), for [`Shared::measurer`].
    font_options: RendererOptions,
    /// Text layout off the render thread (scripture splits), built on first use.
    measurer: std::sync::OnceLock<Mutex<render::TextMeasurer>>,
    wake: Mutex<Sender<Cmd>>,
    /// Server start time.
    pub started: Instant,
    /// Events for every WebSocket (notices, document changes).
    pub events: broadcast::Sender<Arc<ServerMessage>>,
    recent_notices: Mutex<VecDeque<Arc<ServerMessage>>>,
    /// Presentations whose missing media were announced (since the last data load).
    announced: Mutex<std::collections::HashSet<model::PresentationId>>,
    /// Wakes the show actor when screen / output / diagnostics status may have changed.
    pub status_changed: Notify,
    /// Server identity and pairing.
    pub auth: auth::Auth,
    /// The site (the Site Manager module).
    pub site: Arc<site::Site>,
    /// Video players by play ID (see [`playback`]); kept in step with the show state by the
    /// show actor.
    pub videos: Mutex<std::collections::HashMap<u64, Arc<playback::LiveVideo>>>,
    /// Video elements of the live slide playing with sound (see [`element_video`]); kept in
    /// step with the show state by the show actor.
    pub element_plays: element_video::ElementPlays,
    /// The video-input layer's live source whose sound plays (see [`live_sound`]); kept in
    /// step with the show state by the show actor.
    pub live_sound: Mutex<Option<live_sound::Playing>>,
    /// Video thumbnails (JPEG) by file, size, modification time and width.
    pub video_thumbs: Mutex<thumbs::BytesLru>,
    /// Durations of video files without a probe in their asset, by path.
    pub video_durations: Mutex<std::collections::HashMap<PathBuf, Option<f64>>>,
    /// The audio bus.
    pub sound: sound::Hub,
    /// Live network video sources and their receivers (see [`live_sources`]).
    pub live_sources: live_sources::LiveSources,
    /// The box's physical displays (see [`displays`]).
    pub displays: displays::Displays,
    /// Plays of the audio layer by play ID (kept in step with the show state by the actor).
    pub audios: Mutex<std::collections::HashMap<u64, Arc<sound::LiveAudio>>>,
    /// Who has what open, and the edit locks (see [`presence`]).
    pub presence: presence::Presence,
    /// Live captions (see [`captions`]).
    pub captions: captions::Hub,
    /// Recording and streaming screens (see [`gateways`]).
    pub gateways: gateways::Gateways,
    /// CCLI song usage (see [`ccli`]).
    pub ccli: ccli::CcliLog,
    /// The site's update channel and its Launchers (see [`updates`]).
    pub updates: updates::Updates,
    /// Vesper's releases on GitHub, brought into the update channel (see [`releases`]).
    pub releases: Arc<releases::Releases>,
    /// The site's display boxes and channels (see [`mosaic`]): on the site's server.
    pub mosaic: mosaic::Mosaic,
    /// The site's signage channels (see [`signage`]): rendered on the site's server.
    pub signage: Arc<signage::Signage>,
    /// Machine translation: the model store and previews (see [`translate`]).
    pub translate: translate::Translation,
}

impl Shared {
    /// The text measurer: the renderer's fonts, fallback and substitution table without a GPU
    /// (loading the fonts on first use; call it from a blocking thread).
    pub fn measurer(&self) -> &Mutex<render::TextMeasurer> {
        self.measurer
            .get_or_init(|| Mutex::new(render::TextMeasurer::new(&self.font_options)))
    }

    /// Who makes a request (see [`auth::Auth::classify`]).
    pub fn classify(&self, ip: std::net::IpAddr, token: Option<&str>) -> auth::Access {
        self.auth.classify(&self.site, ip, token)
    }

    /// The pairing code this server shows (in its log, `GET /api/v1/pairing/code`,
    /// `ServerInfo`): its own, or at a machine of another server's site (a relay) the one the
    /// site's Site Manager gave it (`None` until it has).
    pub fn pairing_code(&self) -> Option<protocol::PairingCode> {
        if self.site.relays_pairings() {
            self.site.relay_code()
        } else {
            Some(self.auth.pairing_code())
        }
    }

    /// This box holds the site's Codex (role `host`).
    pub fn codex_is_host(&self) -> bool {
        self.codex().config.role == model::CodexRole::Host
    }

    /// The current documents.
    pub fn data(&self) -> Arc<ShowData> {
        self.data.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Replaces the documents (reload).
    pub fn set_data(&self, data: Arc<ShowData>) {
        *self.data.write().unwrap_or_else(|e| e.into_inner()) = data;
        self.search.changed();
    }

    /// The Codex: its collections and this box's active one, with every other collection's
    /// documents. The active collection's documents are [`ShowData::content`] (see
    /// [`codex`]); [`Shared::collection_content`] reads either.
    pub fn codex(&self) -> Arc<codex::CodexState> {
        self.codex.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Replaces the Codex state (reload, the Codex writer, a switch of the active collection).
    pub fn set_codex(&self, codex: Arc<codex::CodexState>) {
        *self.codex.write().unwrap_or_else(|e| e.into_inner()) = codex;
        self.search.changed();
    }

    /// A collection's current documents: the show's for the active collection, the Codex
    /// state's for any other. `None` for a collection that isn't there.
    pub fn collection_content(
        &self,
        id: model::CollectionId,
    ) -> Option<Arc<data::CollectionContent>> {
        // Both locks, the Codex's first (as in `set_both`): a switch of the active collection
        // is never seen half done.
        let codex = self.codex.read().unwrap_or_else(|e| e.into_inner());
        if codex.active == id {
            let data = self.data.read().unwrap_or_else(|e| e.into_inner());
            return Some(data.content.clone());
        }
        codex.collection(id)?.content.clone()
    }

    /// Replaces the documents and the Codex state together (a switch of the active collection,
    /// a reload): readers of [`Shared::collection_content`] see both or neither.
    pub fn set_both(&self, data: Arc<ShowData>, codex: Arc<codex::CodexState>) {
        {
            let mut c = self.codex.write().unwrap_or_else(|e| e.into_inner());
            let mut d = self.data.write().unwrap_or_else(|e| e.into_inner());
            *d = data;
            *c = codex;
        }
        self.search.changed();
    }

    /// Render-side version of the show state (bumped on every publish).
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// The render-side copy of the show state.
    pub fn state(&self) -> ShowState {
        lock(&self.show).clone()
    }

    /// Hands a new show state to the render thread (called by the show actor only).
    pub fn publish(&self, state: ShowState) {
        *lock(&self.show) = state;
        self.version.fetch_add(1, Ordering::AcqRel);
        self.send_render(Cmd::Wake);
    }

    /// Asks the render thread to plan every screen again without a show state change (signage
    /// channels' new pictures).
    pub fn wake_render(&self) {
        self.version.fetch_add(1, Ordering::AcqRel);
        self.send_render(Cmd::Wake);
    }

    /// Sends a message to the render thread (false when it is gone).
    pub fn send_render(&self, cmd: Cmd) -> bool {
        lock(&self.wake).send(cmd).is_ok()
    }

    /// Logs a notice and sends it to every client (and to clients that connect soon after).
    pub fn notice(&self, level: NoticeLevel, message: impl Into<String>) {
        self.notice_keyed(level, message, None);
    }

    /// A notice with a stable `key`: clients replace an earlier notice with the same key
    /// instead of stacking it, and so does the list replayed to new connections.
    pub fn notice_keyed(
        &self,
        level: NoticeLevel,
        message: impl Into<String>,
        key: Option<String>,
    ) {
        let message = message.into();
        match level {
            NoticeLevel::Info => tracing::info!("{message}"),
            NoticeLevel::Warning => tracing::warn!("{message}"),
            NoticeLevel::Error => tracing::error!("{message}"),
        }
        let m = Arc::new(ServerMessage::Notice {
            level,
            message,
            key: key.clone(),
        });
        {
            let mut r = lock(&self.recent_notices);
            if let Some(k) = &key {
                r.retain(|old| {
                    !matches!(old.as_ref(), ServerMessage::Notice { key: Some(o), .. } if o == k)
                });
            }
            if r.len() == RECENT_NOTICES {
                r.pop_front();
            }
            r.push_back(m.clone());
        }
        let _ = self.events.send(m);
    }

    /// The most recent notices (oldest first).
    pub fn recent_notices(&self) -> Vec<Arc<ServerMessage>> {
        lock(&self.recent_notices).iter().cloned().collect()
    }

    /// Announces the media files a presentation is missing, once per presentation per data
    /// load (when it is opened or triggered): one notice keyed `missingMedia:<id>`. The files
    /// are added to the diagnostics' missing-media list. Does file I/O (one `stat` per asset).
    pub fn announce_presentation_media(&self, id: model::PresentationId) {
        let data = self.data();
        let Some(p) = data.content.presentation(id) else {
            return;
        };
        if !lock(&self.announced).insert(id) {
            return;
        }
        let assets = data.content.presentation_assets(p);
        // A show box copies what it doesn't have yet (the presentation is about to be shown).
        codex::mirror::want(self, &assets);
        let missing: Vec<String> = assets
            .into_iter()
            .filter_map(|a| data.content.asset_path(a).err())
            .collect();
        if missing.is_empty() {
            return;
        }
        {
            let mut d = lock(&self.diagnostics);
            for why in &missing {
                d.known_missing(why);
            }
        }
        self.status_changed.notify_one();
        let n = missing.len();
        self.notice_keyed(
            NoticeLevel::Warning,
            format!(
                "{n} media file{} missing in '{}'",
                if n == 1 { "" } else { "s" },
                p.name
            ),
            Some(format!("missingMedia:{}", id.0)),
        );
    }

    /// Forgets which presentations were announced (after a reload).
    pub fn reset_announced(&self) {
        lock(&self.announced).clear();
    }

    /// The running screens, in sort order.
    pub fn screens(&self) -> Arc<Vec<Arc<ScreenRuntime>>> {
        self.screens
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(crate) fn set_screens(&self, list: Vec<Arc<ScreenRuntime>>) {
        *self.screens.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(list);
    }

    /// Screen runtime by ID.
    pub fn screen(&self, id: model::ScreenId) -> Option<Arc<ScreenRuntime>> {
        self.screens().iter().find(|s| s.id == id).cloned()
    }

    /// The live audio of a play.
    pub fn live_audio(&self, play: u64) -> Option<Arc<sound::LiveAudio>> {
        lock(&self.audios).get(&play).cloned()
    }

    /// The live video of a play.
    pub fn video(&self, play: u64) -> Option<Arc<playback::LiveVideo>> {
        lock(&self.videos).get(&play).cloned()
    }

    /// Asks the render thread to re-render (screens were added or reconfigured).
    pub fn rerender(&self) {
        self.version.fetch_add(1, Ordering::AcqRel);
        self.send_render(Cmd::Wake);
    }
}

/// A font that did not resolve exactly, with its use count.
#[derive(Clone, Debug)]
struct FontDiag {
    info: FontIssueInfo,
}

/// Render diagnostics accumulated since start, deduplicated.
///
/// Operators see them as a few **rolling notices** with stable keys (`fonts`, `unsupported`,
/// `notes`) that are re-sent, replacing the previous one, only when their count grows; the full
/// lists are in `GET /api/v1/diagnostics`. Missing media are announced per presentation (see
/// [`Shared::announce_presentation_media`]), not per file.
#[derive(Default)]
pub struct Diagnostics {
    fonts: BTreeMap<String, FontDiag>,
    media: BTreeMap<(String, String), u64>,
    unsupported: BTreeMap<String, u64>,
    notes: BTreeMap<String, u64>,
    /// Categories whose rolling notice is out of date.
    fonts_changed: bool,
    unsupported_changed: bool,
    /// The newest note, when a new one arrived since the last notice.
    new_note: Option<String>,
}

impl Diagnostics {
    /// Adds a renderer report.
    pub fn merge(&mut self, r: &RenderReport) {
        for f in &r.fonts {
            let matched = format!("{:?}", f.matched);
            let matched = matched[..1].to_lowercase() + &matched[1..];
            let key = format!(
                "{}|{}|{}|{:?}|{}|{}|{}|{}",
                f.requested.family,
                f.requested.weight,
                f.requested.italic,
                f.requested.postscript_name,
                f.used_family,
                matched,
                f.synthetic_bold,
                f.synthetic_italic
            );
            match self.fonts.get_mut(&key) {
                Some(e) => e.info.uses += f.uses,
                None => {
                    self.fonts_changed = true;
                    self.fonts.insert(
                        key,
                        FontDiag {
                            info: FontIssueInfo {
                                family: f.requested.family.clone(),
                                weight: f.requested.weight as u32,
                                italic: f.requested.italic,
                                used_family: f.used_family.clone(),
                                matched,
                                synthetic_bold: f.synthetic_bold,
                                synthetic_italic: f.synthetic_italic,
                                uses: f.uses,
                            },
                        },
                    );
                }
            }
        }
        for m in &r.media {
            self.media_issue(&m.source, &m.problem, m.uses);
        }
        for (u, n) in &r.unsupported {
            let e = self.unsupported.entry(u.clone()).or_default();
            if *e == 0 {
                self.unsupported_changed = true;
            }
            *e += n;
        }
    }

    fn media_issue(&mut self, source: &str, problem: &str, uses: u64) {
        *self
            .media
            .entry((source.to_string(), problem.to_string()))
            .or_default() += uses;
    }

    /// Records a media file found missing without drawing it (a presentation's media list):
    /// it shows in the diagnostics list with no uses yet.
    pub fn known_missing(&mut self, why: &str) {
        self.media_issue(why, "not located", 0);
    }

    /// Records a server-side issue: `kind` `media` = a background that could not be located,
    /// anything else = a note.
    pub fn note(&mut self, kind: &str, text: &str) {
        if kind == "media" {
            self.media_issue(text, "not located", 1);
            return;
        }
        let e = self.notes.entry(text.to_string()).or_default();
        if *e == 0 {
            self.new_note = Some(text.to_string());
        }
        *e += 1;
    }

    /// The rolling notices that changed since the last call: `(level, message, key)`.
    pub fn take_new(&mut self) -> Vec<(NoticeLevel, String, String)> {
        let mut out = Vec::new();
        if std::mem::take(&mut self.fonts_changed) {
            let fonts = self.fonts_sorted();
            let n = fonts.len();
            let list: Vec<String> = fonts
                .iter()
                .take(3)
                .map(|f| {
                    format!(
                        "{}{}{} → {}",
                        f.family,
                        if f.weight == 400 {
                            String::new()
                        } else {
                            format!(" {}", f.weight)
                        },
                        if f.italic { " italic" } else { "" },
                        f.used_family
                    )
                })
                .collect();
            let mut msg = format!(
                "{n} font{} not matched exactly: {}",
                if n == 1 { "" } else { "s" },
                list.join("; ")
            );
            if n > list.len() {
                msg.push_str(&format!("; and {} more", n - list.len()));
            }
            msg.push_str(" (see Diagnostics)");
            out.push((NoticeLevel::Info, msg, "fonts".to_string()));
        }
        if std::mem::take(&mut self.unsupported_changed) {
            let n = self.unsupported.len();
            let list: Vec<&str> = self
                .unsupported
                .keys()
                .take(3)
                .map(String::as_str)
                .collect();
            let more = if n > list.len() { ", …" } else { "" };
            out.push((
                NoticeLevel::Info,
                format!(
                    "{n} unsupported feature{} met: {}{more} (see Diagnostics)",
                    if n == 1 { "" } else { "s" },
                    list.join(", ")
                ),
                "unsupported".to_string(),
            ));
        }
        if let Some(note) = self.new_note.take() {
            let others = self.notes.len().saturating_sub(1);
            let msg = if others > 0 {
                format!(
                    "{note} (and {others} other note{}, see Diagnostics)",
                    if others == 1 { "" } else { "s" }
                )
            } else {
                note
            };
            out.push((NoticeLevel::Warning, msg, "notes".to_string()));
        }
        out
    }

    fn fonts_sorted(&self) -> Vec<FontIssueInfo> {
        let mut fonts: Vec<FontIssueInfo> = self.fonts.values().map(|f| f.info.clone()).collect();
        fonts.sort_by_key(|f| std::cmp::Reverse(f.uses));
        fonts
    }

    /// Issue counts.
    pub fn summary(&self, load_issues: usize) -> DiagnosticsSummary {
        let fonts = self.fonts.values().map(|f| f.info.matched.as_str());
        let (mut subs, mut misses) = (0, 0);
        for m in fonts {
            match m {
                "substituted" => subs += 1,
                "fallback" => misses += 1,
                _ => {}
            }
        }
        DiagnosticsSummary {
            font_substitutions: subs,
            font_misses: misses,
            missing_media: self.media.len() as u32,
            unsupported: self.unsupported.len() as u32,
            notes: self.notes.len() as u32,
            load_issues: load_issues as u32,
            skipped_actions: 0,
        }
    }

    /// Full report.
    pub fn report(&self, shared: &Shared) -> DiagnosticsReport {
        let count = |m: &BTreeMap<String, u64>| -> Vec<IssueCount> {
            m.iter()
                .map(|(k, n)| IssueCount {
                    what: k.clone(),
                    detail: String::new(),
                    count: *n,
                })
                .collect()
        };
        let data = shared.data();
        DiagnosticsReport {
            adapter: lock(&shared.adapter).clone(),
            uptime_seconds: shared.started.elapsed().as_secs(),
            fonts: self.fonts_sorted(),
            missing_media: self
                .media
                .iter()
                .map(|((s, p), n)| IssueCount {
                    what: s.clone(),
                    detail: p.clone(),
                    count: *n,
                })
                .collect(),
            unsupported: count(&self.unsupported),
            notes: count(&self.notes),
            load_issues: data
                .report
                .issues
                .iter()
                .map(|i| LoadIssueInfo {
                    file: std::path::Path::new(&i.file)
                        .strip_prefix(&data.root)
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| i.file.clone()),
                    error: i.error.clone(),
                    too_new: i.too_new,
                })
                .collect(),
        }
    }
}

/// A running server (API + show actor + render thread + outputs + mDNS).
pub struct RunningServer {
    /// The address the API listens on.
    pub addr: SocketAddr,
    /// Shared state (tests inspect it).
    pub shared: Arc<Shared>,
    shutdown: Option<tokio::sync::watch::Sender<bool>>,
    /// Stops the media jobs (probes and media preparation) first thing in a shutdown, before
    /// anything else stops: a conversion cut off by the stop is kept for the next start, never
    /// taken for a failure (R1-1).
    media_stop: tokio::sync::watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    mdns: Option<protocol::discover::Advertiser>,
}

impl RunningServer {
    /// Stops the recording and streams (files finished), the API (in-flight requests finish,
    /// WebSockets close), the render thread, every output and the mDNS advertisement.
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        let _ = self.media_stop.send(true);
        if let Some(ad) = self.mdns.take() {
            tokio::task::spawn_blocking(move || ad.stop()).await?;
        }
        let sh = self.shared.clone();
        tokio::task::spawn_blocking(move || {
            // First, while the render thread still runs: recordings are finished (trailer
            // written) and streams closed (X3-1).
            sh.gateways.shutdown();
            sh.mirror.stop();
            sh.codex_link.stop();
            sh.site.shutdown();
            sh.codex_member.stop();
            sh.library.stop();
            sh.live_sources.shutdown();
            sh.captions.stop();
        })
        .await?;
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(true);
        }
        match self.task.take() {
            Some(task) => task.await?,
            None => Ok(()),
        }
    }

    /// The current pairing code (what the server log shows; empty at a relay that has no code
    /// from its site yet).
    pub fn pairing_code(&self) -> String {
        self.shared
            .pairing_code()
            .map(|c| c.code)
            .unwrap_or_default()
    }
}

/// Dropped without [`RunningServer::shutdown`] (a panicking test, an aborted start-up): the
/// server still stops, without blocking the caller, which may be an async thread or a runtime
/// being torn down. The API, the render thread and the outputs stop through the shutdown
/// signal (or with the runtime's tasks); the recording and streams, the Site Manager and the
/// mDNS advertisement, which hold threads and ports of their own, stop on a thread of their
/// own.
impl Drop for RunningServer {
    fn drop(&mut self) {
        let Some(tx) = self.shutdown.take() else {
            return;
        };
        let _ = self.media_stop.send(true);
        let _ = tx.send(true);
        let (sh, mdns) = (self.shared.clone(), self.mdns.take());
        let stop = move || {
            sh.gateways.shutdown();
            sh.mirror.stop();
            sh.codex_link.stop();
            sh.library.stop();
            if let Some(ad) = mdns {
                ad.stop();
            }
            sh.site.shutdown();
            sh.codex_member.stop();
            sh.live_sources.shutdown();
            sh.captions.stop();
        };
        if let Err(e) = std::thread::Builder::new()
            .name("server-drop".into())
            .spawn(stop)
        {
            tracing::warn!("stopping the dropped server's site: {e}");
        }
    }
}

/// The environment variable that keeps FFmpeg's Vulkan Video encoders out of IDN's FFmpeg
/// probe ([`vulkan_encode_off`]).
pub const VULKAN_ENCODE_SWITCH: &str = "IDN_VULKAN_ENCODE";

/// Turns IDN's Vulkan Video encoders off for this process (`IDN_VULKAN_ENCODE=off`), unless
/// the variable is already set (the user's choice wins). The renderer owns Vulkan in this
/// process: with the switch off, IDN's FFmpeg probe makes neither its own wgpu instance nor
/// an FFmpeg Vulkan device, and no Vulkan Video encoder opens one later (for example when a
/// recording starts while the renderer draws), so nothing else creates Vulkan instances or
/// devices beside the renderer (the Vulkan loader race in the `render` crate docs; M-4).
/// VA-API, NVENC and the CPU encoders still work.
///
/// Call it at the very start of the process, before any thread is started: setting the
/// environment is unsafe while other threads may read it. The `vesper` binary does this in
/// `main`; an embedder of [`start`] must do it itself (or set the variable before launching
/// the process), as [`start`] runs inside an async runtime whose threads already exist.
pub fn vulkan_encode_off() {
    if std::env::var_os(VULKAN_ENCODE_SWITCH).is_none() {
        // SAFETY: documented to run before the process starts any thread.
        unsafe { std::env::set_var(VULKAN_ENCODE_SWITCH, "off") };
    }
}

/// Loads the data directory, starts the render thread, the outputs, the show actor, the API
/// and the mDNS advertisement.
///
/// Call [`vulkan_encode_off`] first, before the process starts any thread (the `vesper`
/// binary does): it isn't safe to set here.
pub async fn start(config: ServerConfig) -> anyhow::Result<RunningServer> {
    let t0 = Instant::now();
    let config_github_tick = config.github_tick;
    // An empty data folder (a fresh install) becomes a data directory; a missing one is
    // refused (a mistyped --data path).
    anyhow::ensure!(
        config.data_dir.is_dir(),
        "the data directory {} does not exist",
        config.data_dir.display()
    );
    let portable = config.data_dir.join("portable");
    if !portable.is_dir() {
        std::fs::create_dir_all(&portable)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", portable.display()))?;
        tracing::info!("created {}", portable.display());
    }
    // The server's identity first: a new Codex collection is named after the server.
    let dir = config.data_dir.clone();
    let trust = config.trust_localhost;
    let auth = tokio::task::spawn_blocking(move || auth::Auth::load(&dir, trust)).await??;
    let dir = config.data_dir.clone();
    let name = auth.identity().name;
    let library = codex::library::MediaLibrary::new();
    let lib = library.clone();
    let (data, codex) =
        tokio::task::spawn_blocking(move || codex::load(&dir, &name, &lib)).await??;
    tracing::info!(
        presentations = data.content.presentations.len(),
        playlists = data.content.playlists.len(),
        screens = data.screens.len(),
        looks = data.looks.len(),
        media_assets = data.content.assets.len(),
        output_bindings = data.outputs.len(),
        issues = data.report.issues.len(),
        ms = t0.elapsed().as_millis() as u64,
        "data directory loaded from {}",
        config.data_dir.display()
    );
    let too_new = data.report.issues.iter().filter(|i| i.too_new).count();
    if too_new > 0 {
        tracing::warn!("{too_new} documents come from a newer schema and were refused");
    }
    if data.screens.is_empty() {
        tracing::warn!("the data directory has no screens; nothing will be rendered");
    }
    let data = Arc::new(data);
    // Room for a collection switch's or a reload's burst of documentChanged events (one per
    // document); a client that still falls behind gets the `lagged` notice and reloads.
    let (events, _) = broadcast::channel(4096);
    library.set_events(events.clone());
    let dir = config.data_dir.clone();
    let (site_cfg, tx_events, name) = (config.site.clone(), events.clone(), auth.identity().name);
    let site =
        tokio::task::spawn_blocking(move || site::Site::load(&dir, &name, site_cfg, tx_events))
            .await??;

    let mut ropts = config.renderer.clone();
    ropts.font_substitutions = data.font_substitutions.clone();
    // Images decode on the renderer's worker pool; live renders never wait for them.
    ropts.wait_for_images = false;
    let font_dir = data.root.join("fonts");
    if font_dir.is_dir() {
        ropts.font_dirs.push(font_dir);
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let initial = ShowState::initial(&data, show::load_live_look(&data.root));
    let shared = Arc::new(Shared {
        decoder: std::sync::OnceLock::new(),
        show: Mutex::new(initial.clone()),
        data: RwLock::new(data.clone()),
        codex: RwLock::new(Arc::new(codex)),
        codex_writer: codex::writer::Writer::new(),
        uploads: codex::uploads::Uploads::new(config.codex.clone()),
        codex_member: codex::member::OwnMembership::new(&config.data_dir),
        library: library.clone(),
        codex_link: codex::link::Link::new(&config.codex),
        mirror: codex::mirror::Mirror::new(&config.codex),
        search: codex::search::SearchIndex::new(),
        backups: codex::backups::Backups::new(config.codex.backups.clone()),
        exports: codex::export::Exports::new(config.codex.export.clone()),
        adopt: codex::adopt::Adopter::new(),
        mdns_on: std::sync::atomic::AtomicBool::new(false),
        version: AtomicU64::new(0),
        screens: RwLock::new(Arc::new(Vec::new())),
        dev: config.dev.clone(),
        config_lock: Mutex::new(()),
        diagnostics: Mutex::new(Diagnostics::default()),
        adapter: Mutex::new(None),
        render_caches: Mutex::new(Vec::new()),
        fonts: Mutex::new((Vec::new(), String::new())),
        font_options: ropts.clone(),
        measurer: std::sync::OnceLock::new(),
        wake: Mutex::new(tx.clone()),
        started: Instant::now(),
        presence: presence::Presence::new(events.clone()),
        updates: updates::Updates::open(&config.data_dir, &config.update_keys)?,
        releases: Arc::new(releases::Releases::open(
            &config.data_dir,
            config.github_api.clone(),
        )),
        mosaic: {
            let mut m = mosaic::Mosaic::load(
                &config.data_dir,
                config.mosaic_poll,
                config
                    .mosaic_nodes
                    .clone()
                    .unwrap_or_else(|| Arc::new(mosaic::IdnNodes::new(false))),
            );
            m.logo_settle = config.mosaic_logo_settle;
            m
        },
        signage: Arc::new(signage::Signage::load(&config.data_dir)),
        events,
        recent_notices: Mutex::new(VecDeque::new()),
        announced: Mutex::new(Default::default()),
        status_changed: Notify::new(),
        auth,
        site,
        videos: Mutex::new(Default::default()),
        element_plays: Default::default(),
        live_sound: Mutex::new(None),
        video_thumbs: Mutex::new(thumbs::BytesLru::new(32 << 20)),
        video_durations: Mutex::new(Default::default()),
        sound: sound::Hub::start(&data.audio_device, config.audio_device.clone()),
        live_sources: live_sources::LiveSources::default(),
        displays: displays::Displays::new(
            config
                .displays
                .clone()
                .unwrap_or_else(displays::platform_source),
        ),
        audios: Mutex::new(Default::default()),
        captions: captions::Hub::with_options(&data.root, config.captions.clone()),
        gateways: gateways::Gateways::load(&config.data_dir),
        ccli: ccli::CcliLog::new(&config.data_dir),
        translate: translate::Translation::new(&data.root, &config.translation),
    });
    shared.live_sources.bind(Arc::downgrade(&shared));
    shared.captions.bind(Arc::downgrade(&shared));
    if let Some(look) = initial
        .look
        .and_then(|id| data.looks.iter().find(|l| l.id == id))
    {
        tracing::info!(look = %look.name, "active look");
    }
    if !data.report.issues.is_empty() {
        shared.notice(
            NoticeLevel::Warning,
            format!(
                "{} documents could not be loaded (see diagnostics)",
                data.report.issues.len()
            ),
        );
    }

    // FFmpeg is probed (once per process) before the render thread makes its GPU device:
    // the probe stands up FFmpeg's own Vulkan instance and device, and creating an instance
    // while another thread renders through the Vulkan loader can crash it (see the `render`
    // crate docs, "One GPU device per process"; M-4).
    tokio::task::spawn_blocking(media::warm_up).await?;
    let sh = shared.clone();
    let engine = tokio::task::spawn_blocking(move || Engine::start(sh, ropts, tx, rx)).await??;
    let sh = shared.clone();
    let d = data.clone();
    tokio::task::spawn_blocking(move || engine::apply_config(&sh, &d)).await?;

    let show = actor::spawn(shared.clone(), initial);
    let signage_show = show.clone();
    let mirror_show = show.clone();
    let thumbs = Arc::new(thumbs::Thumbnails::new(shared.clone()));
    let (stx, srx) = tokio::sync::watch::channel(false);
    let (media_stop, media_srx) = tokio::sync::watch::channel(false);
    let media_jobs = codex::worker::MediaWorker::start(
        shared.codex_server(),
        Arc::new(show.clone()),
        config.codex.media_workers,
        config.codex.media_prep,
        config.codex.converter.clone(),
        media_srx,
    );
    codex::search::start(&shared.codex_server());
    let app = api::AppState {
        shared: shared.clone(),
        codex: shared.codex_server(),
        show,
        thumbs,
        previews: Arc::new(preview::PreviewHub::default()),
        shutdown: srx.clone(),
        ui_dir: config
            .ui_dir
            .clone()
            .filter(|d| d.join("index.html").is_file()),
        media_thumbs: Arc::new(tokio::sync::Semaphore::new(4)),
        media_jobs,
        ws_ping: config.ws_ping.max(std::time::Duration::from_millis(10)),
        resplits: Default::default(),
        pptx: Arc::new(codex::pptx::PptxJobs::new(config.slide_pictures.clone())),
        remote_compat: Arc::new(remote_compat::RemoteCompat::new(config.listen.ip())),
        scripture: {
            let mut dirs = Vec::new();
            if let Some(d) = &config.scripture_dir {
                dirs.push((d.clone(), protocol::ScriptureOrigin::Bundled));
            }
            dirs.push((
                config.data_dir.join("scripture"),
                protocol::ScriptureOrigin::Local,
            ));
            Arc::new(
                tokio::task::spawn_blocking(move || vesper_scripture::Scripture::load(&dirs))
                    .await?,
            )
        },
    };
    match (&config.ui_dir, &app.ui_dir) {
        (Some(_), Some(d)) => tracing::info!("serving the control UI from {}", d.display()),
        (Some(d), None) => tracing::warn!(
            "no control UI at {} (index.html missing); / will answer 404",
            d.display()
        ),
        _ => {}
    }

    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    let addr = listener.local_addr()?;
    let identity = shared.auth.identity();
    tracing::info!(
        server = %identity.name,
        id = %identity.id,
        trust_localhost = config.trust_localhost,
        "control API listening on http://{addr}{}/",
        protocol::API_PREFIX
    );
    // Remote API compatibility (off unless switched on), on the API's interface.
    let compat = app.remote_compat.clone();
    let root = config.data_dir.clone();
    let compat_settings =
        tokio::task::spawn_blocking(move || remote_compat::load_settings(&root)).await?;
    compat.apply(&app, compat_settings).await;
    let router = api::router(app);
    // The Codex runs show boxes' forwarded writes through these routes (write-through).
    shared.codex_link.set_router(router.clone(), addr);
    let mut srx_serve = srx;
    let task = tokio::spawn(async move {
        // TCP_NODELAY: state + result are two small WebSocket frames written back to back;
        // with Nagle's algorithm the second waits for the client's delayed ACK (~40 ms).
        let listener = axum::serve::ListenerExt::tap_io(listener, |tcp| {
            if let Err(e) = tcp.set_nodelay(true) {
                tracing::debug!("TCP_NODELAY: {e}");
            }
        });
        let served = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = srx_serve.wait_for(|stop| *stop).await;
        })
        .await;
        tracing::info!("API stopped; stopping the render thread");
        tokio::task::spawn_blocking(move || engine.stop()).await?;
        tracing::info!("server stopped");
        served.map_err(anyhow::Error::from)
    });

    let mdns = if config.mdns {
        let ad = protocol::discover::Advertisement {
            id: identity.id.to_string(),
            name: identity.name.clone(),
            version: VERSION.to_string(),
            port: addr.port(),
        };
        match tokio::task::spawn_blocking(move || protocol::discover::Advertiser::start(&ad))
            .await?
        {
            Ok(a) => {
                tracing::info!(
                    "advertising {} over mDNS ({})",
                    a.fullname(),
                    protocol::MDNS_SERVICE_TYPE
                );
                shared.mdns_on.store(true, Ordering::Release);
                Some(a)
            }
            Err(e) => {
                shared.notice(
                    NoticeLevel::Warning,
                    format!("mDNS advertisement failed ({e}); clients must enter the address"),
                );
                None
            }
        }
    } else {
        None
    };

    // Hot-plug of the attached displays.
    displays::watch(shared.clone(), stx.subscribe());

    // Live meters: at most 10 a second, only while a level changes.
    let sh = shared.clone();
    let mut stop_levels = stx.subscribe();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last: (Vec<f64>, u64) = (Vec::new(), 0);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = stop_levels.wait_for(|s| *s) => break,
            }
            let now = (sh.sound.engine.levels(), sh.sound.engine.xruns());
            if now != last {
                last = now.clone();
                let _ = sh.events.send(Arc::new(ServerMessage::AudioLevels {
                    levels: now.0,
                    xruns: now.1,
                }));
            }
        }
    });

    // The site: start the Site Manager on the designated server (its events come on a thread
    // of its own), then look at what isn't an event (the approval window, PIN unlocks).
    let sh = shared.clone();
    tokio::task::spawn_blocking(move || {
        sh.site.start_up();
        // The Codex joins its own site (at once: the Site Manager runs beside it).
        sh.codex_member.follow(&sh.site, sh.codex_is_host());
        sh.codex_link
            .follow(&sh.codex_server(), sh.codex_member.enrollment());
    })
    .await?;
    // A show box follows the site's Codex (and a box that joins a site with one starts to).
    shared
        .mirror
        .start(&shared.codex_server(), Arc::new(mirror_show));
    // At a relay (a machine of another server's site) the site's code is shown, not this
    // server's own (logged once the site gives it).
    shared
        .auth
        .set_local_code_shown(!shared.site.relays_pairings());
    shared.auth.log_pairing_code();
    let sh = shared.clone();
    let mut stop_site = stx.subscribe();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(sh.site.config().tick);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = stop_site.wait_for(|s| *s) => break,
            }
            let s = sh.clone();
            let ticked = tokio::task::spawn_blocking(move || {
                s.site.tick();
                s.codex_member.follow(&s.site, s.codex_is_host());
                s.codex_link
                    .follow(&s.codex_server(), s.codex_member.enrollment());
                s.adopt.resume(&s.codex_server());
            });
            if ticked.await.is_err() {
                break;
            }
            sh.auth.set_local_code_shown(!sh.site.relays_pairings());
        }
    });

    // The Codex's upkeep: expired trash entries go, change logs due are compacted (at start,
    // then hourly).
    let sh = shared.clone();
    let mut stop_codex = stx.subscribe();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = stop_codex.wait_for(|s| *s) => break,
            }
            let cx = sh.codex_server();
            sh.codex_writer
                .tick(&cx, std::time::SystemTime::now())
                .await;
            sh.uploads.clean(&*sh, std::time::SystemTime::now()).await;
        }
    });

    // Mosaic: the site's display boxes, read every few seconds (on the site's server).
    mosaic::start(&shared, stx.subscribe());

    // Signage: the channels' screens and outputs, their schedules and feeds (on the site's
    // server).
    signage::start(&shared, signage_show, stx.subscribe());

    // Presence and edit locks site-wide (a show box reports to the site's Codex; the Codex box
    // pushes the site's list to the boxes).
    codex::site_presence::start(&shared);

    // Settings backups: this box's settings looked at every `poll` (the daily snapshot, and
    // one a while after they last changed).
    let sh = shared.clone();
    let mut stop_backups = stx.subscribe();
    tokio::spawn(async move {
        let poll = sh.backups.options().poll;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(poll) => {}
                _ = stop_backups.wait_for(|s| *s) => break,
            }
            sh.backups
                .tick(&sh.codex_server(), std::time::SystemTime::now())
                .await;
        }
    });

    // Scheduled Codex exports: the schedule looked at every `tick` (and uploaded imports that
    // waited too long dropped).
    let sh = shared.clone();
    let mut stop_exports = stx.subscribe();
    tokio::spawn(async move {
        let tick = sh.exports.options().tick;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(tick) => {}
                _ = stop_exports.wait_for(|s| *s) => break,
            }
            sh.exports.tick(&sh.codex_server()).await;
        }
    });

    // Changes noted as pending in the collections' change logs (a stop between a write and
    // its log entry, a failed append, the start's migrations): logged now, for show boxes.
    let sh = shared.clone();
    tokio::spawn(async move { sh.codex_writer.log_pending(&sh.codex_server()).await });

    // The media library's "in use" marks: at start, then after writes that change media
    // references (gathered for half a second).
    let sh = shared.clone();
    let mut stop_marks = stx.subscribe();
    shared.library.marks_due();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = sh.library.wait_marks_due() => {}
                _ = stop_marks.wait_for(|s| *s) => break,
            }
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
                _ = stop_marks.wait_for(|s| *s) => break,
            }
            sh.codex_writer.refresh_marks(&sh.codex_server()).await;
        }
    });

    // Vesper's releases on GitHub, into the site's update channel (when a check is due).
    let sh = shared.clone();
    let stop_releases = stx.subscribe();
    let tick = config_github_tick;
    tokio::spawn(async move {
        sh.releases
            .clone()
            .run(sh.clone(), tick, stop_releases)
            .await
    });

    // Rotate the pairing code on schedule so the log always shows a valid one.
    let sh = shared.clone();
    let mut stop = stx.subscribe();
    tokio::spawn(async move {
        loop {
            let left = sh.auth.pairing_code().expires_in_seconds.max(1);
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(left)) => {
                    sh.auth.rotate_if_expired();
                }
                _ = stop.wait_for(|s| *s) => break,
            }
        }
    });

    Ok(RunningServer {
        addr,
        shared,
        shutdown: Some(stx),
        media_stop,
        task: Some(task),
        mdns,
    })
}
