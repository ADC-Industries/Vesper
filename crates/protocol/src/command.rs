//! Commands: everything a client can ask the server to do to the live show.
//!
//! Sent inside a WebSocket `command` message or as the body of `POST /api/v1/commands`
//! (same JSON). Commands are applied one at a time, in arrival order, by the single owner of
//! the show state; every change is broadcast to every client as a new `state`.
//!
//! Commands marked **M4** are part of the protocol already but the server answers them with
//! `notImplemented` until that milestone.
//!
//! ## Transitions
//! A layer change plays the most specific transition that is set: the command's own
//! `transition` (`triggerSlide`, `triggerMedia`; one-shot), then the slide's, then the
//! presentation's (slide layer) or the media trigger's own (media layer), then the layer's
//! override set with `setTransitionOverride` (sticky, until set back to `null`), then the
//! show's per-layer default (built in: a 0.5 s dissolve). Vendor and unknown kinds play as a
//! dissolve of the same duration.
//!
//! ## Media transport
//! `mediaPlay`, `mediaPause`, `mediaToggle`, `mediaSeek`, `mediaSkip`, `mediaRestart` and
//! `mediaStop` act on the video of `layer` (`media` = background, `slide` = foreground video)
//! or the audio of the audio layer (`audio`), or, when `layer` is omitted, on the transport
//! target shown in `state.transport` (the foreground video when there is one, else the
//! background video, else the audio layer). Positions count from the start of the played
//! range (the trigger's in point).
//!
//! ## Audio
//! An audio file goes to the audio layer (`triggerMedia` with an audio asset, or an item of an
//! audio playlist), replacing what plays there with a cross-fade. A video's soundtrack plays
//! with the video, following its trigger's audio settings (level, mute, channel routing).
//! `setVolume` / `setMute` change the master bus, a layer (`audio`, `media`, `slide`) or the
//! audio embedded into one output. Levels are not stored (a restart forgets them), except the
//! master gain set in the machine's audio device configuration (`PUT /api/v1/audio-device`).

use crate::state::ScreenRole;
use model::{
    ClearGroupId, Layer, LookId, MacroId, MediaAssetId, MessageId, OutputId, PlaylistId,
    PlaylistItemId, PresentationId, PropId, ScreenId, SlideId, StageLayoutId, TestPattern, TimerId,
    TokenValue, Transition, VideoInputId,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Command {
    /// Make a slide live. Give exactly one of `slide` (ID) or `index` (0-based, document order
    /// over all sections). With `playlistItem`, `next` / `prev` continue through that playlist.
    TriggerSlide {
        /// The presentation.
        presentation: PresentationId,
        /// Slide ID.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        slide: Option<SlideId>,
        /// Slide index (document order).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        index: Option<u32>,
        /// The playlist entry the presentation was triggered from.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        playlist_item: Option<PlaylistItemRef>,
        /// Which occurrence of a repeated slide: its place in the play order (`SlidePosition
        /// .step`). Taken when that step holds the slide; otherwise (or without it) the slide's
        /// first occurrence at or after the current position.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        step: Option<u32>,
        /// Transition for this change only (overrides the slide's, the presentation's and the
        /// layer default).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        transition: Option<Transition>,
    },
    /// Next enabled slide. At the end of a presentation triggered from a playlist, moves on to
    /// the first slide of the next presentation item of that playlist (unless the
    /// `continuousPlaylist` option is off: then it stops).
    Next,
    /// Previous enabled slide; at the start of a playlist presentation, the last slide of the
    /// previous presentation item.
    Prev,
    /// Trigger a playlist item (a presentation item starts at its first enabled slide).
    TriggerPlaylistItem {
        /// The playlist.
        playlist: PlaylistId,
        /// The item.
        item: PlaylistItemId,
    },
    /// Clear one layer.
    ClearLayer {
        /// The layer.
        layer: Layer,
    },
    /// Bring back what the last clear took off a layer (`media`, `slide`, `announcements`,
    /// `props`, `messages` or `audio`), while the layer is still empty (the state's
    /// `restorableLayers`). Slides come back without running their actions; videos and audio
    /// start again from their in point. `conflict` when the layer is not empty or nothing was
    /// cleared off it.
    RestoreLayer {
        /// The layer.
        layer: Layer,
    },
    /// Clear every layer (including announcements and audio).
    ClearAll,
    /// Run a clear group (its targets, for the layers this server has).
    ClearGroup {
        /// The clear group.
        id: ClearGroupId,
    },
    /// Activate a look (`null` = every layer on every screen).
    SetLook {
        /// The look.
        #[serde(default)]
        look: Option<LookId>,
    },
    /// Put a media asset (a media bin item) on a layer: an image or a video goes to the media
    /// layer (a foreground video to the slide layer), an audio file to the audio layer
    /// (replacing what plays there, with a cross-fade), with the display, playback and audio
    /// settings of the first bin item holding the asset (else the defaults). Triggering the
    /// video that already plays there follows its retrigger setting.
    TriggerMedia {
        /// The asset.
        asset: MediaAssetId,
        /// Target layer: `media` (background), `slide` (foreground video) or `audio`; omitted
        /// = the bin item's layer (default `media`; `audio` for audio files).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
        /// Transition for this change only (overrides the bin item's and the layer default).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        transition: Option<Transition>,
    },
    /// Switch every screen of a role on or off (the operator's Audience / Stage toggles).
    /// A disabled screen keeps its outputs running but sends black (transparent on screens
    /// with alpha); nothing is torn down, so switching back on is instant.
    SetScreensEnabled {
        /// Audience or stage screens.
        role: ScreenRole,
        /// On or off.
        enabled: bool,
    },
    /// Overlay a large label on every running screen (its name, role, size and outputs),
    /// drawn above everything, so an operator can tell which projector or feed is which.
    /// Turns itself off after 60 s. Not part of the show: previews show it too.
    IdentifyScreens {
        /// On or off.
        enabled: bool,
    },
    /// Replace a screen's picture with a test pattern (`null` = back to the show). Lasts
    /// until cleared, the screen is deleted or the server restarts.
    SetTestPattern {
        /// The screen.
        screen: ScreenId,
        /// The pattern.
        #[serde(default)]
        pattern: Option<TestPattern>,
    },
    /// The box-wide clock format (`state.clockFormat`). Only the fields given change.
    SetClockFormat {
        /// 24-hour time.
        #[serde(default, rename = "use24Hour", skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        use_24_hour: Option<bool>,
        /// Show seconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        seconds: Option<bool>,
    },
    /// Change show options. Only the options given change.
    SetOption {
        /// `next` past the end of a presentation triggered from a playlist moves on to the
        /// next playlist item (`true`, the default) or stops (`false`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        continuous_playlist: Option<bool>,
    },
    /// Start (or resume) a timer. A countdown to a time of day counts to the next occurrence
    /// of its time; an expired timer starts over.
    TimerStart {
        /// The timer.
        timer: TimerId,
    },
    /// Stop (pause) a timer; `timerStart` resumes it.
    TimerStop {
        /// The timer.
        timer: TimerId,
    },
    /// Reset a timer to its configured start value (stopped).
    TimerReset {
        /// The timer.
        timer: TimerId,
    },
    /// Set a timer's current value: `seconds`, or `timeOfDay` (seconds after local midnight,
    /// 24-hour): a countdown to a time of day counts to that time from now on, any other timer
    /// gets the seconds until it. Give exactly one. Not stored (`timerReset` goes back to the
    /// configuration; change the configuration with `PUT /api/v1/timers/{id}`).
    TimerSet {
        /// The timer.
        timer: TimerId,
        /// New value, seconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        seconds: Option<f64>,
        /// Target time of day, seconds after local midnight (`0..86400`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        time_of_day: Option<f64>,
    },
    /// Add seconds to a timer's current value (negative subtracts), in any state.
    TimerAdjust {
        /// The timer.
        timer: TimerId,
        /// Seconds to add.
        delta_seconds: f64,
    },
    /// Show a prop on the props layer (with its transition, else the layer default). Props
    /// stack: several can be up at once.
    PropShow {
        /// The prop.
        prop: PropId,
    },
    /// Hide a prop.
    PropHide {
        /// The prop.
        prop: PropId,
    },
    /// Show the prop when it is hidden, hide it when it is showing.
    PropToggle {
        /// The prop.
        prop: PropId,
    },
    /// Run a macro: its enabled actions in order, like a slide's macro action (media actions
    /// go to their layer, nested macros at most 4 deep; missing targets are skipped and
    /// counted). `notFound` for an unknown macro.
    TriggerMacro {
        /// The macro.
        #[serde(rename = "macro")]
        macro_id: MacroId,
    },
    /// Show a message on the messages layer with token values (tokens not given use their
    /// defaults). Showing a message that is up again updates its tokens and brings it to the
    /// top.
    MessageShow {
        /// The message.
        message: MessageId,
        /// Token values.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(as = "Option<Vec<TokenValue>>", optional)]
        tokens: Vec<TokenValue>,
    },
    /// Hide a message (`message` omitted = every message).
    MessageHide {
        /// The message.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        message: Option<MessageId>,
    },
    /// Choose the stage layout a stage screen shows (`layout: null` = back to the default:
    /// the first stage layout, else the built-in "Current / Next"). The choice is stored in
    /// the screen document (`kind.layout`), so it survives a restart and travels with the
    /// data folder; it replaces a choice made by a slide action for that screen.
    SetStageLayout {
        /// The stage screen.
        screen: ScreenId,
        /// The layout.
        #[serde(default)]
        layout: Option<StageLayoutId>,
    },
    /// Show a message to the performers on every stage screen (never on audience screens):
    /// in the layout's stage-message element, else in a built-in banner along the top. A new
    /// message replaces the one shown. `flash` makes it flash for a few seconds.
    StageMessage {
        /// The text (line breaks allowed; not empty).
        text: String,
        /// Flash to get attention.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(as = "Option<bool>", optional)]
        flash: bool,
    },
    /// Take the stage message down.
    StageMessageClear,
    /// Play (resume) a video; a video that ended plays again from the in point.
    MediaPlay {
        /// `media`, `slide` or `audio` (omitted = the transport target).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
    },
    /// Pause a video (it holds its frame).
    MediaPause {
        /// `media`, `slide` or `audio` (omitted = the transport target).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
    },
    /// Play when paused, pause when playing.
    MediaToggle {
        /// `media`, `slide` or `audio` (omitted = the transport target).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
    },
    /// Jump to a position (seconds from the start of the played range, clamped to it).
    MediaSeek {
        /// Position, seconds.
        #[serde(alias = "position")]
        seconds: f64,
        /// `media`, `slide` or `audio` (omitted = the transport target).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
    },
    /// Jump forward (positive) or back (negative) by some seconds.
    MediaSkip {
        /// Seconds, ±.
        seconds: f64,
        /// `media`, `slide` or `audio` (omitted = the transport target).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
    },
    /// Play again from the in point.
    MediaRestart {
        /// `media`, `slide` or `audio` (omitted = the transport target).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
    },
    /// Loop on or off for what plays (a video, or the audio layer's file): with loop on it
    /// plays again and again; with loop off it plays to its out point and ends (its end
    /// behaviour then applies). It continues from where it is. `conflict` when nothing plays.
    MediaSetLoop {
        /// Loop on.
        #[serde(rename = "loop")]
        looping: bool,
        /// `media`, `slide` or `audio` (omitted = the transport target).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
    },
    /// Stop the video and clear its layer (with its fade-out when it has one).
    MediaStop {
        /// `media`, `slide` or `audio` (omitted = the transport target).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
    },
    /// The next item of the audio playlist playing on the audio layer (in shuffled order when
    /// `shuffle` is on; after the last item, the first when `repeat` is on).
    AudioNext,
    /// The previous item of the audio playlist (restarts the item when it has played for
    /// more than 3 seconds).
    AudioPrev,
    /// Audio playlist options. Only the options given change.
    SetAudioOptions {
        /// Play the playlist in a shuffled order.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        shuffle: Option<bool>,
        /// Start over after the last item.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        repeat: Option<bool>,
    },
    /// Set a level: the master bus, a layer's audio (`layer`: `audio`, `media` or `slide`) or
    /// the audio embedded into an output (`output`). Clamped to −96 … +12 dB.
    SetVolume {
        /// What to change.
        target: VolumeTarget,
        /// The layer (`target: "layer"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
        /// The output binding (`target: "output"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        output: Option<OutputId>,
        /// Gain, dB (0 = unity).
        db: f64,
    },
    /// Mute or unmute the master bus, a layer's audio or an output's embedded audio.
    SetMute {
        /// What to change.
        target: VolumeTarget,
        /// The layer (`target: "layer"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        layer: Option<Layer>,
        /// The output binding (`target: "output"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        output: Option<OutputId>,
        /// Muted.
        muted: bool,
    },
    /// Play a test tone on the master bus (default 1 kHz at −18 dBFS for 2 seconds; at most
    /// 30 seconds), to check the device and the embedded audio.
    PlayTestTone {
        /// Frequency, Hz (20 – 20 000).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        frequency: Option<f64>,
        /// Length, seconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        seconds: Option<f64>,
    },
    /// Put a video input on the video-input layer (**M4**).
    TriggerVideoInput {
        /// The input.
        input: VideoInputId,
    },
    /// Put a live network source (`HOST (name)` from `GET /sources`, or `ip:port`) on the
    /// video-input layer, with the layer's transition unless one is given (see
    /// [`crate::sources`]).
    TriggerLiveSource {
        /// The source.
        source: String,
        /// Transition override.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        transition: Option<Transition>,
    },
    /// Replace a layer's default transition until set back (`null` = the show's default
    /// again). Not stored: a restart forgets it. For a one-off transition, give `transition`
    /// on `triggerSlide` / `triggerMedia` instead.
    SetTransitionOverride {
        /// The layer.
        layer: Layer,
        /// The transition.
        #[serde(default)]
        transition: Option<Transition>,
    },
    /// WebSocket only: push JPEG previews of a screen as binary frames (see
    /// [`crate::preview`]). A new subscription to the same screen replaces the old one.
    SubscribePreview {
        /// The screen.
        screen: ScreenId,
        /// Maximum frames per second (capped at 15). Frames are only sent when the picture
        /// changed.
        fps: f64,
        /// Width in px (height follows the screen's aspect ratio; capped at 1920).
        width: u32,
    },
    /// WebSocket only: stop previews of one screen (`null` = all).
    UnsubscribePreview {
        /// The screen.
        #[serde(default)]
        screen: Option<ScreenId>,
    },
    /// Render and cache every slide thumbnail of a presentation in the background, so the
    /// slide grid fills instantly (same as `POST /api/v1/presentations/{id}/thumbnails`).
    PrefetchThumbnails {
        /// The presentation.
        presentation: PresentationId,
        /// Thumbnail width (default 320).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        width: Option<u32>,
    },
    /// Reload the data directory and announce changed documents (`documentChanged`).
    /// Screens and output bindings changed on disk are applied live, like edits through
    /// `/api/v1/screens` and `/api/v1/outputs`.
    ReloadData,
}

impl Command {
    /// The command's `type` tag (`"triggerSlide"`), for logs.
    pub fn name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
            .unwrap_or_default()
    }
}

/// What `setVolume` / `setMute` change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum VolumeTarget {
    /// The master bus (the device and every output that embeds audio).
    Master,
    /// One layer's audio (`layer`).
    Layer,
    /// The audio embedded into one output (`output`).
    Output,
}

/// A playlist entry: the playlist and the item in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItemRef {
    /// The playlist.
    pub playlist: PlaylistId,
    /// The item.
    pub item: PlaylistItemId,
}
