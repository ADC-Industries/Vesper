//! The full operator state ([`ShowState`]), broadcast by the server after every change.
//!
//! The shape models everything an operator sees from day one. Parts the server cannot produce
//! yet stay empty / default and are marked with their milestone: **M4** (video inputs). Fields in this module are never
//! omitted: `null` means "nothing there", so the shape is fixed.

use model::{
    FrameRate, Layer, LookId, MediaAssetId, MediaKind, MessageId, OutputId, PlaylistId,
    PlaylistItemId, PresentationId, PropId, ScreenId, SectionId, SlideId, StageLayoutId,
    TestPattern, TimerId, TimerKind, TokenValue, TransitionKind, VideoInputId,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Everything that is live, plus the server's screens and a diagnostics summary.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShowState {
    /// What is live on each layer.
    pub layers: LayerStates,
    /// Empty layers whose last cleared content `restoreLayer` can bring back (in the order
    /// media, slide, announcements, props, messages, audio).
    pub restorable_layers: Vec<Layer>,
    /// The active look (`null` = every layer on every screen).
    pub active_look: Option<LookId>,
    /// Transitions in progress (as of this revision; progress is not streamed).
    pub transitions: Vec<TransitionState>,
    /// What the transport controls act on when no layer is given: the foreground video when
    /// there is one, else the background video, else the audio layer (`null` = nothing
    /// playable is live). Its position is refreshed at most 4 times a second while it plays
    /// (the audio layer's own position is in `layers.audio.playback`, refreshed the same way).
    pub transport: Option<MediaTransport>,
    /// Every timer with its live value (in the timers' sort order). Between states, while a
    /// timer runs, `timerTick` messages carry the values (at most once a second).
    pub timers: Vec<TimerState>,
    /// Stage layouts chosen by slide actions (`stageLayout`) or `setStageLayout`, per stage
    /// screen (`screen: null` = every stage screen; a later choice for one screen wins).
    pub stage_layouts: Vec<StageLayoutState>,
    /// What each stage screen shows: its effective layout (see [`StageScreenState`]), in
    /// screen order. Every stage screen document is listed, running or not.
    pub stage_screens: Vec<StageScreenState>,
    /// The message shown to the performers on the stage screens (`null` = none).
    pub stage_message: Option<StageMessageState>,
    /// The server's running screens (every enabled screen document) and their outputs.
    pub screens: Vec<ScreenState>,
    /// Which screen roles are switched on (`setScreensEnabled`).
    pub screens_enabled: ScreensEnabled,
    /// Screen labels are shown on every output (`identifyScreens`; off again after 60 s).
    pub identify: bool,
    /// Show options (`setOption`).
    pub options: ShowOptions,
    /// The slide `next` would trigger (`null` = `next` does nothing), so the UI can highlight
    /// it.
    pub cursor: Option<SlidePosition>,
    /// Counts of render and load issues (details: `GET /api/v1/diagnostics`).
    pub diagnostics: DiagnosticsSummary,
    /// The audio bus: output device, master level and meters, layer levels, audio playlist
    /// options.
    pub audio: AudioState,
    /// The box-wide clock format (`setClockFormat`).
    pub clock_format: ClockFormatState,
    /// The monitors and projectors attached to this box (connected or not), refreshed when
    /// one is plugged in or out. Also `GET /api/v1/displays`.
    #[serde(default)]
    pub displays: Vec<DisplayInfo>,
    /// The object builds of the live slide (`null` = it has none): `next` takes the next
    /// build step before it moves to the next slide (see `docs/BUILDS-PLAN.md`).
    #[serde(default)]
    pub build: Option<BuildProgress>,
    /// Live captions: whether they run, how the engine is doing, the lines on screen
    /// (`docs/CAPTIONS-PLAN.md`).
    #[serde(default)]
    pub captions: CaptionsState,
}

/// Live captions as the operator sees them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CaptionsState {
    /// Captions are switched on (`CaptionsConfig.enabled`).
    pub enabled: bool,
    /// What the captions engine is doing.
    pub status: CaptionsStatus,
    /// Why it stopped (`error`), or what it is waiting for.
    pub detail: Option<String>,
    /// Download progress of the model, 0–100 (`downloading`, when its size is known).
    pub progress: Option<u32>,
    /// Bytes of the model downloaded so far (`downloading`).
    #[serde(default)]
    pub downloaded: Option<u64>,
    /// The model's size in bytes, when known (`downloading`).
    #[serde(default)]
    pub download_size: Option<u64>,
    /// The model in use (`base.en`, …).
    pub model: Option<String>,
    /// What whisper runs on: the GPU's name, or "CPU (8 threads)".
    pub device: Option<String>,
    /// `device` is a GPU (while running; servers before GPU captions leave it out).
    #[serde(default)]
    pub gpu: Option<bool>,
    /// Why whisper isn't on the processor chosen (`CaptionsConfig.processor`): no GPU, or the
    /// GPU failed and captions run on the CPU instead.
    #[serde(default)]
    pub note: Option<String>,
    /// Work per second of sound (below 1: it keeps up), rounded to hundredths.
    pub load: Option<f64>,
    /// The lines on screen, oldest first (the last may be a draft still being corrected).
    pub lines: Vec<CaptionLineState>,
}

/// What the captions engine is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum CaptionsStatus {
    /// Switched off.
    #[default]
    Off,
    /// Loading the model, choosing one (`auto`) or waiting for its source.
    Starting,
    /// Fetching the model (see `progress`).
    Downloading,
    /// Listening.
    Running,
    /// Stopped by a problem (see `detail`); switching captions off and on again retries.
    Error,
}

/// One caption line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CaptionLineState {
    /// The line's number: drafts of one line share it.
    pub line: u64,
    /// What was said.
    pub text: String,
    /// No more corrections to this line.
    #[serde(rename = "final")]
    pub is_final: bool,
}

/// `POST /api/v1/captions/lines`: a caption line from another captioner (a person typing, a
/// CART service). A draft is replaced by the next line until one is final.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PushCaption {
    /// The text (empty: takes the current draft down).
    pub text: String,
    /// The line is done (the next line starts after it); default true.
    #[serde(default = "yes", rename = "final")]
    #[ts(as = "Option<bool>", optional)]
    pub is_final: bool,
}

/// One speech model offered (`GET /api/v1/captions/models`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CaptionModel {
    /// Its name (`CaptionsConfig.model`).
    pub name: String,
    /// Its download size, MB.
    pub size_mb: u32,
    /// What it suits.
    pub note: String,
    /// It is on this box already.
    pub downloaded: bool,
}

fn yes() -> bool {
    true
}

/// How far the live slide's object builds are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BuildProgress {
    /// The layer showing the slide (`slide` or `announcements`).
    pub layer: Layer,
    /// The presentation.
    pub presentation: PresentationId,
    /// The slide.
    pub slide: SlideId,
    /// Build steps taken (0 = only the builds that play with the slide).
    pub step: u32,
    /// Build steps the slide has (`next` takes them in turn).
    pub steps: u32,
}

/// A physical display output of this box (an HDMI / DisplayPort connector).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    /// The connector (`"HDMI-A-1"`, `"DP-2"`): a display output binding's
    /// `selector.connector`.
    pub id: String,
    /// Position in the box's display list.
    pub index: u32,
    /// The monitor's model from its EDID (`"Example Monitor"`), else the connector.
    pub name: String,
    /// Manufacturer code from the EDID (`"ABC"`), when known.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// A monitor is plugged in.
    pub connected: bool,
    /// The modes the monitor offers (empty when not connected), largest first.
    pub modes: Vec<model::DisplayMode>,
    /// The monitor's preferred mode.
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred: Option<model::DisplayMode>,
    /// Where it comes from: `"kms"` (driven directly, no desktop), `"window"` (a window on
    /// this computer's desktop: Windows, or Linux with a desktop session), `"fake"` (tests).
    /// Absent from older servers (KMS).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Its top left corner on the desktop, px (`"window"` displays).
    #[ts(optional)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<DisplayPosition>,
    /// The desktop's main monitor (`"window"` displays, where the platform says): a full
    /// screen output there covers the operator's own screen.
    #[ts(optional, as = "Option<bool>")]
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub primary: bool,
    /// The output bindings that use this display.
    pub bound_to: Vec<model::OutputId>,
}

/// A monitor's position on the desktop, px.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPosition {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
}

/// The box-wide clock format: what clocks on this box's screens show where their own format
/// leaves it open (a clock data link or message token without `use24Hour` follows
/// `use24Hour`; a short time gets seconds with `seconds`). Kept on the box, across restarts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
pub struct ClockFormatState {
    /// 24-hour time.
    #[serde(rename = "use24Hour")]
    pub use_24_hour: bool,
    /// Show seconds.
    pub seconds: bool,
}

/// The audio bus (see [`ShowState::audio`]).
///
/// Every source (the background video's soundtrack, a foreground video's soundtrack, the
/// audio layer) is mixed at 48 kHz into the master bus, which plays on the machine's audio
/// device and is embedded into IDN outputs that have `embedAudio`. Without a usable device the
/// bus runs on a **null output** that keeps time (`outputState` `null` or `error`), so video
/// sync and embedded audio keep working.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AudioState {
    /// The device the bus plays on (`""` = none: null output).
    pub device: String,
    /// `ok` (playing on the device), `null` (no device configured: null output) or `error`
    /// (the configured device cannot be used: null output, see `detail`).
    pub output_state: AudioOutputState,
    /// Why the device cannot be used (with `error`).
    pub detail: Option<String>,
    /// Master gain, dB (the machine's `masterGainDb` until changed with `setVolume`).
    pub master_db: f64,
    /// Master mute.
    pub muted: bool,
    /// Peak level of each master bus channel, dBFS, rounded to whole dB (`-60` = silence or
    /// below), as of this state's revision. Live meters arrive separately in `audioLevels`
    /// messages (at most 10 a second, only while they change).
    pub levels: Vec<f64>,
    /// Buffer underruns of the output device since the server started.
    pub xruns: u64,
    /// Levels of the layers that carry audio.
    pub layers: AudioLayerVolumes,
    /// The audio playlist plays in shuffled order.
    pub shuffle: bool,
    /// The audio playlist starts over after its last item.
    pub repeat: bool,
    /// The audio playlist item playing on the audio layer (`null` = none, or a bin item).
    pub playlist: Option<PlaylistContext>,
}

/// State of the audio output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AudioOutputState {
    /// Playing on the configured device.
    Ok,
    /// No device configured: a null output keeps the audio clock running.
    #[default]
    Null,
    /// The configured device cannot be opened or failed: null output (see `detail`).
    Error,
}

/// Level of each layer that carries audio (`setVolume` / `setMute` with `target: "layer"`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AudioLayerVolumes {
    /// The audio layer (audio bins and audio playlists).
    pub audio: VolumeState,
    /// The background video's soundtrack.
    pub media: VolumeState,
    /// A foreground video's soundtrack, and the sound of the live slide's video elements.
    pub slide: VolumeState,
    /// The sound of the live source on the video-input layer.
    #[serde(default)]
    pub video_input: VolumeState,
}

/// A gain and a mute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct VolumeState {
    /// Gain, dB (0 = unity).
    pub db: f64,
    /// Muted.
    pub muted: bool,
}

/// The role of a screen: what the operator's Audience / Stage toggles switch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ScreenRole {
    /// Audience screens (projectors, streams, key feeds).
    #[default]
    Audience,
    /// Stage screens (confidence monitors).
    Stage,
}

/// Whether each screen role is switched on. A disabled screen sends black (transparent on
/// screens with alpha) while its outputs keep running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScreensEnabled {
    /// Audience screens on.
    pub audience: bool,
    /// Stage screens on.
    pub stage: bool,
}

impl Default for ScreensEnabled {
    fn default() -> Self {
        ScreensEnabled {
            audience: true,
            stage: true,
        }
    }
}

impl ScreensEnabled {
    /// Whether screens of `role` are on.
    pub fn get(&self, role: ScreenRole) -> bool {
        match role {
            ScreenRole::Audience => self.audience,
            ScreenRole::Stage => self.stage,
        }
    }

    /// Switches screens of `role`.
    pub fn set(&mut self, role: ScreenRole, enabled: bool) {
        match role {
            ScreenRole::Audience => self.audience = enabled,
            ScreenRole::Stage => self.stage = enabled,
        }
    }
}

/// Server-side show options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShowOptions {
    /// `next` past the end of a playlist presentation moves on to the next item (default
    /// `true`); when `false` it stops at the last slide.
    pub continuous_playlist: bool,
}

impl Default for ShowOptions {
    fn default() -> Self {
        ShowOptions {
            continuous_playlist: true,
        }
    }
}

/// What is live on each layer (bottom to top).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LayerStates {
    /// Video input layer: a live network source.
    pub video_input: Option<VideoInputLayerState>,
    /// Background media layer.
    pub media: Option<MediaLayerState>,
    /// A foreground video on the slide layer (it hides the slide while it is up).
    pub foreground: Option<MediaLayerState>,
    /// Slide layer.
    pub slide: Option<SlidePosition>,
    /// Announcements layer (presentations whose destination is `announcements`).
    pub announcements: Option<SlidePosition>,
    /// Props showing, in the order they were shown (later ones are drawn on top).
    pub props: Vec<PropLayerState>,
    /// Messages showing, in the order they were shown. Only the **last** one is drawn (a
    /// message shown later covers earlier ones; hiding it reveals the one before).
    pub messages: Vec<MessageLayerState>,
    /// Audio layer (an audio file from a bin or an audio playlist).
    pub audio: Option<MediaLayerState>,
}

/// A slide of a presentation: what is live on the slide / announcements layer, or the cursor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SlidePosition {
    /// The presentation.
    pub presentation: PresentationId,
    /// Its name (display convenience).
    pub presentation_name: String,
    /// The slide.
    pub slide: SlideId,
    /// 0-based index over all slides in document order.
    pub slide_index: u32,
    /// The section (group) holding the slide.
    pub section: SectionId,
    /// The section's name.
    pub section_name: String,
    /// The playlist entry it was triggered from (`next` / `prev` follow that playlist).
    pub playlist: Option<PlaylistContext>,
    /// Its place in the play order (0-based): the chosen arrangement as `next` plays it, else
    /// document order (then it equals `slideIndex`). Tells the occurrences of a repeated slide
    /// apart (the second chorus is a later step than the first). Absent when the slide is not
    /// in the play order (a slide the arrangement leaves out, triggered by hand) or was deleted
    /// while live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub step: Option<u32>,
}

/// The playlist a slide was triggered from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistContext {
    /// The playlist.
    pub playlist: PlaylistId,
    /// The item.
    pub item: PlaylistItemId,
}

/// The media (or audio) layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaLayerState {
    /// The asset.
    pub asset: MediaAssetId,
    /// Its name.
    pub name: String,
    /// Image, video or audio.
    pub kind: MediaKind,
    /// `false` when the file cannot be found on the server (the layer then draws nothing).
    pub online: bool,
    /// Playback (stills report `playing` at position 0 with no duration).
    pub playback: MediaPlayback,
}

/// The transport target (see [`ShowState::transport`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaTransport {
    /// `media` (background), `slide` (foreground video) or `audio`.
    pub layer: Layer,
    /// The asset.
    pub asset: MediaAssetId,
    /// Its name.
    pub name: String,
    /// Playing, paused or stopped (ended).
    pub state: PlaybackState,
    /// Position, seconds from the start of the played range (the in point).
    pub position: f64,
    /// Length of the played range (in → out), seconds (`null` = unknown).
    pub duration: Option<f64>,
    /// Loops at the end.
    #[serde(rename = "loop")]
    pub looping: bool,
    /// Playback speed.
    pub rate: f64,
    /// Decoded in hardware (VA-API).
    pub hw: bool,
}

/// Playback of a media item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaPlayback {
    /// Playing, paused or stopped.
    pub state: PlaybackState,
    /// Position, seconds from the start of the played range (the in point).
    pub position: f64,
    /// Length of the played range, seconds (`null` = still image or unknown).
    pub duration: Option<f64>,
    /// Loops at the end.
    #[serde(rename = "loop")]
    pub looping: bool,
    /// Playback speed (1 = normal).
    pub rate: f64,
    /// Decoded in hardware (VA-API).
    pub hw: bool,
}

/// Playback state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackState {
    /// Playing.
    Playing,
    /// Paused.
    Paused,
    /// Stopped (a video that reached its end and holds its last frame).
    Stopped,
}

/// The video input layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct VideoInputLayerState {
    /// The logical input, when it went live as one (`triggerVideoInput`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub input: Option<VideoInputId>,
    /// Its name (an input's name, else the source's).
    pub name: String,
    /// The network source received (`HOST (name)` or `ip:port`); its status is in
    /// `sourcesChanged` / `GET /sources`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// A prop on the props layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PropLayerState {
    /// The prop.
    pub prop: PropId,
    /// Its name.
    pub name: String,
}

/// A message on the messages layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessageLayerState {
    /// The message.
    pub message: MessageId,
    /// Its name.
    pub name: String,
    /// The text as drawn now (tokens substituted; timer and clock tokens as of this state).
    pub text: String,
    /// The token values it was shown with (tokens not listed use their defaults).
    pub tokens: Vec<TokenValue>,
    /// Drawn: the last message shown is the one on screen.
    pub visible: bool,
}

/// A stage layout chosen for the show (by a slide action or `setStageLayout`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StageLayoutState {
    /// The stage screen (`null` = every stage screen).
    pub screen: Option<ScreenId>,
    /// The layout.
    pub layout: StageLayoutId,
}

/// The layout a stage screen shows. It is the most specific of: a choice in
/// [`ShowState::stage_layouts`] for the screen, one for every stage screen, the screen
/// document's stored layout (`kind.layout`), the first stage layout, the built-in
/// "Current / Next" layout (`layout: null`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StageScreenState {
    /// The stage screen.
    pub screen: ScreenId,
    /// Its name.
    pub name: String,
    /// The layout it shows (`null` = the built-in "Current / Next").
    pub layout: Option<StageLayoutId>,
    /// The layout's name.
    pub layout_name: String,
}

/// The stage message (see [`crate::Command::StageMessage`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StageMessageState {
    /// The text.
    pub text: String,
    /// It was shown with `flash` (it flashes for a few seconds after it was shown).
    pub flash: bool,
}

/// A transition in progress.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TransitionState {
    /// The layer.
    pub layer: Layer,
    /// Its style.
    pub kind: TransitionKind,
    /// Progress `0..=1`.
    pub progress: f64,
    /// Total duration, seconds.
    pub duration: f64,
}

/// A timer and its live value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TimerState {
    /// The timer.
    pub id: TimerId,
    /// Its name.
    pub name: String,
    /// Its configuration (countdown, countdown to a time of day, elapsed).
    pub kind: TimerKind,
    /// A countdown keeps counting below zero.
    pub allow_overrun: bool,
    /// Running, paused, stopped, overrun or expired.
    pub state: TimerRunState,
    /// Current value, seconds, to the millisecond (negative = overrun). Countdowns show
    /// whole seconds rounded **up**, elapsed timers rounded down.
    pub current_seconds: f64,
    /// The value formatted with the default timer format (`h:mm:ss`, hours hidden while zero;
    /// `-` in front when overrun).
    pub display: String,
    /// A countdown turns amber with this many seconds (or fewer) left (`null` = never).
    pub warning_seconds: Option<f64>,
}

/// A timer's run state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TimerRunState {
    /// Counting.
    Running,
    /// Paused mid-way.
    Paused,
    /// Not started (or reset): shows its start value.
    Stopped,
    /// A countdown with `allowOverrun` went past zero and keeps counting (negative values).
    Overrun,
    /// Reached its end (zero for countdowns, `end` for elapsed timers) and stopped there.
    Expired,
}

/// One timer's value in a `timerTick` message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TimerTick {
    /// The timer.
    pub id: TimerId,
    /// Run state.
    pub state: TimerRunState,
    /// Current value, seconds.
    pub current_seconds: f64,
    /// Formatted value (see [`TimerState::display`]).
    pub display: String,
}

/// One screen of the server, with live numbers. Frame counters and the measured fps are as of
/// the state's revision: the server refreshes them with every broadcast and at least every 10 s
/// while they change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScreenState {
    /// The screen.
    pub id: ScreenId,
    /// Its name.
    pub name: String,
    /// Audience or stage.
    pub role: ScreenRole,
    /// Switched on (see [`ShowState::screens_enabled`]). A disabled screen sends black.
    pub enabled: bool,
    /// Width, px.
    pub width: u32,
    /// Height, px.
    pub height: u32,
    /// Configured frame rate.
    pub frame_rate: FrameRate,
    /// Frames offered to outputs per second, measured (rounded to 0.1).
    pub measured_fps: f64,
    /// Duration of the last render, ms (`null` = nothing rendered yet).
    pub last_render_ms: Option<f64>,
    /// The test pattern replacing the show on this screen (`setTestPattern`), or `null`.
    pub test_pattern: Option<TestPattern>,
    /// Outputs of this screen.
    pub outputs: Vec<OutputState>,
    /// A signage channel's screen (its channel's id; absent otherwise): not an operator
    /// screen, so Show leaves it out (see `docs/SIGNAGE-PLAN.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub signage: Option<String>,
}

/// One output of a screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OutputState {
    /// Display name ("IDN: Program Feed").
    pub name: String,
    /// Transport: `idn`, `omt`, `display`, `sdi`, `png`, `preview`.
    pub kind: String,
    /// Where it came from: `binding` (machine configuration) or `cli`.
    pub origin: String,
    /// The output binding (`GET /api/v1/outputs`) it runs, `null` for command-line outputs.
    pub binding: Option<OutputId>,
    /// Delivered size and rate (after the binding's mode override), and its video delay in
    /// frames.
    pub width: u32,
    /// Delivered height, px.
    pub height: u32,
    /// Delivered frame rate.
    pub frame_rate: FrameRate,
    /// Video delay, frames.
    pub delay_frames: u32,
    /// Health.
    pub status: OutputStatus,
    /// Why it is unavailable / the last error.
    pub detail: Option<String>,
    /// Frames delivered.
    pub frames_sent: u64,
    /// Frames dropped because the output was busy.
    pub frames_dropped: u64,
    /// Send errors.
    pub errors: u64,
    /// The output carries the program audio (its binding's `embedAudio`, IDN only).
    pub embed_audio: bool,
    /// Gain of the embedded audio, dB (`setVolume` with `target: "output"`).
    pub audio_db: f64,
    /// The embedded audio is muted (`setMute` with `target: "output"`).
    pub audio_muted: bool,
}

/// Health of an output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum OutputStatus {
    /// Sending frames.
    Ok,
    /// Not running (runtime missing, transport not implemented); see `detail`.
    Unavailable,
    /// Running, but sends fail; see `detail`.
    Error,
}

/// Issue counts (details: `GET /api/v1/diagnostics`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSummary {
    /// Fonts replaced through the machine's substitution table.
    pub font_substitutions: u32,
    /// Fonts that are not installed (fallback font used).
    pub font_misses: u32,
    /// Media files that could not be found or decoded.
    pub missing_media: u32,
    /// Unsupported features met while rendering.
    pub unsupported: u32,
    /// Other render notes.
    pub notes: u32,
    /// Documents that failed to load.
    pub load_issues: u32,
    /// Slide / macro actions skipped since the server started because this server cannot run
    /// their kind (video input, trigger, unknown or unsupported imported kinds) or their target
    /// is missing.
    pub skipped_actions: u32,
}
