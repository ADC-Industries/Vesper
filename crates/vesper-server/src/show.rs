//! In-memory show state (what is live), the commands that change it, and how each screen turns
//! it into layers.
//!
//! - **Slide layer / announcements layer:** a slide of a presentation (by document-order
//!   index), or cleared. Presentations whose destination is `announcements` go to the
//!   announcements layer, everything else to the slide layer.
//! - **Media layer:** the background (image or video) of the most recent enabled background
//!   media action at or before the triggered slide, in document order (what
//!   `render-slide --background` shows). Triggering a slide with no such action leaves the
//!   media layer as it is; an inherited background (from an earlier slide) only replaces a
//!   different asset, so stepping through a song never restarts its background video. A media
//!   playlist item or `triggerMedia` puts its asset there directly.
//! - **Videos** ([`LiveMedia::play`]): each start of a video gets a new play ID; the show actor
//!   opens a player for it ([`crate::playback`]). Triggering the asset that already plays on
//!   the layer follows the trigger's retrigger setting: `always` restarts, `never` ignores the
//!   trigger, `automatic` keeps it playing unless it ended (then it restarts). A **foreground**
//!   video ([`model::MediaLayer::Foreground`]) takes over the slide layer until the next slide
//!   trigger or a clear.
//! - **Transitions:** every layer change records a [`LayerChange`] (which transition, when),
//!   resolved as the protocol describes (command → slide → presentation / media trigger →
//!   `setTransitionOverride` → show default). Screens play it when their picture of that
//!   layer changes (see [`crate::engine`]).
//! - **Active look:** at start, the look the operator last chose (`setLook`, or a slide's look
//!   action), kept in `<data>/machine/live-look.json` ([`LIVE_LOOK_FILE`]) so a restart keeps
//!   it, "every layer" (`None`) included; a chosen look that no longer exists starts as every
//!   layer. Before any choice, the show settings' startup look (an import sets it), else every
//!   layer on every screen. Looks route layers per screen ([`model::LayerVisibility`]) and
//!   may restyle the slide through a template ([`model::TemplateOverride`]).
//!
//! ## Navigation
//! `next` / `prev` walk the **play order** of the presentation of the last triggered slide and
//! skip disabled slides. The play order ([`play_order`]) is the arrangement of the playlist item
//! the slide was triggered from, else the presentation's selected arrangement, else document
//! order: repeated sections play again, sections left out of the arrangement are skipped. The
//! position remembers *which* occurrence of a repeated slide is live ([`LiveSlide::step`]); a
//! slide triggered by ID or index is taken at its first occurrence at or after the current
//! position (wrapping), so clicking a chorus after the second verse continues with the second
//! chorus. A slide that is not in the play order (triggered by hand) sits just before the first
//! entry that comes after it in document order: `next` goes there, `prev` to the entry before. After a clear, the position stays where it
//! was, so `next` continues. When the slide was triggered from a playlist, `next` past the last
//! slide moves on to the first enabled slide of the next enabled presentation item of that
//! playlist (in that item's play order); `prev` before the first slide goes to the *last*
//! enabled slide of the previous one. Other item kinds (headers, media, video inputs,
//! placeholders) are skipped by `next` / `prev`. The stage display's next slide is what `next`
//! would trigger ([`ShowState::cursor`]).
//!
//! The `continuousPlaylist` option (default on) can stop `next` at the last slide instead.
//!
//! ## Screen roles
//! Audience and stage screens can be switched off per role (`setScreensEnabled`). A disabled
//! screen draws nothing but black (opaque screens) or transparency (screens with alpha); its
//! outputs keep running. Stage screens draw their stage layout and the stage message, never
//! the audience layers (see [`crate::stage`]).
//!
//! ## Audio layer
//! An audio file (`triggerMedia` with an audio asset, an audio playlist item) plays on the
//! audio layer, replacing what played there (the actor cross-fades them). Items of an audio
//! playlist play in order — or in a shuffled order while `shuffle` is on — and the next one
//! starts when one ends; after the last item the playlist stops, or starts over while
//! `repeat` is on. `audioNext` / `audioPrev` step through it by hand.
//!
//! ## Clear groups
//! Each [`model::ClearTarget`] clears a layer: `slide`, `media`, `announcements`, `audio`,
//! `props`, `messages` directly; `all` clears the slide, media, audio, props and messages
//! layers, or the announcements layer when its destination is `announcements` (the pair the
//! default "Clear All" stores). Video input holds nothing yet (M4); masks, audio effects and
//! timelines are ignored. The `clearAll` command clears every layer, announcements and audio
//! included.
//!
//! ## Restoring a cleared layer
//! Every clear (`clearLayer`, `clearAll`, clear groups, clear actions, `mediaStop`) keeps what
//! it took off each layer: the slide (and a foreground video), the announcement, the
//! background media, the audio file, the props or the messages showing. `restoreLayer` brings
//! it back while the layer is still empty ([`ShowState::restorable`], the state's
//! `restorableLayers`), with the layer's default transition: slides come back without running
//! their actions (the navigation position goes back to them), videos and audio start again
//! from their in point, and what was deleted since is left out. A later clear of the same
//! layer replaces what is kept; a video that ends and clears itself keeps nothing.
//!
//! ## Timers, props and messages
//! - **Timers** run in [`crate::timers::Timers`] (part of this state, so the render thread
//!   computes their values for the frame it draws).
//! - **Props** stack on the props layer in the order shown; each one dissolves in and out with
//!   its own transition (else the props layer default), independently of the others (a change
//!   record per prop in [`ShowState::slot_changes`]).
//! - **Messages**: several can be active; the **last one shown is drawn** (showing one that is
//!   up brings it to the top with its new token values; hiding the top one reveals the one
//!   below). Changes of the drawn message play the message's transition, else the messages
//!   layer default.
//!
//! ## Slide actions
//! Triggering a slide (`triggerSlide`, `next` / `prev`, a playlist item) puts it live, then
//! applies its media (as above), then runs its other enabled actions **in list order**: timer
//! (start / stop / reset / restart / set / adjust), message (show with token values / hide),
//! prop (show / hide), look, clear layer, clear group, stage layout (recorded in
//! [`ShowState::stage_layouts`]: the stage screens switch layouts) and macro (its actions in
//! order, nested at most [`MAX_MACRO_DEPTH`] deep; media actions inside macros go to their
//! layer). Disabled actions are skipped silently; kinds this server cannot run (video input,
//! trigger, unsupported or unknown imported kinds) and actions whose target is missing are
//! skipped and counted ([`ShowState::skipped_actions`], shown in the diagnostics). Action
//! delays are not honoured yet (they run at once).

use crate::data::ShowData;
use crate::timers::{Clock, Timers};
use model::{
    ActionKind, AudioSettings, ClearGroupId, ClearLayer, ClearTarget, ContentDestination, Layer,
    LayerVisibility, LookId, MediaAssetId, MediaDisplay, MediaKind, MediaLayer, MediaTrigger,
    MessageId, Playback, PlaylistId, PlaylistItem, PlaylistItemId, PlaylistItemKind,
    PlaylistNodeKind, Presentation, PresentationId, PropId, Repeat, Retrigger, Screen, ScreenId,
    ScreenKind, Size, Slide, SlideAction, SlideId, StageLayoutId, TimerId, TimerOperation,
    TokenValue, Transition,
};
use protocol::{
    ApiError, ErrorCode, PlaylistContext, PlaylistItemRef, ScreenRole, ScreensEnabled, ShowOptions,
};
use render::live::{DataContext, LiveUse};
use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// A live slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveSlide {
    /// Presentation.
    pub presentation: PresentationId,
    /// Index over all slides in document order.
    pub index: usize,
    /// The slide's ID (re-resolves the index after a reload).
    pub slide: SlideId,
    /// Playlist it was triggered from.
    pub playlist: Option<PlaylistContext>,
    /// The slide was deleted by an edit while live: the screens keep showing it (from
    /// [`ShowState::held_slide`] / [`ShowState::held_announcement`]) until the next trigger or
    /// clear, and `index` is the slide that took its place (`next` goes there, `prev` to the
    /// one before).
    pub detached: bool,
    /// Its place in the play order ([`play_order`]): which occurrence of a repeated slide is
    /// live. Checked against `index` where it is used (an edit can change the order); `None`
    /// = not in the play order.
    pub step: Option<usize>,
}

/// A copy of a live slide that an edit deleted (see [`LiveSlide::detached`]).
#[derive(Clone, Debug, PartialEq)]
pub struct HeldSlide {
    /// The slide as it was.
    pub slide: Slide,
    /// Its presentation's canvas.
    pub canvas: Size,
    /// Presentation name.
    pub presentation_name: String,
    /// Its section.
    pub section: model::SectionId,
    /// Section name.
    pub section_name: String,
}

/// A live network source on the video-input layer (see [`crate::live_sources`]).
#[derive(Clone, Debug, PartialEq)]
pub struct LiveVideoInput {
    /// The source received (`HOST (name)` or `ip:port`).
    pub source: String,
    /// The name shown (the source part of the full name).
    pub name: String,
    /// The logical input it went live as, if any.
    pub input: Option<model::VideoInputId>,
    /// How it is drawn.
    pub display: MediaDisplay,
    /// Level, mute and routing of its sound (a logical input's `VideoInput.audio`; as is
    /// for a source put live by name).
    pub audio: model::AudioSettings,
}

/// Live media on a layer (the background, or a foreground video on the slide layer).
#[derive(Clone, Debug, PartialEq)]
pub struct LiveMedia {
    /// Asset.
    pub asset: MediaAssetId,
    /// How it is drawn.
    pub display: MediaDisplay,
    /// Image or video.
    pub kind: MediaKind,
    /// How a video plays.
    pub playback: Playback,
    /// The trigger's own transition (else the layer default).
    pub transition: Option<Transition>,
    /// The video's play ID (a new one per start; the actor's player registry key). `None`
    /// for images, and for a video that could not be opened.
    pub play: Option<u64>,
    /// The video reached its end and holds its last frame (or shows black).
    pub ended: bool,
    /// Level and channel routing of its sound.
    pub audio: AudioSettings,
}

/// A change of one layer: the transition it plays and when it happened.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerChange {
    /// Increases with every change of any layer.
    pub seq: u64,
    /// What to play (already resolved; vendor kinds still need [`Transition::renderable`]).
    pub transition: Transition,
    /// When the change was applied.
    pub at: Instant,
}

/// Object builds of a live slide (see `docs/BUILDS-PLAN.md`), one per layer: `next` takes the
/// next build step of the slide the position is on before it moves on to the next slide;
/// `prev` takes one back. A slide with no entry is at step 0. The number of steps is not kept:
/// it always comes from the slide as it is now (an edit of the live slide's builds counts at
/// once).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildState {
    /// The layer showing the slide.
    pub layer: Layer,
    /// Its presentation.
    pub presentation: PresentationId,
    /// The slide.
    pub slide: SlideId,
    /// Build steps taken (0 = only the builds that play with the slide).
    pub step: usize,
    /// When the current step started (`None`: step 0, which starts with the slide, or a step
    /// taken back to, which shows settled).
    pub at: Option<Instant>,
}

/// The build step a planned slide is drawn at (see [`ScreenPlan::builds`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildPlay {
    /// Steps taken.
    pub step: usize,
    /// When step `step` started (`None`: step 0 plays from when the slide went live; a
    /// later step shows settled).
    pub at: Option<Instant>,
}

static CHANGE_SEQ: AtomicU64 = AtomicU64::new(1);
static PLAY_ID: AtomicU64 = AtomicU64::new(1);

fn next_play_id() -> u64 {
    PLAY_ID.fetch_add(1, Ordering::Relaxed)
}

/// A message on the messages layer.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveMessage {
    /// The message.
    pub message: MessageId,
    /// The token values it was shown with.
    pub tokens: Vec<TokenValue>,
}

/// How deep macros may run macros.
pub const MAX_MACRO_DEPTH: u32 = 4;

/// A timer operation (commands and timer actions).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimerOp {
    /// Start / resume.
    Start,
    /// Pause.
    Stop,
    /// Back to the configuration.
    Reset,
    /// Reset, then start.
    Restart,
    /// Set the value, seconds.
    Set(f64),
    /// Count to a time of day (seconds after midnight).
    TimeOfDay(f64),
    /// Add seconds.
    Adjust(f64),
}

/// What a clear took off a layer, kept so `restoreLayer` can bring it back (see the module
/// docs, "Restoring a cleared layer").
#[derive(Clone, Debug, PartialEq)]
pub enum Cleared {
    /// The slide layer: its slide (with the held copy of a deleted one) and foreground video.
    Slide {
        /// The slide.
        slide: Option<LiveSlide>,
        /// The held copy, when the slide was deleted while live.
        held: Option<std::sync::Arc<HeldSlide>>,
        /// A foreground video.
        foreground: Option<LiveMedia>,
    },
    /// The announcements layer.
    Announcements {
        /// The slide.
        slide: LiveSlide,
        /// The held copy, when the slide was deleted while live.
        held: Option<std::sync::Arc<HeldSlide>>,
    },
    /// The media layer.
    Media(LiveMedia),
    /// The video-input layer.
    VideoInput(LiveVideoInput),
    /// The audio layer and the audio playlist item it played.
    Audio(LiveMedia, Option<PlaylistContext>),
    /// The props showing.
    Props(Vec<PropId>),
    /// The messages showing.
    Messages(Vec<ActiveMessage>),
}

/// The layers `restoreLayer` works on, in the order the state lists them.
pub const RESTORABLE: [Layer; 7] = [
    Layer::VideoInput,
    Layer::Media,
    Layer::Slide,
    Layer::Announcements,
    Layer::Props,
    Layer::Messages,
    Layer::Audio,
];

/// What is live. Small; cloned by the render thread on every change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShowState {
    /// Slide layer (`None` = cleared).
    pub slide: Option<LiveSlide>,
    /// Announcements layer (`None` = cleared).
    pub announcements: Option<LiveSlide>,
    /// Media layer (`None` = cleared).
    pub media: Option<LiveMedia>,
    /// Video-input layer: a live network source (`None` = cleared).
    pub video_input: Option<LiveVideoInput>,
    /// Navigation position: the last triggered slide (kept across clears).
    pub position: Option<LiveSlide>,
    /// Active look (`None` = every layer on every screen).
    pub look: Option<LookId>,
    /// The last time the look changed (`setLook`, a look action, a saved edit of the live
    /// look), with the look's transition: layers whose picture changes because of it play it.
    pub look_change: Option<LayerChange>,
    /// Screen roles switched on.
    pub screens_enabled: ScreensEnabled,
    /// Show options.
    pub options: ShowOptions,
    /// Screen labels drawn over every screen (`identifyScreens`).
    pub identify: bool,
    /// Test patterns replacing a screen's picture (`setTestPattern`), by screen.
    pub test_patterns: std::collections::BTreeMap<model::ScreenId, model::TestPattern>,
    /// The deleted live slide of the slide layer (while `slide.detached`).
    pub held_slide: Option<std::sync::Arc<HeldSlide>>,
    /// The deleted live slide of the announcements layer.
    pub held_announcement: Option<std::sync::Arc<HeldSlide>>,
    /// A foreground video on the slide layer (hides the slide while it is up).
    pub slide_media: Option<LiveMedia>,
    /// The last change of each layer.
    pub changes: HashMap<Layer, LayerChange>,
    /// Per-layer default transitions set with `setTransitionOverride` (not stored).
    pub transition_overrides: HashMap<Layer, Transition>,
    /// The audio layer.
    pub audio: Option<LiveMedia>,
    /// The audio playlist item playing on the audio layer.
    pub audio_playlist: Option<PlaylistContext>,
    /// Audio playlists play shuffled.
    pub audio_shuffle: bool,
    /// Audio playlists start over after their last item.
    pub audio_repeat: bool,
    /// The shuffled order of the playing audio playlist (empty: not shuffled yet).
    pub audio_order: Vec<PlaylistItemId>,
    /// Timers.
    pub timers: Timers,
    /// Props showing, in the order shown (later ones on top).
    pub props: Vec<PropId>,
    /// Messages showing, in the order shown (the last one is drawn).
    pub messages: Vec<ActiveMessage>,
    /// The last change of each prop (keyed by the prop's UUID): props dissolve independently.
    pub slot_changes: HashMap<uuid::Uuid, LayerChange>,
    /// Stage layouts chosen by actions and `setStageLayout`: (stage screen, `None` = every
    /// stage screen; layout). See [`crate::stage::layout_id_for`].
    pub stage_layouts: Vec<(Option<ScreenId>, StageLayoutId)>,
    /// The message shown to the performers on stage screens.
    pub stage_message: Option<crate::stage::StageMessage>,
    /// The caption lines the captions layer draws, with the box's style (copied from
    /// [`crate::captions::Hub::shown`] by the actor).
    pub captions: crate::captions::Shown,
    /// Actions skipped because they cannot run here (see the module docs).
    pub skipped_actions: u64,
    /// A look-ahead copy (`cursor`, prefetch): triggering slides runs no actions.
    pub probe: bool,
    /// The last thing cleared off each layer (`restoreLayer`).
    pub cleared: HashMap<Layer, Cleared>,
    /// The box-wide clock format (`setClockFormat`), kept in [`CLOCK_FORMAT_FILE`].
    pub clock: protocol::ClockFormatState,
    /// Plays that continue another one (`mediaSetLoop`), by their play ID: opened at the old
    /// one's position, which keeps playing until the new one can (see [`crate::playback`]).
    pub continues: HashMap<u64, Continue>,
    /// Object builds of the live slide of each layer (slide, announcements) that took a step
    /// (triggering a slide starts it over at step 0; a layer's entry never touches another's).
    pub builds: HashMap<Layer, BuildState>,
}

/// A play that continues another one (see [`ShowState::continues`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Continue {
    /// The play it replaces.
    pub from: u64,
    /// Timeline seconds to start at (the old play's position in its pass).
    pub at: f64,
    /// The old play was paused: so is the new one.
    pub paused: bool,
}

/// Whether anything live uses one of `assets` (so a change of them must re-render): the
/// media, foreground video and audio layers; the presentations of the live slide, the live
/// announcement and the navigation position (their next slides too, for stage screens and
/// prefetching) and slides held after a deletion; the props and messages showing; the template
/// themes of the active look; and the stage layouts, when there are stage screens. Documents
/// are searched for the asset IDs as written, so any use of them counts.
pub fn uses_assets(state: &ShowState, data: &ShowData, assets: &[model::MediaAssetId]) -> bool {
    if assets.is_empty() {
        return false;
    }
    let media = [&state.media, &state.slide_media, &state.audio];
    if media
        .into_iter()
        .flatten()
        .any(|m| assets.contains(&m.asset))
    {
        return true;
    }
    let ids: Vec<String> = assets.iter().map(|a| a.0.to_string()).collect();
    let mentions = |doc: &dyn doc_ids::Doc| doc.mentions(&ids);
    let c = &data.content;
    let live = [&state.slide, &state.announcements, &state.position];
    if live
        .into_iter()
        .flatten()
        .filter_map(|l| c.presentation(l.presentation))
        .any(|p| mentions(p))
    {
        return true;
    }
    let held = [&state.held_slide, &state.held_announcement];
    if held.into_iter().flatten().any(|h| mentions(&h.slide)) {
        return true;
    }
    if c.props
        .iter()
        .filter(|p| state.props.contains(&p.id))
        .any(|p| mentions(p))
    {
        return true;
    }
    let shown = |id: &model::MessageId| state.messages.iter().any(|m| m.message == *id);
    if c.messages
        .iter()
        .filter(|m| shown(&m.id))
        .any(|m| mentions(m))
    {
        return true;
    }
    let look = state
        .look
        .and_then(|id| data.looks.iter().find(|l| l.id == id));
    let themes = look
        .into_iter()
        .flat_map(|l| l.screens.iter())
        .filter_map(|s| s.template_override.as_ref())
        .filter_map(|ov| c.themes.get(&ov.theme.id));
    for t in themes {
        if mentions(t) {
            return true;
        }
    }
    let stage = data
        .screens
        .iter()
        .any(|s| matches!(s.kind, ScreenKind::Stage { .. }));
    stage && c.stage_layouts.iter().any(|l| mentions(l))
}

/// A document searched for IDs as written in its JSON, for [`uses_assets`].
mod doc_ids {
    pub trait Doc {
        fn mentions(&self, ids: &[String]) -> bool;
    }
    impl<T: serde::Serialize> Doc for T {
        fn mentions(&self, ids: &[String]) -> bool {
            serde_json::to_string(self).is_ok_and(|j| ids.iter().any(|id| j.contains(id.as_str())))
        }
    }
}

/// The role a screen's on/off toggle belongs to.
pub fn screen_role(screen: &Screen) -> ScreenRole {
    match screen.kind {
        ScreenKind::Stage { .. } => ScreenRole::Stage,
        _ => ScreenRole::Audience,
    }
}

/// Why a command was refused.
#[derive(Debug)]
pub enum ShowError {
    /// Unknown presentation / slide / look / playlist.
    NotFound(String),
    /// The request does not make sense in the current state.
    Conflict(String),
    /// Invalid arguments.
    BadRequest(String),
    /// Not implemented yet.
    NotImplemented(String),
}

impl std::fmt::Display for ShowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShowError::NotFound(s)
            | ShowError::Conflict(s)
            | ShowError::BadRequest(s)
            | ShowError::NotImplemented(s) => f.write_str(s),
        }
    }
}

impl From<ShowError> for ApiError {
    fn from(e: ShowError) -> Self {
        let code = match e {
            ShowError::NotFound(_) => ErrorCode::NotFound,
            ShowError::Conflict(_) => ErrorCode::Conflict,
            ShowError::BadRequest(_) => ErrorCode::BadRequest,
            ShowError::NotImplemented(_) => ErrorCode::NotImplemented,
        };
        ApiError::new(code, e.to_string())
    }
}

/// All slides of a presentation in document order.
pub fn doc_slides(p: &Presentation) -> Vec<&Slide> {
    p.slides().collect()
}

/// The most recent enabled background media trigger at or before slide `index`, and whether
/// it is the slide's own (`false` = inherited from an earlier slide).
pub fn background_for<'a>(slides: &[&'a Slide], index: usize) -> Option<(&'a MediaTrigger, bool)> {
    if slides.is_empty() {
        return None;
    }
    let index = index.min(slides.len() - 1);
    for (i, s) in slides[..=index].iter().enumerate().rev() {
        for a in s.actions.iter().rev().filter(|a| a.enabled) {
            if let ActionKind::Media(t) = &a.kind
                && matches!(t.layer, MediaLayer::Background | MediaLayer::Unknown)
            {
                return Some((t, i == index));
            }
        }
    }
    None
}

/// The slide's own enabled foreground media trigger (a video taking over the slide layer).
pub fn foreground_of(slide: &Slide) -> Option<&MediaTrigger> {
    slide
        .actions
        .iter()
        .rev()
        .filter(|a| a.enabled)
        .find_map(|a| match &a.kind {
            ActionKind::Media(t) if t.layer == MediaLayer::Foreground => Some(t),
            _ => None,
        })
}

/// The items of a playlist (`NotFound` for unknown IDs and folders).
pub fn playlist_items(data: &ShowData, id: PlaylistId) -> Result<&[PlaylistItem], ShowError> {
    match data.content.playlists.get(&id).map(|n| &n.kind) {
        Some(PlaylistNodeKind::Playlist { items, .. }) => Ok(items),
        Some(_) => Err(ShowError::BadRequest(format!("{} is a folder", id.0))),
        None => Err(ShowError::NotFound(format!("playlist {} not found", id.0))),
    }
}

/// Whether a play loops (`loop`, `loopFor`, `loopCount` above 1).
pub fn loops(p: &model::Playback) -> bool {
    match p.repeat {
        Repeat::Loop | Repeat::LoopFor { .. } => true,
        Repeat::LoopCount { count } => count > 1,
        _ => false,
    }
}

/// The first (or last) enabled entry of a play order: (step, document index).
fn first_enabled(slides: &[&Slide], order: &[usize], from_end: bool) -> Option<(usize, usize)> {
    let on = |d: &usize| slides.get(*d).is_some_and(|s| s.enabled);
    let k = if from_end {
        order.iter().rposition(on)
    } else {
        order.iter().position(on)
    }?;
    Some((k, order[k]))
}

/// The play order of a presentation: document-order slide indices, as the arrangement of the
/// playlist item `playlist` plays them, else the presentation's selected arrangement, else
/// document order (see the module docs). Repeated sections appear again; unknown section IDs
/// are ignored; an arrangement that plays no slide at all falls back to document order.
pub fn play_order(
    data: &ShowData,
    p: &Presentation,
    playlist: Option<PlaylistContext>,
) -> Vec<usize> {
    let find = |id: model::ArrangementId| p.arrangements.iter().find(|a| a.id == id);
    let item = playlist.and_then(|c| {
        playlist_items(data, c.playlist)
            .ok()?
            .iter()
            .find(|i| i.id == c.item)
            .and_then(|i| match &i.kind {
                PlaylistItemKind::Presentation {
                    presentation,
                    arrangement,
                    ..
                } if presentation.id == p.id => *arrangement,
                _ => None,
            })
    });
    let total = p.slides().count();
    let arr = item
        .and_then(find)
        .or_else(|| p.selected_arrangement.and_then(find));
    let Some(arr) = arr else {
        return (0..total).collect();
    };
    let mut starts: HashMap<model::SectionId, (usize, usize)> = HashMap::new();
    let mut at = 0;
    for sec in &p.sections {
        starts.insert(sec.id, (at, sec.slides.len()));
        at += sec.slides.len();
    }
    let order: Vec<usize> = arr
        .sections
        .iter()
        .filter_map(|id| starts.get(id))
        .flat_map(|(s, n)| *s..*s + *n)
        .collect();
    if order.is_empty() {
        (0..total).collect()
    } else {
        order
    }
}

/// The live slide's place in `order`: its remembered step when that still holds its slide,
/// else the slide's first occurrence (`None`: not in the order, or detached).
pub fn step_in(order: &[usize], live: &LiveSlide) -> Option<usize> {
    if live.detached {
        return None;
    }
    match live.step {
        Some(k) if order.get(k) == Some(&live.index) => Some(k),
        _ => order.iter().position(|d| *d == live.index),
    }
}

/// Where the live look the operator last chose is kept, under the data folder (machine-local
/// state, not a document: no backups, no `documentChanged`).
pub const LIVE_LOOK_FILE: &str = "machine/live-look.json";

/// The contents of [`LIVE_LOOK_FILE`].
#[derive(serde::Serialize, serde::Deserialize)]
struct LiveLookFile {
    /// The look (`None` = every layer on every screen).
    look: Option<LookId>,
}

/// The live look the operator last chose (`Some(None)`: every layer), or `None` when no choice
/// was ever saved (or the file can't be read).
pub fn load_live_look(root: &Path) -> Option<Option<LookId>> {
    let path = root.join(LIVE_LOOK_FILE);
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice::<LiveLookFile>(&bytes) {
        Ok(f) => Some(f.look),
        Err(e) => {
            tracing::warn!("{} is not readable, ignored: {e}", path.display());
            None
        }
    }
}

/// Saves the live look the operator chose (atomically), for the next start.
pub fn save_live_look(root: &Path, look: Option<LookId>) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(&LiveLookFile { look })?;
    crate::store::write_atomic(&root.join(LIVE_LOOK_FILE), &bytes)
}

/// Where the box-wide clock format is kept, under the data folder (machine-local).
pub const CLOCK_FORMAT_FILE: &str = "machine/clock-format.json";

/// The box-wide clock format saved last (the default when none was saved or it can't be read).
pub fn load_clock_format(root: &Path) -> protocol::ClockFormatState {
    let path = root.join(CLOCK_FORMAT_FILE);
    let Ok(bytes) = std::fs::read(&path) else {
        return Default::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        tracing::warn!("{} is not readable, ignored: {e}", path.display());
        Default::default()
    })
}

/// Saves the box-wide clock format (atomically).
pub fn save_clock_format(root: &Path, f: protocol::ClockFormatState) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(&f)?;
    crate::store::write_atomic(&root.join(CLOCK_FORMAT_FILE), &bytes)
}

impl ShowState {
    /// The initial state for a show: nothing live; the look the operator last chose (`chosen`,
    /// from [`load_live_look`]; every layer when that look is gone), else the show settings'
    /// startup look, else every layer.
    pub fn initial(data: &ShowData, chosen: Option<Option<LookId>>) -> Self {
        let exists = |id: &LookId| data.looks.iter().any(|l| l.id == *id);
        let look = match chosen {
            Some(look) => look.filter(exists),
            None => data.settings.startup_look.filter(exists),
        };
        ShowState {
            look,
            clock: load_clock_format(&data.root),
            ..ShowState::default()
        }
    }

    /// Makes a slide live (and its background media, see the module docs). Give exactly one of
    /// `slide` and `index`; `step` picks an occurrence of a repeated slide (ignored unless that
    /// place in the play order holds the slide).
    #[allow(clippy::too_many_arguments)]
    pub fn trigger(
        &mut self,
        data: &ShowData,
        presentation: PresentationId,
        slide: Option<SlideId>,
        index: Option<usize>,
        playlist_item: Option<PlaylistItemRef>,
        step: Option<usize>,
        transition: Option<&Transition>,
    ) -> Result<(), ShowError> {
        let p = data.content.presentation(presentation).ok_or_else(|| {
            ShowError::NotFound(format!("presentation {} not found", presentation.0))
        })?;
        let slides = doc_slides(p);
        let index = match (slide, index) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(ShowError::BadRequest(
                    "give exactly one of `slide` (ID) and `index`".into(),
                ));
            }
            (None, Some(i)) if i < slides.len() => i,
            (None, Some(i)) => {
                return Err(ShowError::NotFound(format!(
                    "slide index {i} out of range (presentation has {} slides)",
                    slides.len()
                )));
            }
            (Some(id), None) => slides.iter().position(|s| s.id == id).ok_or_else(|| {
                ShowError::NotFound(format!("slide {} not in presentation", id.0))
            })?,
        };
        let playlist = match playlist_item {
            None => None,
            Some(r) => {
                let item = playlist_items(data, r.playlist)?
                    .iter()
                    .find(|i| i.id == r.item)
                    .ok_or_else(|| {
                        ShowError::NotFound(format!("item {} not in playlist", r.item.0))
                    })?;
                match &item.kind {
                    PlaylistItemKind::Presentation {
                        presentation: l, ..
                    } if l.id == presentation => {}
                    _ => {
                        return Err(ShowError::BadRequest(format!(
                            "playlist item {} does not hold presentation {}",
                            r.item.0, presentation.0
                        )));
                    }
                }
                Some(PlaylistContext {
                    playlist: r.playlist,
                    item: r.item,
                })
            }
        };
        // Which occurrence: the one asked for, else the first at or after the current position
        // (wrapping) when the position is in the same presentation and playlist item, else the
        // first.
        let order = play_order(data, p, playlist);
        let from = self
            .position
            .filter(|pos| pos.presentation == p.id && pos.playlist == playlist)
            .and_then(|pos| step_in(&order, &pos))
            .unwrap_or(0);
        let n = order.len();
        let step = step
            .filter(|k| order.get(*k) == Some(&index))
            .or_else(|| (0..n).map(|k| (from + k) % n).find(|k| order[*k] == index));
        self.go(data, p, &slides, index, step, playlist, transition);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn go(
        &mut self,
        data: &ShowData,
        p: &Presentation,
        slides: &[&Slide],
        index: usize,
        step: Option<usize>,
        playlist: Option<PlaylistContext>,
        transition: Option<&Transition>,
    ) {
        let live = LiveSlide {
            presentation: p.id,
            index,
            slide: slides[index].id,
            playlist,
            detached: false,
            step,
        };
        let slide = slides[index];
        let layer = if p.destination == ContentDestination::Announcements {
            self.announcements = Some(live);
            self.held_announcement = None;
            Layer::Announcements
        } else {
            self.slide = Some(live);
            self.held_slide = None;
            // The slide takes the slide layer back from a foreground video, unless it starts
            // one itself.
            self.slide_media = None;
            Layer::Slide
        };
        let t = Transition::resolve(
            transition.or(slide.transition.as_ref()),
            p.transition.as_ref(),
            self.layer_default(data, layer),
        )
        .clone();
        self.record_change(layer, t);
        self.position = Some(live);
        // The slide starts at step 0; the other layer's builds stay where they are.
        self.builds.remove(&layer);
        if layer == Layer::Slide
            && let Some(fg) = foreground_of(slide)
        {
            self.apply_media(data, Layer::Slide, fg, true, transition);
        }
        if let Some((m, own)) = background_for(slides, index) {
            self.apply_media(data, Layer::Media, m, own, transition);
        }
        if !self.probe {
            self.run_actions(data, &slide.actions, true, 0, Clock::now());
        }
    }

    /// The live slide of `layer` (slide or announcements) whose builds can step: shown and
    /// not detached (a foreground video on the slide layer hides it).
    fn build_slide(&self, layer: Layer) -> Option<LiveSlide> {
        match layer {
            Layer::Announcements => self.announcements,
            Layer::Slide => self.slide.filter(|_| self.slide_media.is_none()),
            _ => None,
        }
        .filter(|l| !l.detached)
    }

    /// The build progress of the slide live on `layer`, while that layer still shows it
    /// (`None`: no step taken, step 0).
    pub fn layer_build(&self, layer: Layer) -> Option<&BuildState> {
        let b = self.builds.get(&layer)?;
        let live = self.build_slide(layer)?;
        (live.presentation == b.presentation && live.slide == b.slide).then_some(b)
    }

    /// The live slide the position is on (the one `next` / `prev` step the builds of), with
    /// its layer.
    fn position_slide<'a>(&self, data: &'a ShowData) -> Option<(Layer, LiveSlide, &'a Slide)> {
        let pos = self.position?;
        let (layer, live) = [Layer::Slide, Layer::Announcements]
            .into_iter()
            .filter_map(|l| self.build_slide(l).map(|live| (l, live)))
            .find(|(_, live)| live.presentation == pos.presentation && live.slide == pos.slide)?;
        let slide = data
            .content
            .presentation(live.presentation)?
            .slides()
            .nth(live.index)
            .filter(|s| s.id == live.slide)?;
        Some((layer, live, slide))
    }

    /// The builds `next` / `prev` step (those of the live slide the position is on, while it
    /// has builds), with the number of steps the slide has now.
    pub fn build_progress(&self, data: &ShowData) -> Option<(BuildState, usize)> {
        let (layer, live, slide) = self.position_slide(data)?;
        if model::build::steps(slide).is_empty() {
            return None;
        }
        let steps = model::build::step_count(slide);
        let b = self.layer_build(layer).copied().unwrap_or(BuildState {
            layer,
            presentation: live.presentation,
            slide: live.slide,
            step: 0,
            at: None,
        });
        Some((
            BuildState {
                step: b.step.min(steps),
                ..b
            },
            steps,
        ))
    }

    /// Takes a build step of the live slide the position is on forward (`delta > 0`) or back;
    /// `false` when there is none to take that way (`next` / `prev` then move to another
    /// slide).
    fn build_step(&mut self, data: &ShowData, delta: i64) -> bool {
        let Some((b, steps)) = self.build_progress(data) else {
            return false;
        };
        let step = match delta {
            d if d > 0 && b.step < steps => b.step + 1,
            d if d < 0 && b.step > 0 => b.step - 1,
            _ => return false,
        };
        self.builds.insert(
            b.layer,
            BuildState {
                step,
                // Forward plays the step; back shows the earlier step settled.
                at: (delta > 0).then(Instant::now),
                ..b
            },
        );
        true
    }

    /// The default transition of `layer`: the `setTransitionOverride` one, else the show's.
    pub fn layer_default<'a>(&'a self, data: &'a ShowData, layer: Layer) -> &'a Transition {
        self.transition_overrides
            .get(&layer)
            .unwrap_or_else(|| data.settings.default_transitions.for_layer(layer))
    }

    /// Puts a live network source on the video-input layer (see [`crate::live_sources`]).
    pub fn trigger_live_source(
        &mut self,
        data: &ShowData,
        source: &str,
        transition: Option<&Transition>,
    ) -> Result<(), ShowError> {
        let source = source.trim();
        if source.is_empty() || source.len() > 512 {
            return Err(ShowError::BadRequest(
                "give a source name (`HOST (name)`) or address (`ip:port`)".into(),
            ));
        }
        let name = output::recv::source_part(source).to_string();
        let same = self
            .video_input
            .as_ref()
            .is_some_and(|v| v.source == source && v.input.is_none());
        if same {
            return Ok(());
        }
        self.video_input = Some(LiveVideoInput {
            source: source.to_string(),
            name,
            input: None,
            display: MediaDisplay::default(),
            audio: model::AudioSettings::default(),
        });
        let t = transition
            .cloned()
            .unwrap_or_else(|| self.layer_default(data, Layer::VideoInput).clone());
        self.record_change(Layer::VideoInput, t);
        Ok(())
    }

    /// Puts video input `input` on the video-input layer: the network source it is bound to
    /// on this machine (its `InputBinding`), drawn as `display` says (else whole, with the
    /// input's alpha setting), its sound as `audio` says (else, or when that is the default,
    /// the input's own `audio`), with `transition` (else the layer's). The same input already
    /// live (drawn and heard the same) is left as it is.
    pub fn trigger_video_input(
        &mut self,
        data: &ShowData,
        input: model::VideoInputId,
        display: Option<&MediaDisplay>,
        audio: Option<&model::AudioSettings>,
        transition: Option<&Transition>,
    ) -> Result<(), ShowError> {
        use crate::data::InputProblem;
        let source = data.input_source(input).map_err(|p| match p {
            InputProblem::Missing => {
                ShowError::NotFound(format!("video input {} not found", input.0))
            }
            InputProblem::Unbound => ShowError::Conflict(format!(
                "video input {} is not bound to a source on this machine",
                input.0
            )),
            InputProblem::Unsupported(what) => ShowError::Conflict(format!(
                "video input {} is bound to {what}, which this server doesn't receive yet",
                input.0
            )),
        })?;
        let doc = data.video_inputs.iter().find(|v| v.id == input);
        let mut display = display.cloned().unwrap_or_default();
        if display.alpha == model::SourceAlpha::Auto
            && let Some(v) = doc
        {
            display.alpha = v.alpha;
        }
        let name = input_name(data, input, &source);
        let live = LiveVideoInput {
            source,
            name,
            input: Some(input),
            display,
            audio: audio
                .filter(|a| **a != model::AudioSettings::default())
                .cloned()
                .or_else(|| doc.map(|v| v.audio.clone()))
                .unwrap_or_default(),
        };
        if self.video_input.as_ref() == Some(&live) {
            return Ok(());
        }
        self.video_input = Some(live);
        let t = transition
            .cloned()
            .unwrap_or_else(|| self.layer_default(data, Layer::VideoInput).clone());
        self.record_change(Layer::VideoInput, t);
        Ok(())
    }

    /// Records a change of `layer` playing `transition` (now).
    pub fn record_change(&mut self, layer: Layer, transition: Transition) {
        // New content on the layer: what an earlier clear kept is no longer offered.
        if self.taken(layer).is_some() {
            self.cleared.remove(&layer);
        }
        self.changes.insert(
            layer,
            LayerChange {
                seq: CHANGE_SEQ.fetch_add(1, Ordering::Relaxed),
                transition,
                at: Instant::now(),
            },
        );
    }

    /// The live media of `layer` (`media`, or `slide` for a foreground video).
    pub fn layer_media(&self, layer: Layer) -> Option<&LiveMedia> {
        match layer {
            Layer::Media => self.media.as_ref(),
            Layer::Slide => self.slide_media.as_ref(),
            _ => None,
        }
    }

    /// The live media of `layer`, to change it.
    pub fn layer_media_mut(&mut self, layer: Layer) -> Option<&mut LiveMedia> {
        self.layer_slot(layer).and_then(|s| s.as_mut())
    }

    fn layer_slot(&mut self, layer: Layer) -> Option<&mut Option<LiveMedia>> {
        match layer {
            Layer::Media => Some(&mut self.media),
            Layer::Slide => Some(&mut self.slide_media),
            _ => None,
        }
    }

    /// Puts `trigger`'s asset on `layer` (`media` or `slide`), following the retrigger rules
    /// (see the module docs). `explicit`: triggered now (a slide's own action, a bin item, a
    /// playlist item) rather than inherited from an earlier slide. Returns whether the layer
    /// changed.
    pub fn apply_media(
        &mut self,
        data: &ShowData,
        layer: Layer,
        trigger: &MediaTrigger,
        explicit: bool,
        transition: Option<&Transition>,
    ) -> bool {
        let kind = data
            .content
            .assets
            .get(&trigger.asset)
            .map(|a| a.kind)
            .unwrap_or(MediaKind::Unknown);
        let same = self
            .layer_media(layer)
            .filter(|c| c.asset == trigger.asset)
            .cloned();
        let mut restart_same = false;
        if let Some(cur) = &same {
            if !explicit {
                return false;
            }
            if kind != MediaKind::Video {
                if cur.display == trigger.display {
                    return false;
                }
            } else {
                match trigger.playback.retrigger {
                    Retrigger::Never => return false,
                    Retrigger::Automatic | Retrigger::Unknown if !cur.ended => return false,
                    _ => restart_same = true,
                }
            }
        }
        let live = LiveMedia {
            asset: trigger.asset,
            display: trigger.display.clone(),
            kind,
            playback: trigger.playback.clone(),
            transition: trigger.transition.clone(),
            play: (kind == MediaKind::Video).then(next_play_id),
            ended: false,
            audio: trigger.audio.clone(),
        };
        let t = if restart_same {
            // Restarting the same video: no dissolve from its own last frame.
            Transition::CUT
        } else {
            Transition::resolve(
                transition.or(trigger.transition.as_ref()),
                None,
                self.layer_default(data, layer),
            )
            .clone()
        };
        if let Some(slot) = self.layer_slot(layer) {
            *slot = Some(live);
            self.record_change(layer, t);
            true
        } else {
            false
        }
    }

    /// The transition of clearing `layer`: a fade-out of the video there when it has one
    /// enabled, else the layer default.
    fn clear_transition(&self, data: &ShowData, layer: Layer) -> Transition {
        if let Some(m) = self.layer_media(layer)
            && m.kind == MediaKind::Video
            && let Some(s) = m
                .playback
                .fade_out
                .effective(data.settings.default_media_fade)
        {
            return Transition::dissolve(s);
        }
        self.layer_default(data, layer).clone()
    }

    /// Steps `delta` (+1 / -1) enabled slides along the play order from the position,
    /// crossing into the next / previous presentation item of the playlist context. Returns
    /// `false` when there is nowhere to go (nothing changes).
    pub fn step(&mut self, data: &ShowData, delta: i64) -> Result<bool, ShowError> {
        let cur = self
            .position
            .ok_or_else(|| ShowError::Conflict("no presentation has been triggered yet".into()))?;
        let p = data.content.presentation(cur.presentation).ok_or_else(|| {
            ShowError::NotFound(format!("presentation {} not found", cur.presentation.0))
        })?;
        // Builds of the live slide first (a look-ahead skips them: it wants the next slide).
        if !self.probe && self.build_step(data, delta) {
            return Ok(true);
        }
        let slides = doc_slides(p);
        let order = play_order(data, p, cur.playlist);
        let n = order.len() as i64;
        // The step before the first candidate (in the direction of travel).
        let mut i = match step_in(&order, &cur) {
            Some(k) => k as i64,
            // Not in the order (triggered by hand, or deleted while live: `index` is the
            // slide that took its place): it sits just before the first entry that comes
            // after it in document order.
            None => {
                let after = order
                    .iter()
                    .position(|d| *d > cur.index || (cur.detached && *d == cur.index))
                    .map_or(n, |k| k as i64);
                if delta > 0 { after - 1 } else { after }
            }
        };
        loop {
            i += delta;
            if i < 0 || i >= n {
                break;
            }
            let d = order[i as usize];
            if slides[d].enabled {
                self.go(data, p, &slides, d, Some(i as usize), cur.playlist, None);
                return Ok(true);
            }
        }
        // Past either end: continue in the playlist, if any (forwards only while the
        // continuous-playlist option is on).
        let Some(ctx) = cur.playlist else {
            return Ok(false);
        };
        if delta > 0 && !self.options.continuous_playlist {
            return Ok(false);
        }
        let Ok(items) = playlist_items(data, ctx.playlist) else {
            return Ok(false);
        };
        let Some(at) = items.iter().position(|it| it.id == ctx.item) else {
            return Ok(false);
        };
        let mut j = at as i64;
        loop {
            j += delta;
            if j < 0 || j >= items.len() as i64 {
                return Ok(false);
            }
            let item = &items[j as usize];
            if !item.enabled {
                continue;
            }
            if let PlaylistItemKind::Presentation { presentation, .. } = &item.kind
                && let Some(np) = data.content.presentation(presentation.id)
            {
                let ns = doc_slides(np);
                let ctx = PlaylistContext {
                    playlist: ctx.playlist,
                    item: item.id,
                };
                let no = play_order(data, np, Some(ctx));
                if let Some((k, d)) = first_enabled(&ns, &no, delta < 0) {
                    self.go(data, np, &ns, d, Some(k), Some(ctx), None);
                    return Ok(true);
                }
            }
        }
    }

    /// Triggers a playlist item: a presentation starts at its first enabled slide, a media
    /// item goes to the media layer.
    pub fn trigger_playlist_item(
        &mut self,
        data: &ShowData,
        playlist: PlaylistId,
        item: PlaylistItemId,
    ) -> Result<(), ShowError> {
        let it = playlist_items(data, playlist)?
            .iter()
            .find(|i| i.id == item)
            .ok_or_else(|| ShowError::NotFound(format!("item {} not in playlist", item.0)))?;
        match &it.kind {
            PlaylistItemKind::Presentation { presentation, .. } => {
                let p = data.content.presentation(presentation.id).ok_or_else(|| {
                    ShowError::NotFound(format!(
                        "presentation {} ({}) is not in the library",
                        presentation.id.0,
                        presentation.name.as_deref().unwrap_or("?")
                    ))
                })?;
                let slides = doc_slides(p);
                let ctx = PlaylistContext { playlist, item };
                let order = play_order(data, p, Some(ctx));
                let (k, d) = first_enabled(&slides, &order, false).ok_or_else(|| {
                    ShowError::Conflict(format!("presentation {:?} has no enabled slide", p.name))
                })?;
                self.go(data, p, &slides, d, Some(k), Some(ctx), None);
                Ok(())
            }
            PlaylistItemKind::Media(t) => {
                self.check_playable(data, t.asset)?;
                let kind = data.content.assets.get(&t.asset).map(|a| a.kind);
                let layer = match t.layer.layer() {
                    _ if kind == Some(MediaKind::Audio) => Layer::Audio,
                    Layer::Audio => {
                        return Err(ShowError::BadRequest(
                            "only audio files can go to the audio layer".into(),
                        ));
                    }
                    Layer::Slide => Layer::Slide,
                    _ => Layer::Media,
                };
                if layer == Layer::Audio {
                    let ctx = PlaylistContext { playlist, item };
                    if self.audio_playlist.map(|c| c.playlist) != Some(playlist) {
                        self.audio_order.clear();
                    }
                    self.apply_audio(t, Some(ctx));
                } else {
                    self.apply_media(data, layer, t, true, None);
                }
                Ok(())
            }
            PlaylistItemKind::VideoInput(v) => self.trigger_video_input(
                data,
                v.input.id,
                Some(&v.display),
                Some(&v.audio),
                v.transition.as_ref(),
            ),
            _ => Err(ShowError::BadRequest(
                "headers and placeholders cannot be triggered".into(),
            )),
        }
    }

    /// Puts a media asset on a layer (see `triggerMedia`): an image or video on the media
    /// layer, a foreground video on the slide layer. The settings of the first media bin item
    /// holding the asset are used (else the defaults); `layer` overrides its layer.
    pub fn trigger_media(
        &mut self,
        data: &ShowData,
        asset: MediaAssetId,
        layer: Option<Layer>,
        transition: Option<&Transition>,
    ) -> Result<(), ShowError> {
        self.check_playable(data, asset)?;
        let trigger = data
            .content
            .media_bins
            .iter()
            .flat_map(|b| match &b.kind {
                model::MediaBinKind::Manual { items } => items.as_slice(),
                model::MediaBinKind::Watched { last_scan, .. } => last_scan.as_slice(),
                _ => &[],
            })
            .find(|i| i.trigger.asset == asset)
            .map(|i| i.trigger.clone())
            .unwrap_or_else(|| MediaTrigger::new(asset, MediaLayer::Background));
        let kind = data.content.assets.get(&asset).map(|a| a.kind);
        if kind == Some(MediaKind::Audio) {
            if layer.is_some_and(|l| l != Layer::Audio) {
                return Err(ShowError::BadRequest(
                    "audio files play on the audio layer".into(),
                ));
            }
            self.apply_audio(&trigger, None);
            return Ok(());
        }
        let target = match layer.unwrap_or_else(|| trigger.layer.layer()) {
            Layer::Media | Layer::VideoInput => Layer::Media,
            Layer::Slide => Layer::Slide,
            Layer::Audio => {
                return Err(ShowError::BadRequest(
                    "only audio files can go to the audio layer".into(),
                ));
            }
            other => {
                return Err(ShowError::BadRequest(format!(
                    "media cannot go to the {other:?} layer (use media, slide or audio)"
                )));
            }
        };
        if target == Layer::Slide && kind != Some(MediaKind::Video) {
            return Err(ShowError::BadRequest(
                "only videos can go to the slide layer (foreground)".into(),
            ));
        }
        self.apply_media(data, target, &trigger, true, transition);
        Ok(())
    }

    /// Puts an audio file on the audio layer (from an audio playlist when `playlist` is set).
    /// The same file already playing follows the trigger's retrigger setting.
    pub fn apply_audio(&mut self, trigger: &MediaTrigger, playlist: Option<PlaylistContext>) {
        if let Some(cur) = &self.audio
            && cur.asset == trigger.asset
            && self.audio_playlist == playlist
        {
            match trigger.playback.retrigger {
                Retrigger::Never => return,
                Retrigger::Automatic | Retrigger::Unknown if !cur.ended => return,
                _ => {}
            }
        }
        self.audio = Some(LiveMedia {
            asset: trigger.asset,
            display: MediaDisplay::default(),
            kind: MediaKind::Audio,
            playback: trigger.playback.clone(),
            transition: None,
            play: Some(next_play_id()),
            ended: false,
            audio: trigger.audio.clone(),
        });
        self.audio_playlist = playlist;
        if playlist.is_none() {
            self.audio_order.clear();
        }
        self.record_change(Layer::Audio, Transition::CUT);
    }

    /// The playing audio playlist's items in play order (enabled audio items only; shuffled
    /// while `shuffle` is on — the order is drawn once and kept).
    pub(crate) fn audio_items(&mut self, data: &ShowData) -> Vec<(PlaylistItemId, MediaTrigger)> {
        let Some(ctx) = self.audio_playlist else {
            return Vec::new();
        };
        let Ok(items) = playlist_items(data, ctx.playlist) else {
            return Vec::new();
        };
        let mut list: Vec<(PlaylistItemId, MediaTrigger)> = items
            .iter()
            .filter(|i| i.enabled)
            .filter_map(|i| match &i.kind {
                PlaylistItemKind::Media(t)
                    if data.content.assets.get(&t.asset).map(|a| a.kind)
                        == Some(MediaKind::Audio) =>
                {
                    Some((i.id, t.clone()))
                }
                _ => None,
            })
            .collect();
        if self.audio_shuffle {
            let fresh = self.audio_order.len() != list.len()
                || list.iter().any(|(id, _)| !self.audio_order.contains(id));
            if fresh {
                // Shuffled once (Fisher–Yates), the current item first.
                let mut ids: Vec<PlaylistItemId> = list.iter().map(|(id, _)| *id).collect();
                let mut seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(1)
                    ^ (ids.len() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    ^ std::process::id() as u64;
                for i in (1..ids.len()).rev() {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    ids.swap(i, (seed % (i as u64 + 1)) as usize);
                }
                if let Some(p) = ids.iter().position(|id| *id == ctx.item) {
                    let cur = ids.remove(p);
                    ids.insert(0, cur);
                }
                self.audio_order = ids;
            }
            let order = self.audio_order.clone();
            list.sort_by_key(|(id, _)| order.iter().position(|o| o == id).unwrap_or(usize::MAX));
        }
        list
    }

    /// Steps the audio playlist by `delta` items (`repeat` wraps around). `Ok(false)` when
    /// there is nowhere to go.
    pub fn audio_step(&mut self, data: &ShowData, delta: i64) -> Result<bool, ShowError> {
        let ctx = self
            .audio_playlist
            .ok_or_else(|| ShowError::Conflict("no audio playlist is playing".into()))?;
        let list = self.audio_items(data);
        if list.is_empty() {
            return Ok(false);
        }
        let at = list.iter().position(|(id, _)| *id == ctx.item);
        let n = list.len() as i64;
        let next = match at {
            Some(i) => i as i64 + delta,
            None => 0,
        };
        let next = if (0..n).contains(&next) {
            next
        } else if self.audio_repeat {
            next.rem_euclid(n)
        } else {
            return Ok(false);
        };
        let (id, t) = list[next as usize].clone();
        let mut t = t;
        // Stepping always (re)starts the item.
        t.playback.retrigger = Retrigger::Always;
        self.apply_audio(
            &t,
            Some(PlaylistContext {
                playlist: ctx.playlist,
                item: id,
            }),
        );
        Ok(true)
    }

    /// Sets the audio playlist options.
    pub fn set_audio_options(&mut self, shuffle: Option<bool>, repeat: Option<bool>) {
        if let Some(s) = shuffle
            && s != self.audio_shuffle
        {
            self.audio_shuffle = s;
            self.audio_order.clear();
        }
        if let Some(r) = repeat {
            self.audio_repeat = r;
        }
    }

    /// Refuses media that cannot be played: unknown assets, unknown kinds.
    fn check_playable(&self, data: &ShowData, asset: MediaAssetId) -> Result<(), ShowError> {
        let a = data
            .content
            .assets
            .get(&asset)
            .ok_or_else(|| ShowError::NotFound(format!("media asset {} not found", asset.0)))?;
        match a.kind {
            MediaKind::Image | MediaKind::Video | MediaKind::Audio => Ok(()),
            MediaKind::Unknown => Err(ShowError::BadRequest(format!(
                "media asset {} has an unknown kind",
                a.name
            ))),
        }
    }

    /// Clears a layer (layers that hold nothing yet are a no-op). Clearing the slide layer
    /// also clears a foreground video.
    pub fn clear_layer(&mut self, data: &ShowData, layer: Layer) -> Result<(), ShowError> {
        if let Some(c) = self.taken(layer) {
            self.cleared.insert(layer, c);
        }
        let had = match layer {
            Layer::Slide => self.slide.is_some() || self.slide_media.is_some(),
            Layer::Announcements => self.announcements.is_some(),
            Layer::Media => self.media.is_some(),
            Layer::Audio => self.audio.is_some(),
            Layer::Props => {
                for id in self.props.clone() {
                    self.hide_prop(data, id);
                }
                return Ok(());
            }
            Layer::Messages => {
                self.hide_message(data, None);
                return Ok(());
            }
            Layer::Captions => {
                // The lines come off the screens; captions keep running (the actor clears
                // the hub's lines too).
                self.captions.lines.clear();
                return Ok(());
            }
            Layer::VideoInput => self.video_input.is_some(),
            Layer::Unknown => return Err(ShowError::BadRequest("unknown layer".into())),
        };
        if !had {
            return Ok(());
        }
        let t = self.clear_transition(data, layer);
        match layer {
            Layer::Slide => {
                self.slide = None;
                self.slide_media = None;
            }
            Layer::Announcements => self.announcements = None,
            Layer::Media => self.media = None,
            Layer::VideoInput => self.video_input = None,
            Layer::Audio => {
                self.audio = None;
                self.audio_playlist = None;
            }
            _ => {}
        }
        self.record_change(layer, t);
        Ok(())
    }

    /// What clearing `layer` would take off it (`None`: it is empty).
    fn taken(&self, layer: Layer) -> Option<Cleared> {
        match layer {
            Layer::Slide if self.slide.is_some() || self.slide_media.is_some() => {
                Some(Cleared::Slide {
                    slide: self.slide,
                    held: self.held_slide.clone(),
                    foreground: self.slide_media.clone(),
                })
            }
            Layer::Announcements => self.announcements.map(|slide| Cleared::Announcements {
                slide,
                held: self.held_announcement.clone(),
            }),
            Layer::Media => self.media.clone().map(Cleared::Media),
            Layer::VideoInput => self.video_input.clone().map(Cleared::VideoInput),
            Layer::Audio => self
                .audio
                .clone()
                .map(|m| Cleared::Audio(m, self.audio_playlist)),
            Layer::Props if !self.props.is_empty() => Some(Cleared::Props(self.props.clone())),
            Layer::Messages if !self.messages.is_empty() => {
                Some(Cleared::Messages(self.messages.clone()))
            }
            _ => None,
        }
    }

    /// The layers `restoreLayer` can bring back now: each one is empty and something was
    /// cleared off it.
    pub fn restorable(&self) -> Vec<Layer> {
        RESTORABLE
            .into_iter()
            .filter(|l| self.cleared.contains_key(l) && self.taken(*l).is_none())
            .collect()
    }

    /// Brings back what the last clear took off an empty layer (see the module docs). Slides
    /// come back without running their actions; videos and audio start again from their in
    /// point; props, messages, slides and media deleted since are left out; a logical video
    /// input comes back on its binding as it is now (unbound or deleted since: left out).
    pub fn restore_layer(&mut self, data: &ShowData, layer: Layer) -> Result<(), ShowError> {
        if !RESTORABLE.contains(&layer) {
            return Err(ShowError::BadRequest(format!(
                "the {layer:?} layer cannot be restored"
            )));
        }
        if self.taken(layer).is_some() {
            return Err(ShowError::Conflict(format!(
                "the {layer:?} layer is not empty"
            )));
        }
        let c = self.cleared.remove(&layer).ok_or_else(|| {
            ShowError::Conflict(format!("nothing was cleared off the {layer:?} layer"))
        })?;
        let fresh = |m: LiveMedia| -> Option<LiveMedia> {
            data.content
                .assets
                .contains_key(&m.asset)
                .then(|| LiveMedia {
                    play: m.play.map(|_| next_play_id()),
                    ended: false,
                    ..m
                })
        };
        // A slide that still exists comes back at its current index.
        let find = |l: LiveSlide, held: Option<std::sync::Arc<HeldSlide>>| {
            let p = data.content.presentation(l.presentation);
            match p.and_then(|p| p.slides().position(|s| s.id == l.slide)) {
                Some(i) => Some((
                    LiveSlide {
                        index: i,
                        detached: false,
                        ..l
                    },
                    None,
                )),
                None if l.detached && held.is_some() => Some((l, held)),
                None => None,
            }
        };
        let gone = || ShowError::NotFound("what was cleared is not there any more".into());
        match c {
            Cleared::Slide {
                slide,
                held,
                foreground,
            } => {
                let slide = slide.and_then(|l| find(l, held));
                let fg = foreground.and_then(fresh);
                if slide.is_none() && fg.is_none() {
                    return Err(gone());
                }
                if let Some((l, h)) = slide {
                    self.slide = Some(l);
                    self.held_slide = h;
                    self.position = Some(l);
                }
                self.slide_media = fg;
            }
            Cleared::Announcements { slide, held } => {
                let (l, h) = find(slide, held).ok_or_else(gone)?;
                self.announcements = Some(l);
                self.held_announcement = h;
                self.position = Some(l);
            }
            Cleared::Media(m) => self.media = Some(fresh(m).ok_or_else(gone)?),
            // A logical input comes back on its binding as it is now (F02-2).
            Cleared::VideoInput(v) => {
                self.video_input = Some(follow_input(data, v).ok_or_else(gone)?)
            }
            Cleared::Audio(m, ctx) => {
                self.audio = Some(fresh(m).ok_or_else(gone)?);
                self.audio_playlist = ctx;
            }
            Cleared::Props(ids) => {
                for id in ids {
                    let _ = self.show_prop(data, id);
                }
                return Ok(());
            }
            Cleared::Messages(list) => {
                for m in list {
                    let _ = self.show_message(data, m.message, m.tokens);
                }
                return Ok(());
            }
        }
        let t = if layer == Layer::Audio {
            Transition::CUT
        } else {
            self.layer_default(data, layer).clone()
        };
        self.record_change(layer, t);
        Ok(())
    }

    /// Loop on or off for the play on `layer` (`media`, `slide` or `audio`), which is at
    /// `position` (seconds into its pass) and maybe `paused`: it continues under a new play ID
    /// with the new repeat (see [`ShowState::continues`]). Returns the old and new play IDs
    /// (`None`: it already loops, or already doesn't).
    pub fn set_loop(
        &mut self,
        layer: Layer,
        looping: bool,
        position: f64,
        paused: bool,
    ) -> Result<Option<(u64, u64)>, ShowError> {
        let m = match layer {
            Layer::Audio => self.audio.as_mut(),
            l => self.layer_media_mut(l),
        }
        .ok_or_else(|| ShowError::Conflict("nothing is playing".into()))?;
        if loops(&m.playback) == looping {
            return Ok(None);
        }
        let old = m.play.ok_or_else(|| {
            ShowError::Conflict("only a playing video or audio file can loop".into())
        })?;
        m.playback.repeat = if looping { Repeat::Loop } else { Repeat::Once };
        // A play that ended starts again from its in point.
        let at = if m.ended { 0.0 } else { position.max(0.0) };
        m.ended = false;
        let new = next_play_id();
        m.play = Some(new);
        self.continues.insert(
            new,
            Continue {
                from: old,
                at,
                paused,
            },
        );
        Ok(Some((old, new)))
    }

    /// Forgets continued plays (`continues`) that no layer plays any more.
    pub fn prune_continues(&mut self) {
        let live: Vec<u64> = [&self.media, &self.slide_media, &self.audio]
            .into_iter()
            .filter_map(|m| m.as_ref().and_then(|m| m.play))
            .collect();
        self.continues.retain(|new, _| live.contains(new));
    }

    /// Clears every layer.
    pub fn clear_all(&mut self, data: &ShowData) {
        for layer in [
            Layer::VideoInput,
            Layer::Slide,
            Layer::Announcements,
            Layer::Media,
            Layer::Audio,
            Layer::Props,
            Layer::Messages,
        ] {
            let _ = self.clear_layer(data, layer);
        }
    }

    /// Runs a macro (`triggerMacro`): its enabled actions in order, as a slide's macro action
    /// would (nested macros count from here).
    pub fn trigger_macro(
        &mut self,
        data: &ShowData,
        id: model::MacroId,
        c: Clock,
    ) -> Result<(), ShowError> {
        let m = data
            .content
            .macros
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| ShowError::NotFound(format!("macro {} not found", id.0)))?;
        let actions = m.actions.clone();
        self.run_actions(data, &actions, false, 1, c);
        Ok(())
    }

    /// Runs a clear group (see the module docs).
    pub fn clear_group(&mut self, data: &ShowData, id: ClearGroupId) -> Result<(), ShowError> {
        let g = data
            .content
            .clear_groups
            .iter()
            .find(|g| g.id == id)
            .ok_or_else(|| ShowError::NotFound(format!("clear group {} not found", id.0)))?;
        let targets = g.targets.clone();
        self.clear_targets(data, &targets)
    }

    /// Clears what each target names (see the module docs).
    pub fn clear_targets(
        &mut self,
        data: &ShowData,
        targets: &[ClearTarget],
    ) -> Result<(), ShowError> {
        for t in targets {
            let announcements = t.destination == Some(ContentDestination::Announcements);
            let layers: &[Layer] = match t.layer {
                ClearLayer::All if announcements => &[Layer::Announcements],
                ClearLayer::All => &[
                    Layer::VideoInput,
                    Layer::Slide,
                    Layer::Media,
                    Layer::Audio,
                    Layer::Props,
                    Layer::Messages,
                ],
                ClearLayer::Slide if announcements => &[Layer::Announcements],
                ClearLayer::Slide => &[Layer::Slide],
                ClearLayer::Announcements => &[Layer::Announcements],
                ClearLayer::Media => &[Layer::Media],
                ClearLayer::VideoInput => &[Layer::VideoInput],
                ClearLayer::Audio => &[Layer::Audio],
                ClearLayer::Props => &[Layer::Props],
                ClearLayer::Messages => &[Layer::Messages],
                _ => &[],
            };
            for l in layers {
                self.clear_layer(data, *l)?;
            }
        }
        Ok(())
    }

    // ---- timers, props, messages ------------------------------------------------------------

    /// Applies a timer operation.
    pub fn timer(
        &mut self,
        data: &ShowData,
        id: TimerId,
        op: TimerOp,
        c: Clock,
    ) -> Result<(), ShowError> {
        let t = data
            .content
            .timers
            .iter()
            .find(|t| t.id == id)
            .ok_or_else(|| ShowError::NotFound(format!("timer {} not found", id.0)))?;
        let finite = |v: f64| {
            if v.is_finite() {
                Ok(v)
            } else {
                Err(ShowError::BadRequest("the value must be a number".into()))
            }
        };
        match op {
            TimerOp::Start => self.timers.start(t, c),
            TimerOp::Stop => self.timers.stop(t, c),
            TimerOp::Reset => self.timers.reset(t),
            TimerOp::Restart => self.timers.restart(t, c),
            TimerOp::Set(v) => self.timers.set(t, finite(v)?, c),
            TimerOp::TimeOfDay(v) => {
                let v = finite(v)?;
                if !(0.0..crate::timers::DAY).contains(&v) {
                    return Err(ShowError::BadRequest(
                        "timeOfDay must be 0 – 86399 seconds after midnight".into(),
                    ));
                }
                self.timers.set_time_of_day(t, v, c)
            }
            TimerOp::Adjust(v) => self.timers.adjust(t, finite(v)?, c),
        };
        Ok(())
    }

    fn prop_transition(&self, data: &ShowData, id: PropId) -> Transition {
        let own = data
            .content
            .props
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.transition.as_ref());
        Transition::resolve(own, None, self.layer_default(data, Layer::Props)).clone()
    }

    fn record_slot(&mut self, slot: uuid::Uuid, transition: Transition) {
        self.slot_changes.insert(
            slot,
            LayerChange {
                seq: CHANGE_SEQ.fetch_add(1, Ordering::Relaxed),
                transition,
                at: Instant::now(),
            },
        );
    }

    /// Shows a prop (on top of the others). Already showing: nothing changes.
    pub fn show_prop(&mut self, data: &ShowData, id: PropId) -> Result<(), ShowError> {
        if !data.content.props.iter().any(|p| p.id == id) {
            return Err(ShowError::NotFound(format!("prop {} not found", id.0)));
        }
        if !self.props.contains(&id) {
            self.cleared.remove(&Layer::Props);
            self.props.push(id);
            let t = self.prop_transition(data, id);
            self.record_slot(id.0, t);
        }
        Ok(())
    }

    /// Hides a prop (not showing: nothing changes).
    pub fn hide_prop(&mut self, data: &ShowData, id: PropId) {
        if let Some(i) = self.props.iter().position(|p| *p == id) {
            self.props.remove(i);
            let t = self.prop_transition(data, id);
            self.record_slot(id.0, t);
        }
    }

    /// Shows the prop when hidden, hides it when showing.
    pub fn toggle_prop(&mut self, data: &ShowData, id: PropId) -> Result<(), ShowError> {
        if self.props.contains(&id) {
            self.hide_prop(data, id);
            Ok(())
        } else {
            self.show_prop(data, id)
        }
    }

    fn message_transition(&self, data: &ShowData, id: Option<MessageId>) -> Transition {
        let own = id.and_then(|id| {
            data.content
                .messages
                .iter()
                .find(|m| m.id == id)
                .and_then(|m| m.transition.as_ref())
        });
        Transition::resolve(own, None, self.layer_default(data, Layer::Messages)).clone()
    }

    /// Shows a message with token values; one that is up comes to the top with the new values.
    pub fn show_message(
        &mut self,
        data: &ShowData,
        id: MessageId,
        tokens: Vec<TokenValue>,
    ) -> Result<(), ShowError> {
        if !data.content.messages.iter().any(|m| m.id == id) {
            return Err(ShowError::NotFound(format!("message {} not found", id.0)));
        }
        let new = ActiveMessage {
            message: id,
            tokens,
        };
        if self.messages.last() == Some(&new) {
            return Ok(());
        }
        self.messages.retain(|m| m.message != id);
        self.messages.push(new);
        let t = self.message_transition(data, Some(id));
        self.record_change(Layer::Messages, t);
        Ok(())
    }

    /// Hides a message (`None` = every message).
    pub fn hide_message(&mut self, data: &ShowData, id: Option<MessageId>) {
        let top = self.messages.last().map(|m| m.message);
        let before = self.messages.len();
        match id {
            Some(id) => self.messages.retain(|m| m.message != id),
            None => self.messages.clear(),
        }
        if self.messages.len() != before {
            let t = self.message_transition(data, top);
            self.record_change(Layer::Messages, t);
        }
    }

    /// Runs actions in order (see the module docs). `from_slide`: a slide's own list, whose
    /// media actions were already applied by the trigger.
    pub fn run_actions(
        &mut self,
        data: &ShowData,
        actions: &[SlideAction],
        from_slide: bool,
        depth: u32,
        c: Clock,
    ) {
        for a in actions.iter().filter(|a| a.enabled) {
            let ran = self.run_action(data, &a.kind, from_slide, depth, c);
            if !ran {
                self.skipped_actions += 1;
                tracing::debug!(action = %a.id.0, "slide action skipped");
            }
        }
    }

    /// Runs one action; `false` when it was skipped.
    fn run_action(
        &mut self,
        data: &ShowData,
        kind: &ActionKind,
        from_slide: bool,
        depth: u32,
        c: Clock,
    ) -> bool {
        match kind {
            ActionKind::Media(t) => {
                if from_slide && t.layer != MediaLayer::Audio {
                    return true; // applied with the slide
                }
                let asset_kind = data.content.assets.get(&t.asset).map(|a| a.kind);
                match (t.layer, asset_kind) {
                    (_, None | Some(MediaKind::Unknown)) => false,
                    (_, Some(MediaKind::Audio)) => {
                        self.apply_audio(t, None);
                        true
                    }
                    (MediaLayer::Audio, _) => false,
                    (MediaLayer::Foreground, Some(MediaKind::Video)) => {
                        self.apply_media(data, Layer::Slide, t, true, None);
                        true
                    }
                    (MediaLayer::Background | MediaLayer::Unknown, _) => {
                        self.apply_media(data, Layer::Media, t, true, None);
                        true
                    }
                    _ => false,
                }
            }
            ActionKind::Timer { timer, operation } => {
                let op = match operation {
                    TimerOperation::Start => TimerOp::Start,
                    TimerOperation::Stop => TimerOp::Stop,
                    TimerOperation::Reset => TimerOp::Reset,
                    TimerOperation::Restart => TimerOp::Restart,
                    TimerOperation::Set { seconds } => TimerOp::Set(*seconds),
                    TimerOperation::Adjust { seconds } => TimerOp::Adjust(*seconds),
                    TimerOperation::Unknown => return false,
                };
                self.timer(data, timer.id, op, c).is_ok()
            }
            ActionKind::Message {
                message,
                show,
                values,
            } => {
                if *show {
                    self.show_message(data, message.id, values.clone()).is_ok()
                } else {
                    self.hide_message(data, Some(message.id));
                    true
                }
            }
            ActionKind::Prop { prop, show } => {
                if *show {
                    self.show_prop(data, prop.id).is_ok()
                } else {
                    self.hide_prop(data, prop.id);
                    true
                }
            }
            ActionKind::Look { look } => self.set_look(data, Some(look.id)).is_ok(),
            ActionKind::ClearLayer { targets } => self.clear_targets(data, targets).is_ok(),
            ActionKind::ClearGroup { group } => self.clear_group(data, group.id).is_ok(),
            ActionKind::StageLayout { assignments } => {
                let mut ok = true;
                for a in assignments {
                    if !data
                        .content
                        .stage_layouts
                        .iter()
                        .any(|l| l.id == a.layout.id)
                    {
                        ok = false;
                        continue;
                    }
                    let screen = a.screen.as_ref().map(|s| s.id);
                    match screen {
                        // Every stage screen: replaces the per-screen choices.
                        None => self.stage_layouts.clear(),
                        Some(sc) => self.stage_layouts.retain(|(s, _)| *s != Some(sc)),
                    }
                    self.stage_layouts.push((screen, a.layout.id));
                }
                ok
            }
            ActionKind::Macro { target } => {
                if depth >= MAX_MACRO_DEPTH {
                    return false;
                }
                let Some(m) = data.content.macros.iter().find(|m| m.id == target.id) else {
                    return false;
                };
                let actions = m.actions.clone();
                self.run_actions(data, &actions, false, depth + 1, c);
                true
            }
            ActionKind::VideoInput(v) => self
                .trigger_video_input(
                    data,
                    v.input.id,
                    Some(&v.display),
                    Some(&v.audio),
                    v.transition.as_ref(),
                )
                .is_ok(),
            ActionKind::Trigger { .. } | ActionKind::Unsupported { .. } | ActionKind::Unknown => {
                false
            }
        }
    }

    /// Drops props, messages and timer runs whose documents are gone.
    pub fn revalidate_show_docs(&mut self, data: &ShowData) {
        self.timers.retain(&data.content.timers);
        self.stage_layouts
            .retain(|(_, l)| data.content.stage_layouts.iter().any(|x| x.id == *l));
        let props: Vec<PropId> = self
            .props
            .iter()
            .copied()
            .filter(|id| !data.content.props.iter().any(|p| p.id == *id))
            .collect();
        for id in props {
            self.hide_prop(data, id);
        }
        let gone: Vec<MessageId> = self
            .messages
            .iter()
            .map(|m| m.message)
            .filter(|id| !data.content.messages.iter().any(|m| m.id == *id))
            .collect();
        for id in gone {
            self.hide_message(data, Some(id));
        }
    }

    /// Activates a look (`None` = every layer on every screen).
    pub fn set_look(&mut self, data: &ShowData, look: Option<LookId>) -> Result<(), ShowError> {
        if let Some(id) = look
            && !data.looks.iter().any(|l| l.id == id)
        {
            return Err(ShowError::NotFound(format!("look {} not found", id.0)));
        }
        let transition = look
            .and_then(|id| data.looks.iter().find(|l| l.id == id))
            .and_then(|l| l.transition.clone())
            .unwrap_or(Transition::CUT);
        self.look = look;
        self.record_look_change(transition);
        Ok(())
    }

    /// Records a change of the look (now), played with `transition` by the layers it changes.
    pub fn record_look_change(&mut self, transition: Transition) {
        self.look_change = Some(LayerChange {
            seq: CHANGE_SEQ.fetch_add(1, Ordering::Relaxed),
            transition,
            at: Instant::now(),
        });
    }

    /// The held copy of a deleted live slide on `layer` (slide or announcements).
    pub fn held(&self, layer: Layer) -> Option<&std::sync::Arc<HeldSlide>> {
        match layer {
            Layer::Slide => self.held_slide.as_ref(),
            Layer::Announcements => self.held_announcement.as_ref(),
            _ => None,
        }
    }

    /// Follows an edit of the library content (`old` → `new`). Live slides and the position
    /// follow their slide ID. A live slide that was **deleted** stays on screen (a held copy)
    /// until the next trigger or clear, as operators expect; the position moves to the slide
    /// that took its place, so `next` continues there. When the whole presentation was deleted,
    /// the slide stays on screen and the position is cleared.
    pub fn revalidate_edit(&mut self, old: &ShowData, new: &ShowData) {
        fn follow(live: &mut LiveSlide, old: &ShowData, new: &ShowData) -> Option<HeldSlide> {
            let np = new.content.presentation(live.presentation);
            if let Some(i) = np.and_then(|p| p.slides().position(|s| s.id == live.slide)) {
                live.index = i;
                live.detached = false;
                return None;
            }
            let op = old.content.presentation(live.presentation);
            let held = op.and_then(|p| {
                p.sections.iter().find_map(|sec| {
                    sec.slides
                        .iter()
                        .find(|s| s.id == live.slide)
                        .map(|s| HeldSlide {
                            slide: s.clone(),
                            canvas: p.canvas_size,
                            presentation_name: p.name.clone(),
                            section: sec.id,
                            section_name: sec.name.clone(),
                        })
                })
            });
            if let (Some(op), Some(np)) = (op, np)
                && !live.detached
            {
                // The first old successor that still exists takes the deleted slide's place.
                let old_ids: Vec<SlideId> = op.slides().map(|s| s.id).collect();
                let k = old_ids.iter().position(|id| *id == live.slide).unwrap_or(0);
                let new_ids: Vec<SlideId> = np.slides().map(|s| s.id).collect();
                live.index = old_ids[k + 1..]
                    .iter()
                    .find_map(|id| new_ids.iter().position(|n| n == id))
                    .unwrap_or(new_ids.len());
            } else if let Some(np) = np {
                live.index = live.index.min(np.slides().count());
            }
            live.detached = true;
            held
        }
        if let Some(l) = &mut self.slide {
            if let Some(h) = follow(l, old, new) {
                self.held_slide = Some(std::sync::Arc::new(h));
            } else if !l.detached {
                self.held_slide = None;
            }
        }
        if let Some(l) = &mut self.announcements {
            if let Some(h) = follow(l, old, new) {
                self.held_announcement = Some(std::sync::Arc::new(h));
            } else if !l.detached {
                self.held_announcement = None;
            }
        }
        if let Some(l) = &mut self.position {
            follow(l, old, new);
            if new.content.presentation(l.presentation).is_none() {
                self.position = None;
            }
        }
    }

    /// What `next` would trigger (`None` = nothing).
    pub fn cursor(&self, data: &ShowData) -> Option<LiveSlide> {
        let mut probe = self.clone();
        probe.probe = true;
        match probe.step(data, 1) {
            Ok(true) => probe.position,
            _ => None,
        }
    }

    /// Re-validates the state against reloaded data: slides follow their ID (or are cleared
    /// when gone), unknown looks and media are dropped.
    pub fn revalidate(&mut self, data: &ShowData) {
        let fix = |l: &mut Option<LiveSlide>| {
            if let Some(live) = l {
                let index = data
                    .content
                    .presentation(live.presentation)
                    .and_then(|p| p.slides().position(|s| s.id == live.slide));
                match index {
                    Some(i) => live.index = i,
                    None => *l = None,
                }
            }
        };
        fix(&mut self.slide);
        fix(&mut self.announcements);
        fix(&mut self.position);
        for l in [&mut self.slide, &mut self.announcements, &mut self.position]
            .into_iter()
            .flatten()
        {
            l.detached = false;
        }
        self.held_slide = None;
        self.held_announcement = None;
        self.follow_inputs(data);
        if let Some(m) = &self.media
            && !data.content.assets.contains_key(&m.asset)
        {
            self.media = None;
        }
        if let Some(m) = &self.slide_media
            && !data.content.assets.contains_key(&m.asset)
        {
            self.slide_media = None;
        }
        if let Some(m) = &self.audio
            && !data.content.assets.contains_key(&m.asset)
        {
            self.audio = None;
            self.audio_playlist = None;
        }
        self.revalidate_config(data);
        self.revalidate_show_docs(data);
    }

    /// A live logical input follows its binding (rebound: the new source; renamed: the new
    /// name; unbound or deleted: off the layer).
    pub fn follow_inputs(&mut self, data: &ShowData) {
        self.video_input = self.video_input.take().and_then(|v| follow_input(data, v));
    }

    /// Follows a configuration change (screens, looks, outputs): only what those documents
    /// name leaves the show (an unknown look, test patterns and stage layout choices of screens
    /// that are gone). Slides, media and held copies stay: the content did not change.
    pub fn revalidate_config(&mut self, data: &ShowData) {
        if let Some(id) = self.look
            && !data.looks.iter().any(|l| l.id == id)
        {
            self.look = None;
        }
        self.test_patterns
            .retain(|id, _| data.screens.iter().any(|s| s.id == *id));
        self.stage_layouts
            .retain(|(s, _)| s.is_none_or(|s| data.screens.iter().any(|x| x.id == s)));
    }
}

/// The name a logical input shows with on the layer: its document's name, else the source
/// part of its `source`.
fn input_name(data: &ShowData, input: model::VideoInputId, source: &str) -> String {
    data.video_inputs
        .iter()
        .find(|v| v.id == input)
        .map(|v| v.name.trim())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| output::recv::source_part(source).to_string())
}

/// `v` following its logical input's binding and name as they are now (`None`: the input was
/// deleted or unbound since). A source put live by name stays as it is.
fn follow_input(data: &ShowData, mut v: LiveVideoInput) -> Option<LiveVideoInput> {
    if let Some(input) = v.input {
        v.source = data.input_source(input).ok()?;
        v.name = input_name(data, input, &v.source);
    }
    Some(v)
}

/// The slide drawn on one screen.
pub struct PlannedSlide<'a> {
    /// The slide (borrowed from the library, or a template-override render).
    pub slide: Cow<'a, Slide>,
    /// Its canvas size.
    pub canvas: Size,
    /// Compositor layer (slide or announcements).
    pub layer: Layer,
}

/// Media drawn on a layer.
#[derive(Clone, Debug, PartialEq)]
pub enum PlannedMedia {
    /// An image file.
    Image(PathBuf),
    /// A video (its play ID in the player registry).
    Video(u64),
    /// Nothing can be drawn (missing file, video that could not be opened): why.
    Missing(String),
}

/// The slide a stage layout's slide picture shows, with its canvas.
pub type PictureSource<'a> = (Cow<'a, Slide>, Size);

/// What one screen draws for the current state.
#[derive(Default)]
pub struct ScreenPlan<'a> {
    /// Props showing (bottom to top), each with its prop's UUID.
    pub props: Vec<(uuid::Uuid, PlannedSlide<'a>)>,
    /// The message drawn on the messages layer.
    pub message: Option<PlannedSlide<'a>>,
    /// The caption lines drawn on the captions layer.
    pub captions: Option<PlannedSlide<'a>>,
    /// What the planned slides' live data depends on (clock, timers, media countdowns).
    pub live: LiveUse,
    /// The live source on the video-input layer, with its display settings.
    pub video_input: Option<(String, MediaDisplay)>,
    /// Background media (or why it cannot be drawn), with its display settings.
    pub media: Option<(PlannedMedia, MediaDisplay)>,
    /// A foreground video on the slide layer (replaces the slide).
    pub foreground: Option<(PlannedMedia, MediaDisplay)>,
    /// The slide and announcements layers.
    pub slides: Vec<PlannedSlide<'a>>,
    /// Problems worth reporting (missing template, stage screens, ...).
    pub notes: Vec<String>,
    /// The screen's role is switched off: draw black (transparent with alpha), nothing else.
    pub blank: bool,
    /// A stage screen: `slides` holds its layout, `message` the stage message banner; changes
    /// cut (see [`crate::stage`]).
    pub stage: bool,
    /// Slide pictures of a stage layout: the element (in `slides[0]`) and the slide it shows
    /// with its canvas (`None` = no such slide: the element draws nothing).
    pub pictures: Vec<(model::ElementId, Option<PictureSource<'a>>)>,
    /// The build step the slide and announcements layers' slides are drawn at (a slide with
    /// object builds; see `docs/BUILDS-PLAN.md`).
    pub builds: Vec<(Layer, BuildPlay)>,
    /// Content fingerprint: the screen re-renders only when it changes.
    pub fingerprint: u64,
    /// The layers' changes this screen follows instead of the show's (a signage channel's
    /// items, see [`crate::signage`]); a screen with them never plays a look's transition.
    pub changes: Option<HashMap<Layer, LayerChange>>,
}

/// Resolves a planned slide's live data (data links, visibility, colour triggers) when a
/// context is given.
fn live_slide<'a>(
    slide: Cow<'a, Slide>,
    ctx: Option<&DataContext>,
    used: &mut LiveUse,
) -> Cow<'a, Slide> {
    let Some(ctx) = ctx else { return slide };
    match slide {
        Cow::Borrowed(s) => {
            let (r, u) = render::live::resolve_slide(s, ctx);
            used.merge(u);
            r
        }
        Cow::Owned(s) => {
            let (r, u) = render::live::resolve_slide(&s, ctx);
            used.merge(u);
            let resolved = match r {
                Cow::Owned(x) => Some(x),
                Cow::Borrowed(_) => None,
            };
            Cow::Owned(resolved.unwrap_or(s))
        }
    }
}

/// A stage screen's plan (see [`crate::stage`]): its layout with live data, the slide
/// pictures it shows, the stage message banner when the layout has no message element. No
/// audience layer.
fn plan_stage<'a>(
    data: &'a ShowData,
    state: &ShowState,
    screen: &Screen,
    ctx: Option<&DataContext>,
    plan: &mut ScreenPlan<'a>,
) {
    let mut used = LiveUse::default();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    plan.stage = true;
    data.content_generation.hash(&mut h);
    let (layout, canvas, id) = crate::stage::layout_for(data, state, screen);
    id.map(|i| i.0).hash(&mut h);
    // The layout's chord view (transposition, capo, notation) for its chord charts and key
    // line: the frame's context with it.
    let view = crate::stage::chord_view_for(data, id);
    let with_view;
    let ctx = match ctx {
        Some(c) if c.chord_view != view && crate::stage::has_chord_links(&layout) => {
            view.hash(&mut h);
            with_view = DataContext {
                chord_view: view,
                ..c.clone()
            };
            Some(&with_view)
        }
        c => c,
    };
    let has_message = crate::stage::has_message_element(&layout);
    let pictures = crate::stage::picture_links(&layout);
    if !pictures.is_empty() {
        let cursor = pictures
            .iter()
            .any(|(_, w)| *w == model::SlideWhich::Next)
            .then(|| state.cursor(data))
            .flatten();
        for (element, which) in pictures {
            let source = crate::stage::picture_source(data, state, which, cursor)
                .map(|(s, c)| (live_slide(s, ctx, &mut used), c));
            plan.pictures.push((element, source));
        }
    }
    plan.slides.push(PlannedSlide {
        slide: live_slide(layout, ctx, &mut used),
        canvas,
        layer: Layer::Slide,
    });
    if let Some(m) = &state.stage_message
        && !has_message
    {
        let on = ctx.is_some_and(|c| c.stage_flash);
        used.flash |= m.flash;
        m.text.hash(&mut h);
        on.hash(&mut h);
        plan.message = Some(PlannedSlide {
            slide: Cow::Owned(crate::stage::banner(&m.text, on)),
            canvas: crate::stage::TEMPLATE_CANVAS,
            layer: Layer::Messages,
        });
    }
    if state.captions.stage {
        plan_captions(state, screen, &mut plan.captions, &mut h);
    }
    plan.live = used;
    plan.fingerprint = h.finish();
}

/// The captions layer of `screen` (nothing without lines).
fn plan_captions<'a>(
    state: &ShowState,
    screen: &Screen,
    out: &mut Option<PlannedSlide<'a>>,
    h: &mut std::collections::hash_map::DefaultHasher,
) {
    let shown = &state.captions;
    if shown.lines.is_empty() {
        return;
    }
    9u8.hash(h);
    shown.lines.hash(h);
    serde_json::to_vec(&shown.style).unwrap_or_default().hash(h);
    *out = Some(PlannedSlide {
        slide: Cow::Owned(crate::captions::slide(shown, screen.size)),
        canvas: screen.size,
        layer: Layer::Captions,
    });
}

/// `slide` with its second language shown as `look` says (`None`: drawn as authored). The
/// second text's style is the theme's the slide (else its presentation) was styled from.
/// `poured`: the look's template override pours the slide into a template, which takes one
/// text per slot, so side by side is drawn stacked there (a right half would land in another
/// slot).
fn with_language(
    data: &ShowData,
    slide: &Slide,
    pres: Option<&Presentation>,
    look: Option<&model::ScreenLook>,
    poured: bool,
    h: &mut std::collections::hash_map::DefaultHasher,
) -> Option<Slide> {
    slide.translation.as_ref()?;
    let mut display = look
        .map_or(model::LanguageDisplay::Auto, |l| l.language)
        .resolve(slide);
    if poured && display == model::LanguageDisplay::SideBySide {
        display = model::LanguageDisplay::Stacked;
    }
    let theme = slide
        .template_origin
        .as_ref()
        .or_else(|| pres.and_then(|p| p.template_origin.as_ref()))
        .and_then(|t| data.content.themes.get(&t.theme.id));
    let style = theme.map(|t| t.second_language.clone()).unwrap_or_default();
    let out = model::language::show_language(slide, display, &style)?;
    10u8.hash(h);
    display.resolve(slide).hash(h);
    serde_json::to_vec(&style).unwrap_or_default().hash(h);
    Some(out)
}

/// Decides what `screen` draws for `state` (look routing and template override included).
/// With a data context, slides show their live data (see [`render::live`]); without one
/// (look-ahead prefetching) they are as authored.
pub fn plan_screen<'a>(
    data: &'a ShowData,
    state: &ShowState,
    screen: &Screen,
    ctx: Option<&DataContext>,
) -> ScreenPlan<'a> {
    let mut plan = ScreenPlan::default();
    let mut used = LiveUse::default();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if !state.screens_enabled.get(screen_role(screen)) {
        plan.blank = true;
        plan.fingerprint = 2;
        return plan;
    }
    if matches!(screen.kind, ScreenKind::Stage { .. }) {
        plan_stage(data, state, screen, ctx, &mut plan);
        return plan;
    }
    if matches!(screen.kind, ScreenKind::Signage { .. }) {
        // Drawn from its channel (the render thread asks `crate::signage`), never the show.
        plan.blank = true;
        plan.fingerprint = 3;
        return plan;
    }
    // A reload creates new library content: its generation is part of the fingerprint so
    // every screen re-renders against the new documents (a configuration edit keeps it).
    data.content_generation.hash(&mut h);
    let look = state
        .look
        .and_then(|id| data.looks.iter().find(|l| l.id == id))
        .map(|l| l.for_screen(screen.id));
    let layers = look
        .as_ref()
        .map(|l| l.layers)
        .unwrap_or(LayerVisibility::ALL);

    let planned = |m: &LiveMedia| -> PlannedMedia {
        match (m.kind, m.play, data.content.asset_path(m.asset)) {
            (_, _, Err(why)) => PlannedMedia::Missing(why),
            (MediaKind::Video, Some(play), Ok(_)) => PlannedMedia::Video(play),
            (MediaKind::Video, None, Ok(p)) => {
                PlannedMedia::Missing(format!("video {} cannot be played", p.display()))
            }
            (_, _, Ok(p)) => PlannedMedia::Image(p),
        }
    };
    if layers.video_input
        && let Some(v) = &state.video_input
    {
        plan.video_input = Some((v.source.clone(), v.display.clone()));
        5u8.hash(&mut h);
        v.source.hash(&mut h);
        serde_json::to_vec(&v.display)
            .unwrap_or_default()
            .hash(&mut h);
    }
    if layers.media
        && let Some(m) = &state.media
    {
        plan.media = Some((planned(m), m.display.clone()));
        m.asset.0.hash(&mut h);
        m.play.hash(&mut h);
        serde_json::to_vec(&m.display)
            .unwrap_or_default()
            .hash(&mut h);
    }
    if layers.slide
        && let Some(m) = &state.slide_media
    {
        plan.foreground = Some((planned(m), m.display.clone()));
        3u8.hash(&mut h);
        m.asset.0.hash(&mut h);
        m.play.hash(&mut h);
    }

    for (live, layer) in [
        (state.slide, Layer::Slide),
        (state.announcements, Layer::Announcements),
    ] {
        let Some(live) = live else { continue };
        if !layers.contains(layer) || (layer == Layer::Slide && plan.foreground.is_some()) {
            continue;
        }
        let mut ccli = false;
        let (slide, canvas, pres): (Cow<'a, Slide>, Size, Option<&Presentation>) = if live.detached
        {
            match state.held(layer) {
                Some(held) => {
                    held.slide.id.0.hash(&mut h);
                    (Cow::Owned(held.slide.clone()), held.canvas, None)
                }
                None => continue,
            }
        } else {
            let Some(p) = data.content.presentation(live.presentation) else {
                continue;
            };
            let Some(slide) = p.slides().nth(live.index) else {
                continue;
            };
            ccli = layer == Layer::Slide && crate::live::ccli_shown(data, p, &live);
            (Cow::Borrowed(slide), p.canvas_size, Some(p))
        };
        // Second-language text as this screen's look shows it (`model::language`).
        let poured =
            layer == Layer::Slide && look.as_ref().is_some_and(|l| l.template_override.is_some());
        let slide = match with_language(data, &slide, pres, look.as_ref(), poured, &mut h) {
            Some(s) => Cow::Owned(s),
            None => slide,
        };
        (layer as u8).hash(&mut h);
        live.presentation.0.hash(&mut h);
        live.index.hash(&mut h);
        ccli.hash(&mut h);
        // The song's copyright line on its own (CCLI settings), over the slide.
        let with_ccli = |s: Cow<'a, Slide>, canvas: Size| match ccli {
            true => crate::live::with_ccli(data, s, canvas),
            false => s,
        };
        let ov = look.as_ref().and_then(|l| l.template_override.as_ref());
        match ov.filter(|_| layer == Layer::Slide) {
            None if !slide.builds.is_empty() => {
                let b = state.layer_build(layer).filter(|b| b.slide == slide.id);
                let play = BuildPlay {
                    step: b.map_or(0, |b| b.step.min(model::build::step_count(&slide))),
                    at: b.and_then(|b| b.at),
                };
                play.step.hash(&mut h);
                play.at.hash(&mut h);
                plan.builds.push((layer, play));
                plan.slides.push(PlannedSlide {
                    slide: live_slide(with_ccli(slide, canvas), ctx, &mut used),
                    canvas,
                    layer,
                });
            }
            None => plan.slides.push(PlannedSlide {
                slide: live_slide(with_ccli(slide, canvas), ctx, &mut used),
                canvas,
                layer,
            }),
            Some(ov) => {
                1u8.hash(&mut h);
                let template = data.content.themes.get(&ov.theme.id).and_then(|t| {
                    t.slides
                        .iter()
                        .find(|s| s.id == ov.slide)
                        .map(|s| (t, &s.slide))
                });
                match template {
                    Some((theme, tslide)) => {
                        // The live slide's visibility conditions first: only text the
                        // audience render would draw goes into the template (A09-2).
                        let source = live_slide(slide, ctx, &mut used);
                        if let Some(s) = ov.render(&source, tslide) {
                            let s = with_ccli(Cow::Owned(s), theme.canvas_size);
                            plan.slides.push(PlannedSlide {
                                slide: live_slide(s, ctx, &mut used),
                                canvas: theme.canvas_size,
                                layer,
                            });
                        }
                    }
                    None => plan.notes.push(format!(
                        "screen {:?}: template override theme {} / slide {} not found; slide \
                         layer not drawn",
                        screen.name,
                        ov.theme.name.as_deref().unwrap_or("?"),
                        ov.slide.0
                    )),
                }
            }
        }
    }
    if layers.props {
        for id in &state.props {
            let Some(p) = data.content.props.iter().find(|p| p.id == *id) else {
                continue;
            };
            id.0.hash(&mut h);
            plan.props.push((
                id.0,
                PlannedSlide {
                    slide: live_slide(Cow::Borrowed(&p.slide), ctx, &mut used),
                    canvas: p.size,
                    layer: Layer::Props,
                },
            ));
        }
    }
    if layers.messages
        && let Some(m) = state.messages.last()
        && let Some(doc) = data.content.messages.iter().find(|d| d.id == m.message)
    {
        let fallback;
        let c = match ctx {
            Some(c) => c,
            None => {
                fallback = DataContext::default();
                &fallback
            }
        };
        let text = crate::live::message_text(doc, &m.tokens, c);
        // Timer and clock tokens make the message live.
        for t in &doc.tokens {
            match t.kind {
                model::MessageTokenKind::Timer { .. } => used.timers = true,
                model::MessageTokenKind::Clock { .. } => used.clock = true,
                _ => {}
            }
        }
        let (slide, canvas) = crate::live::message_slide(data, doc, &text);
        m.message.0.hash(&mut h);
        text.hash(&mut h);
        plan.message = Some(PlannedSlide {
            slide: live_slide(Cow::Owned(slide), ctx, &mut used),
            canvas,
            layer: Layer::Messages,
        });
    }
    if layers.captions && state.captions.audience {
        plan_captions(state, screen, &mut plan.captions, &mut h);
    }
    plan.live = used;
    plan.fingerprint = h.finish();
    plan
}

#[cfg(test)]
mod tests {
    //! `next` / `prev` and the cursor along arrangements; restoring cleared layers.
    use super::*;
    use model::{
        Arrangement, ArrangementId, Element, Library, Link, PlaylistNode, Rect, Section, TextBox,
    };

    struct Fx {
        data: ShowData,
        song: PresentationId,
        other: PresentationId,
        playlist: PlaylistId,
        items: Vec<PlaylistItemId>,
        short: ArrangementId,
    }

    fn put<T: serde::Serialize>(dir: &Path, kind: &str, id: uuid::Uuid, doc: &T) {
        let d = dir.join("portable").join(kind);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join(format!("{id}.json")),
            serde_json::to_vec(doc).unwrap(),
        )
        .unwrap();
    }

    fn slide(t: &str) -> Slide {
        let mut s = Slide::new();
        s.elements = vec![Element::text(
            Rect::new(0.0, 0.0, 100.0, 50.0),
            TextBox::plain(t),
        )];
        s
    }

    fn section(name: &str, slides: &[&str]) -> Section {
        let mut s = Section::new(name);
        s.slides = slides.iter().map(|t| slide(t)).collect();
        s
    }

    /// "Song": V1 [v1a v1b], C [c1 c2], B [b1], V2 [v2a] in document order; arrangement
    /// "Short" = V1 C V2 C (B left out); "Long" = C V1. "Other": [x1 x2]. A playlist plays
    /// the song (`item_arr`), then the other one. `selected`: the song's own arrangement.
    fn fx(item_arr: bool, selected: bool, disabled: &[&str]) -> Fx {
        let dir = std::env::temp_dir().join(format!("nav-{}", uuid::Uuid::new_v4()));
        let lib = Library::default();
        put(&dir, "library", lib.id.0, &lib);
        let mut sections = vec![
            section("V1", &["v1a", "v1b"]),
            section("C", &["c1", "c2"]),
            section("B", &["b1"]),
            section("V2", &["v2a"]),
        ];
        for sec in &mut sections {
            for s in &mut sec.slides {
                if disabled.contains(&text_of(s).as_str()) {
                    s.enabled = false;
                }
            }
        }
        let id = |i: usize| sections[i].id;
        let short = Arrangement {
            id: ArrangementId::new(),
            name: "Short".into(),
            sections: vec![id(0), id(1), id(3), id(1)],
        };
        let long = Arrangement {
            id: ArrangementId::new(),
            name: "Long".into(),
            sections: vec![id(1), id(0)],
        };
        let song = Presentation {
            library: Some(lib.id),
            arrangements: vec![short.clone(), long.clone()],
            selected_arrangement: selected.then_some(long.id),
            sections,
            ..Presentation::new("Song")
        };
        let other = Presentation {
            library: Some(lib.id),
            sections: vec![section("X", &["x1", "x2"])],
            ..Presentation::new("Other")
        };
        put(&dir, "presentation", song.id.0, &song);
        put(&dir, "presentation", other.id.0, &other);
        let item = |p: &Presentation, arrangement: Option<ArrangementId>| {
            PlaylistItem::new(PlaylistItemKind::Presentation {
                presentation: Link::named(p.id, p.name.clone()),
                arrangement,
                unresolved: None,
            })
        };
        let items = vec![
            item(&song, item_arr.then_some(short.id)),
            item(&other, None),
        ];
        let ids = items.iter().map(|i| i.id).collect();
        let playlist = PlaylistNode {
            name: "Sunday".into(),
            kind: PlaylistNodeKind::Playlist {
                purpose: model::PlaylistPurpose::Service,
                items,
            },
            ..PlaylistNode::default()
        };
        put(&dir, "playlistNode", playlist.id.0, &playlist);
        let data = ShowData::load(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        Fx {
            data,
            song: song.id,
            other: other.id,
            playlist: playlist.id,
            items: ids,
            short: short.id,
        }
    }

    fn text_of(s: &Slide) -> String {
        s.elements[0].text_box().unwrap().plain_text()
    }

    fn name(f: &Fx, l: Option<LiveSlide>) -> String {
        let Some(l) = l else { return "-".into() };
        let p = f.data.content.presentation(l.presentation).unwrap();
        text_of(p.slides().nth(l.index).unwrap())
    }

    fn walk(f: &Fx, st: &mut ShowState, delta: i64, n: usize) -> Vec<String> {
        (0..n)
            .map(|_| {
                if !st.step(&f.data, delta).unwrap() {
                    return "-".into();
                }
                name(f, st.position)
            })
            .collect()
    }

    fn by_text(f: &Fx, p: PresentationId, t: &str) -> SlideId {
        let p = f.data.content.presentation(p).unwrap();
        p.slides().find(|s| text_of(s) == t).unwrap().id
    }

    #[test]
    fn next_and_prev_follow_the_items_arrangement_across_repeats_and_items() {
        let f = fx(true, true, &[]);
        let mut st = ShowState::default();
        st.trigger_playlist_item(&f.data, f.playlist, f.items[0])
            .unwrap();
        assert_eq!(name(&f, st.position), "v1a");
        // The item's arrangement wins over the song's own ("Long"); B is left out, C repeats,
        // and past the end `next` crosses into the next item.
        assert_eq!(
            walk(&f, &mut st, 1, 8),
            ["v1b", "c1", "c2", "v2a", "c1", "c2", "x1", "x2"]
        );
        // Back again: into the last slide of the song's play order (the second chorus), and
        // through the repeat the other way.
        assert_eq!(
            walk(&f, &mut st, -1, 8),
            ["x1", "c2", "c1", "v2a", "c2", "c1", "v1b", "v1a"]
        );
        assert_eq!(
            walk(&f, &mut st, -1, 1),
            ["-"],
            "nothing before the first item"
        );
        // The cursor (the stage display's next slide) is what `next` triggers, occurrence
        // included.
        st.trigger_playlist_item(&f.data, f.playlist, f.items[0])
            .unwrap();
        walk(&f, &mut st, 1, 2); // c1, first time
        assert_eq!(name(&f, st.cursor(&f.data)), "c2");
        walk(&f, &mut st, 1, 1); // c2, first time
        assert_eq!(name(&f, st.cursor(&f.data)), "v2a");
        walk(&f, &mut st, 1, 3); // c2, second time
        assert_eq!(st.position.unwrap().step, Some(6));
        assert_eq!(name(&f, st.cursor(&f.data)), "x1");
        let cur = st.cursor(&f.data).unwrap();
        assert_eq!(cur.presentation, f.other);
    }

    #[test]
    fn stage_next_section_and_slide_count_follow_the_arrangement() {
        // A01-5: V1 C V2 C (B left out). Each step: (live slide, next section, place, count).
        let f = fx(true, false, &[]);
        let mut st = ShowState::default();
        st.trigger_playlist_item(&f.data, f.playlist, f.items[0])
            .unwrap();
        let probe = |st: &ShowState| {
            let ctx = crate::live::data_context(&f.data, st, crate::timers::Clock::now());
            (
                name(&f, st.slide),
                ctx.next_section.unwrap_or_else(|| "-".into()),
                ctx.slide_count.unwrap(),
            )
        };
        let mut seen = vec![probe(&st)];
        for _ in 0..6 {
            st.step(&f.data, 1).unwrap();
            seen.push(probe(&st));
        }
        let want = [
            ("v1a", "C", (0, 7)),
            ("v1b", "C", (1, 7)),
            ("c1", "V2", (2, 7)),
            ("c2", "V2", (3, 7)),
            ("v2a", "C", (4, 7)),
            ("c1", "-", (5, 7)),
            ("c2", "-", (6, 7)),
        ];
        let want: Vec<(String, String, (usize, usize))> = want
            .iter()
            .map(|(a, b, c)| (a.to_string(), b.to_string(), *c))
            .collect();
        assert_eq!(seen, want);
        // A section played twice in a row is the next section again.
        let p = f.data.content.presentation(f.song).unwrap();
        let mut p2 = p.clone();
        let c = p2.sections[1].id;
        p2.arrangements[0].sections = vec![c, c];
        let mut d = f.data.clone();
        let mut content = (*d.content).clone();
        let mut list = (*content.presentations).clone();
        list[content.presentation_index[&f.song]] = std::sync::Arc::new(p2);
        content.presentations = std::sync::Arc::new(list);
        d.content = std::sync::Arc::new(content);
        let mut st = ShowState::default();
        st.trigger_playlist_item(&d, f.playlist, f.items[0])
            .unwrap();
        let ctx = crate::live::data_context(&d, &st, crate::timers::Clock::now());
        assert_eq!(ctx.next_section.as_deref(), Some("C"));
        assert_eq!(ctx.slide_count, Some((0, 4)));
    }

    #[test]
    fn disabled_slides_are_skipped_in_every_occurrence() {
        let f = fx(true, false, &["c2"]);
        let mut st = ShowState::default();
        st.trigger_playlist_item(&f.data, f.playlist, f.items[0])
            .unwrap();
        assert_eq!(walk(&f, &mut st, 1, 5), ["v1b", "c1", "v2a", "c1", "x1"]);
    }

    #[test]
    fn a_clicked_slide_is_the_occurrence_after_the_position() {
        let f = fx(true, false, &[]);
        let item = Some(PlaylistItemRef {
            playlist: f.playlist,
            item: f.items[0],
        });
        let mut st = ShowState::default();
        // First click: the first occurrence.
        st.trigger(
            &f.data,
            f.song,
            Some(by_text(&f, f.song, "c1")),
            None,
            item,
            None,
            None,
        )
        .unwrap();
        assert_eq!(st.position.unwrap().step, Some(2));
        assert_eq!(walk(&f, &mut st, 1, 2), ["c2", "v2a"]);
        // After the second verse, the chorus is the second one: `next` past it leaves the song.
        st.trigger(
            &f.data,
            f.song,
            Some(by_text(&f, f.song, "c1")),
            None,
            item,
            None,
            None,
        )
        .unwrap();
        assert_eq!(st.position.unwrap().step, Some(5));
        assert_eq!(walk(&f, &mut st, 1, 2), ["c2", "x1"]);
        // Clicking the live slide again keeps its occurrence; a slide before the position
        // wraps to its first occurrence.
        st.trigger(&f.data, f.song, None, Some(0), item, None, None)
            .unwrap();
        assert_eq!(st.position.unwrap().step, Some(0));
        // A slide the arrangement leaves out (the bridge, between C and V2 in document
        // order) continues with V2; `prev` goes to the chorus before it.
        st.trigger(
            &f.data,
            f.song,
            Some(by_text(&f, f.song, "b1")),
            None,
            item,
            None,
            None,
        )
        .unwrap();
        assert_eq!(st.position.unwrap().step, None);
        let mut back = st.clone();
        assert_eq!(walk(&f, &mut st, 1, 2), ["v2a", "c1"]);
        assert_eq!(walk(&f, &mut back, -1, 2), ["c2", "c1"]);
        assert_eq!(back.position.unwrap().step, Some(2));
    }

    #[test]
    fn a_step_picks_the_occurrence_that_was_clicked() {
        // "Short" = V1 C V2 C: steps 0-1 v1a v1b, 2-3 c1 c2, 4 v2a, 5-6 c1 c2.
        let f = fx(true, false, &[]);
        let item = Some(PlaylistItemRef {
            playlist: f.playlist,
            item: f.items[0],
        });
        let c1 = Some(by_text(&f, f.song, "c1"));
        let mut st = ShowState::default();
        // The second chorus straight away, from nothing live.
        st.trigger(&f.data, f.song, c1, None, item, Some(5), None)
            .unwrap();
        assert_eq!(st.position.unwrap().step, Some(5));
        assert_eq!(walk(&f, &mut st, 1, 2), ["c2", "x1"]);
        // Back to the first chorus, although the position is after it.
        st.trigger(&f.data, f.song, c1, None, item, Some(2), None)
            .unwrap();
        assert_eq!(st.position.unwrap().step, Some(2));
        // A step that holds another slide is ignored: the usual rule (first at or after the
        // position) applies.
        st.trigger(&f.data, f.song, c1, None, item, Some(0), None)
            .unwrap();
        assert_eq!(st.position.unwrap().step, Some(2));
        st.trigger(&f.data, f.song, c1, None, item, Some(99), None)
            .unwrap();
        assert_eq!(st.position.unwrap().step, Some(2));
    }

    #[test]
    fn without_an_item_arrangement_the_songs_own_then_document_order() {
        // The song's selected arrangement ("Long" = C V1), from the playlist and the library.
        let f = fx(false, true, &[]);
        let mut st = ShowState::default();
        st.trigger_playlist_item(&f.data, f.playlist, f.items[0])
            .unwrap();
        assert_eq!(name(&f, st.position), "c1");
        assert_eq!(walk(&f, &mut st, 1, 4), ["c2", "v1a", "v1b", "x1"]);
        let mut st = ShowState::default();
        st.trigger(&f.data, f.song, None, Some(2), None, None, None)
            .unwrap();
        assert_eq!(walk(&f, &mut st, 1, 4), ["c2", "v1a", "v1b", "-"]);
        // No arrangement chosen anywhere: document order, the bridge included.
        let f = fx(false, false, &[]);
        let mut st = ShowState::default();
        st.trigger_playlist_item(&f.data, f.playlist, f.items[0])
            .unwrap();
        assert_eq!(
            walk(&f, &mut st, 1, 6),
            ["v1b", "c1", "c2", "b1", "v2a", "x1"]
        );
        let _ = f.short;
    }

    #[test]
    fn a_cleared_slide_comes_back_while_its_layer_is_empty() {
        let f = fx(true, false, &[]);
        let mut st = ShowState::default();
        assert!(matches!(
            st.restore_layer(&f.data, Layer::Slide),
            Err(ShowError::Conflict(_))
        ));
        st.trigger_playlist_item(&f.data, f.playlist, f.items[0])
            .unwrap();
        walk(&f, &mut st, 1, 2); // c1
        let live = st.slide;
        assert!(st.restorable().is_empty(), "nothing cleared yet");
        assert!(matches!(
            st.restore_layer(&f.data, Layer::Slide),
            Err(ShowError::Conflict(_))
        ));
        st.clear_layer(&f.data, Layer::Slide).unwrap();
        assert_eq!(st.slide, None);
        assert_eq!(st.restorable(), vec![Layer::Slide]);
        st.restore_layer(&f.data, Layer::Slide).unwrap();
        assert_eq!(st.slide, live);
        assert!(st.restorable().is_empty());
        assert_eq!(walk(&f, &mut st, 1, 1), ["c2"], "next continues after it");
        // Clear All keeps it too; new content on the layer drops it.
        st.clear_all(&f.data);
        assert_eq!(st.restorable(), vec![Layer::Slide]);
        walk(&f, &mut st, 1, 1);
        assert!(st.restorable().is_empty());
        st.clear_layer(&f.data, Layer::Slide).unwrap();
        st.trigger(&f.data, f.other, None, Some(0), None, None, None)
            .unwrap();
        assert!(matches!(
            st.restore_layer(&f.data, Layer::Slide),
            Err(ShowError::Conflict(_))
        ));
        assert!(matches!(
            st.restore_layer(&f.data, Layer::Unknown),
            Err(ShowError::BadRequest(_))
        ));
    }

    #[test]
    fn loop_on_and_off_continues_the_play_under_a_new_id() {
        let mut st = ShowState::default();
        assert!(matches!(
            st.set_loop(Layer::Media, true, 0.0, false),
            Err(ShowError::Conflict(_))
        ));
        st.media = Some(LiveMedia {
            asset: MediaAssetId::new(),
            display: MediaDisplay::default(),
            kind: MediaKind::Video,
            playback: model::Playback::default(),
            transition: None,
            play: Some(7),
            ended: false,
            audio: AudioSettings::default(),
        });
        assert_eq!(st.set_loop(Layer::Media, false, 1.0, false).unwrap(), None);
        let (old, new) = st.set_loop(Layer::Media, true, 1.5, true).unwrap().unwrap();
        assert_eq!(old, 7);
        let m = st.media.as_ref().unwrap();
        assert_eq!(m.play, Some(new));
        assert!(loops(&m.playback));
        assert_eq!(
            st.continues[&new],
            Continue {
                from: 7,
                at: 1.5,
                paused: true
            }
        );
        // An ended play starts again from its in point.
        st.media.as_mut().unwrap().ended = true;
        let (_, newer) = st
            .set_loop(Layer::Media, false, 2.0, false)
            .unwrap()
            .unwrap();
        assert_eq!(st.continues[&newer].at, 0.0);
        st.media = None;
        st.prune_continues();
        assert!(st.continues.is_empty());
    }

    #[test]
    fn a_template_override_draws_side_by_side_stacked() {
        // H07-5: a right half would land in another template slot.
        let f = fx(false, false, &[]);
        let mut s = slide("Grace so free");
        let id = s.elements[0].id;
        let mut t = model::SlideTranslation::new("es");
        t.set_plain(id, "Gracia tan libre");
        t.layout = Some(model::LanguageDisplay::SideBySide);
        s.translation = Some(t);
        let look = model::ScreenLook::default();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let plain = with_language(&f.data, &s, None, Some(&look), false, &mut h).unwrap();
        assert_eq!(plain.elements.len(), 2, "side by side: two halves");
        let poured = with_language(&f.data, &s, None, Some(&look), true, &mut h).unwrap();
        assert_eq!(poured.elements.len(), 1, "stacked in the one box");
        assert_eq!(text_of(&poured), "Grace so free\nGracia tan libre");
    }
}
