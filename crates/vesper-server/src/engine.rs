//! Rendering and frame pacing.
//!
//! - **One render thread** owns the [`Renderer`] (GPU device, font system, slide and media
//!   caches are shared by every screen). It sleeps until the show state changes, then compares
//!   every screen's [`plan_screen`](crate::show::plan_screen) fingerprint with the last one it
//!   rendered and **renders only the screens that changed**, submitting them together so the
//!   GPU overlaps them, and publishes each new frame as the screen's latest frame.
//! - **Images are decoded off this thread**, by the renderer's decode workers
//!   ([`render::ImageDecoder`]: a few threads, a byte-bounded LRU of decoded images shared by
//!   live and thumbnail rendering, images decoded at most at the size they are drawn). The
//!   render thread only uploads decoded images. A live change never waits for a decode: see
//!   `render_live` for what is shown meanwhile. Decodes for live screens run first, then
//!   prefetches of the next slide's images (queued on every live change), then thumbnails.
//!   Every finished decode sends [`Cmd::ImagesReady`].
//! - **Thumbnails** ([`ThumbJob`]) queue on the same thread at lower priority: interactive
//!   requests before prefetches, and the thread checks for a live change before every
//!   thumbnail, so a burst of thumbnail work delays a live change by at most one thumbnail
//!   render (a GPU render of an already decoded, thumbnail-sized image). A thumbnail whose
//!   images are still decoding is parked and retried after the next finished decode, so it
//!   never blocks the queue. Thumbnails use the renderer's separate scratch caches (slide
//!   layers, uploaded images), so they cannot evict the live screens' cached textures. JPEG
//!   encoding happens elsewhere (the frame is handed back).
//! - **One clock thread per screen** runs at the screen's portable frame rate and offers the
//!   screen's latest frame to each of its outputs on every tick, re-sending the same frame
//!   while nothing changes (network video receivers expect a continuous stream). Outputs are offered the
//!   frame without waiting ([`OutputRuntime::offer`]), and a slow render (cold text layout)
//!   never stalls the clocks: they keep sending the previous frame. An output with a frame
//!   rate override has a clock thread of its own that samples the same latest frame.
//!
//! - **Videos and transitions render every tick.** A screen showing a video or running a
//!   transition is "animated": its clock asks the render thread, on every tick, for the frame
//!   presented at the next tick (`Cmd::Tick` carries that instant). The render thread picks each
//!   video's frame nearest to its media clock at that instant and the transition progress at
//!   that instant, and renders only if the picture changed (a 25 fps video on a 30 fps screen
//!   re-renders 25 times a second; the clock repeats the frame in between). Screens without
//!   either keep rendering on change only. A tick render that finishes after its frame was due
//!   is counted (`late_renders`); the clock never waits for it.
//!
//! A clock that falls behind skips the missed ticks (counted as `late_ticks`) instead of
//! bunching them up.
//!
//! ## Live reconfiguration ([`apply_config`])
//! Screens and outputs change while everything runs: a new screen gets a render target, a
//! clock and outputs; a removed (or disabled) screen's clock stops and its outputs close; a
//! changed screen keeps its clock (a new rate takes effect on the next tick) and the render
//! thread recreates its target when size, alpha or background changed (a new colour
//! correction or corner pin is set on the same target: the next frame has it, and no output
//! restarts). Outputs are matched by binding ID: an unchanged one keeps running untouched, a
//! changed one is closed and rebuilt (an IDN rename withdraws the old source, then creates
//! the new one), a name or video-delay change is applied in place. Every output that goes
//! away is closed **before** any new one is created, so a source name can move from one
//! output to another in one change.
//! Screens and outputs that did not change never stop, so they drop no frames.
//!
//! ## Stage screens
//! Stage screens ([`crate::stage`]) plan their layout like any slide and always cut. Slide
//! pictures in a layout are rendered here (`draw_pictures`): the slide is drawn into an
//! in-memory image at the element's size (the renderer's scratch target, as thumbnails) and
//! the element is filled with it through a made-up media asset ID that the live screens'
//! media resolver answers; complete pictures are reused by content (a few dozen kept). A
//! flashing stage message wakes the render thread at each flash phase.
//!
//! ## Overlays
//! A test pattern ([`render::patterns::test_pattern`]) replaces a screen's composite; the
//! identify label ([`render::patterns::identify_overlay`]) is drawn above everything. Both
//! are part of the screen's render fingerprint. The motion pattern changes every frame: the
//! screen's clock asks the render thread for a new frame on every tick ([`Cmd::Tick`]).

use crate::data::ShowData;
use crate::outputs::{self, OutputRuntime};
use crate::playback::LiveVideo;
use crate::show::{PlannedMedia, ScreenPlan, ShowState, plan_screen, screen_role};
use crate::{ScreenRuntime, Shared, lock};
use model::{MediaDisplay, Screen, Size, Slide};
use render::{
    DecodePriority, ImageSource, LayerContent, LayerMix, LayerSource, MediaResolver, MixInput,
    RenderedFrame, Renderer, RendererOptions, ScreenConfig,
};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Messages to the render thread.
pub enum Cmd {
    /// The show state changed.
    Wake,
    /// An image decode finished (re-render screens waiting for one, retry parked thumbnails).
    ImagesReady,
    /// Render a thumbnail (after any pending live change).
    Thumbnail(Box<ThumbJob>),
    /// A clock tick of a screen that changes every frame (video, transition, animated test
    /// pattern): render the frame presented at `at`.
    Tick {
        /// The screen.
        screen: model::ScreenId,
        /// Its clock's frame counter.
        frame: u64,
        /// When that frame goes out (the screen's next tick).
        at: Instant,
    },
    /// Stop rendering.
    Stop,
}

/// A thumbnail render: one slide over its background, opaque, at a small size.
pub struct ThumbJob {
    /// The slide.
    pub slide: Slide,
    /// Canvas size of its presentation.
    pub canvas: Size,
    /// Background image file and how it is drawn.
    pub background: Option<(PathBuf, MediaDisplay)>,
    /// Output width, px.
    pub width: u32,
    /// Output height, px.
    pub height: u32,
    /// Someone is waiting for it (rendered before prefetches).
    pub interactive: bool,
    /// Keep alpha (transparent where the slide draws nothing) instead of flattening on black.
    pub transparent: bool,
    /// Where the frame goes.
    pub reply: tokio::sync::oneshot::Sender<anyhow::Result<RenderedFrame>>,
}

/// Live numbers of one screen.
#[derive(Clone, Debug, Default)]
pub struct ScreenStats {
    /// Frames offered to outputs per second (measured over the last second).
    pub fps: f64,
    /// Submit-to-readback time of the last render, ms (screens changed together are
    /// submitted together, so this includes the GPU time of the others).
    pub last_render_ms: Option<f64>,
    /// Slowest render so far, ms.
    pub max_render_ms: f64,
    /// Sum of all render times, ms (for the average).
    pub total_render_ms: f64,
    /// Renders so far (only on change).
    pub renders: u64,
    /// Renders that left out an image still decoding (a complete one followed).
    pub incomplete_renders: u64,
    /// Clock ticks so far.
    pub ticks: u64,
    /// Ticks skipped because the clock thread was late.
    pub late_ticks: u64,
    /// Tick renders (videos, transitions) that finished after their frame was due: the
    /// clock sent the previous frame again.
    pub late_renders: u64,
    /// The most recent render times, ms (for percentiles).
    pub recent_ms: VecDeque<f64>,
}

/// Render times kept per screen for percentiles.
const RECENT_RENDERS: usize = 1800;

impl ScreenStats {
    fn record(&mut self, ms: f64) {
        if self.recent_ms.len() == RECENT_RENDERS {
            self.recent_ms.pop_front();
        }
        self.recent_ms.push_back(ms);
    }

    /// (p50, p95, max) of the recent render times, ms.
    pub fn percentiles(&self) -> Option<(f64, f64, f64)> {
        if self.recent_ms.is_empty() {
            return None;
        }
        let mut v: Vec<f64> = self.recent_ms.iter().copied().collect();
        v.sort_by(f64::total_cmp);
        let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        Some((at(0.5), at(0.95), v[v.len() - 1]))
    }
}

/// Handle to the render thread (screen clocks belong to their [`ScreenRuntime`]s).
pub struct Engine {
    tx: Sender<Cmd>,
    render: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl Engine {
    /// Starts the render thread. Returns once the renderer is initialised (or failed to).
    /// Screens, their clocks and outputs are started by [`apply_config`].
    pub fn start(
        shared: Arc<Shared>,
        options: RendererOptions,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
    ) -> anyhow::Result<Engine> {
        let (init_tx, init_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();
        let sh = shared.clone();
        let render = std::thread::Builder::new()
            .name("render".into())
            .spawn(move || {
                let renderer = match Renderer::new(options) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };
                let adapter = renderer.adapter().clone();
                tracing::info!(adapter = %adapter, software = adapter.is_software, "renderer ready");
                if adapter.is_software {
                    tracing::warn!(
                        "rendering on a software adapter: fine for tests, too slow for playout"
                    );
                }
                *lock(&sh.adapter) = Some(adapter.to_string());
                *lock(&sh.fonts) = (
                    renderer.font_families(),
                    renderer.fallback_family().to_string(),
                );
                let _ = sh.decoder.set(renderer.decoder().clone());
                let _ = init_tx.send(Ok(()));
                render_loop(renderer, &sh, rx);
            })?;
        match init_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = render.join();
                return Err(e.context("creating the renderer"));
            }
            Err(_) => {
                let _ = render.join();
                anyhow::bail!("render thread died during start-up");
            }
        }
        Ok(Engine {
            tx,
            render: Some(render),
            shared,
        })
    }

    /// Stops the render thread, every screen clock and every output, and waits for them;
    /// outputs are flushed and closed (IDN sources are withdrawn).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(h) = self.render.take() {
            let _ = h.join();
        }
        let _g = lock(&self.shared.config_lock);
        for rt in self.shared.screens().iter() {
            stop_screen(rt);
        }
        self.shared.set_screens(Vec::new());
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(crate) fn period_of(fr: model::FrameRate) -> Duration {
    let (num, den) = (fr.num.max(1) as u64, fr.den.max(1) as u64);
    Duration::from_nanos(1_000_000_000 * den / num)
}

/// The first tick at or after `t` of the process-wide grid of ticks `period` apart (from
/// [`audio::epoch`]). Every clock (a screen's, an output's own) ticks on it, so two clocks at
/// one rate tick at the same instants and stamp equal timecodes: the two halves of a
/// Fill/Key pair with frame rate overrides flip together (E03-4).
pub(crate) fn on_grid(t: Instant, period: Duration) -> Instant {
    let epoch = audio::epoch();
    let p = period.as_nanos().max(1);
    let Some(since) = t.checked_duration_since(epoch) else {
        return epoch;
    };
    let k = since.as_nanos().div_ceil(p);
    epoch + Duration::from_nanos((k * p).min(u64::MAX as u128) as u64)
}

/// Stops a screen's clock and closes its outputs (blocks until they ended).
fn stop_screen(rt: &ScreenRuntime) {
    rt.stop.store(true, Ordering::Release);
    if let Some(h) = lock(&rt.clock).take() {
        let _ = h.join();
    }
    for o in lock(&rt.outputs).drain(..) {
        o.shutdown();
    }
}

/// Paces one screen: offers its latest frame to its outputs at the screen's frame rate (read
/// on every tick, so a rate change applies at once).
fn clock_loop(shared: &Shared, rt: &ScreenRuntime) {
    let mut fr = rt.frame_rate();
    let mut period = period_of(fr);
    let mut next_due = on_grid(Instant::now(), period);
    let mut frame_count: u64 = 0;
    let mut window_start = Instant::now();
    let mut window_ticks = 0u64;
    while !rt.stop.load(Ordering::Acquire) {
        let now = Instant::now();
        let cur = rt.frame_rate();
        if cur != fr {
            fr = cur;
            period = period_of(fr);
            next_due = on_grid(now, period);
            window_start = now;
            window_ticks = 0;
        }
        if now < next_due {
            // Short sleeps keep shutdown prompt.
            std::thread::sleep((next_due - now).min(Duration::from_millis(50)));
            continue;
        }
        if rt.animated.load(Ordering::Relaxed) {
            // Render the next tick's frame now: it is offered when that tick comes.
            shared.send_render(Cmd::Tick {
                screen: rt.id,
                frame: frame_count + 1,
                at: next_due + period,
            });
        }
        let frame = lock(&rt.latest).clone();
        if let Some(f) = &frame {
            // The instant this tick presents (the frame was rendered for it), on the scale
            // the program audio is stamped with: equal timecodes are presented together.
            let timecode = audio::timecode(next_due);
            for o in rt.outputs().iter().filter(|o| !o.has_own_clock()) {
                o.offer(f, timecode, fr);
            }
        }
        frame_count += 1;
        window_ticks += 1;
        next_due += period;
        let mut late = 0;
        if next_due < now {
            late = ((now - next_due).as_nanos() / period.as_nanos().max(1)) as u64 + 1;
            next_due = on_grid(now, period);
        }
        let elapsed = now - window_start;
        let mut st = lock(&rt.stats);
        st.ticks += 1;
        st.late_ticks += late;
        if elapsed >= Duration::from_secs(1) {
            st.fps = window_ticks as f64 / elapsed.as_secs_f64();
            window_start = now;
            window_ticks = 0;
        }
    }
    tracing::debug!(screen = %rt.screen().name, "clock stopped");
}

/// The clock of an output with a frame rate override: samples the screen's latest frame at
/// its own rate.
fn output_clock(rt: Arc<ScreenRuntime>, out: Arc<OutputRuntime>, stop: Arc<AtomicBool>) {
    let Some(fr) = out.delivery.rate else { return };
    let period = period_of(fr);
    let mut next_due = on_grid(Instant::now(), period);
    while !stop.load(Ordering::Acquire) {
        let now = Instant::now();
        if now < next_due {
            std::thread::sleep((next_due - now).min(Duration::from_millis(50)));
            continue;
        }
        if let Some(f) = lock(&rt.latest).clone() {
            out.offer(&f, audio::timecode(next_due), fr);
        }
        next_due += period;
        if next_due < now {
            next_due = on_grid(now, period);
        }
    }
}

fn start_output_clock(rt: &Arc<ScreenRuntime>, out: &Arc<OutputRuntime>) {
    if !out.has_own_clock() || !out.active {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let (r, o, s) = (rt.clone(), out.clone(), stop.clone());
    match std::thread::Builder::new()
        .name(format!("oclk {}", out.name).chars().take(15).collect())
        .spawn(move || output_clock(r, o, s))
    {
        Ok(h) => *lock(&out.own_clock) = Some((stop, h)),
        Err(e) => tracing::error!(output = %out.name, "cannot start the output clock: {e}"),
    }
}

/// Builds again the display outputs that are unavailable (they could not open their display),
/// in place: when a display's identify label is taken down, an output that found no free
/// display controller meanwhile gets it now (R1-2). Nothing else changes; a stopped engine
/// (no screens) does nothing.
pub fn retry_unavailable_displays(shared: &Arc<Shared>) {
    let _g = lock(&shared.config_lock);
    for rt in shared.screens().iter() {
        let retry = |o: &OutputRuntime| {
            !o.active
                && o.kind == "display"
                && o.binding
                    .as_ref()
                    .is_some_and(|b| matches!(b.transport, model::OutputTransport::Display { .. }))
        };
        let current = rt.outputs();
        if !current.iter().any(|o| retry(o)) {
            continue;
        }
        let screen = rt.screen();
        let mut created = Vec::new();
        let next: Vec<Arc<OutputRuntime>> = current
            .into_iter()
            .map(|o| match &o.binding {
                Some(b) if retry(&o) => {
                    let n = Arc::new(outputs::build_binding(
                        &screen,
                        b,
                        Some(&shared.sound),
                        Some(&shared.displays),
                    ));
                    if n.active {
                        tracing::info!(screen = %screen.name, output = %n.name, "output available again");
                    }
                    o.shutdown();
                    created.push(n.clone());
                    n
                }
                _ => o,
            })
            .collect();
        *lock(&rt.outputs) = next;
        for o in &created {
            start_output_clock(rt, o);
        }
    }
}

/// Brings the running screens and outputs in line with `data` (start-up, reloads, edits).
/// Unchanged screens and outputs keep running untouched; see the module docs. Blocks while
/// outputs are closed and created (call it off the async threads). Outputs that cannot run
/// are kept as `unavailable` entries and announced as notices.
pub fn apply_config(shared: &Arc<Shared>, data: &ShowData) {
    let _g = lock(&shared.config_lock);
    let old = shared.screens();
    let dev = shared.dev.clone();
    let wanted: Vec<&Screen> = data.screens.iter().filter(|s| s.enabled).collect();

    // Phase 1: close everything that goes away (removed screens, removed / changed outputs),
    // so names are free before anything new is created.
    for rt in old.iter() {
        let Some(new) = wanted.iter().find(|s| s.id == rt.id) else {
            tracing::info!(screen = %rt.screen().name, "screen stopped");
            stop_screen(rt);
            continue;
        };
        let bindings = outputs::bindings_of(data, new, &dev);
        let cli_changed = rt.screen().name != new.name;
        let current = rt.outputs();
        let (keep, close): (Vec<_>, Vec<_>) = current.into_iter().partition(|o| match &o.binding {
            // A display output is rebuilt when its display was plugged in or out.
            // So is an output whose crop or blend changed with its screen's arrangement or
            // size.
            Some(_) => bindings.iter().any(|b| {
                Some(b.id) == o.binding_id()
                    && o.can_update_in_place(b)
                    && o.display_signature == shared.displays.signature(b)
                    && o.delivery == outputs::Delivery::for_screen(b, new)
                    && o.problem == outputs::Delivery::problem(b, new)
            }),
            None => !cli_changed,
        });
        if !close.is_empty() {
            *lock(&rt.outputs) = keep;
            for o in close {
                tracing::info!(screen = %new.name, output = %o.name, "output closed");
                o.shutdown();
            }
        }
    }

    // Phase 2: update, create.
    let mut list: Vec<Arc<ScreenRuntime>> = Vec::new();
    let mut unavailable: Vec<String> = Vec::new();
    for s in wanted {
        let (rt, fresh) = match old.iter().find(|r| r.id == s.id) {
            Some(rt) => (rt.clone(), false),
            None => (Arc::new(ScreenRuntime::new(s.clone())), true),
        };
        if !fresh && rt.screen() != *s {
            tracing::info!(screen = %s.name, "screen reconfigured");
        }
        let cli_rebuild = fresh || rt.screen().name != s.name;
        rt.set_screen(s.clone());
        let current = rt.outputs();
        let mut next: Vec<Arc<OutputRuntime>> = Vec::new();
        let mut created: Vec<Arc<OutputRuntime>> = Vec::new();
        for b in outputs::bindings_of(data, s, &dev) {
            match current.iter().find(|o| o.binding_id() == Some(b.id)) {
                Some(o) => {
                    // Same sink: only the name, the delay or the screen's frame rate changed
                    // (or nothing).
                    o.set_delay(b.video_delay);
                    o.set_frame_rate(b.frame_rate_override.unwrap_or(s.frame_rate));
                    next.push(o.clone());
                }
                None => {
                    let o = Arc::new(outputs::build_binding(
                        s,
                        b,
                        Some(&shared.sound),
                        Some(&shared.displays),
                    ));
                    if !o.active && o.kind != "placeholder" {
                        unavailable.push(format!(
                            "output {} unavailable: {}",
                            o.name,
                            o.detail.as_deref().unwrap_or("?")
                        ));
                    }
                    created.push(o.clone());
                    next.push(o);
                }
            }
        }
        if cli_rebuild {
            for o in outputs::build_cli(s, &dev) {
                let o = Arc::new(o);
                created.push(o.clone());
                next.push(o);
            }
        } else {
            next.extend(current.iter().filter(|o| o.binding.is_none()).cloned());
        }
        // A binding's updated document (new name) replaces the runtime's copy lazily: keep
        // the runtime, its sink is identical.
        *lock(&rt.outputs) = next;
        for o in &created {
            start_output_clock(&rt, o);
        }
        if fresh {
            let (sh, r) = (shared.clone(), rt.clone());
            let name: String = format!("clock {}", s.name).chars().take(15).collect();
            match std::thread::Builder::new()
                .name(name)
                .spawn(move || clock_loop(&sh, &r))
            {
                Ok(h) => *lock(&rt.clock) = Some(h),
                Err(e) => tracing::error!(screen = %s.name, "cannot start the clock: {e}"),
            }
            tracing::info!(screen = %s.name, "screen started");
        }
        list.push(rt);
    }
    shared.set_screens(list);
    drop(_g);
    for msg in unavailable {
        shared.notice(protocol::NoticeLevel::Warning, msg);
    }
    shared.rerender();
    shared.status_changed.notify_one();
}

/// Render-thread state of one live screen.
struct LiveScreen {
    target: render::ScreenTarget,
    config: ScreenConfig,
    /// Fingerprint of the picture last rendered.
    fingerprint: Option<u64>,
    /// The last render left out an image that was still decoding (or a transition waits for
    /// one): render again when the decoder reports progress, even if nothing else changed.
    incomplete: bool,
    /// Frame number for an animated test pattern (from the screen's clock).
    frame: u64,
    /// Presentation time of the frame the screen's clock asked for (a tick).
    present: Option<Instant>,
    /// What each layer shows, and its transition (see [`Track`]).
    tracks: Vec<Track>,
    /// The tracks follow the show state (false: a fresh screen, or one that was blank; its
    /// first picture is a cut).
    planned: bool,
    /// What the screen's live data depended on at its last plan (decides when the render
    /// thread re-plans it for a new clock / timer value).
    live: render::live::LiveUse,
    /// The last look change ([`ShowState::look_change`]) this screen followed.
    look_seq: u64,
}

/// One layer of one screen (or one prop of the props layer): its content and the transition
/// into it.
struct Track {
    layer: model::Layer,
    /// The prop a props-layer track shows (nil for the other layers).
    slot: uuid::Uuid,
    current: Option<Content>,
    /// Identity of `current` (0 = nothing).
    key: u64,
    trans: Option<Trans>,
    /// The last [`crate::show::LayerChange`] of this layer the track followed.
    change_seq: u64,
}

/// A transition in progress on a track.
struct Trans {
    from: From,
    /// `None` while the incoming content still waits for an image decode (the outgoing
    /// content stays up meanwhile).
    start: Option<Instant>,
    /// Seconds (0 = a cut, applied once the incoming content is ready).
    duration: f64,
    /// The style the renderer draws (renderable: vendor kinds are already a dissolve).
    kind: model::TransitionKind,
    /// How the drawn progress follows time.
    easing: model::Easing,
    /// Key of the texture the renderer keeps the blend in.
    store: u64,
    /// Rendered at least once (its store holds a blend).
    rendered: bool,
}

/// Where a transition comes from.
enum From {
    /// The previous content (keeps playing, if it is a video).
    Content(Option<Content>),
    /// The last blend of an interrupted transition.
    Snapshot(u64),
}

/// Content of a layer, owned by the render thread (it outlives the show state that set it,
/// for the outgoing side of a dissolve).
#[derive(Clone)]
enum Content {
    Slide {
        slide: Arc<Slide>,
        canvas: Size,
        /// It has scrolling text or video elements (the screen renders every tick while it
        /// is up).
        scrolls: bool,
        /// It has video elements (their frames come from the element players).
        videos: bool,
        /// It has live-source fills (their frames come from the live-source receivers; the
        /// screen renders when a new picture arrived).
        lives: bool,
        /// When it went live on its track (the clock of its scrolling text, video elements
        /// and step-0 builds); set by the track, and kept while the same slide stays up
        /// with new content (an edit, a new value of live data).
        since: Option<Instant>,
        /// Which slide it is, whatever its content ([`slide_ident`]).
        ident: u64,
        /// The build step it is drawn at (a slide with object builds on the slide or
        /// announcements layer); follows the plan without a layer transition.
        build: Option<crate::show::BuildPlay>,
    },
    Image {
        path: PathBuf,
        display: MediaDisplay,
    },
    Video {
        video: Arc<LiveVideo>,
        display: MediaDisplay,
    },
    /// A live network source on the video-input layer (see [`crate::live_sources`]).
    Live {
        rx: Arc<crate::live_sources::LiveReceiver>,
        display: MediaDisplay,
    },
}

impl Content {
    fn slide(slide: Arc<Slide>, canvas: Size, layer: model::Layer, data: &ShowData) -> Self {
        let videos = !crate::element_video::video_elements(&slide, &data.content.assets).is_empty();
        Content::Slide {
            scrolls: videos || render::has_scrolling_text(&slide),
            videos,
            lives: crate::live_sources::has_live_fills(&slide),
            ident: slide_ident(&slide, canvas, layer),
            slide,
            canvas,
            since: None,
            build: None,
        }
    }

    /// An object build of the slide moves at `at` (the screen renders every tick until it
    /// settles).
    fn building(&self, at: Instant) -> bool {
        let Content::Slide {
            slide,
            since,
            build: Some(b),
            ..
        } = self
        else {
            return false;
        };
        let started = if b.step == 0 { *since } else { b.at };
        started.is_some_and(|s| {
            at.saturating_duration_since(s).as_secs_f64() < model::build::step_length(slide, b.step)
        })
    }

    /// Changes every frame (a video, scrolling text).
    fn moves(&self) -> bool {
        matches!(
            self,
            Content::Video { .. }
                | Content::Live { .. }
                | Content::Slide { scrolls: true, .. }
                | Content::Slide { lives: true, .. }
        )
    }
}

static STORE_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Track {
    fn new(layer: model::Layer, slot: uuid::Uuid) -> Self {
        Track {
            layer,
            slot,
            current: None,
            key: 0,
            trans: None,
            change_seq: 0,
        }
    }

    /// Progress of the transition at `at` (`None` = no transition).
    fn progress(&self, at: Instant) -> Option<f32> {
        let t = self.trans.as_ref()?;
        Some(match t.start {
            None => 0.0,
            Some(_) if t.duration <= 0.0 => 1.0,
            Some(s) => (at.saturating_duration_since(s).as_secs_f64() / t.duration) as f32,
        })
    }

    /// Follows new content: starts a transition when the content changed (`cut`: no
    /// transition at all, for fresh screens). The same slide with new content (an edit, a new
    /// value of live data) keeps its clock unless `restart` (a new change of the layer or a
    /// new look: it goes live again). `again`: a new change of the layer (the same slide
    /// triggered again goes live again too: its clock restarts at `start`, with no
    /// transition from itself, as the show actor counts its timed builds and video element
    /// sound, H04-2).
    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        new: Option<Content>,
        key: u64,
        transition: &model::Transition,
        start: Instant,
        now: Instant,
        cut: bool,
        restart: bool,
        again: bool,
    ) {
        if key == self.key {
            // The same slide at another build step: no layer transition.
            if let (
                Some(Content::Slide { build, since, .. }),
                Some(Content::Slide { build: nb, .. }),
            ) = (&mut self.current, &new)
            {
                *build = *nb;
                if again {
                    *since = Some(start.min(now));
                }
            }
            return;
        }
        let kept = match (&self.current, &new) {
            (
                Some(Content::Slide {
                    ident: a, since, ..
                }),
                Some(Content::Slide { ident: b, .. }),
            ) if a == b && !restart => *since,
            _ => None,
        };
        let old = std::mem::replace(&mut self.current, new);
        self.key = key;
        if let Some(Content::Slide { since, .. }) = &mut self.current {
            *since = Some(kept.unwrap_or(start.min(now)));
        }
        if cut {
            self.trans = None;
            return;
        }
        let t = transition.renderable();
        let duration = match t.kind {
            model::TransitionKind::Cut => 0.0,
            _ => t.duration.max(0.0),
        };
        let from = match self.trans.take() {
            // Interrupted mid-way: start from what is on screen.
            Some(tr) if tr.rendered && tr.start.is_some() && self.progress_of(&tr, now) < 1.0 => {
                From::Snapshot(tr.store)
            }
            // Not started yet (waiting for an image): its outgoing content is still up.
            Some(tr) if tr.start.is_none() => tr.from,
            _ => From::Content(old),
        };
        self.trans = Some(Trans {
            from,
            start: Some(start.min(now)),
            duration,
            kind: t.kind,
            easing: t.easing,
            store: STORE_KEY.fetch_add(1, Ordering::Relaxed),
            rendered: false,
        });
    }

    fn progress_of(&self, t: &Trans, at: Instant) -> f32 {
        match t.start {
            None => 0.0,
            Some(_) if t.duration <= 0.0 => 1.0,
            Some(s) => (at.saturating_duration_since(s).as_secs_f64() / t.duration) as f32,
        }
    }

    /// An object build moves on its slide at `at`.
    fn building(&self, at: Instant) -> bool {
        self.current.as_ref().is_some_and(|c| c.building(at))
    }

    fn animated(&self) -> bool {
        self.trans.is_some()
            || self.current.as_ref().is_some_and(Content::moves)
            || matches!(
                &self.trans,
                Some(Trans { from: From::Content(Some(c)), .. }) if c.moves()
            )
    }
}

/// Renders changed screens whenever the show state changes; renders thumbnails when idle.
fn render_loop(mut renderer: Renderer, shared: &Shared, rx: Receiver<Cmd>) {
    let mut screens: HashMap<model::ScreenId, LiveScreen> = HashMap::new();
    // Slide pictures of stage layouts (rendered slides, by content).
    let pictures = Pictures::default();
    // Players of video elements on live slides.
    let mut element_videos = crate::element_video::ElementVideos::default();
    // Finished decodes wake this loop (live screens waiting for an image re-render, parked
    // thumbnails are retried).
    let wake = lock(&shared.wake).clone();
    renderer.decoder().set_notify(move || {
        let _ = wake.send(Cmd::ImagesReady);
    });
    let mut rendered_version = u64::MAX;
    let mut live_pending = true; // render the initial frames right away
    let mut interactive: VecDeque<Box<ThumbJob>> = VecDeque::new();
    let mut prefetch: VecDeque<Box<ThumbJob>> = VecDeque::new();
    // Thumbnails waiting for image decodes.
    let mut parked: Vec<Box<ThumbJob>> = Vec::new();
    let mut last_summary = Instant::now();
    let mut last_caches: Option<Instant> = None;
    // When live data on a screen (a clock, a running timer) shows a new value next.
    let mut data_deadline: Option<Instant> = None;
    // The renderer's device generation the screens were last drawn on.
    let mut device_generation = renderer.device_generation();

    'run: loop {
        let idle = !live_pending && interactive.is_empty() && prefetch.is_empty();
        let mut incoming = Vec::new();
        if idle {
            // Wake up now and then even without messages (a dropped Tick must not freeze
            // an animated pattern), and when live data changes.
            let wait = data_deadline
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::from_millis(500))
                .min(Duration::from_millis(500));
            match rx.recv_timeout(wait) {
                Ok(c) => incoming.push(c),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        // Coalesce a burst of commands into one render.
        while let Ok(c) = rx.try_recv() {
            incoming.push(c);
        }
        let mut images_ready = false;
        let mut ticked = false;
        for c in incoming {
            match c {
                Cmd::Wake => live_pending = true,
                Cmd::ImagesReady => images_ready = true,
                Cmd::Stop => break 'run,
                Cmd::Tick { screen, frame, at } => {
                    if let Some(ls) = screens.get_mut(&screen) {
                        ls.frame = ls.frame.max(frame);
                        ls.present = Some(at);
                        ticked = true;
                    }
                }
                Cmd::Thumbnail(j) if j.interactive => interactive.push_back(j),
                Cmd::Thumbnail(j) => prefetch.push_back(j),
            }
        }
        let mut retry_incomplete = false;
        if images_ready {
            retry_incomplete = screens.values().any(|s| s.incomplete);
            live_pending |= retry_incomplete;
            // Parked thumbnails go back to the front of their queues (oldest first).
            for j in parked.drain(..).rev() {
                if j.interactive {
                    interactive.push_front(j);
                } else {
                    prefetch.push_front(j);
                }
            }
        }
        live_pending |= ticked;
        let refresh = data_deadline.is_some_and(|d| Instant::now() >= d);
        live_pending |= refresh;
        if last_caches.is_none_or(|t| t.elapsed() >= CACHES_EVERY) {
            last_caches = Some(Instant::now());
            *lock(&shared.render_caches) = renderer.cache_sizes();
        }
        if last_summary.elapsed() >= SUMMARY_EVERY {
            last_summary = Instant::now();
            log_summary(shared);
        }
        // A lost GPU device (driver reset): go on with a new one and draw every screen again.
        // The device may also have been replaced by a screen's submit during the last pass
        // (E03-1): the frames other screens had in flight were dropped then, so the
        // generation, not this call's result, decides.
        if let Err(e) = renderer.recover() {
            tracing::error!("render: {e:#}");
        }
        if renderer.device_generation() != device_generation {
            device_generation = renderer.device_generation();
            screens.values_mut().for_each(|ls| ls.fingerprint = None);
            rendered_version = u64::MAX;
            live_pending = true;
        }
        if live_pending {
            live_pending = false;
            let version = shared.version();
            if version != rendered_version || retry_incomplete || ticked || refresh {
                let changed = version != rendered_version;
                rendered_version = version;
                data_deadline = render_live(
                    &mut renderer,
                    shared,
                    &mut screens,
                    &pictures,
                    &mut element_videos,
                    changed,
                    refresh,
                );
                // Replaced during the pass: draw again now, not at the next change.
                live_pending = renderer.device_generation() != device_generation;
            }
            continue;
        }
        if let Some(job) = interactive.pop_front().or_else(|| prefetch.pop_front()) {
            if job.reply.is_closed() {
                continue;
            }
            match render_thumbnail(&mut renderer, shared, &job) {
                Some(res) => {
                    merge_report(&mut renderer, shared, Vec::new());
                    let _ = job.reply.send(res);
                }
                None => parked.push(job),
            }
        }
    }
    // Pending thumbnail requests get a clear error instead of a dropped channel.
    for j in interactive.into_iter().chain(prefetch).chain(parked) {
        let _ = j
            .reply
            .send(Err(anyhow::anyhow!("the renderer is stopping")));
    }
    tracing::info!("render thread stopped");
}

/// How often (at most) the render thread publishes its caches' sizes (`/server/stats`).
const CACHES_EVERY: Duration = Duration::from_secs(2);

/// How often the render thread logs render times and video counters (`debug` level).
const SUMMARY_EVERY: Duration = Duration::from_secs(10);

/// Logs per-screen render times (p50 / p95 / max of the recent renders), late ticks and late
/// renders, and every live video's decoder counters, at `debug` level.
fn log_summary(shared: &Shared) {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }
    for rt in shared.screens().iter() {
        let st = lock(&rt.stats).clone();
        if let Some((p50, p95, max)) = st.percentiles() {
            tracing::debug!(
                screen = %rt.screen().name,
                renders = st.renders,
                p50 = format!("{p50:.2}"),
                p95 = format!("{p95:.2}"),
                max = format!("{max:.2}"),
                late_ticks = st.late_ticks,
                late_renders = st.late_renders,
                fps = format!("{:.2}", st.fps),
                "render summary"
            );
        }
    }
    let videos: Vec<Arc<LiveVideo>> = lock(&shared.videos).values().cloned().collect();
    for v in videos {
        let s = v.player.stats();
        tracing::debug!(
            video = %v.name,
            play = v.play,
            hw = s.hw,
            decoded = s.decoded,
            shown = s.shown,
            skipped = s.skipped,
            late = s.late,
            queued = s.queued,
            decode_ms = format!("{:.2}", s.decode_ms),
            errors = s.errors,
            "video summary"
        );
    }
}

/// The lines of a screen's identify label: name, then role, size and rate, then outputs.
pub fn identify_lines(screen: &Screen, outputs: &[Arc<OutputRuntime>]) -> Vec<String> {
    let role = match screen_role(screen) {
        protocol::ScreenRole::Stage => "Stage",
        protocol::ScreenRole::Audience => "Audience",
    };
    let fps = screen.frame_rate.as_f64();
    let fps = if (fps - fps.round()).abs() < 0.005 {
        format!("{}", fps.round())
    } else {
        format!("{fps:.2}")
    };
    let mut lines = vec![
        if screen.name.is_empty() {
            "(unnamed screen)".to_string()
        } else {
            screen.name.clone()
        },
        format!(
            "{role} · {}×{} @ {fps}",
            screen.size.width.round() as u32,
            screen.size.height.round() as u32
        ),
    ];
    let names: Vec<&str> = outputs.iter().map(|o| o.name.as_str()).collect();
    lines.push(if names.is_empty() {
        "no outputs".to_string()
    } else {
        names.join("  ·  ")
    });
    lines
}

/// Which slide a planned slide is, whatever its content: a new value of live data or an edit
/// keeps it (its clock and video element players carry on).
fn slide_ident(slide: &Slide, canvas: Size, layer: model::Layer) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (layer as u8).hash(&mut h);
    slide.id.0.hash(&mut h);
    canvas.width.to_bits().hash(&mut h);
    canvas.height.to_bits().hash(&mut h);
    h.finish()
}

/// Content identity of a slide (what the renderer caches it by).
fn slide_key(slide: &Slide, canvas: Size, layer: model::Layer) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (layer as u8).hash(&mut h);
    serde_json::to_vec(slide).unwrap_or_default().hash(&mut h);
    canvas.width.to_bits().hash(&mut h);
    canvas.height.to_bits().hash(&mut h);
    h.finish()
}

fn display_hash(d: &MediaDisplay, h: &mut impl Hasher) {
    serde_json::to_vec(d).unwrap_or_default().hash(h);
}

/// The plan's content per layer (owned), with its identity. Slides are only cloned when the
/// track does not show them already. Props come one per prop (by its UUID), plus an empty
/// entry for each prop track the plan no longer has (it dissolves out).
fn plan_contents(
    shared: &Shared,
    plan: &ScreenPlan<'_>,
    tracks: &[Track],
) -> Vec<(model::Layer, uuid::Uuid, Option<Content>, u64)> {
    let current = |layer: model::Layer, key: u64| -> Option<Content> {
        tracks
            .iter()
            .find(|t| t.layer == layer && t.key == key)
            .and_then(|t| t.current.clone())
    };
    let nil = uuid::Uuid::nil();
    let data = shared.data();
    let media_content = |m: &(PlannedMedia, MediaDisplay), layer: model::Layer| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (layer as u8).hash(&mut h);
        display_hash(&m.1, &mut h);
        match &m.0 {
            PlannedMedia::Image(p) => {
                p.hash(&mut h);
                Some((
                    Content::Image {
                        path: p.clone(),
                        display: m.1.clone(),
                    },
                    h.finish(),
                ))
            }
            PlannedMedia::Video(play) => {
                play.hash(&mut h);
                let video = shared.video(*play)?;
                Some((
                    Content::Video {
                        video,
                        display: m.1.clone(),
                    },
                    h.finish(),
                ))
            }
            PlannedMedia::Missing(_) => None,
        }
    };
    let mut out = Vec::new();
    // The live source of the video-input layer: its shared receiver (opened off this thread),
    // asked for pictures no larger than the largest screen. The key names the receiver too:
    // one the registry closed while the track kept it (nobody drew it: a test pattern or a
    // disabled screen was up) is asked for again, never drawn frozen.
    let live = plan.video_input.as_ref().map(|(source, display)| {
        let key = |stream: u64| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (model::Layer::VideoInput as u8).hash(&mut h);
            source.hash(&mut h);
            display_hash(display, &mut h);
            stream.hash(&mut h);
            h.finish()
        };
        let running = tracks
            .iter()
            .filter(|t| t.layer == model::Layer::VideoInput)
            .find_map(|t| match &t.current {
                Some(c @ Content::Live { rx, .. })
                    if !rx.is_stopped() && t.key == key(rx.stream) =>
                {
                    Some((c.clone(), t.key))
                }
                _ => None,
            });
        running.unwrap_or_else(|| {
            let max = data.screens.iter().fold((0u32, 0u32), |m, s| {
                (
                    m.0.max(s.size.width.round() as u32),
                    m.1.max(s.size.height.round() as u32),
                )
            });
            let rx = shared
                .live_sources
                .receiver(source, (max.0 > 0 && max.1 > 0).then_some(max));
            let k = key(rx.stream);
            (
                Content::Live {
                    rx,
                    display: display.clone(),
                },
                k,
            )
        })
    });
    out.push(match live {
        Some((c, k)) => (model::Layer::VideoInput, nil, Some(c), k),
        None => (model::Layer::VideoInput, nil, None, 0),
    });
    let media = plan
        .media
        .as_ref()
        .and_then(|m| media_content(m, model::Layer::Media));
    out.push(match media {
        Some((c, k)) => (model::Layer::Media, nil, Some(c), k),
        None => (model::Layer::Media, nil, None, 0),
    });
    let mut slide_layer: Option<(Option<Content>, u64)> =
        plan.foreground
            .as_ref()
            .map(|m| match media_content(m, model::Layer::Slide) {
                Some((c, k)) => (Some(c), k),
                None => (None, 0),
            });
    let mut announcements = None;
    for ps in &plan.slides {
        let key = slide_key(&ps.slide, ps.canvas, ps.layer);
        let mut content = current(ps.layer, key).or_else(|| {
            Some(Content::slide(
                Arc::new(ps.slide.clone().into_owned()),
                ps.canvas,
                ps.layer,
                &data,
            ))
        });
        if let Some(Content::Slide { build, .. }) = &mut content {
            *build = plan
                .builds
                .iter()
                .find(|(l, _)| *l == ps.layer)
                .map(|(_, b)| *b);
        }
        match ps.layer {
            model::Layer::Announcements => announcements = Some((content, key)),
            _ => {
                if slide_layer.is_none() {
                    slide_layer = Some((content, key));
                }
            }
        }
    }
    let (c, k) = slide_layer.unwrap_or((None, 0));
    out.push((model::Layer::Slide, nil, c, k));
    let (c, k) = announcements.unwrap_or((None, 0));
    out.push((model::Layer::Announcements, nil, c, k));
    let slide_content = |ps: &crate::show::PlannedSlide<'_>| {
        let key = slide_key(&ps.slide, ps.canvas, ps.layer);
        let content = current(ps.layer, key).or_else(|| {
            Some(Content::slide(
                Arc::new(ps.slide.clone().into_owned()),
                ps.canvas,
                ps.layer,
                &data,
            ))
        });
        (content, key)
    };
    for (slot, ps) in &plan.props {
        let (c, k) = slide_content(ps);
        out.push((model::Layer::Props, *slot, c, k));
    }
    for t in tracks.iter().filter(|t| t.layer == model::Layer::Props) {
        if !plan.props.iter().any(|(slot, _)| *slot == t.slot) {
            out.push((model::Layer::Props, t.slot, None, 0));
        }
    }
    let (c, k) = match &plan.message {
        Some(ps) => slide_content(ps),
        None => (None, 0),
    };
    out.push((model::Layer::Messages, nil, c, k));
    let (c, k) = match &plan.captions {
        Some(ps) => slide_content(ps),
        None => (None, 0),
    };
    out.push((model::Layer::Captions, nil, c, k));
    out
}

/// One side of a layer as drawn at an instant (owns what the borrowed render layers point
/// into: video frames, slides).
enum Side {
    Empty,
    /// A slide, the seconds it has been live when it has scrolling text or video elements,
    /// its video elements' frames (element, stream, frame), and the slide as its object builds
    /// draw it at this instant with that picture's fingerprint (`None`: no builds).
    Slide(
        Arc<Slide>,
        Size,
        Option<f64>,
        Vec<(model::ElementId, u64, Arc<media::VideoFrame>)>,
        Option<(Arc<Slide>, u64)>,
        /// Live-source fills' newest pictures (element, stream, frame, alpha).
        Vec<LiveFillFrame>,
    ),
    Image(PathBuf, MediaDisplay),
    Video {
        stream: u64,
        frame: Option<Arc<media::VideoFrame>>,
        display: MediaDisplay,
        opacity: f32,
        black: bool,
    },
    /// A live source's newest picture (stream, frame).
    Live {
        stream: u64,
        frame: Option<Arc<crate::live_sources::LiveFrame>>,
        display: MediaDisplay,
    },
    Snapshot(u64),
}

impl Side {
    fn of(c: Option<&Content>, at: Instant) -> Side {
        match c {
            None => Side::Empty,
            Some(Content::Slide {
                slide,
                canvas,
                scrolls,
                since,
                build,
                ..
            }) => Side::Slide(
                slide.clone(),
                *canvas,
                // Scrolling text and video elements run on the screen clock from when the
                // slide went live.
                scrolls
                    .then(|| since.map_or(0.0, |s| at.saturating_duration_since(s).as_secs_f64())),
                Vec::new(),
                build.map(|b| built(slide, *canvas, b, *since, at)),
                Vec::new(),
            ),
            Some(Content::Image { path, display }) => Side::Image(path.clone(), display.clone()),
            Some(Content::Live { rx, display }) => Side::Live {
                stream: rx.stream,
                frame: rx.frame(),
                display: display.clone(),
            },
            Some(Content::Video { video, display }) => {
                let v = video.view(at);
                Side::Video {
                    stream: video.player.id(),
                    frame: v.frame,
                    display: display.clone(),
                    opacity: v.opacity,
                    black: v.black,
                }
            }
        }
    }

    /// A slide side with its video elements' current frames (see [`crate::element_video`]).
    fn with_videos(
        mut self,
        c: Option<&Content>,
        data: &ShowData,
        players: &mut crate::element_video::ElementVideos,
        plays: &crate::element_video::ElementPlays,
        at: Instant,
    ) -> Side {
        if let (
            Side::Slide(s, _, t, frames, _, _),
            Some(Content::Slide {
                videos: true,
                ident,
                since,
                ..
            }),
        ) = (&mut self, c)
        {
            let now = Instant::now();
            // Players belong to the slide's run (it and when it went live), not to its
            // content: a new value of live data keeps them; going live again starts over.
            let run = {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                ident.hash(&mut h);
                since.hash(&mut h);
                h.finish()
            };
            for v in crate::element_video::video_elements(s, &data.content.assets) {
                let path = || data.content.asset_path(v.asset).ok();
                if let Some((stream, f)) =
                    players.frame(run, s.id, &v, plays, path, t.unwrap_or(0.0), at, now)
                {
                    frames.push((v.element, stream, f));
                }
            }
        }
        self
    }

    /// A slide side with its live-source fills' newest pictures (see
    /// [`crate::live_sources::live_fills`]): each source's shared receiver, asked for every
    /// frame (a map lookup; one the registry closed is opened again), no larger than `max`.
    fn with_live_fills(
        mut self,
        c: Option<&Content>,
        data: &ShowData,
        live: &crate::live_sources::LiveSources,
        max: Option<(u32, u32)>,
    ) -> Side {
        if let (Side::Slide(s, .., frames), Some(Content::Slide { lives: true, .. })) =
            (&mut self, c)
        {
            for f in crate::live_sources::live_fills(s, data) {
                let rx = live.receiver(&f.source, max);
                if let Some(frame) = rx.frame() {
                    frames.push((f.element, rx.stream, frame, f.alpha));
                }
            }
        }
        self
    }

    fn fingerprint(&self, h: &mut impl Hasher) {
        match self {
            Side::Empty => 0u8.hash(h),
            Side::Slide(s, c, t, v, b, l) => {
                1u8.hash(h);
                for (e, stream, f, _) in l {
                    e.hash(h);
                    stream.hash(h);
                    f.seq.hash(h);
                }
                (Arc::as_ptr(s) as usize).hash(h);
                b.as_ref().map(|(_, k)| *k).hash(h);
                c.width.to_bits().hash(h);
                t.map(f64::to_bits).hash(h);
                for (e, stream, f) in v {
                    e.hash(h);
                    stream.hash(h);
                    f.seq.hash(h);
                }
            }
            Side::Image(p, d) => {
                2u8.hash(h);
                p.hash(h);
                display_hash(d, h);
            }
            Side::Video {
                stream,
                frame,
                display,
                opacity,
                black,
            } => {
                3u8.hash(h);
                stream.hash(h);
                frame.as_ref().map(|f| f.seq).hash(h);
                display_hash(display, h);
                opacity.to_bits().hash(h);
                black.hash(h);
            }
            Side::Snapshot(k) => {
                4u8.hash(h);
                k.hash(h);
            }
            Side::Live {
                stream,
                frame,
                display,
            } => {
                5u8.hash(h);
                stream.hash(h);
                frame.as_ref().map(|f| f.seq).hash(h);
                display_hash(display, h);
            }
        }
    }

    /// The render source and its opacity (`None` = draws nothing).
    fn source<'a>(&'a self, resolver: &'a dyn MediaResolver) -> Option<(LayerSource<'a>, f32)> {
        match self {
            Side::Empty | Side::Snapshot(_) => None,
            Side::Slide(s, c, t, v, b, l) => Some((
                LayerSource::Slide {
                    slide: b.as_ref().map_or(s, |(built, _)| built),
                    canvas: *c,
                    media: Some(resolver),
                    time: *t,
                    videos: v
                        .iter()
                        .map(|(e, stream, f)| (*e, video_ref(*stream, f)))
                        .chain(
                            l.iter()
                                .map(|(e, stream, f, a)| (*e, f.video_ref_as(*stream, *a))),
                        )
                        .collect(),
                },
                1.0,
            )),
            Side::Image(p, d) => Some((
                LayerSource::Media {
                    image: ImageSource::Path(p.clone()),
                    display: d.clone(),
                },
                1.0,
            )),
            Side::Video {
                stream,
                frame,
                display,
                opacity,
                ..
            } => {
                let f = frame.as_ref()?;
                Some((
                    LayerSource::Video {
                        frame: video_ref(*stream, f),
                        display: display.clone(),
                    },
                    *opacity,
                ))
            }
            Side::Live {
                stream,
                frame,
                display,
            } => {
                let f = frame.as_ref()?;
                Some((
                    LayerSource::Video {
                        frame: f.video_ref_as(*stream, display.alpha),
                        display: display.clone(),
                    },
                    1.0,
                ))
            }
        }
    }

    fn input<'a>(&'a self, resolver: &'a dyn MediaResolver) -> MixInput<'a> {
        if let Side::Snapshot(k) = self {
            return MixInput::Snapshot(*k);
        }
        match self.source(resolver) {
            Some((s, o)) => MixInput::Content(s, o),
            None => MixInput::Empty,
        }
    }
}

/// `slide` as its object builds draw it at `at` (step `b`, the slide live since `since`),
/// with a fingerprint of the phase (a settled build always gives the same one).
fn built(
    slide: &Arc<Slide>,
    canvas: Size,
    b: crate::show::BuildPlay,
    since: Option<Instant>,
    at: Instant,
) -> (Arc<Slide>, u64) {
    let secs = |s: Option<Instant>| s.map(|s| at.saturating_duration_since(s).as_secs_f64());
    let length = model::build::step_length(slide, b.step);
    // Past the step's end it is settled: the same picture (and fingerprint) every frame.
    let moving = |t: Option<f64>| t.filter(|t| *t < length);
    let (t0, t) = if b.step == 0 {
        (moving(secs(since)), None)
    } else {
        (None, moving(secs(b.at)))
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.step.hash(&mut h);
    t0.map(f64::to_bits).hash(&mut h);
    t.map(f64::to_bits).hash(&mut h);
    let s = match model::build::frame(slide, canvas, b.step, t0, t) {
        std::borrow::Cow::Owned(s) => Arc::new(s),
        std::borrow::Cow::Borrowed(_) => slide.clone(),
    };
    (s, h.finish())
}

/// A live-source fill's newest picture: element, render stream, frame, alpha reading.
type LiveFillFrame = (
    model::ElementId,
    u64,
    Arc<crate::live_sources::LiveFrame>,
    model::SourceAlpha,
);

/// The largest screen (live sources are asked for pictures no larger), `None` without one.
fn largest_screen(data: &ShowData) -> Option<(u32, u32)> {
    let max = data.screens.iter().fold((0u32, 0u32), |m, s| {
        (
            m.0.max(s.size.width.round() as u32),
            m.1.max(s.size.height.round() as u32),
        )
    });
    (max.0 > 0 && max.1 > 0).then_some(max)
}

/// A decoded frame as the renderer takes it.
fn video_ref(stream: u64, f: &media::VideoFrame) -> render::VideoFrameRef<'_> {
    let (y, ys) = f.plane(0);
    let (uv, uvs) = f.plane(1);
    render::VideoFrameRef {
        stream,
        seq: f.seq,
        width: f.width,
        height: f.height,
        planes: match f.format {
            media::PlaneFormat::Nv12 => render::VideoPlanes::Nv12,
            media::PlaneFormat::P010 => render::VideoPlanes::P010,
        },
        y,
        y_stride: ys,
        uv,
        uv_stride: uvs,
        matrix: match f.matrix {
            media::Matrix::Bt601 => render::VideoMatrix::Bt601,
            media::Matrix::Bt709 => render::VideoMatrix::Bt709,
            media::Matrix::Bt2020 => render::VideoMatrix::Bt2020,
        },
        full_range: f.full_range,
        rotation: f.rotation,
    }
}

/// A layer as drawn at an instant: its sides and transition (outgoing side, eased progress,
/// store key, style).
struct LayerView {
    layer: model::Layer,
    to: Side,
    from: Option<(Side, f32, u64, model::TransitionKind)>,
}

/// Renders the live screens that need it: on a show change (`changed`) every screen whose
/// picture changed; on a clock tick the screens that asked for that frame (videos,
/// transitions, animated patterns); and screens whose last render left out an image still
/// decoding. A screen is only submitted when its picture fingerprint changed.
///
/// **Layers and transitions.** Each screen keeps a [`Track`] per layer (media, slide,
/// announcements). When a layer's content changes, the track starts the transition of the
/// show's latest [`crate::show::LayerChange`] for that layer, timed from the change, so every
/// screen runs it in step. A look change ([`ShowState::look_change`]) plays the look's
/// transition on the layers it changes. Content that changes otherwise (an edit of the live
/// slide, a reload) cuts. The outgoing content stays live (a video keeps playing); a change during a
/// transition starts from the blend last shown (the renderer's mix store), so nothing jumps.
/// Fresh screens and screens coming back from blank cut.
///
/// **Images never hold up a live change:** text goes live right away; a background image that
/// is still decoding keeps the previous content up (the transition starts once it is decoded).
/// A media fill inside a slide that is still decoding is left out until it is ready.
///
/// **Videos:** a screen showing a video (or a transition) renders every tick of its clock, at
/// the tick's presentation time: the frame whose timestamp is nearest to the video's clock
/// then (25 fps media on a 30 fps screen repeats one frame in six; 60 fps media shows every
/// other frame). Late decodes repeat the previous frame instead of waiting.
///
/// **Live data** (clocks, timers, media countdowns; see [`render::live`]) is resolved into the
/// planned slides with one [`render::live::DataContext`] per pass. `refresh` re-plans every
/// screen for a new value without a show change; only screens whose picture changed render
/// (a clock showing minutes renders once a minute). Returns when a screen's live data next
/// shows a new value (`None` = nothing live), so the loop wakes up then.
fn render_live(
    renderer: &mut Renderer,
    shared: &Shared,
    screens: &mut HashMap<model::ScreenId, LiveScreen>,
    pictures: &Pictures,
    element_videos: &mut crate::element_video::ElementVideos,
    changed: bool,
    refresh: bool,
) -> Option<Instant> {
    let data = shared.data();
    let resolver = |id: model::MediaAssetId| {
        pictures
            .image(id)
            .or_else(|| data.content.asset_path(id).ok().map(ImageSource::Path))
    };
    let state: ShowState = shared.state();
    let runtimes = shared.screens();
    let now = Instant::now();
    let clock = crate::timers::Clock {
        now,
        wall: crate::timers::local_seconds(),
    };
    // Players of video elements no slide drew lately close.
    element_videos.prune(now);
    let live_max = largest_screen(&data);
    let replan = changed || refresh;
    let ctx = if replan {
        Some(live_context(shared, &data, &state, clock))
    } else {
        None
    };
    // Render targets follow the running screens and their size / alpha / background.
    screens.retain(|id, _| runtimes.iter().any(|r| r.id == *id));
    let configs: Vec<Screen> = runtimes.iter().map(|r| r.screen()).collect();
    for (rt, s) in runtimes.iter().zip(&configs) {
        let config = ScreenConfig::from(s);
        let ls = screens.entry(rt.id).or_insert_with(|| LiveScreen {
            target: renderer.create_screen(config),
            config,
            fingerprint: None,
            incomplete: false,
            frame: 0,
            present: None,
            tracks: Vec::new(),
            planned: false,
            live: render::live::LiveUse::default(),
            look_seq: state.look_change.as_ref().map_or(0, |c| c.seq),
        });
        if ls.config != config {
            ls.target = renderer.create_screen(config);
            ls.config = config;
            ls.fingerprint = None;
        }
        // Colour correction and corner pin change in place (same target, next frame).
        if ls.target.set_adjustments(&s.color, &s.corner_pin) {
            ls.fingerprint = None;
        }
    }
    let mut notes: Vec<(&str, String)> = Vec::new();
    let mut submitted: Vec<(usize, Instant, Option<Instant>)> = Vec::new();
    for (i, screen) in configs.iter().enumerate() {
        let rt = &runtimes[i];
        let ls = screens.get_mut(&rt.id).expect("live screen");
        let present = ls.present.take();
        if !replan && present.is_none() && !ls.incomplete {
            continue;
        }
        // A data refresh only concerns screens showing live data.
        if refresh && !changed && present.is_none() && !ls.incomplete && !ls.live.any() {
            continue;
        }
        let at = present.unwrap_or(now);
        let size = (ls.config.width, ls.config.height);
        let pattern = state.test_patterns.get(&rt.id);
        let label = state
            .identify
            .then(|| identify_lines(screen, &rt.outputs()));
        let blank: bool;
        let mut pictures_waiting = false;
        if replan {
            // A signage channel's screen draws its channel, never the show.
            let mut plan = match &screen.kind {
                model::ScreenKind::Signage { channel } => shared.signage.plan(channel),
                _ => plan_screen(&data, &state, screen, ctx.as_ref()),
            };
            if !plan.pictures.is_empty() {
                pictures_waiting = draw_pictures(renderer, &data, &mut plan, size, pictures);
            }
            let stage = plan.stage;
            ls.live = plan.live;
            blank = plan.blank;
            if let Some((PlannedMedia::Missing(why), _)) = &plan.media {
                notes.push(("media", why.clone()));
            }
            notes.extend(plan.notes.iter().map(|n| ("note", n.clone())));
            if plan.blank {
                ls.tracks.clear();
                ls.planned = false;
            } else {
                let contents = plan_contents(shared, &plan, &ls.tracks);
                let own_changes = plan.changes.clone();
                let cut = !ls.planned;
                ls.planned = true;
                // A new look change: layers it changes play the look's transition.
                let look_change = state
                    .look_change
                    .as_ref()
                    .filter(|c| c.seq != ls.look_seq && !stage && own_changes.is_none());
                if let Some(c) = &state.look_change {
                    ls.look_seq = c.seq;
                }
                for (layer, slot, content, key) in contents {
                    if !ls.tracks.iter().any(|t| t.layer == layer && t.slot == slot) {
                        ls.tracks.push(Track::new(layer, slot));
                    }
                    let t = ls
                        .tracks
                        .iter_mut()
                        .find(|t| t.layer == layer && t.slot == slot)
                        .expect("track");
                    // A new change of the layer plays its transition; content that changed
                    // otherwise (an edit of the live slide, a reload, a look, a new value
                    // of live data) cuts. Each prop has its own change record.
                    let change = if slot.is_nil() {
                        own_changes.as_ref().unwrap_or(&state.changes).get(&layer)
                    } else {
                        state.slot_changes.get(&slot)
                    };
                    // A new change of the layer or a new look puts it live again (its clock
                    // restarts); other new content keeps the clock of the same slide. A new
                    // change restarts it even when the content is the same (the same slide
                    // triggered again); a look only where it changes what is drawn.
                    let again = change.is_some_and(|c| c.seq != t.change_seq);
                    let restart = again || look_change.is_some();
                    // Stage screens always cut. Content changed by a new look plays the
                    // look's transition (the same engine path as a layer change).
                    let (transition, start) = match (change, look_change) {
                        (Some(c), _) if c.seq != t.change_seq && !stage => {
                            (c.transition.clone(), c.at)
                        }
                        (_, Some(l)) => (l.transition.clone(), l.at),
                        _ => (model::Transition::CUT, now),
                    };
                    t.update(content, key, &transition, start, now, cut, restart, again);
                    if let Some(c) = change {
                        t.change_seq = c.seq;
                    }
                }
            }
        } else {
            blank = !matches!(screen.kind, model::ScreenKind::Signage { .. })
                && !state.screens_enabled.get(screen_role(screen));
        }

        // Incoming content waiting for an image decode: start its transition once ready.
        let mut waiting = pictures_waiting;
        for t in ls.tracks.iter_mut() {
            let ready = match (&t.trans, &t.current) {
                (Some(tr), Some(c)) if tr.start.is_none() => {
                    let side = Side::of(Some(c), at);
                    match side.source(&resolver) {
                        Some((src, _)) => {
                            let lc = LayerContent {
                                layer: t.layer,
                                source: src,
                                opacity: 1.0,
                            };
                            renderer.request_images(&[lc], size, DecodePriority::Live) == 0
                        }
                        None => true,
                    }
                }
                _ => true,
            };
            if let Some(tr) = &mut t.trans {
                if tr.start.is_none() && ready {
                    tr.start = Some(now);
                }
                if tr.start.is_none() {
                    waiting = true;
                }
            }
        }
        // Newly decided transitions whose incoming images are not decoded yet wait.
        for t in ls.tracks.iter_mut() {
            if let (Some(tr), Some(c)) = (&mut t.trans, &t.current)
                && !tr.rendered
                && tr.start.is_some()
                && matches!(c, Content::Image { .. } | Content::Slide { .. })
            {
                let side = Side::of(Some(c), at);
                if let Some((src, _)) = side.source(&resolver) {
                    let lc = LayerContent {
                        layer: t.layer,
                        source: src,
                        opacity: 1.0,
                    };
                    if renderer.request_images(&[lc], size, DecodePriority::Live) > 0
                        && matches!(c, Content::Image { .. })
                    {
                        tr.start = None;
                        waiting = true;
                    }
                }
            }
        }
        // Finished transitions end (the outgoing content is released); a hidden prop's track
        // goes once it has dissolved out.
        for t in ls.tracks.iter_mut() {
            if t.progress(at).is_some_and(|p| p >= 1.0) {
                t.trans = None;
            }
        }
        ls.tracks
            .retain(|t| t.slot.is_nil() || t.current.is_some() || t.trans.is_some());

        let animated_pattern = pattern.is_some_and(render::patterns::is_animated);
        let animated = animated_pattern
            || (pattern.is_none()
                && !blank
                && ls.tracks.iter().any(|t| t.animated() || t.building(at)));
        rt.animated.store(animated, Ordering::Relaxed);

        // What each layer shows at `at`.
        let views: Vec<LayerView> = if pattern.is_some() || blank {
            Vec::new()
        } else {
            ls.tracks
                .iter()
                .map(|t| {
                    let to = Side::of(t.current.as_ref(), at)
                        .with_videos(
                            t.current.as_ref(),
                            &data,
                            element_videos,
                            &shared.element_plays,
                            at,
                        )
                        .with_live_fills(t.current.as_ref(), &data, &shared.live_sources, live_max);
                    let from = t.trans.as_ref().map(|tr| {
                        let side = match &tr.from {
                            From::Content(c) => Side::of(c.as_ref(), at)
                                .with_videos(
                                    c.as_ref(),
                                    &data,
                                    element_videos,
                                    &shared.element_plays,
                                    at,
                                )
                                .with_live_fills(c.as_ref(), &data, &shared.live_sources, live_max),
                            From::Snapshot(k) => Side::Snapshot(*k),
                        };
                        let linear = t.progress(at).unwrap_or(0.0).clamp(0.0, 1.0);
                        (
                            side,
                            tr.easing.apply(f64::from(linear)) as f32,
                            tr.store,
                            tr.kind.clone(),
                        )
                    });
                    LayerView {
                        layer: t.layer,
                        to,
                        from,
                    }
                })
                .collect()
        };

        // The screen's mask (the live look's; a stage screen's own), over every layer.
        let mask = if pattern.is_none() && !blank {
            mask_picture(
                renderer,
                &data,
                &state,
                screen,
                size,
                pictures,
                &mut waiting,
            )
        } else {
            None
        };

        // Fingerprint: the picture.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        mask.as_ref().map(|(k, _, _)| *k).hash(&mut h);
        match pattern {
            Some(p) => {
                serde_json::to_vec(p).unwrap_or_default().hash(&mut h);
                screen.name.hash(&mut h);
                screen.frame_rate.hash(&mut h);
                if animated_pattern {
                    ls.frame.hash(&mut h);
                }
            }
            None => {
                blank.hash(&mut h);
                data.content_generation.hash(&mut h);
                for v in &views {
                    (v.layer as u8).hash(&mut h);
                    v.to.fingerprint(&mut h);
                    if let Some((f, p, k, _)) = &v.from {
                        f.fingerprint(&mut h);
                        p.to_bits().hash(&mut h);
                        k.hash(&mut h);
                    }
                }
            }
        }
        label.hash(&mut h);
        let fingerprint = h.finish();
        if ls.fingerprint == Some(fingerprint) && !ls.incomplete {
            continue;
        }

        let overlay = label
            .as_ref()
            .map(|l| render::patterns::identify_overlay(l, size));
        let pattern_scene = pattern.map(|p| {
            render::patterns::test_pattern(
                p,
                size,
                &render::patterns::PatternInfo {
                    name: screen.name.clone(),
                    fps: screen.frame_rate.as_f64(),
                    frame: ls.frame,
                },
            )
        });
        let mut layers: Vec<LayerContent<'_>> = Vec::new();
        if let Some(ps) = &pattern_scene {
            layers.extend(ps.layers());
        } else if blank {
            if !matches!(
                screen.alpha,
                model::AlphaMode::Straight | model::AlphaMode::Premultiplied
            ) {
                // Switched off: black over whatever background colour the screen has.
                // Screens with alpha draw nothing (transparent).
                layers.push(LayerContent::color(
                    model::Layer::Media,
                    model::Color::BLACK,
                ));
            }
        } else {
            for v in &views {
                let black_under = matches!(v.to, Side::Video { black: true, .. });
                match &v.from {
                    Some((from, p, store, kind)) => {
                        layers.push(LayerContent::mix(
                            v.layer,
                            LayerMix {
                                from: from.input(&resolver),
                                to: v.to.input(&resolver),
                                progress: *p,
                                store: *store,
                                kind: kind.clone(),
                            },
                        ));
                    }
                    None => {
                        if black_under {
                            layers.push(LayerContent::color(v.layer, model::Color::BLACK));
                        }
                        if let Some((src, o)) = v.to.source(&resolver) {
                            layers.push(LayerContent {
                                layer: v.layer,
                                source: src,
                                opacity: o,
                            });
                        }
                    }
                }
            }
        }
        if let Some((_, img, erase)) = &mask {
            if let Some(e) = erase {
                // The mask is drawn last: labelled with the top-most layer (captions), it
                // stays after every layer's content when the renderer sorts them.
                layers.push(LayerContent {
                    layer: model::Layer::Captions,
                    source: render::LayerSource::Erase { image: e.clone() },
                    opacity: 1.0,
                });
            }
            layers.push(LayerContent {
                layer: model::Layer::Captions,
                source: render::LayerSource::Media {
                    image: img.clone(),
                    display: MediaDisplay::default(),
                },
                opacity: 1.0,
            });
        }
        if let Some(o) = &overlay {
            layers.extend(o.layers());
        }
        let t = Instant::now();
        match renderer.submit(&mut ls.target, &layers) {
            Ok(_) => {
                submitted.push((i, t, present));
                ls.fingerprint = Some(fingerprint);
                ls.incomplete = waiting || renderer.missing_images() > 0;
                for tr in ls.tracks.iter_mut().filter_map(|t| t.trans.as_mut()) {
                    if tr.start.is_some() {
                        tr.rendered = true;
                    }
                }
            }
            Err(e) => {
                tracing::error!(screen = %screen.name, "render failed: {e:#}");
                notes.push(("note", format!("render failed: {e:#}")));
            }
        }
    }
    let any = !submitted.is_empty();
    for (i, t, present) in submitted {
        let rt = &runtimes[i];
        let Some(ls) = screens.get_mut(&rt.id) else {
            continue;
        };
        match renderer.wait(&mut ls.target) {
            Ok(Some(f)) => {
                let done = Instant::now();
                let ms = (done - t).as_secs_f64() * 1000.0;
                *lock(&rt.latest) = Some(Arc::new(f));
                let mut st = lock(&rt.stats);
                st.last_render_ms = Some(ms);
                st.max_render_ms = st.max_render_ms.max(ms);
                st.total_render_ms += ms;
                st.renders += 1;
                st.record(ms);
                if present.is_some_and(|p| done > p) {
                    st.late_renders += 1;
                }
                if ls.incomplete {
                    st.incomplete_renders += 1;
                }
                tracing::trace!(screen = %configs[i].name, ms, incomplete = ls.incomplete, "rendered");
            }
            Ok(None) => {}
            Err(e) => tracing::error!("readback failed: {e:#}"),
        }
    }
    if changed {
        prefetch_next(renderer, &data, &state, &configs, screens);
    }
    merge_report(renderer, shared, notes);
    if any && changed {
        shared.status_changed.notify_one();
    }
    pictures.prune();
    next_live_change(&data, &state, screens)
}

/// Slide pictures of stage layouts: rendered slides kept as in-memory images under made-up
/// media asset IDs that the live screens' media resolver answers (see [`draw_pictures`]).
#[derive(Default)]
struct Pictures {
    inner: std::cell::RefCell<PicturesInner>,
    /// Finished mask pictures ([`mask_picture`]) by content key (mask, screen size), with the
    /// erase picture of a see-through keyhole, oldest
    /// first.
    masks: std::cell::RefCell<VecDeque<(u64, ImageSource, Option<ImageSource>)>>,
}

/// Mask pictures kept (a few screens, each with its current and previous mask).
const MAX_MASKS: usize = 8;

/// The mask drawn over `screen`: an audience screen's from the live look, a stage screen's its
/// own (`Screen::mask`). Its content key, finished picture ([`render::mask::finish_mask`]) and,
/// for a keyhole with a see-through colour, the picture that erases the screen first
/// ([`render::mask::keyhole_erase`]); rendered at the screen's size and cached. `None` when
/// the screen has no mask; a mask whose images are still decoding is drawn without them and not
/// cached (`waiting` is set, so the screen renders again).
fn mask_picture(
    renderer: &mut Renderer,
    data: &ShowData,
    state: &ShowState,
    screen: &Screen,
    size: (u32, u32),
    pictures: &Pictures,
    waiting: &mut bool,
) -> Option<(u64, ImageSource, Option<ImageSource>)> {
    let id = match screen.kind {
        model::ScreenKind::Audience => {
            let look = state
                .look
                .and_then(|id| data.looks.iter().find(|l| l.id == id))?;
            look.for_screen(screen.id).mask?
        }
        model::ScreenKind::Stage { .. } | model::ScreenKind::Signage { .. } => screen.mask?,
        model::ScreenKind::Unknown => return None,
    };
    let mask = data.masks.iter().find(|m| m.id == id)?;
    let mut hk = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(mask).unwrap_or_default().hash(&mut hk);
    size.hash(&mut hk);
    data.content_generation.hash(&mut hk);
    let key = hk.finish();
    if let Some((_, img, erase)) = pictures.masks.borrow().iter().find(|(k, _, _)| *k == key) {
        return Some((key, img.clone(), erase.clone()));
    }
    let resolver =
        |id: model::MediaAssetId| data.content.asset_path(id).ok().map(ImageSource::Path);
    let layers = [LayerContent::slide_with_media(
        &mask.slide,
        mask.size,
        &resolver,
    )];
    let pending = renderer.request_images(&layers, size, DecodePriority::Live) > 0;
    let frame = match renderer.render_layers(ScreenConfig::transparent(size.0, size.1), &layers) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(mask = %mask.name, "mask render failed: {e:#}");
            return None;
        }
    };
    let complete = !pending && renderer.missing_images() == 0;
    let mut img = frame.to_straight_rgba();
    let scale = size.0 as f64 / mask.size.width.max(1.0);
    let erase = (mask.mode == model::MaskMode::Keyhole)
        .then(|| render::mask::keyhole_erase(&img, mask.color, (mask.feather * scale) as f32))
        .flatten()
        .map(|e| ImageSource::Rgba {
            key: STORE_KEY.fetch_add(1, Ordering::Relaxed) ^ 0x4552_4153_0000_0000,
            image: Arc::new(e),
        });
    render::mask::finish_mask(
        &mut img,
        mask.mode == model::MaskMode::Keyhole,
        mask.color,
        (mask.feather * scale) as f32,
    );
    let n = STORE_KEY.fetch_add(1, Ordering::Relaxed);
    let src = ImageSource::Rgba {
        key: n ^ 0x4d41_534b_0000_0000,
        image: Arc::new(img),
    };
    if complete {
        let mut m = pictures.masks.borrow_mut();
        m.push_back((key, src.clone(), erase.clone()));
        while m.len() > MAX_MASKS {
            m.pop_front();
        }
    } else {
        *waiting = true;
    }
    Some((key, src, erase))
}

#[derive(Default)]
struct PicturesInner {
    /// Content key (slide, canvas, size) → its picture's asset ID (complete renders only).
    by_key: HashMap<u64, model::MediaAssetId>,
    /// Asset ID → image, in insertion order (oldest first).
    images: VecDeque<(model::MediaAssetId, ImageSource)>,
}

/// Pictures kept (a few per stage screen: current and next, recent slides).
const MAX_PICTURES: usize = 24;

impl Pictures {
    fn image(&self, id: model::MediaAssetId) -> Option<ImageSource> {
        let p = self.inner.borrow();
        p.images
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, img)| img.clone())
    }

    fn get(&self, key: u64) -> Option<model::MediaAssetId> {
        self.inner.borrow().by_key.get(&key).copied()
    }

    /// Keeps a new picture; `complete` ones are reused for the same key.
    fn insert(&self, key: u64, image: image::RgbaImage, complete: bool) -> model::MediaAssetId {
        let n = STORE_KEY.fetch_add(1, Ordering::Relaxed);
        let id = model::MediaAssetId::from(uuid::Uuid::from_u64_pair(0x5354_4147_4549_4d47, n));
        let mut p = self.inner.borrow_mut();
        p.images.push_back((
            id,
            ImageSource::Rgba {
                key: n ^ 0x5354_4147_0000_0000,
                image: Arc::new(image),
            },
        ));
        if complete {
            p.by_key.insert(key, id);
        }
        id
    }

    /// Drops the oldest pictures beyond [`MAX_PICTURES`].
    fn prune(&self) {
        let mut p = self.inner.borrow_mut();
        while p.images.len() > MAX_PICTURES {
            if let Some((id, _)) = p.images.pop_front() {
                p.by_key.retain(|_, v| *v != id);
            }
        }
    }
}

/// Draws the slide pictures of a stage plan (see [`crate::stage`]): each picture element of
/// the layout is filled with its slide (slide layer only, on black, with live data) rendered
/// at the element's size on this screen, fitted to the slide's aspect. The element's own text
/// is not drawn. Returns true when a picture left out an image that is still decoding (the
/// screen renders again once it is decoded).
fn draw_pictures(
    renderer: &mut Renderer,
    data: &ShowData,
    plan: &mut ScreenPlan<'_>,
    screen: (u32, u32),
    pictures: &Pictures,
) -> bool {
    let crate::show::ScreenPlan {
        slides,
        pictures: wanted,
        ..
    } = plan;
    let Some(layout) = slides.first_mut() else {
        return false;
    };
    let canvas = layout.canvas;
    let scale =
        (screen.0 as f64 / canvas.width.max(1.0)).min(screen.1 as f64 / canvas.height.max(1.0));
    let resolver =
        |id: model::MediaAssetId| data.content.asset_path(id).ok().map(ImageSource::Path);
    let mut incomplete = false;
    let slide = layout.slide.to_mut();
    for (element, source) in wanted.iter() {
        let Some(e) = slide.elements.iter_mut().find(|e| e.id == *element) else {
            continue;
        };
        if let Some(tb) = e.text_box_mut() {
            render::live::replace_text(tb, "");
        }
        let Some((src, src_canvas)) = source else {
            e.fill.enabled = false;
            continue;
        };
        let (bw, bh) = (e.bounds.width * scale, e.bounds.height * scale);
        if bw < 1.0 || bh < 1.0 {
            continue;
        }
        let aspect = src_canvas.width.max(1.0) / src_canvas.height.max(1.0);
        let (w, h) = if bw / bh > aspect {
            (bh * aspect, bh)
        } else {
            (bw, bw / aspect)
        };
        let (w, h) = (
            (w.round() as u32).clamp(16, 3840),
            (h.round() as u32).clamp(16, 2160),
        );
        let mut hk = std::collections::hash_map::DefaultHasher::new();
        slide_key(src, *src_canvas, model::Layer::Slide).hash(&mut hk);
        (w, h).hash(&mut hk);
        let key = hk.finish();
        let id = match pictures.get(key) {
            Some(id) => id,
            None => {
                let layers = [LayerContent::slide_with_media(src, *src_canvas, &resolver)];
                let pending = renderer.request_images(&layers, (w, h), DecodePriority::Live) > 0;
                match renderer.render_layers(ScreenConfig::opaque(w, h), &layers) {
                    Ok(f) => {
                        let complete = !pending && renderer.missing_images() == 0;
                        incomplete |= !complete;
                        pictures.insert(key, f.to_straight_rgba(), complete)
                    }
                    Err(err) => {
                        tracing::error!("slide picture failed: {err:#}");
                        continue;
                    }
                }
            }
        };
        e.fill = model::Fill::enabled(model::FillKind::Media {
            asset: id,
            display: MediaDisplay::default(),
            playback: Default::default(),
            audio: Default::default(),
        });
    }
    incomplete
}

/// The data context of one render pass: timers, clock, slide texts, messages (see
/// [`crate::live::data_context`]) plus the remaining time and states of the live videos and
/// audio (the video element sounds use it too: [`crate::element_video`]).
pub(crate) fn live_context(
    shared: &Shared,
    data: &ShowData,
    state: &ShowState,
    clock: crate::timers::Clock,
) -> render::live::DataContext {
    let mut ctx = crate::live::data_context(data, state, clock);
    let remaining = |play: Option<u64>| -> Option<f64> {
        let v = shared.video(play?)?;
        Some((v.duration()? - v.position_at(clock.now)).max(0.0))
    };
    ctx.video_remaining = remaining(state.slide_media.as_ref().and_then(|m| m.play))
        .or_else(|| remaining(state.media.as_ref().and_then(|m| m.play)));
    ctx.audio_remaining = state
        .audio
        .as_ref()
        .and_then(|m| m.play)
        .and_then(|p| shared.live_audio(p))
        .and_then(|a| Some((a.region_len()? - a.position_at(clock.now)).max(0.0)));
    // Media states for the countdown conditions.
    let playing = |s: protocol::PlaybackState| s == protocol::PlaybackState::Playing;
    let status = |play: Option<u64>| -> Option<render::live::MediaStatus> {
        let v = shared.video(play?)?;
        let state = v.state_at(clock.now);
        Some(render::live::MediaStatus {
            remaining: if state == protocol::PlaybackState::Stopped {
                0.0
            } else {
                v.duration()
                    .map_or(f64::INFINITY, |d| (d - v.position_at(clock.now)).max(0.0))
            },
            running: playing(state),
            looping: v.looping(),
        })
    };
    ctx.video = status(state.slide_media.as_ref().and_then(|m| m.play))
        .or_else(|| status(state.media.as_ref().and_then(|m| m.play)));
    ctx.audio = state
        .audio
        .as_ref()
        .and_then(|m| m.play)
        .and_then(|p| shared.live_audio(p))
        .map(|a| {
            let state = a.state_at(clock.now);
            render::live::MediaStatus {
                remaining: if state == protocol::PlaybackState::Stopped {
                    0.0
                } else {
                    a.region_len()
                        .map_or(f64::INFINITY, |l| (l - a.position_at(clock.now)).max(0.0))
                },
                running: playing(state),
                looping: a.looping(),
            }
        });
    ctx
}

/// When a screen's live data next shows a new value: the next shown second of a running
/// timer (screens showing timers), the next wall-clock second (screens showing a clock; a
/// clock without seconds simply renders nothing new), a second from now for media countdowns.
/// A few milliseconds late, so the new value is certain.
fn next_live_change(
    data: &ShowData,
    state: &ShowState,
    screens: &HashMap<model::ScreenId, LiveScreen>,
) -> Option<Instant> {
    let mut used = render::live::LiveUse::default();
    for ls in screens.values() {
        used.merge(ls.live);
    }
    if !used.any() {
        return None;
    }
    let now = Instant::now();
    let clock = crate::timers::Clock {
        now,
        wall: crate::timers::local_seconds(),
    };
    let mut next: Option<Instant> = None;
    let mut at = |i: Instant| next = Some(next.map_or(i, |n: Instant| n.min(i)));
    if used.timers
        && let Some(t) = state.timers.next_change(&data.content.timers, clock)
    {
        at(t);
    }
    if used.clock {
        let frac = clock.wall - clock.wall.floor();
        at(now + Duration::from_secs_f64((1.0 - frac).clamp(0.0, 1.0)));
    }
    if used.media {
        at(now + Duration::from_secs(1));
    }
    if used.flash
        && let Some(t) = state
            .stage_message
            .as_ref()
            .and_then(|m| m.next_flash_change(now))
    {
        at(t);
    }
    next.map(|n| n + Duration::from_millis(3))
}

/// Queues decodes (prefetch priority) of the images the next slide (`next` from the current
/// position) would show on each screen, so stepping through a song with backgrounds finds
/// them decoded.
fn prefetch_next(
    renderer: &Renderer,
    data: &ShowData,
    state: &ShowState,
    configs: &[Screen],
    screens: &HashMap<model::ScreenId, LiveScreen>,
) {
    let mut next = state.clone();
    next.probe = true;
    if !matches!(next.step(data, 1), Ok(true)) {
        return;
    }
    let resolver =
        |id: model::MediaAssetId| data.content.asset_path(id).ok().map(ImageSource::Path);
    for s in configs {
        let Some(ls) = screens.get(&s.id) else {
            continue;
        };
        let plan = plan_screen(data, &next, s, None);
        let (media, mut layers) = plan_layers(&plan, &resolver);
        layers.extend(media);
        renderer.request_images(
            &layers,
            (ls.config.width, ls.config.height),
            DecodePriority::Prefetch,
        );
    }
}

/// Renders a thumbnail, or `None` when one of its images is still decoding (the decode is
/// queued at thumbnail priority; retry after the decoder's next notification).
fn render_thumbnail(
    renderer: &mut Renderer,
    shared: &Shared,
    job: &ThumbJob,
) -> Option<anyhow::Result<RenderedFrame>> {
    let data = shared.data();
    let resolver =
        |id: model::MediaAssetId| data.content.asset_path(id).ok().map(ImageSource::Path);
    let mut layers = Vec::new();
    if let Some((path, display)) = &job.background {
        layers.push(LayerContent::media(
            ImageSource::Path(path.clone()),
            display.clone(),
        ));
    }
    layers.push(LayerContent::slide_with_media(
        &job.slide, job.canvas, &resolver,
    ));
    let size = (job.width, job.height);
    if renderer.request_images(&layers, size, DecodePriority::Thumbnail) > 0 {
        return None;
    }
    let config = if job.transparent {
        ScreenConfig::transparent(job.width, job.height)
    } else {
        ScreenConfig::opaque(job.width, job.height)
    };
    let res = renderer.render_layers(config, &layers);
    // An image evicted between the check and the render: wait for it again.
    if res.is_ok() && renderer.missing_images() > 0 {
        return None;
    }
    Some(res)
}

/// Merges the renderer's report into the diagnostics and announces new issues.
fn merge_report(renderer: &mut Renderer, shared: &Shared, notes: Vec<(&str, String)>) {
    let report = renderer.take_report();
    let new = {
        let mut d = lock(&shared.diagnostics);
        d.merge(&report);
        for (kind, n) in notes {
            d.note(kind, &n);
        }
        d.take_new()
    };
    if !new.is_empty() {
        for (level, msg, key) in new {
            shared.notice_keyed(level, msg, Some(key));
        }
        shared.status_changed.notify_one();
    }
}

/// The image layers of a screen plan (for prefetching): the background image (when it is
/// one) and the slides.
fn plan_layers<'a>(
    plan: &'a ScreenPlan<'_>,
    resolver: &'a dyn MediaResolver,
) -> (Option<LayerContent<'a>>, Vec<LayerContent<'a>>) {
    let media = match &plan.media {
        Some((PlannedMedia::Image(p), display)) => Some(LayerContent::media(
            ImageSource::Path(p.clone()),
            display.clone(),
        )),
        _ => None,
    };
    let slides = plan
        .slides
        .iter()
        .map(|ps| LayerContent::slide_with_media(&ps.slide, ps.canvas, resolver).on_layer(ps.layer))
        .collect();
    (media, slides)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E03-4: two clocks at one rate started at different moments (the fill's and the key's
    /// own output clocks) tick at the same instants, so their frames carry equal timecodes.
    #[test]
    fn clocks_at_one_rate_tick_together() {
        let period = period_of(model::FrameRate { num: 60, den: 1 });
        let a = Instant::now();
        let b = a + Duration::from_micros(4_321);
        let (mut ta, mut tb) = (on_grid(a, period), on_grid(b, period));
        assert!(ta >= a && ta - a < period && tb >= b && tb - b < period);
        // Advance the earlier one to the later one's first tick: same instants from then on.
        while ta < tb {
            ta += period;
        }
        for _ in 0..3 {
            assert_eq!(audio::timecode(ta), audio::timecode(tb));
            ta += period;
            tb += period;
        }
        // A late clock skips to the next tick of the same grid.
        assert_eq!(on_grid(ta + period / 3, period), ta + period);
        assert_eq!(on_grid(ta, period), ta);
    }
}
