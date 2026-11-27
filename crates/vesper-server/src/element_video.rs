//! Video elements: slide elements whose media fill is a video play it inside their box (G16).
//!
//! - **With sound: the slide layer's elements.** The show actor keeps one [`ElementPlay`] per
//!   video element of the live slide on the slide layer ([`ElementPlays`], in
//!   [`crate::Shared::element_plays`]), opened when the slide goes live (off the actor, on a
//!   thread of its own) as a [`LiveVideo`] with its soundtrack. Its [`crate::sound::SyncedClock`]
//!   starts when it is open, **at the slide's time**: the seconds since the slide layer's last
//!   change (when the slide went live, the clock of the render thread's muted players) times
//!   the rate, so the picture never steps back when the play takes over from a muted player,
//!   and the sound comes in at the matching point (a loop at its pass, a clip that already
//!   ended stays silent on its last frame). Its sound is a [`crate::sound::PlaySource`] on the
//!   slide group ([`crate::sound::GROUP_SLIDE`]: the slide layer's level and mute apply) with
//!   the fill's [`model::AudioSettings`]. The render thread draws the frame at the clock's
//!   time for the screen's present instant, so picture and sound stay in step (the sound is
//!   the clock; after an output stall the sound skips the gap, so the picture never steps
//!   back), and one player serves every screen showing the slide. The sound fades in and
//!   out with the slide layer's transition (a play opened later fades in over what is left of
//!   it); the outgoing side of a dissolve keeps its player until it is gone.
//!   **The sound follows what the screens draw:** an element hidden by an object build (before
//!   its build-in step, after its build-out step, before a delayed build-in or after a timed
//!   build-out of the live step plays), by a visibility condition that fails, or by
//!   the live look (no audience screen draws the slide layer as authored: it is hidden or
//!   replaced by a template override) keeps its play open and in step, with its source muted,
//!   so a later reveal is heard in sync. The actor re-checks this with every change of its
//!   state (build steps, timers, video inputs, the look, media transport), when a timer's
//!   state changes, with every media tick while media plays (the video and audio states:
//!   playing, ended, looping), and when a timed build of the live step starts or ends.
//!   A play that could not be opened (a missing or unreadable file) is reported to the
//!   operator and opened again when the slide layer changes (the slide triggered again: its
//!   time counts from that trigger, so it starts near its in point).
//! - **Muted: everywhere else** (props, messages, announcements, a look's template override, a
//!   slide the actor has no play for). The render thread keeps one [`ElementVideos`] registry:
//!   the first frame asked for opens a muted [`media::Player`] on its own thread (opening is
//!   blocking: file, decoder, first frames), timed by the seconds since the slide went live.
//!
//! Until a player is open, and while a frame is not decoded yet, the element shows no
//! picture. Players follow the fill's [`model::Playback`]: `repeat` (loop forever, a count, or
//! once, holding the last frame), `inPoint` (the start offset) and `outPoint`; sound plays at
//! rate 1 only. A render-thread player nobody asked for in [`IDLE`] is closed (off the render
//! thread: closing joins its decoder).

use crate::playback::LiveVideo;
use media::{Loops, Player, PlayerOptions, VideoFrame};
use model::{
    AudioSettings, ElementId, FillKind, Layer, MediaAssetId, MediaKind, Playback, Repeat, Slide,
    SlideId,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a player stays open after its element was last drawn.
pub const IDLE: Duration = Duration::from_secs(2);

/// One video element of a slide: the element, its asset and how it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoElement {
    /// The element.
    pub element: ElementId,
    /// The video.
    pub asset: MediaAssetId,
    /// Loop, in / out points.
    pub playback: Playback,
    /// Its sound's level, mute and routing.
    pub audio: AudioSettings,
}

impl VideoElement {
    /// Plays the same picture as `o` (another sound level does not reopen a player).
    pub fn same_video(&self, o: &VideoElement) -> bool {
        self.element == o.element && self.asset == o.asset && self.playback == o.playback
    }
}

/// The video elements of `slide` (visible elements with an enabled media fill whose asset is
/// a video in `assets`), their playback with the asset's trim (media preparation) as its
/// default range, for every player: the sounding play and the muted ones (other screens,
/// signage, held slides).
pub fn video_elements(
    slide: &Slide,
    assets: &HashMap<MediaAssetId, model::MediaAsset>,
) -> Vec<VideoElement> {
    slide
        .elements
        .iter()
        .filter(|e| !e.hidden && e.fill.enabled)
        .filter_map(|e| match &e.fill.kind {
            FillKind::Media {
                asset,
                playback,
                audio,
                ..
            } => {
                let a = assets.get(asset).filter(|a| a.kind == MediaKind::Video)?;
                Some(VideoElement {
                    element: e.id,
                    asset: *asset,
                    playback: a.trimmed(playback),
                    audio: audio.clone(),
                })
            }
            _ => None,
        })
        .collect()
}

type Slot = Arc<Mutex<Option<Result<Player, String>>>>;

/// Where an element's frames come from.
enum Source {
    /// A muted player of the render thread's own.
    Local(Slot),
    /// The show actor's play with sound (shared by every screen).
    Shared(Arc<ElementPlay>),
}

struct Entry {
    source: Source,
    /// The muted player the play with sound took over from, drawn instead while the play has
    /// no frame (still opening) or one behind it; with when the play first had a frame (see
    /// [`HANDOVER`]).
    handover: Option<(Slot, Option<Instant>)>,
    /// What it was opened for (another asset or playback reopens it).
    video: VideoElement,
    last_used: Instant,
}

impl Entry {
    /// The play `p` takes over from what the entry drew (the render's slide time `t` at
    /// `at`): a muted player becomes the hand-over when the play is on its timeline; returns
    /// what is closed.
    fn take_over(&mut self, p: Arc<ElementPlay>, t: f64, at: Instant) -> Vec<Source> {
        let same = p.on_timeline(t, at);
        match std::mem::replace(&mut self.source, Source::Shared(p)) {
            Source::Local(slot) if same => self
                .handover
                .replace((slot, None))
                .map(|(old, _)| Source::Local(old))
                .into_iter()
                .collect(),
            old @ Source::Local(_) => {
                let mut gone = vec![old];
                gone.extend(self.handover.take().map(|(h, _)| Source::Local(h)));
                gone
            }
            old => vec![old],
        }
    }
}

/// Longest a muted player is drawn after the play with sound has its first frame, while the
/// play's decoder catches up with its clock (a slow open under load starts it behind): the
/// picture never steps back at the hand-over (nor goes blank while the play still opens). Both are on the slide's timeline, so the play's
/// frame is the muted one's or newer once it has caught up.
pub const HANDOVER: Duration = Duration::from_secs(1);

/// Largest difference, s, between the slide times of a play and a render entry that are on
/// the same timeline (they count from the same instant: only the reading moments differ).
const SAME_TIMELINE: f64 = 0.25;

/// The frame of a muted player at the slide's time `t` (s since the slide went live).
fn local_frame(slot: &Slot, playback: &Playback, t: f64) -> Option<(u64, Arc<VideoFrame>)> {
    let guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    let player = guard.as_ref()?.as_ref().ok()?;
    let rate = if playback.rate > 0.0 {
        playback.rate
    } else {
        1.0
    };
    let f = player.frame_for(t.max(0.0) * rate)?;
    Some((player.id(), f))
}

/// What a hand-over draws: the play's frame `shared`, unless it is behind the muted player's
/// `muted` (by more than half a frame of `frame_dur`) or has none yet; `None` when the
/// muted player is done with.
fn handover_frame(
    shared: &Option<(u64, Arc<VideoFrame>)>,
    muted: Option<(u64, Arc<VideoFrame>)>,
) -> Option<(u64, Arc<VideoFrame>)> {
    let m = muted?;
    shared_behind(shared.as_ref().map(|(_, f)| (f.t, f.duration)), m.1.t).then_some(m)
}

/// The play's frame (its timeline time and duration; `None`: none yet) is behind the muted
/// player's at timeline time `muted` by more than half a frame.
fn shared_behind(shared: Option<(f64, f64)>, muted: f64) -> bool {
    shared.is_none_or(|(t, duration)| t + duration * 0.5 < muted)
}

/// The render thread's open element players, by (slide run, element).
#[derive(Default)]
pub struct ElementVideos {
    players: HashMap<(u64, ElementId), Entry>,
}

impl ElementVideos {
    /// The frame of video element `v` on slide `slide` (an identity: the same slide shown
    /// keeps its player), with its player's stream ID. `plays` are the show actor's plays
    /// with sound: one for slide `id`'s element `v` is used (its frame at present time `at`,
    /// on its clock); otherwise a muted player of this registry is opened on first use, and
    /// again when the element's video or playback changed (`path` resolves the asset's file),
    /// timed by `t`, the seconds since the slide went live. `None` until it is open and has a
    /// frame, or when the file can't be played.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        slide: u64,
        id: SlideId,
        v: &VideoElement,
        plays: &ElementPlays,
        path: impl FnOnce() -> Option<PathBuf>,
        t: f64,
        at: Instant,
        now: Instant,
    ) -> Option<(u64, Arc<VideoFrame>)> {
        let shared = || {
            lock(plays)
                .get(&(id, v.element))
                .filter(|p| p.video.same_video(v) && !p.failed())
                .cloned()
        };
        let source = |path: Option<PathBuf>| match shared() {
            Some(p) => Source::Shared(p),
            None => Source::Local(open(path, &v.playback)),
        };
        let entry = match self.players.entry((slide, v.element)) {
            std::collections::hash_map::Entry::Occupied(o) => {
                let e = o.into_mut();
                if !e.video.same_video(v) {
                    let old = std::mem::replace(&mut e.source, source(path()));
                    e.video = v.clone();
                    close(vec![old]);
                    if let Some((h, _)) = e.handover.take() {
                        close(vec![Source::Local(h)]);
                    }
                } else if let Some(p) = shared()
                    && !matches!(&e.source, Source::Shared(o) if Arc::ptr_eq(o, &p))
                {
                    // The actor's play arrived after this muted one opened (new content was
                    // drawn before its state went out), or it replaced the play this entry
                    // drew (the element left the slide and came back within IDLE, a failed
                    // play opened again): the registered one, with the sound, takes over (on
                    // the same timeline: it starts at the slide's time, so the picture does
                    // not step back: the muted player is drawn until the play's decoder has
                    // caught up, see HANDOVER). A play on another timeline (R2-2: it counts
                    // from a later trigger than this entry) takes over at once: holding the
                    // muted picture would show it out of step with the sound, then step back.
                    // An entry whose play left the registry keeps it (the outgoing side of a
                    // dissolve).
                    close(e.take_over(p, t, at));
                }
                e
            }
            std::collections::hash_map::Entry::Vacant(o) => o.insert(Entry {
                source: source(path()),
                handover: None,
                video: v.clone(),
                last_used: now,
            }),
        };
        entry.last_used = now;
        match &entry.source {
            Source::Shared(p) => {
                let f = p.frame(at);
                if let Some((slot, since)) = &mut entry.handover {
                    if f.is_some() && since.is_none() {
                        *since = Some(now);
                    }
                    let held = since
                        .is_none_or(|s| now.saturating_duration_since(s) < HANDOVER)
                        .then(|| handover_frame(&f, local_frame(slot, &v.playback, t)))
                        .flatten();
                    if held.is_some() {
                        return held;
                    }
                    if let Some((slot, _)) = entry.handover.take() {
                        close(vec![Source::Local(slot)]);
                    }
                }
                f
            }
            Source::Local(slot) => local_frame(slot, &v.playback, t),
        }
    }

    /// Closes players not asked for in [`IDLE`] (off this thread).
    pub fn prune(&mut self, now: Instant) {
        let gone: Vec<(u64, ElementId)> = self
            .players
            .iter()
            .filter(|(_, e)| now.saturating_duration_since(e.last_used) > IDLE)
            .map(|(k, _)| *k)
            .collect();
        let mut closing = Vec::new();
        for k in gone {
            if let Some(e) = self.players.remove(&k) {
                closing.push(e.source);
                closing.extend(e.handover.map(|(h, _)| Source::Local(h)));
            }
        }
        close(closing);
    }

    /// Open (or opening) players.
    pub fn len(&self) -> usize {
        self.players.len()
    }

    /// No players.
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }
}

/// Closes players off the render thread (closing joins their decoders; the last holder of
/// a shared play may be this thread).
fn close(sources: Vec<Source>) {
    if !sources.is_empty() {
        std::thread::spawn(move || drop(sources));
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- plays with sound (the show actor's) -----------------------------------------------

/// The show actor's element plays with sound, by (slide, element).
pub type ElementPlays = Mutex<HashMap<(SlideId, ElementId), Arc<ElementPlay>>>;

/// Audio-engine source IDs of element plays (apart from play IDs and the tone's).
static SOURCE_ID: AtomicU64 = AtomicU64::new(1 << 61);

/// One video element of the live slide playing with its sound (see the [module docs](self)).
pub struct ElementPlay {
    /// Its source ID in the audio engine.
    pub id: u64,
    /// What it plays.
    pub video: VideoElement,
    /// The player, once open (`None` while opening, or when it could not be opened).
    player: Mutex<Option<Arc<LiveVideo>>>,
    /// The sound settings in use (an edit of the level changes them in place).
    audio: Mutex<AudioSettings>,
    /// The screens draw the element: its sound is heard (otherwise its source is muted).
    heard: AtomicBool,
    /// Its player could not be opened.
    failed: AtomicBool,
    /// The slide layer's change it was opened for (a failed play opens again on a new one).
    run: u64,
    /// When the slide went live, for this play: its timeline counts from then.
    since: Option<Instant>,
}

impl ElementPlay {
    /// The open player.
    pub fn player(&self) -> Option<Arc<LiveVideo>> {
        lock(&self.player).clone()
    }

    /// Whether its sound is heard (the screens draw the element).
    pub fn heard(&self) -> bool {
        self.heard.load(Ordering::Relaxed)
    }

    /// Whether its player could not be opened.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// Whether the play is on the timeline of a render entry whose slide time at `at` is `t`
    /// (both count from the same go-live; a play opened again after a failure counts from
    /// its new trigger, a look that restyled the slide restarted the render's clock).
    fn on_timeline(&self, t: f64, at: Instant) -> bool {
        let own = self
            .since
            .map_or(0.0, |s| at.saturating_duration_since(s).as_secs_f64());
        (own - t).abs() <= SAME_TIMELINE
    }

    /// Its source's parameters: the fill's sound settings, muted while it is not heard.
    fn params(&self) -> audio::SourceParams {
        let mut p = crate::sound::params_of(Layer::Slide, &lock(&self.audio));
        p.muted |= !self.heard();
        p
    }

    /// The frame at present time `at` on the play's clock (held at the end of a play that
    /// does not repeat), with the player's stream ID.
    pub fn frame(&self, at: Instant) -> Option<(u64, Arc<VideoFrame>)> {
        let v = self.player()?;
        let t = v.time_at(at);
        let t = match v.end() {
            Some(e) => t.min((e - v.player.frame_duration() * 0.5).max(0.0)),
            None => t,
        };
        let f = v.player.frame_for(t.max(0.0))?;
        Some((v.player.id(), f))
    }
}

/// A video element of the live slide that plays with sound (see [`sounding`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Sounding {
    /// Its slide.
    pub slide: SlideId,
    /// The element and its video.
    pub video: VideoElement,
    /// The screens draw it: its sound is heard. Otherwise it plays on with its sound muted
    /// (hidden by a build step, a failed visibility condition or the live look).
    pub heard: bool,
}

/// The video elements of the live slide that play with sound: the slide layer's slide
/// (`None` while a foreground video hides it), with their slide's ID and whether the screens
/// draw them `now` (see the [module docs](self)); and when to check again (a timed build of
/// the live step starts or ends then, which may show or hide one of them). `step0`: when the
/// screens started the slide's first build step (see [`Step0`]).
pub fn sounding(
    shared: &crate::Shared,
    data: &crate::data::ShowData,
    state: &crate::show::ShowState,
    step0: Option<Instant>,
    now: Instant,
) -> (Vec<Sounding>, Option<Instant>) {
    if state.slide_media.is_some() {
        return Default::default();
    }
    let Some(live) = state.slide else {
        return Default::default();
    };
    let held;
    let (slide, canvas) = if live.detached {
        match state.held(Layer::Slide) {
            Some(h) => {
                held = h;
                (&held.slide, held.canvas)
            }
            None => return Default::default(),
        }
    } else {
        match data
            .content
            .presentation(live.presentation)
            .and_then(|p| p.slides().nth(live.index).map(|s| (s, p.canvas_size)))
        {
            Some(s) => s,
            None => return Default::default(),
        }
    };
    let videos = video_elements(slide, &data.content.assets);
    if videos.is_empty() {
        return Default::default();
    }
    let (drawn, recheck) = drawn_elements(shared, data, state, slide, canvas, step0, now);
    let wanted = videos
        .into_iter()
        .map(|v| Sounding {
            slide: slide.id,
            heard: drawn.contains(&v.element),
            video: v,
        })
        .collect();
    (wanted, recheck)
}

/// Whether an audience screen draws the slide layer's slide as authored under the live look
/// (its slide layer shown, without a template override); `true` without a look or without
/// audience screens (nothing to follow).
pub fn drawn_as_authored(data: &crate::data::ShowData, state: &crate::show::ShowState) -> bool {
    let Some(look) = state
        .look
        .and_then(|id| data.looks.iter().find(|l| l.id == id))
    else {
        return true;
    };
    let mut audience = data
        .screens
        .iter()
        // Audience screens only: stage screens and signage channels' screens never draw the
        // show's slide (H02-3).
        .filter(|s| {
            matches!(
                s.kind,
                model::ScreenKind::Audience | model::ScreenKind::Unknown
            )
        })
        .peekable();
    audience.peek().is_none()
        || audience.any(|s| {
            let l = look.for_screen(s.id);
            l.layers.contains(Layer::Slide) && l.template_override.is_none()
        })
}

/// When the screens started the slide layer's first build step, as the render thread counts
/// it (H04-2): from the slide layer's last change (the same slide triggered again included),
/// or from a later look change that made the screens draw the slide as authored again (its
/// slide layer was hidden or replaced by a template override everywhere: they start it over).
/// Kept by the show actor, updated with every state it publishes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Step0 {
    /// The look change last seen.
    look_seq: u64,
    /// The slide was drawn as authored then.
    authored: bool,
    /// When a look change started the slide over.
    look_at: Option<Instant>,
}

impl Step0 {
    /// Follows `state` and returns when step 0 started (`None`: no slide layer change).
    pub fn follow(
        &mut self,
        data: &crate::data::ShowData,
        state: &crate::show::ShowState,
    ) -> Option<Instant> {
        let authored = drawn_as_authored(data, state);
        if let Some(l) = &state.look_change
            && l.seq != self.look_seq
        {
            self.look_seq = l.seq;
            if authored && !self.authored {
                self.look_at = Some(l.at);
            }
        }
        self.authored = authored;
        let change = state.changes.get(&Layer::Slide).map(|c| c.at);
        match (change, self.look_at) {
            (Some(c), Some(l)) => Some(c.max(l)),
            (c, _) => c,
        }
    }
}

/// The elements of the slide layer's live `slide` that the audience screens draw now, as
/// [`crate::show::plan_screen`] and the render thread decide it: no audience screen draws the
/// slide as authored under the live look (its slide layer hidden, or replaced by a template
/// override) = none; otherwise those visible at `now` in the live build step (its timed builds
/// played from the step's start, as the render thread plays them: step 0 from `step0`, see
/// [`Step0`]) with their visibility conditions resolved against the same live data as the
/// screens' (media states included). With the next instant a timed build of the step starts
/// or ends (`None`: the step has settled).
fn drawn_elements(
    shared: &crate::Shared,
    data: &crate::data::ShowData,
    state: &crate::show::ShowState,
    slide: &Slide,
    canvas: model::Size,
    step0: Option<Instant>,
    now: Instant,
) -> (std::collections::HashSet<ElementId>, Option<Instant>) {
    if !drawn_as_authored(data, state) {
        return Default::default();
    }
    let ctx = crate::engine::live_context(shared, data, state, crate::timers::Clock::now());
    let (resolved, _) = render::live::resolve_slide(slide, &ctx);
    let mut recheck = None;
    let shown = if slide.builds.is_empty() {
        resolved
    } else {
        let build = state
            .layer_build(Layer::Slide)
            .filter(|b| b.slide == slide.id);
        let step = build.map_or(0, |b| b.step.min(model::build::step_count(slide)));
        // When the step started: step 0 with the slide (see `Step0`); a later step when it
        // was taken (`None`: taken back to, settled).
        let started = if step == 0 {
            step0
        } else {
            build.and_then(|b| b.at)
        };
        let elapsed = started.map(|s| now.saturating_duration_since(s).as_secs_f64());
        // Settled past the step's end, as on the screens.
        let length = model::build::step_length(&resolved, step);
        let moving = elapsed.filter(|e| *e < length);
        let (t0, t) = if step == 0 {
            (moving, None)
        } else {
            (None, moving)
        };
        if let (Some(s), Some(e)) = (started, moving) {
            recheck = model::build::steps(&resolved)
                .iter()
                .filter(|b| b.step == step)
                .flat_map(|b| [b.start, b.start + b.duration])
                .filter(|&at| at > e)
                .min_by(f64::total_cmp)
                // Just past it: a build-in shows the element once its progress is above 0.
                // Build times are not bounded above (an edit, an imported file): one too far
                // away to be an instant is never checked (H04-1).
                .and_then(|at| recheck_at(s, at));
        }
        std::borrow::Cow::Owned(model::build::frame(&resolved, canvas, step, t0, t).into_owned())
    };
    let drawn = shown
        .elements
        .iter()
        .filter(|e| !e.hidden)
        .map(|e| e.id)
        .collect();
    (drawn, recheck)
}

/// The instant just past `at` s after `started` (a timed build's start or end), `None` when
/// it is out of range (a huge or infinite build delay or duration).
fn recheck_at(started: Instant, at: f64) -> Option<Instant> {
    Duration::try_from_secs_f64(at + 0.001)
        .ok()
        .and_then(|d| started.checked_add(d))
}

/// Keeps the plays with sound in step with `wanted` (called by the show actor before it
/// publishes a state; `run`: the slide layer's last change): plays no longer wanted lose
/// their sound over `fade` s (the slide layer's transition) and leave the registry (the
/// render thread may still hold one for the outgoing side of a dissolve); new ones are opened
/// on a thread of their own and their sound fades in over `fade`; a changed level or routing,
/// or an element the screens stopped (or started) drawing, applies in place (its source is
/// muted, the play keeps going); a play that could not be opened is opened again for a new
/// `run`. `since`: when the slide layer last changed (the slide went live); a new play starts
/// at the time its video has been showing since then (see the [module docs](self)).
pub fn sync_plays(
    shared: &Arc<crate::Shared>,
    wanted: Vec<Sounding>,
    fade: f64,
    run: u64,
    since: Option<Instant>,
) {
    let mut reg = lock(&shared.element_plays);
    let gone: Vec<(SlideId, ElementId)> = reg
        .iter()
        .filter(|(k, p)| {
            (p.failed() && p.run != run)
                || !wanted
                    .iter()
                    .any(|w| (w.slide, w.video.element) == **k && w.video.same_video(&p.video))
        })
        .map(|(k, _)| *k)
        .collect();
    let mut dropped = Vec::new();
    for k in gone {
        if let Some(p) = reg.remove(&k) {
            shared.sound.engine.remove_source(p.id, fade);
            dropped.push(p);
        }
    }
    for w in wanted {
        if let Some(p) = reg.get(&(w.slide, w.video.element)) {
            let changed = {
                let mut a = lock(&p.audio);
                let changed = *a != w.video.audio || p.heard() != w.heard;
                *a = w.video.audio.clone();
                changed
            };
            if changed {
                p.heard.store(w.heard, Ordering::Relaxed);
                shared.sound.engine.set_params(p.id, p.params());
            }
            continue;
        }
        let play = Arc::new(ElementPlay {
            id: SOURCE_ID.fetch_add(1, Ordering::Relaxed),
            audio: Mutex::new(w.video.audio.clone()),
            video: w.video,
            player: Mutex::new(None),
            heard: AtomicBool::new(w.heard),
            failed: AtomicBool::new(false),
            run,
            since,
        });
        reg.insert((w.slide, play.video.element), play.clone());
        start(shared.clone(), w.slide, play, fade, since);
    }
    drop(reg);
    // Stopping a decoder joins it: off the caller's thread.
    if !dropped.is_empty() {
        std::thread::spawn(move || drop(dropped));
    }
}

/// The slide's timeline at `at`: seconds since the slide went live (`since`) times the
/// fill's rate, as the render thread's muted players play it (0 without `since`).
fn slide_time(since: Option<Instant>, playback: &Playback, at: Instant) -> f64 {
    let rate = if playback.rate > 0.0 {
        playback.rate
    } else {
        1.0
    };
    since.map_or(0.0, |s| {
        at.saturating_duration_since(s).as_secs_f64() * rate
    })
}

/// Opens `play`'s player on a thread of its own; once open (and still wanted), starts its
/// clock and its sound at the slide's time (`since`: when the slide went live): the picture
/// continues where a muted player of the render thread was, and the sound comes in at the
/// matching point; a clip that does not repeat and already ended holds its last frame and is
/// not heard. Its sound fades in over what is left of the slide layer's transition (`fade`).
fn start(
    shared: Arc<crate::Shared>,
    slide: SlideId,
    play: Arc<ElementPlay>,
    fade: f64,
    since: Option<Instant>,
) {
    let spawned = std::thread::Builder::new()
        .name("element sound".into())
        .spawn(move || {
            let data = shared.data();
            let v = &play.video;
            let time = |at: Instant| slide_time(since, &v.playback, at);
            // Decoding starts at the slide's time now; opening takes a moment, which the
            // clock skips below (within the decoded queue, or a seek for a slow open).
            let from = time(Instant::now());
            // Could not be opened: the operator is told, the render thread draws its own
            // (muted) player meanwhile, and triggering the slide again opens it again.
            let failed = |why: String| {
                play.failed.store(true, Ordering::Relaxed);
                if lock(&shared.element_plays)
                    .get(&(slide, v.element))
                    .is_some_and(|p| Arc::ptr_eq(p, &play))
                {
                    shared.notice_keyed(
                        protocol::NoticeLevel::Warning,
                        format!(
                            "video element: cannot play {}: {why}",
                            data.content.asset_name(v.asset)
                        ),
                        Some(format!("element-video:{}", v.asset.0)),
                    );
                }
            };
            let path = match data.content.asset_path(v.asset) {
                Ok(p) => p,
                Err(why) => return failed(why),
            };
            let opened = LiveVideo::open(
                play.id,
                v.asset,
                data.content.asset_name(v.asset),
                Layer::Slide,
                &path,
                &match data.content.assets.get(&v.asset) {
                    Some(a) => a.trimmed(&v.playback),
                    None => v.playback.clone(),
                },
                data.settings.default_media_fade,
                Some(shared.sound.channels()),
                from,
            );
            let live = match opened {
                Ok(l) => l,
                Err(e) => return failed(format!("{}: {e:#}", path.display())),
            };
            let reg = lock(&shared.element_plays);
            // Superseded while opening (the slide left, the video changed): never heard.
            if !reg
                .get(&(slide, v.element))
                .is_some_and(|p| Arc::ptr_eq(p, &play))
            {
                drop(reg);
                return;
            }
            let now = Instant::now();
            // When its first sound is heard, at the slide's time then.
            let heard = now + shared.sound.latency();
            let t = time(heard);
            // A clip that does not repeat and ended before the play opened: its last frame,
            // no sound.
            let ended = live.end().is_some_and(|e| t >= e);
            let sound = if ended { None } else { live.take_audio() };
            match sound {
                Some((tap, sync)) => {
                    live.start_at(heard, t);
                    // The tap decodes from `from`: the step to `t` is skipped in its queue.
                    // After an output stall the sound skips the gap, so the picture never
                    // steps back (M-5).
                    let src = crate::sound::PlaySource::new(
                        tap,
                        sync,
                        live.sound_fade_in(),
                        live.sound_fade_out(),
                    )
                    .starting_at(from)
                    .skipping_stalls();
                    // What is left of the slide layer's transition.
                    let shown =
                        since.map_or(0.0, |s| now.saturating_duration_since(s).as_secs_f64());
                    let fade = (fade - shown).max(0.0);
                    // Under the registry's lock: `sync_plays` changes what is heard under it.
                    shared
                        .sound
                        .engine
                        .add_source(play.id, Box::new(src), play.params(), fade);
                }
                None => live.start_at(now, time(now)),
            }
            *lock(&play.player) = Some(Arc::new(live));
        });
    if let Err(e) = spawned {
        tracing::warn!("video element: no thread: {e}");
    }
}

/// Opens a muted player for `playback` on its own thread; the slot fills when it is open.
fn open(path: Option<PathBuf>, playback: &Playback) -> Slot {
    let slot: Slot = Arc::new(Mutex::new(None));
    let Some(path) = path else {
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Err("no file".into()));
        return slot;
    };
    let loops = match playback.repeat {
        Repeat::Loop | Repeat::LoopFor { .. } => Loops::Forever,
        Repeat::LoopCount { count } => Loops::Count(count.max(1)),
        Repeat::Once | Repeat::Unknown => Loops::Once,
    };
    let options = PlayerOptions {
        in_point: playback.in_point.max(0.0),
        out_point: playback.out_point,
        loops,
        audio: None,
        ..Default::default()
    };
    let out = slot.clone();
    let spawned = std::thread::Builder::new()
        .name("element video".into())
        .spawn(move || {
            let r = Player::open(&path, options).map_err(|e| {
                tracing::warn!(file = %path.display(), "video element: {e:#}");
                format!("{e:#}")
            });
            *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        });
    if let Err(e) = spawned {
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Err(e.to_string()));
    }
    slot
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H02-3: a signage channel's screen (or a stage screen) never counts as an audience
    /// screen drawing the slide: a look hiding the slide layer on every audience screen
    /// hides it.
    #[test]
    fn signage_screens_do_not_draw_the_show_slide() {
        let dir = std::env::temp_dir().join(format!("drawn-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("portable")).unwrap();
        let mut data = crate::data::ShowData::load(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let screen = |kind| model::Screen {
            kind,
            ..model::Screen::default()
        };
        let audience = screen(model::ScreenKind::Audience);
        let mut hidden = model::LayerVisibility::ALL;
        hidden.set(Layer::Slide, false);
        let look = model::Look {
            screens: vec![model::ScreenLook {
                screen: audience.id,
                layers: hidden,
                template_override: None,
                mask: None,
                language: Default::default(),
            }],
            ..model::Look::default()
        };
        let state = crate::show::ShowState {
            look: Some(look.id),
            ..Default::default()
        };
        data.looks = vec![look];
        data.screens = vec![
            audience,
            screen(model::ScreenKind::Signage {
                channel: "c1".into(),
            }),
            screen(model::ScreenKind::Stage { layout: None }),
        ];
        assert!(!drawn_as_authored(&data, &state));
        // Another audience screen the look doesn't list draws it.
        data.screens.push(screen(model::ScreenKind::Audience));
        assert!(drawn_as_authored(&data, &state));
    }

    fn video(asset: MediaAssetId, playback: Playback) -> VideoElement {
        VideoElement {
            element: ElementId(uuid::Uuid::from_u128(7)),
            asset,
            playback,
            audio: AudioSettings::default(),
        }
    }

    /// D01-5: the same slide and element with another video or other playback settings gets
    /// a player of its own, not the one opened for what the element had before.
    /// Every player of a video element (the muted ones too: signage, other screens) plays
    /// the asset's trim unless the element sets its own range.
    #[test]
    fn video_elements_take_the_assets_trim() {
        let mut a = model::MediaAsset {
            kind: MediaKind::Video,
            ..model::MediaAsset::default()
        };
        a.prep = Some(model::MediaPrep {
            trim_in: Some(2.0),
            trim_out: Some(28.0),
            ..model::MediaPrep::default()
        });
        let fill = |playback: Playback| {
            let mut e = model::Element::new(model::Rect::new(0.0, 0.0, 10.0, 10.0));
            e.fill = model::Fill::enabled(FillKind::Media {
                asset: a.id,
                display: Default::default(),
                playback,
                audio: Default::default(),
            });
            e
        };
        let mut slide = Slide::new();
        slide.elements.push(fill(Playback::default()));
        slide.elements.push(fill(Playback {
            in_point: 1.0,
            ..Playback::default()
        }));
        let assets: HashMap<_, _> = [(a.id, a.clone())].into();
        let v = video_elements(&slide, &assets);
        assert_eq!(v.len(), 2);
        assert_eq!(
            (v[0].playback.in_point, v[0].playback.out_point),
            (2.0, Some(28.0))
        );
        assert_eq!(
            (v[1].playback.in_point, v[1].playback.out_point),
            (1.0, None)
        );
    }

    #[test]
    fn another_video_or_playback_reopens_the_player() {
        let mut players = ElementVideos::default();
        let plays = ElementPlays::default();
        let slide = SlideId(uuid::Uuid::from_u128(3));
        let now = Instant::now();
        let opened = std::cell::Cell::new(0);
        let path = || {
            opened.set(opened.get() + 1);
            None
        };
        let a = MediaAssetId(uuid::Uuid::from_u128(1));
        let b = MediaAssetId(uuid::Uuid::from_u128(2));
        let v = video(a, Playback::default());
        assert!(
            players
                .frame(9, slide, &v, &plays, path, 0.0, now, now)
                .is_none()
        );
        assert!(
            players
                .frame(9, slide, &v, &plays, path, 0.1, now, now)
                .is_none()
        );
        assert_eq!(opened.get(), 1, "the same element keeps its player");
        players.frame(
            9,
            slide,
            &video(b, Playback::default()),
            &plays,
            path,
            0.2,
            now,
            now,
        );
        assert_eq!(opened.get(), 2, "another video reopens");
        let looped = Playback {
            in_point: 2.0,
            ..Playback::default()
        };
        players.frame(
            9,
            slide,
            &video(b, looped.clone()),
            &plays,
            path,
            0.3,
            now,
            now,
        );
        assert_eq!(opened.get(), 3, "other playback settings reopen");
        players.frame(9, slide, &video(b, looped), &plays, path, 0.4, now, now);
        assert_eq!(opened.get(), 3);
        assert_eq!(players.len(), 1);
    }

    /// The hand-over from a muted player to the play with sound draws the muted picture while
    /// the play has no frame or one behind it (its decoder catching up after a slow open), so
    /// the picture never steps back; the play's frame once it is level or ahead.
    #[test]
    fn the_handover_never_steps_back() {
        let d = 1.0 / 30.0;
        assert!(shared_behind(None, 2.5), "no frame yet");
        assert!(
            shared_behind(Some((2.3, d)), 2.5),
            "behind: the muted one is drawn"
        );
        assert!(!shared_behind(Some((2.5, d)), 2.5), "level");
        assert!(!shared_behind(Some((2.49, d)), 2.5), "within half a frame");
        assert!(!shared_behind(Some((2.6, d)), 2.5), "ahead");
    }

    fn play(v: &VideoElement, failed: bool) -> Arc<ElementPlay> {
        play_since(v, failed, None)
    }

    fn play_since(v: &VideoElement, failed: bool, since: Option<Instant>) -> Arc<ElementPlay> {
        Arc::new(ElementPlay {
            id: SOURCE_ID.fetch_add(1, Ordering::Relaxed),
            video: v.clone(),
            player: Mutex::new(None),
            audio: Mutex::new(v.audio.clone()),
            heard: AtomicBool::new(true),
            failed: AtomicBool::new(failed),
            run: 0,
            since,
        })
    }

    fn drawn_from(
        players: &ElementVideos,
        run: u64,
        v: &VideoElement,
    ) -> Option<*const ElementPlay> {
        match &players.players.get(&(run, v.element))?.source {
            Source::Shared(p) => Some(Arc::as_ptr(p)),
            Source::Local(_) => None,
        }
    }

    /// E01-2: the actor replaced the play of an element (it left the slide and came back
    /// within IDLE: its sound starts again from the in point): the render entry of the same
    /// run draws the new play, not the retired one; an entry whose play left the registry
    /// keeps it (the outgoing side of a dissolve).
    #[test]
    fn a_replaced_play_takes_over_the_render_entry() {
        let mut players = ElementVideos::default();
        let plays = ElementPlays::default();
        let slide = SlideId(uuid::Uuid::from_u128(3));
        let now = Instant::now();
        let v = video(MediaAssetId(uuid::Uuid::from_u128(1)), Playback::default());
        let old = play(&v, false);
        lock(&plays).insert((slide, v.element), old.clone());
        players.frame(9, slide, &v, &plays, || None, 0.0, now, now);
        assert_eq!(drawn_from(&players, 9, &v), Some(Arc::as_ptr(&old)));
        // The play left the registry (the slide left): the entry keeps drawing it.
        lock(&plays).clear();
        players.frame(9, slide, &v, &plays, || None, 0.1, now, now);
        assert_eq!(drawn_from(&players, 9, &v), Some(Arc::as_ptr(&old)));
        // A new play for the same element: it takes over.
        let new = play(&v, false);
        lock(&plays).insert((slide, v.element), new.clone());
        players.frame(9, slide, &v, &plays, || None, 0.2, now, now);
        assert_eq!(drawn_from(&players, 9, &v), Some(Arc::as_ptr(&new)));
    }

    /// A play starts at the slide's time: the seconds since the slide went live times the
    /// fill's rate (the render thread's muted players' clock); 0 without a slide change.
    #[test]
    fn a_play_starts_at_the_slides_time() {
        let at = Instant::now();
        let since = Some(at - Duration::from_millis(2500));
        let at_rate = |rate: f64| Playback {
            rate,
            ..Playback::default()
        };
        assert!((slide_time(since, &at_rate(1.0), at) - 2.5).abs() < 1e-9);
        assert!((slide_time(since, &at_rate(2.0), at) - 5.0).abs() < 1e-9);
        assert!((slide_time(since, &at_rate(0.0), at) - 2.5).abs() < 1e-9);
        assert_eq!(slide_time(None, &at_rate(1.0), at), 0.0);
        // A change recorded after `at` (a race with the actor): not before 0.
        assert_eq!(
            slide_time(Some(at + Duration::from_secs(1)), &at_rate(1.0), at),
            0.0
        );
    }

    /// X3-2: a play whose player could not be opened is not drawn from: the render thread
    /// opens its own (muted) player instead, and the play opened again takes over.
    #[test]
    fn a_failed_play_is_not_drawn_from() {
        let mut players = ElementVideos::default();
        let plays = ElementPlays::default();
        let slide = SlideId(uuid::Uuid::from_u128(3));
        let now = Instant::now();
        let v = video(MediaAssetId(uuid::Uuid::from_u128(1)), Playback::default());
        lock(&plays).insert((slide, v.element), play(&v, true));
        players.frame(9, slide, &v, &plays, || None, 0.0, now, now);
        assert_eq!(drawn_from(&players, 9, &v), None, "its own muted player");
        let again = play(&v, false);
        lock(&plays).insert((slide, v.element), again.clone());
        players.frame(9, slide, &v, &plays, || None, 0.1, now, now);
        assert_eq!(drawn_from(&players, 9, &v), Some(Arc::as_ptr(&again)));
    }

    /// H04-1: a timed build too far away to be an instant (a huge or infinite delay or
    /// duration from an edit or an imported file) is never checked; it does not panic.
    #[test]
    fn a_huge_build_time_is_never_rechecked() {
        let s = Instant::now();
        assert!(recheck_at(s, 2.0).is_some_and(|i| i > s));
        for at in [1e19, 1e300, f64::INFINITY, f64::MAX, f64::NAN] {
            assert_eq!(recheck_at(s, at), None, "{at}");
        }
    }

    /// R2-2: the muted player is held at the hand-over only for a play on its timeline; a
    /// play that counts from a later trigger (opened again after a failure) takes over at
    /// once, so the picture is never held out of step with the sound and then steps back.
    #[test]
    fn the_handover_holds_only_a_play_on_the_same_timeline() {
        let now = Instant::now();
        let v = video(MediaAssetId(uuid::Uuid::from_u128(1)), Playback::default());
        let went_live = now - Duration::from_secs(40);
        for (since, held) in [(went_live, true), (now - Duration::from_millis(300), false)] {
            // The render's own muted player, 40 s into the slide.
            let mut e = Entry {
                source: Source::Local(Arc::new(Mutex::new(None))),
                handover: None,
                video: v.clone(),
                last_used: now,
            };
            let p = play_since(&v, false, Some(since));
            let closed = e.take_over(p.clone(), 40.0, now);
            assert!(matches!(&e.source, Source::Shared(s) if Arc::ptr_eq(s, &p)));
            assert_eq!(
                e.handover.is_some(),
                held,
                "a play counting from {:?} before",
                now - since
            );
            assert_eq!(closed.len(), usize::from(!held), "the muted player closes");
        }
    }
}
