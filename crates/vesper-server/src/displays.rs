//! Physical displays: the monitors and projectors attached to this box, and the output sinks
//! that show a screen on one of them (`docs/DISPLAY-OUTPUTS-PLAN.md`).
//!
//! A [`DisplaySource`] lists the displays and opens a sink on one:
//! - [`Kms`] (Linux): lists the DRM connectors from sysfs (status, EDID) and their modes with
//!   the DRM ioctls, and shows frames with kernel mode setting (dumb buffers, page flips at
//!   vertical blank) with no compositor. The server must be the card's DRM master (no desktop
//!   running on that card). The frames are the screen's CPU frames (the same ones the IDN
//!   outputs send), so presenting never touches the shared GPU device.
//! - [`WindowDisplays`] (Windows, Linux with a desktop session): a borderless window covering
//!   the monitor, or a plain window for rehearsal, presented from the renderer's shared GPU
//!   device (see the `window` module).
//! - [`FakeDisplays`]: a settable list whose sinks count frames (tests, demos).
//! - [`NoDisplays`]: display outputs turned off (`--displays none`), or another platform.
//!
//! [`platform_source`] picks one: windows when the server runs in a desktop session (always on
//! Windows, where in session 0, as the headless startup task, the window backend lists no
//! displays and reports why), KMS when it doesn't (a Linux display box).
//!
//! [`Displays`] keeps the current list for the API and the state; [`watch`] re-lists every
//! [`POLL`] and rebuilds the display outputs whose display was plugged in or out.
//!
//! **Fill/Key.** The fill and key outputs of a screen (`target` `fill` / `key`) are two
//! display outputs; their frames are made by the output worker ([`crate::outputs`]). A source
//! is told which half of which screen's pair it opens ([`Pairing`]): on KMS the two sinks on
//! one card flip together ([`FlipPair`]); windows show the halves in two windows.

use model::{DisplayMode, DisplaySelector, OutputBinding, OutputTarget, OutputTransport};
use output::{Frame, FrameSink};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the display list is refreshed (hot-plug).
pub const POLL: Duration = Duration::from_secs(2);

/// One display output of the box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    /// Connector name (`"HDMI-A-1"`).
    pub connector: String,
    /// Position in the list.
    pub index: u32,
    /// EDID model name (empty = unknown).
    pub model: String,
    /// EDID manufacturer code.
    pub manufacturer: Option<String>,
    /// A monitor is plugged in.
    pub connected: bool,
    /// Its modes, largest first.
    pub modes: Vec<DisplayMode>,
    /// Its preferred mode.
    pub preferred: Option<DisplayMode>,
    /// Backend data: the card's primary node.
    pub card: std::path::PathBuf,
    /// Backend data: the DRM connector ID.
    pub connector_id: u32,
    /// Top left corner on the desktop, px (windows only).
    pub position: Option<(i32, i32)>,
    /// The desktop's main monitor (windows only; the operator's screen on a one-PC setup).
    pub primary: bool,
}

impl Display {
    /// The name shown to operators: the model, else the connector.
    pub fn label(&self) -> &str {
        if self.model.is_empty() {
            &self.connector
        } else {
            &self.model
        }
    }

    /// The mode an output asks for, if the display offers it; else one of the same size
    /// (closest refresh); else the preferred mode.
    pub fn resolve_mode(&self, wanted: Option<DisplayMode>) -> Option<DisplayMode> {
        if let Some(w) = wanted {
            if self.modes.contains(&w) {
                return Some(w);
            }
            if let Some(m) = self
                .modes
                .iter()
                .filter(|m| (m.width, m.height) == (w.width, w.height))
                .min_by_key(|m| m.refresh_mhz.abs_diff(w.refresh_mhz))
            {
                return Some(*m);
            }
        }
        self.preferred.or_else(|| self.modes.first().copied())
    }
}

/// The half of a screen's Fill/Key pair a display output shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    /// The screen the pair belongs to (its ID).
    pub screen: String,
    /// Which part of the screen the pair shows: empty for the whole screen, else its tile
    /// (`tile 2`) or region (`region x,y,w,h`). Each tile of a tiled screen has a pair of its
    /// own (F03-2).
    pub part: String,
    /// The key (else the fill).
    pub key: bool,
}

impl Pairing {
    /// The pairing of a binding (`None` for a `full` output).
    pub fn of(b: &OutputBinding) -> Option<Pairing> {
        match b.target {
            OutputTarget::Fill | OutputTarget::Key => Some(Pairing {
                screen: b.screen.0.to_string(),
                part: match (&b.region, b.tile) {
                    (Some(r), _) => format!("region {},{},{},{}", r.x, r.y, r.width, r.height),
                    (None, Some(t)) => format!("tile {t}"),
                    (None, None) => String::new(),
                },
                key: b.target == OutputTarget::Key,
            }),
            _ => None,
        }
    }

    /// `"fill"` or `"key"`.
    pub fn role(&self) -> &'static str {
        if self.key { "key" } else { "fill" }
    }
}

/// Lists displays and opens sinks on them.
pub trait DisplaySource: Send + Sync + std::fmt::Debug {
    /// Short name of the backend (`kms`, `window`, `fake`, `none`).
    fn backend(&self) -> &'static str;
    /// Why this backend can't show anything here, if it can't (`window` without a desktop).
    fn problem(&self) -> Option<String> {
        None
    }
    /// The displays now (connected or not).
    fn list(&self) -> Vec<Display>;
    /// A sink that shows frames on `display` in `mode`, borderless full screen or (for
    /// rehearsal, windows only) in a plain window; `pair`: the half of a Fill/Key pair it
    /// shows, if it does.
    fn open(
        &self,
        display: &Display,
        mode: DisplayMode,
        fullscreen: bool,
        pair: Option<&Pairing>,
    ) -> anyhow::Result<Box<dyn FrameSink>>;
}

/// Which display source a server uses (`--displays`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    /// Windows in a desktop session (always on Windows), else KMS (Linux).
    #[default]
    Auto,
    /// Direct KMS (Linux, no desktop on the card).
    Kms,
    /// Windows on the desktop.
    Window,
    /// No display outputs.
    None,
}

/// Why windows opened by a process in Windows session `id` would not be seen, if they would
/// not: session 0 holds services and startup tasks (the installer's default SYSTEM task),
/// and no one signs in to it, so its windows go to a desktop no monitor shows.
pub fn session_problem(id: Option<u32>) -> Option<String> {
    (id == Some(0)).then(|| {
        "the server runs as a startup task or service (Windows session 0), whose windows no \
         one sees; for screens on this PC's monitors, run the installer again with \
         \"Show screens on this PC's monitors\""
            .to_string()
    })
}

/// The Windows session this process runs in (`None` if Windows doesn't say).
#[cfg(windows)]
pub fn windows_session() -> Option<u32> {
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    let mut id = 0u32;
    // SAFETY: plain calls with a valid out pointer.
    let ok = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut id) };
    (ok != 0).then_some(id)
}

/// Whether this process runs in a desktop session it can open windows in (on Windows, the
/// window backend itself reports session 0: see [`session_problem`]).
pub fn desktop_session() -> bool {
    cfg!(windows)
        || ["WAYLAND_DISPLAY", "DISPLAY"]
            .iter()
            .any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()))
}

/// The display source for `choice` on this platform.
pub fn source(choice: Choice) -> Arc<dyn DisplaySource> {
    match choice {
        Choice::Auto if desktop_session() => source(Choice::Window),
        Choice::Auto => source(Choice::Kms),
        #[cfg(target_os = "linux")]
        Choice::Kms if std::path::Path::new(kms::SYSFS).is_dir() => Arc::new(Kms::default()),
        #[cfg(any(windows, target_os = "linux"))]
        Choice::Window => Arc::new(WindowDisplays),
        _ => Arc::new(NoDisplays),
    }
}

/// The display source of this platform ([`Choice::Auto`]).
pub fn platform_source() -> Arc<dyn DisplaySource> {
    source(Choice::Auto)
}

/// No display outputs: turned off, or not available on this platform.
#[derive(Debug)]
pub struct NoDisplays;

impl DisplaySource for NoDisplays {
    fn backend(&self) -> &'static str {
        "none"
    }
    fn list(&self) -> Vec<Display> {
        Vec::new()
    }
    fn open(
        &self,
        _: &Display,
        _: DisplayMode,
        _: bool,
        _: Option<&Pairing>,
    ) -> anyhow::Result<Box<dyn FrameSink>> {
        anyhow::bail!("display outputs are turned off on this server")
    }
}

/// What a fake display received.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FakeShown {
    /// Frames shown.
    pub frames: u64,
    /// Size of the last frame.
    pub last_size: (u32, u32),
    /// Mode it was opened in.
    pub mode: Option<DisplayMode>,
    /// Sinks opened on it so far.
    pub opened: u32,
    /// The last sink was full screen (not a window).
    pub fullscreen: Option<bool>,
    /// The half of a Fill/Key pair the last sink was opened for.
    pub pair: Option<Pairing>,
    /// The centre pixel of the last frame (premultiplied RGBA).
    pub last_centre: [u8; 4],
    /// The alpha type of the last frame.
    pub last_alpha: Option<model::AlphaMode>,
    /// The last frame's pixels (premultiplied RGBA8).
    pub last_pixels: Vec<u8>,
}

/// A settable list of displays whose sinks count frames.
#[derive(Debug, Default)]
pub struct FakeDisplays {
    list: Mutex<Vec<Display>>,
    shown: Arc<Mutex<std::collections::HashMap<String, FakeShown>>>,
    /// How many sinks may be open at once (a card's display controllers); `None`: any.
    controllers: Mutex<Option<usize>>,
    /// Sinks open now.
    open_now: Arc<std::sync::atomic::AtomicUsize>,
}

impl FakeDisplays {
    /// A fake display with the usual HD modes.
    pub fn display(connector: &str, model: &str, connected: bool) -> Display {
        let m = |width, height, refresh_mhz| DisplayMode {
            width,
            height,
            refresh_mhz,
        };
        let modes = if connected {
            vec![
                m(1920, 1080, 60000),
                m(1920, 1080, 50000),
                m(1280, 720, 60000),
            ]
        } else {
            Vec::new()
        };
        Display {
            connector: connector.into(),
            index: 0,
            model: model.into(),
            manufacturer: connected.then(|| "XYZ".into()),
            connected,
            preferred: modes.first().copied(),
            modes,
            card: "/dev/null".into(),
            connector_id: 0,
            position: None,
            primary: false,
        }
    }

    /// Replaces the list (indexes are renumbered).
    pub fn set(&self, mut displays: Vec<Display>) {
        for (i, d) in displays.iter_mut().enumerate() {
            d.index = i as u32;
        }
        *crate::lock(&self.list) = displays;
    }

    /// Lets only `n` sinks be open at once, as a card with `n` display controllers (an open
    /// beyond them fails with "no free display controller").
    pub fn set_controllers(&self, n: Option<usize>) {
        *crate::lock(&self.controllers) = n;
    }

    /// What `connector` has shown.
    pub fn shown(&self, connector: &str) -> FakeShown {
        crate::lock(&self.shown)
            .get(connector)
            .cloned()
            .unwrap_or_default()
    }
}

struct FakeSink {
    name: String,
    connector: String,
    shown: Arc<Mutex<std::collections::HashMap<String, FakeShown>>>,
    open_now: Arc<std::sync::atomic::AtomicUsize>,
}

impl Drop for FakeSink {
    fn drop(&mut self) {
        self.open_now
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

impl FrameSink for FakeSink {
    fn name(&self) -> &str {
        &self.name
    }
    fn send(&mut self, frame: &Frame<'_>) -> anyhow::Result<()> {
        frame.validate()?;
        let mut s = crate::lock(&self.shown);
        let e = s.entry(self.connector.clone()).or_default();
        e.frames += 1;
        e.last_size = (frame.width, frame.height);
        let i = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
        e.last_centre.copy_from_slice(&frame.pixels[i..i + 4]);
        e.last_alpha = Some(frame.alpha);
        e.last_pixels.clear();
        e.last_pixels.extend_from_slice(frame.pixels);
        Ok(())
    }
}

impl DisplaySource for FakeDisplays {
    fn backend(&self) -> &'static str {
        "fake"
    }
    fn list(&self) -> Vec<Display> {
        crate::lock(&self.list).clone()
    }
    fn open(
        &self,
        display: &Display,
        mode: DisplayMode,
        fullscreen: bool,
        pair: Option<&Pairing>,
    ) -> anyhow::Result<Box<dyn FrameSink>> {
        let limit = *crate::lock(&self.controllers);
        let n = self
            .open_now
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if limit.is_some_and(|l| n >= l) {
            self.open_now
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            anyhow::bail!("no free display controller for {}", display.connector);
        }
        let mut s = crate::lock(&self.shown);
        let e = s.entry(display.connector.clone()).or_default();
        e.mode = Some(mode);
        e.opened += 1;
        e.fullscreen = Some(fullscreen);
        e.pair = pair.cloned();
        Ok(Box::new(FakeSink {
            name: format!("Display: {}", display.connector),
            connector: display.connector.clone(),
            shown: self.shown.clone(),
            open_now: self.open_now.clone(),
        }))
    }
}

/// The server's displays: the source and its current list.
pub struct Displays {
    source: Arc<dyn DisplaySource>,
    list: Mutex<Vec<Display>>,
}

impl Displays {
    /// Lists the displays of `source` once.
    pub fn new(source: Arc<dyn DisplaySource>) -> Displays {
        let list = source.list();
        Displays {
            source,
            list: Mutex::new(list),
        }
    }

    /// The backend's name.
    pub fn backend(&self) -> &'static str {
        self.source.backend()
    }

    /// Why the backend can't show anything here, if it can't.
    pub fn problem(&self) -> Option<String> {
        self.source.problem()
    }

    /// The current list.
    pub fn list(&self) -> Vec<Display> {
        crate::lock(&self.list).clone()
    }

    /// Lists again; true when anything changed.
    pub fn refresh(&self) -> bool {
        let new = self.source.list();
        let mut cur = crate::lock(&self.list);
        if *cur == new {
            return false;
        }
        *cur = new;
        true
    }

    /// The display a selector names: by connector, else by model name (connected first), else
    /// by index.
    pub fn find(&self, sel: &DisplaySelector) -> Option<Display> {
        find(&self.list(), sel).cloned()
    }

    /// A short signature of what a display output was built against (display, connection,
    /// modes): when it changes, the output is rebuilt.
    pub fn signature(&self, b: &OutputBinding) -> Option<String> {
        let OutputTransport::Display { selector, .. } = &b.transport else {
            return None;
        };
        Some(match self.find(selector) {
            Some(d) => format!(
                "{}|{}|{:?}|{:?}",
                d.connector, d.connected, d.modes, d.position
            ),
            None => "-".into(),
        })
    }

    /// The sink of a display binding. An output wins over an identify label on the same
    /// display: the label is taken down first (F03-1).
    pub fn open(&self, b: &OutputBinding) -> anyhow::Result<Box<dyn FrameSink>> {
        self.open_sink(b, true)
    }

    fn open_sink(&self, b: &OutputBinding, output: bool) -> anyhow::Result<Box<dyn FrameSink>> {
        let OutputTransport::Display {
            selector,
            mode,
            fullscreen,
        } = &b.transport
        else {
            anyhow::bail!("not a display output");
        };
        let d = self
            .find(selector)
            .ok_or_else(|| anyhow::anyhow!("display {} not found", selector_label(selector)))?;
        anyhow::ensure!(d.connected, "{} is not connected", d.connector);
        let m = d
            .resolve_mode(*mode)
            .ok_or_else(|| anyhow::anyhow!("{} reports no modes", d.connector))?;
        if mode.is_some_and(|w| w != m) {
            tracing::warn!(display = %d.connector, "the chosen mode is not offered; using {}", mode_label(m));
        }
        if output {
            end_identify(&d.connector);
        }
        self.source
            .open(&d, m, *fullscreen, Pairing::of(b).as_ref())
    }

    /// The API / state list, with the bindings that use each display.
    pub fn infos(&self, bindings: &[OutputBinding]) -> Vec<protocol::DisplayInfo> {
        let list = self.list();
        list.iter()
            .map(|d| protocol::DisplayInfo {
                id: d.connector.clone(),
                index: d.index,
                name: d.label().to_string(),
                manufacturer: d.manufacturer.clone(),
                connected: d.connected,
                modes: d.modes.clone(),
                preferred: d.preferred,
                source: Some(self.backend().to_string()),
                position: d.position.map(|(x, y)| protocol::DisplayPosition { x, y }),
                primary: d.primary,
                bound_to: bindings
                    .iter()
                    .filter(|b| match &b.transport {
                        OutputTransport::Display { selector, .. } => {
                            find(&list, selector).is_some_and(|x| x.connector == d.connector)
                        }
                        _ => false,
                    })
                    .map(|b| b.id)
                    .collect(),
            })
            .collect()
    }
}

/// Two display IDs that may name one connector: equal, or the same connector where one of
/// them has no card (`HDMI-A-1`, saved on a box with one card, is found as `card1-HDMI-A-1`
/// once the box has several: see [`find`]). Two cards' connectors stay apart.
pub fn same_connector(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    let (ba, bb) = (bare_connector(a), bare_connector(b));
    (ba.len() == a.len() || bb.len() == b.len()) && ba.eq_ignore_ascii_case(bb)
}

/// Two display selectors name one display of `list`: equal connectors, or the same connector
/// where one has no card ([`same_connector`]) and both resolve ([`find`]) to the same
/// display. A bare `HDMI-A-1` on a box with two cards resolves to one of them, so the other
/// card's `HDMI-A-1` is a different display (E06-3); when either cannot be resolved, the
/// connector names decide.
pub fn same_display(list: &[Display], a: &DisplaySelector, b: &DisplaySelector) -> bool {
    match (&a.connector, &b.connector) {
        (Some(ca), Some(cb)) => {
            if ca.eq_ignore_ascii_case(cb) {
                return true;
            }
            if !same_connector(ca, cb) {
                return false;
            }
            match (find(list, a), find(list, b)) {
                (Some(da), Some(db)) => da.connector.eq_ignore_ascii_case(&db.connector),
                _ => true,
            }
        }
        _ => a == b,
    }
}

/// A KMS display ID without its card (`card1-HDMI-A-1` → `HDMI-A-1`).
fn bare_connector(id: &str) -> &str {
    match id.split_once('-') {
        Some((card, rest))
            if card.len() > 4
                && card.starts_with("card")
                && card[4..].bytes().all(|b| b.is_ascii_digit()) =>
        {
            rest
        }
        _ => id,
    }
}

fn find<'a>(list: &'a [Display], sel: &DisplaySelector) -> Option<&'a Display> {
    if let Some(c) = &sel.connector {
        if let Some(d) = list.iter().find(|d| d.connector.eq_ignore_ascii_case(c)) {
            return Some(d);
        }
        // Chosen when the box had connectors on one card (`HDMI-A-1`), now listed with their
        // card (`card0-HDMI-A-1`): the one display of that name, else the one with the chosen
        // model, else the only connected one.
        let same: Vec<&Display> = list
            .iter()
            .filter(|d| bare_connector(&d.connector).eq_ignore_ascii_case(bare_connector(c)))
            .collect();
        let named = |d: &&&Display| {
            sel.name
                .as_deref()
                .is_some_and(|n| !n.is_empty() && d.model.eq_ignore_ascii_case(n))
        };
        let connected: Vec<&&Display> = same.iter().filter(|d| d.connected).collect();
        let pick = match same.as_slice() {
            [d] => Some(*d),
            _ => same
                .iter()
                .find(named)
                .copied()
                .or_else(|| (connected.len() == 1).then(|| *connected[0])),
        };
        if pick.is_some() {
            return pick;
        }
    }
    if let Some(n) = sel.name.as_deref().filter(|n| !n.is_empty()) {
        let mut named = list.iter().filter(|d| d.model.eq_ignore_ascii_case(n));
        let all: Vec<_> = named.by_ref().collect();
        if let Some(d) = all.iter().find(|d| d.connected).or(all.first()) {
            return Some(d);
        }
    }
    sel.index.and_then(|i| list.iter().find(|d| d.index == i))
}

/// "HDMI-A-1", "Example Monitor" or "#2".
pub fn selector_label(sel: &DisplaySelector) -> String {
    sel.connector
        .clone()
        .or_else(|| sel.name.clone())
        .or_else(|| sel.index.map(|i| format!("#{i}")))
        .unwrap_or_else(|| "(none chosen)".into())
}

/// "1920×1080 @ 59.94 Hz".
pub fn mode_label(m: DisplayMode) -> String {
    let hz = m.refresh_mhz as f64 / 1000.0;
    let hz = if m.refresh_mhz.is_multiple_of(1000) {
        format!("{hz:.0}")
    } else {
        format!("{hz:.2}")
    };
    format!("{}×{} @ {hz} Hz", m.width, m.height)
}

/// Where a `fw`×`fh` frame goes on a `dw`×`dh` display: centred, aspect ratio kept,
/// `(x, y, w, h)`.
pub fn letterbox(fw: u32, fh: u32, dw: u32, dh: u32) -> (u32, u32, u32, u32) {
    if fw == 0 || fh == 0 || dw == 0 || dh == 0 {
        return (0, 0, dw, dh);
    }
    let s = (dw as f64 / fw as f64).min(dh as f64 / fh as f64);
    let (w, h) = (
        ((fw as f64 * s).round() as u32).clamp(1, dw),
        ((fh as f64 * s).round() as u32).clamp(1, dh),
    );
    ((dw - w) / 2, (dh - h) / 2, w, h)
}

/// Copies premultiplied RGBA8 (`w`×`h`, tightly packed) into an XRGB8888 buffer (bytes
/// B, G, R, X; `pitch` bytes per row) at `(x, y)`. Premultiplied colour over black is the
/// colour itself, so alpha is dropped.
pub fn write_xrgb(dst: &mut [u8], pitch: usize, src: &[u8], (x, y, w, h): (u32, u32, u32, u32)) {
    let (x, w) = (x as usize, w as usize);
    for row in 0..h as usize {
        let s = &src[row * w * 4..(row + 1) * w * 4];
        let o = (y as usize + row) * pitch + x * 4;
        let Some(d) = dst.get_mut(o..o + w * 4) else {
            return;
        };
        for (d, s) in d
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(s.as_chunks::<4>().0)
        {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = 0xFF;
        }
    }
}

/// The rendezvous of a screen's fill and key sinks on one KMS card: each sink draws its
/// buffer, then the one that comes second flips both (fill first), back to back, so both
/// connectors take the frame of the same tick at their next vertical blank. A sink waits at
/// most `wait` for its partner, then flips alone: a stalled or missing partner never holds the
/// other up. `T` is what a flip needs (KMS: the CRTC and framebuffer). Two CRTCs still scan
/// out on their own timing (see "Fill/Key on two connectors" in the plan).
#[cfg(any(target_os = "linux", test))]
pub(crate) struct FlipPair<T> {
    state: Mutex<PairState<T>>,
    cv: std::sync::Condvar,
}

#[cfg(any(target_os = "linux", test))]
struct PairState<T> {
    /// A sink holds the fill (0) / key (1) role.
    members: [bool; 2],
    /// A drawn frame waiting for its partner: (tick, flip).
    waiting: [Option<(i64, T)>; 2],
    /// The partner flipped this role's frame of this tick: (tick, the flip's error).
    done: [Option<(i64, Option<String>)>; 2],
}

#[cfg(any(target_os = "linux", test))]
impl<T: Copy> FlipPair<T> {
    pub(crate) fn new() -> Self {
        FlipPair {
            state: Mutex::new(PairState {
                members: [false; 2],
                waiting: [None; 2],
                done: [None, None],
            }),
            cv: std::sync::Condvar::new(),
        }
    }

    /// Takes role `role` (0 fill, 1 key); false when another sink has it.
    pub(crate) fn claim(&self, role: usize) -> bool {
        let mut st = crate::lock(&self.state);
        !std::mem::replace(&mut st.members[role], true)
    }

    /// Gives role `role` up (its sink stops).
    pub(crate) fn release(&self, role: usize) {
        let mut st = crate::lock(&self.state);
        st.members[role] = false;
        st.waiting[role] = None;
        st.done[role] = None;
        self.cv.notify_all();
    }

    /// Flips `mine`, drawn for `tick`, together with the partner's flip of the same tick
    /// (`flip` is called for each, fill first, back to back). True when the two flipped
    /// together. The result is always that of this role's own flip (E03-2): when the sink
    /// that comes second flips both, the partner's result is handed to the partner, and one
    /// half's failure never skips the other half's flip.
    pub(crate) fn flip(
        &self,
        role: usize,
        tick: i64,
        mine: T,
        wait: Duration,
        flip: impl Fn(T) -> anyhow::Result<()>,
    ) -> anyhow::Result<bool> {
        let other = 1 - role;
        let mut st = crate::lock(&self.state);
        if !st.members[other] {
            drop(st);
            flip(mine)?;
            return Ok(false);
        }
        if let Some((t, theirs)) = st.waiting[other]
            && t == tick
        {
            st.waiting[other] = None;
            let (r_mine, r_theirs) = if role == 0 {
                let m = flip(mine);
                (m, flip(theirs))
            } else {
                let t = flip(theirs);
                (flip(mine), t)
            };
            st.done[other] = Some((tick, r_theirs.err().map(|e| format!("{e:#}"))));
            self.cv.notify_all();
            return r_mine.map(|()| true);
        }
        st.done[role] = None;
        st.waiting[role] = Some((tick, mine));
        let deadline = std::time::Instant::now() + wait;
        loop {
            if let Some((t, _)) = &st.done[role]
                && *t == tick
            {
                let err = st.done[role].take().and_then(|(_, e)| e);
                return match err {
                    None => Ok(true),
                    Some(e) => Err(anyhow::anyhow!(e)),
                };
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            st = self
                .cv
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        st.waiting[role] = None;
        drop(st);
        flip(mine)?;
        Ok(false)
    }
}

/// How long `POST /displays/{id}/identify` shows a display's label.
pub const IDENTIFY_DISPLAY_FOR: Duration = Duration::from_secs(5);

/// A display showing its identify label: its connector and the flag that takes the label
/// down early.
struct Identifying {
    connector: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

/// Displays showing their identify label now. Held while an identify checks that no output
/// uses its display and opens it, and by an output taking a label down, so the two never
/// open one display at once (F03-1).
static IDENTIFYING: Mutex<Vec<Identifying>> = Mutex::new(Vec::new());
/// Signalled when an identify label is taken down (its sink closed).
static IDENTIFY_ENDED: std::sync::Condvar = std::sync::Condvar::new();
/// How long an output waits for an identify label to be taken down.
const IDENTIFY_END_WAIT: Duration = Duration::from_secs(2);

/// Takes down the identify label on `connector`, if one shows, and waits (a moment) until
/// its sink is closed: an output opening the display wins over the label.
fn end_identify(connector: &str) {
    let mut busy = crate::lock(&IDENTIFYING);
    let Some(i) = busy.iter().find(|i| i.connector == connector) else {
        return;
    };
    i.stop.store(true, std::sync::atomic::Ordering::Release);
    tracing::info!(display = %connector, "an output takes the display: identify ended");
    let until = std::time::Instant::now() + IDENTIFY_END_WAIT;
    while busy.iter().any(|i| i.connector == connector) {
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            tracing::warn!(display = %connector, "the identify label did not close in time");
            return;
        }
        busy = IDENTIFY_ENDED
            .wait_timeout(busy, left)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

/// Whether an output of the current configuration uses the display `connector`.
fn used_by_output(shared: &crate::Shared, connector: &str) -> bool {
    shared
        .displays
        .infos(&shared.data().outputs)
        .iter()
        .any(|i| i.id == connector && !i.bound_to.is_empty())
}

/// Why a display can't show its identify label: it isn't there, isn't connected, an output
/// uses it (that output's screen is identified with `identifyScreens` instead), or it is
/// already showing it. `Ok` with the display otherwise.
pub fn identify_check(
    displays: &Displays,
    bindings: &[OutputBinding],
    id: &str,
) -> Result<Display, (protocol::ErrorCode, String)> {
    use protocol::ErrorCode;
    let list = displays.list();
    let d = list
        .iter()
        .find(|d| d.connector == id)
        .or_else(|| list.iter().find(|d| same_connector(&d.connector, id)))
        .cloned()
        .ok_or_else(|| (ErrorCode::NotFound, format!("no display {id}")))?;
    if !d.connected {
        return Err((
            ErrorCode::Conflict,
            format!("{} is not connected", d.connector),
        ));
    }
    let info = displays
        .infos(bindings)
        .into_iter()
        .find(|i| i.id == d.connector);
    if info.is_some_and(|i| !i.bound_to.is_empty()) {
        return Err((
            ErrorCode::Conflict,
            format!(
                "{} is used by an output: identify its screen instead",
                d.connector
            ),
        ));
    }
    if crate::lock(&IDENTIFYING)
        .iter()
        .any(|i| i.connector == d.connector)
    {
        return Err((
            ErrorCode::Conflict,
            format!("{} is already showing its name", d.connector),
        ));
    }
    Ok(d)
}

/// The identify label of a display no output uses: its name, then connector and mode.
pub fn identify_lines(d: &Display, mode: DisplayMode) -> Vec<String> {
    let mut second = d.connector.clone();
    second.push_str(" · ");
    second.push_str(&mode_label(mode));
    vec![
        d.label().to_string(),
        second,
        "Not used by any screen".into(),
    ]
}

/// Shows `frame` (the display's identify label, rendered at `mode`) full screen on `d` for
/// [`IDENTIFY_DISPLAY_FOR`] on a thread of its own, then closes the display again. Stops early
/// when an output starts using the display (an output opening it takes the label down first).
/// When the label is gone, display outputs that could not open meanwhile (the label held the
/// display's controller) are tried again (R1-2).
pub fn identify_show(
    shared: Arc<crate::Shared>,
    d: Display,
    mode: DisplayMode,
    frame: render::RenderedFrame,
) -> anyhow::Result<()> {
    let b = OutputBinding {
        name: format!("Identify {}", d.connector),
        transport: OutputTransport::Display {
            selector: DisplaySelector {
                connector: Some(d.connector.clone()),
                name: None,
                index: None,
            },
            fullscreen: true,
            mode: Some(mode),
        },
        ..OutputBinding::default()
    };
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut sink = {
        // Checked and opened under the lock an output takes to end a label: an output that
        // uses the display by now wins (F03-1).
        let mut busy = crate::lock(&IDENTIFYING);
        anyhow::ensure!(
            !busy.iter().any(|i| i.connector == d.connector),
            "{} is already showing its name",
            d.connector
        );
        anyhow::ensure!(
            !used_by_output(&shared, &d.connector),
            "{} is used by an output: identify its screen instead",
            d.connector
        );
        let sink = shared.displays.open_sink(&b, false)?;
        busy.push(Identifying {
            connector: d.connector.clone(),
            stop: stop.clone(),
        });
        sink
    };
    let release = |c: &str| {
        crate::lock(&IDENTIFYING).retain(|x| x.connector != c);
        IDENTIFY_ENDED.notify_all();
    };
    let connector = d.connector.clone();
    let sh = shared.clone();
    let spawned = std::thread::Builder::new()
        .name("identify disp".into())
        .spawn(move || {
            let until = std::time::Instant::now() + IDENTIFY_DISPLAY_FOR;
            let rate = model::FrameRate::fps(10);
            let stopped = || stop.load(std::sync::atomic::Ordering::Acquire);
            'show: while std::time::Instant::now() < until && !stopped() {
                if used_by_output(&sh, &connector) {
                    break;
                }
                let f = Frame {
                    width: frame.width,
                    height: frame.height,
                    pixels: &frame.data,
                    alpha: model::AlphaMode::Opaque,
                    frame_rate: rate,
                    timecode: 0,
                };
                if let Err(e) = sink.send(&f) {
                    tracing::warn!(display = %connector, "identify: {e:#}");
                    break;
                }
                // 100 ms between frames, watching for an output that takes the display.
                for _ in 0..5 {
                    if stopped() {
                        break 'show;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            drop(sink);
            crate::lock(&IDENTIFYING).retain(|x| x.connector != connector);
            IDENTIFY_ENDED.notify_all();
            // Outputs that failed to open while the label held the display's controller.
            crate::engine::retry_unavailable_displays(&sh);
        });
    if let Err(e) = spawned {
        release(&d.connector);
        return Err(e.into());
    }
    Ok(())
}

/// Refreshes the list every [`POLL`] until `stop`; on a change, the state is rebroadcast and
/// display outputs whose display came or went are rebuilt ([`crate::engine::apply_config`]).
pub fn watch(shared: Arc<crate::Shared>, mut stop: tokio::sync::watch::Receiver<bool>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = stop.wait_for(|s| *s) => break,
            }
            let sh = shared.clone();
            let changed = tokio::task::spawn_blocking(move || {
                let changed = sh.displays.refresh();
                if changed {
                    tracing::info!("displays changed");
                    crate::engine::apply_config(&sh, &sh.data());
                }
                changed
            })
            .await
            .unwrap_or(false);
            if changed {
                shared.status_changed.notify_one();
            }
        }
    });
}

#[cfg(target_os = "linux")]
pub use kms::Kms;

#[cfg(any(windows, target_os = "linux"))]
mod window;
#[cfg(any(windows, target_os = "linux"))]
pub use window::WindowDisplays;

#[cfg(target_os = "linux")]
mod kms {
    //! DRM connectors from sysfs and the DRM ioctls; scan-out from dumb buffers (KMS).
    use super::*;
    use std::path::{Path, PathBuf};

    pub(super) const SYSFS: &str = "/sys/class/drm";

    /// A connector's modes, each with "preferred".
    type Modes = Vec<(DisplayMode, bool)>;
    /// Which monitor's modes: (card, connector, EDID).
    type ModesKey = (PathBuf, String, Vec<u8>);

    /// Linux KMS displays.
    #[derive(Debug, Default)]
    pub struct Kms {
        /// Modes by (card, connector, EDID): read with an ioctl only when a monitor changes.
        modes: Mutex<std::collections::HashMap<ModesKey, Modes>>,
    }

    struct Connector {
        card: PathBuf,
        /// The sysfs name, `card1-HDMI-A-1`.
        sysfs: String,
        name: String,
        id: u32,
        connected: bool,
        edid: Vec<u8>,
    }

    fn connectors() -> Vec<Connector> {
        connectors_in(Path::new(SYSFS))
    }

    fn connectors_in(sysfs: &Path) -> Vec<Connector> {
        let Ok(rd) = std::fs::read_dir(sysfs) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let file = e.file_name().to_string_lossy().to_string();
            // cardN-<connector>; not renderD*, not the card itself, not writeback connectors.
            let Some((card, name)) = file.split_once('-') else {
                continue;
            };
            if !card.starts_with("card") || name.starts_with("Writeback") {
                continue;
            }
            let p = e.path();
            let read = |f: &str| std::fs::read_to_string(p.join(f)).unwrap_or_default();
            let Ok(id) = read("connector_id").trim().parse::<u32>() else {
                continue;
            };
            out.push(Connector {
                card: Path::new("/dev/dri").join(card),
                sysfs: file.clone(),
                name: name.to_string(),
                id,
                connected: read("status").trim() == "connected",
                edid: std::fs::read(p.join("edid")).unwrap_or_default(),
            });
        }
        // Card, then the kernel's connector order.
        out.sort_by(|a, b| (&a.card, a.id).cmp(&(&b.card, b.id)));
        out
    }

    /// The display IDs of `conns`: the connector name (`HDMI-A-1`), or with its card
    /// (`card1-HDMI-A-1`) on a box with connectors on more than one card, where DRM numbers
    /// each card's connectors from 1 and the bare names repeat.
    fn display_ids(conns: &[Connector]) -> Vec<String> {
        let several = conns.iter().any(|c| c.card != conns[0].card);
        conns
            .iter()
            .map(|c| if several { &c.sysfs } else { &c.name }.clone())
            .collect()
    }

    /// The mode's refresh in mHz exactly as Mesa's display WSI computes it.
    fn refresh_mhz(m: &drm::control::Mode) -> u32 {
        let (_, _, ht) = m.hsync();
        let (_, _, vt) = m.vsync();
        let scan = m.vscan().max(1) as f64;
        let hz = m.clock() as f64 * 1000.0 / (ht as f64 * vt as f64 * scan);
        (hz * 1000.0 + 0.5) as u32
    }

    struct Card(std::fs::File);
    impl std::os::fd::AsFd for Card {
        fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
            self.0.as_fd()
        }
    }
    impl drm::Device for Card {}
    impl drm::control::Device for Card {}

    fn read_modes(card: &Path, id: u32) -> anyhow::Result<Modes> {
        use drm::control::Device;
        let handle = drm::control::from_u32::<drm::control::connector::Handle>(id)
            .ok_or_else(|| anyhow::anyhow!("bad connector id"))?;
        // A full probe (it also refreshes the EDID in sysfs). Only the DRM master's probe
        // finds new modes after a hot-plug, so it goes through our master descriptor when
        // display outputs run; else through a short-lived one.
        // The registry stays locked while a short-lived descriptor is open, so it can't take
        // the master role from a display output starting at the same moment.
        let cards = crate::lock(&CARDS);
        let held = cards
            .iter()
            .filter_map(|w| w.upgrade())
            .find(|c| c.path == card);
        let info = match held {
            Some(m) => m.card.get_connector(handle, true)?,
            None => {
                let f = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(card)?;
                Card(f).get_connector(handle, true)?
            }
        };
        drop(cards);
        let mut modes: Vec<(DisplayMode, bool)> = Vec::new();
        for m in info.modes() {
            if m.flags().contains(drm::control::ModeFlags::INTERLACE) {
                continue;
            }
            let (w, h) = m.size();
            let dm = DisplayMode {
                width: w as u32,
                height: h as u32,
                refresh_mhz: refresh_mhz(m),
            };
            let pref = m
                .mode_type()
                .contains(drm::control::ModeTypeFlags::PREFERRED);
            match modes.iter_mut().find(|(x, _)| *x == dm) {
                Some(e) => e.1 |= pref,
                None => modes.push((dm, pref)),
            }
        }
        Ok(modes)
    }

    /// Manufacturer code and model name from an EDID block.
    pub(crate) fn edid_names(edid: &[u8]) -> (Option<String>, Option<String>) {
        if edid.len() < 128 || edid[0..8] != [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0] {
            return (None, None);
        }
        let v = u16::from_be_bytes([edid[8], edid[9]]);
        let letter = |s: u16| char::from(b'A' - 1 + ((v >> s) & 0x1F) as u8);
        let mfr: String = [letter(10), letter(5), letter(0)].into_iter().collect();
        let mfr = mfr.chars().all(|c| c.is_ascii_uppercase()).then_some(mfr);
        let mut model = None;
        for off in [54usize, 72, 90, 108] {
            let d = &edid[off..off + 18];
            if d[0] == 0 && d[1] == 0 && d[3] == 0xFC {
                let t: String = d[5..18]
                    .iter()
                    .take_while(|b| **b != b'\n')
                    .map(|b| *b as char)
                    .collect();
                let t = t.trim().to_string();
                if !t.is_empty() {
                    model = Some(t);
                }
            }
        }
        (mfr, model)
    }

    impl DisplaySource for Kms {
        fn backend(&self) -> &'static str {
            "kms"
        }

        fn list(&self) -> Vec<Display> {
            let mut out = Vec::new();
            let conns = connectors();
            let ids = display_ids(&conns);
            for (i, (c, id)) in conns.into_iter().zip(ids).enumerate() {
                let (manufacturer, model) = edid_names(&c.edid);
                let modes = if c.connected {
                    let key = (c.card.clone(), c.name.clone(), c.edid.clone());
                    // (Only connected monitors get here: a re-plug with the same EDID reuses
                    // the modes read before.)
                    let mut cache = crate::lock(&self.modes);
                    match cache.get(&key) {
                        Some(m) => m.clone(),
                        None => match read_modes(&c.card, c.id) {
                            Ok(m) => {
                                // An empty list (a monitor still waking up) is asked again.
                                if !m.is_empty() {
                                    cache.insert(key, m.clone());
                                }
                                m
                            }
                            Err(e) => {
                                tracing::warn!(display = %id, "cannot read the modes: {e}");
                                Vec::new()
                            }
                        },
                    }
                } else {
                    Vec::new()
                };
                let preferred = modes.iter().find(|(_, p)| *p).map(|(m, _)| *m);
                let mut list: Vec<DisplayMode> = modes.into_iter().map(|(m, _)| m).collect();
                list.sort_by(|a, b| {
                    (b.width * b.height, b.refresh_mhz).cmp(&(a.width * a.height, a.refresh_mhz))
                });
                out.push(Display {
                    connector: id,
                    index: i as u32,
                    model: model.unwrap_or_default(),
                    manufacturer,
                    connected: c.connected,
                    preferred: preferred.or_else(|| list.first().copied()),
                    modes: list,
                    card: c.card,
                    connector_id: c.id,
                    position: None,
                    primary: false,
                });
            }
            out
        }

        fn open(
            &self,
            d: &Display,
            mode: DisplayMode,
            _fullscreen: bool,
            pair: Option<&Pairing>,
        ) -> anyhow::Result<Box<dyn FrameSink>> {
            // KMS has no windows: always the whole display.
            let mut sink = KmsSink::open(d, mode)?;
            if let Some(p) = pair {
                sink.pairing = Some(p.clone());
                sink.pair = join_pair(&sink.card.path, p);
                if sink.pair.is_none() {
                    tracing::warn!(display = %d.connector, "another {} output of this screen is already paired; this one flips alone until that role is free", p.role());
                }
            }
            tracing::info!(display = %d.connector, role = pair.map_or("full", |p| p.role()), "driving the display at {}", mode_label(mode));
            Ok(Box::new(sink))
        }
    }

    /// The card's primary node opened by this process: its DRM master while any display
    /// output runs (closed, so the master is released, when the last one stops).
    struct MasterCard {
        card: Card,
        path: PathBuf,
        /// CRTCs our outputs use.
        crtcs: Mutex<std::collections::HashSet<u32>>,
    }

    static CARDS: Mutex<Vec<std::sync::Weak<MasterCard>>> = Mutex::new(Vec::new());

    /// What a KMS flip needs.
    type Flip = (
        drm::control::crtc::Handle,
        drm::control::framebuffer::Handle,
    );
    /// A sink's place in a Fill/Key flip pair: the pair and its role (0 fill, 1 key).
    type PairSlot = (Arc<FlipPair<Flip>>, usize);

    /// The flip pairs: (card, screen) → pair, while a sink uses it.
    #[allow(clippy::type_complexity)]
    static PAIRS: Mutex<Vec<(PathBuf, String, std::sync::Weak<FlipPair<Flip>>)>> =
        Mutex::new(Vec::new());

    /// Joins the flip pair of `p`'s screen on `card` in `p`'s role (`None`: that role is taken
    /// by another output of the screen, which then keeps the pair).
    /// A pair's identity on its card: the screen and the part of it.
    fn pair_key(p: &Pairing) -> String {
        format!("{} {}", p.screen, p.part)
    }

    fn join_pair(card: &Path, p: &Pairing) -> Option<PairSlot> {
        let mut pairs = crate::lock(&PAIRS);
        pairs.retain(|(_, _, w)| w.strong_count() > 0);
        let pair = match pairs
            .iter()
            .find(|(c, s, _)| c == card && *s == pair_key(p))
            .and_then(|(_, _, w)| w.upgrade())
        {
            Some(pair) => pair,
            None => {
                let pair = Arc::new(FlipPair::new());
                pairs.push((card.to_path_buf(), pair_key(p), Arc::downgrade(&pair)));
                pair
            }
        };
        let role = usize::from(p.key);
        pair.claim(role).then_some((pair, role))
    }

    /// Joins the pair now when `slot` is empty but the sink shows a half (`pairing`) whose role
    /// has been freed since (cheap: a lock and a short list).
    fn rejoin_pair(
        slot: &mut Option<PairSlot>,
        card: &Path,
        pairing: Option<&Pairing>,
        name: &str,
    ) {
        if slot.is_none()
            && let Some(p) = pairing
        {
            *slot = join_pair(card, p);
            if slot.is_some() {
                tracing::info!(display = %name, "joined the {} role of its screen's flip pair", p.role());
            }
        }
    }

    /// Page-flips `crtc` to `fb` at the next vertical blank; while a flip is pending on it,
    /// waits (up to ~100 ms) for it.
    /// The error names no display: the sink that owns the CRTC adds its own name (a pair's
    /// second sink flips its partner's CRTC too).
    fn page_flip(card: &MasterCard, (crtc, fb): Flip) -> anyhow::Result<()> {
        use drm::control::Device;
        for attempt in 0.. {
            match card
                .card
                .page_flip(crtc, fb, drm::control::PageFlipFlags::empty(), None)
            {
                Ok(()) => break,
                Err(e) if e.raw_os_error() == Some(libc::EBUSY) && attempt < 100 => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => anyhow::bail!("{e}"),
            }
        }
        Ok(())
    }

    fn master(path: &Path) -> anyhow::Result<Arc<MasterCard>> {
        let mut cards = crate::lock(&CARDS);
        cards.retain(|w| w.strong_count() > 0);
        if let Some(c) = cards
            .iter()
            .filter_map(|w| w.upgrade())
            .find(|c| c.path == path)
        {
            return Ok(c);
        }
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| anyhow::anyhow!("cannot open {}: {e}", path.display()))?;
        let card = Card(f);
        // Only the DRM master may authenticate a client: a cheap master check (the legacy
        // auth calls are deprecated for clients, which should use render nodes; we are the
        // master, not a client).
        #[allow(deprecated)]
        let is_master = {
            use drm::Device;
            card.generate_auth_token()
                .and_then(|t| card.authenticate_auth_token(t))
                .is_ok()
        };
        if !is_master {
            anyhow::bail!(
                "another program (a desktop session?) is using the displays of {}",
                path.display()
            );
        }
        let c = Arc::new(MasterCard {
            card,
            path: path.to_path_buf(),
            crtcs: Mutex::new(Default::default()),
        });
        cards.push(Arc::downgrade(&c));
        Ok(c)
    }

    /// Triple-buffered scan-out of CPU frames on one connector: each frame is converted
    /// into the dumb buffer that is neither on screen nor waiting to be (XRGB8888, letterboxed
    /// at the mode's size), then flipped to at the next vertical blank.
    struct KmsSink {
        name: String,
        card: Arc<MasterCard>,
        crtc: drm::control::crtc::Handle,
        size: (u32, u32),
        bufs: Vec<(
            drm::control::dumbbuffer::DumbBuffer,
            drm::control::framebuffer::Handle,
        )>,
        next: usize,
        /// Letterbox rectangle last drawn into each buffer (the bars stay black).
        drawn: Vec<Option<(u32, u32, u32, u32)>>,
        scaled: Vec<u8>,
        /// Its place in the screen's Fill/Key flip pair.
        pair: Option<PairSlot>,
        /// The half it shows; while `pair` is `None` (the role was taken when it opened), it
        /// tries to join again before every flip (E03-3: swapping fill and key edits one
        /// output at a time, so the first one rebuilt finds its new role still taken).
        pairing: Option<Pairing>,
        /// Flips made together with the partner, and alone, while paired.
        paired_flips: (u64, u64),
    }

    impl KmsSink {
        fn open(d: &Display, mode: DisplayMode) -> anyhow::Result<KmsSink> {
            use drm::control::Device;
            let card = master(&d.card)?;
            let c = &card.card;
            let handle = drm::control::from_u32::<drm::control::connector::Handle>(d.connector_id)
                .ok_or_else(|| anyhow::anyhow!("bad connector id"))?;
            let info = c.get_connector(handle, true)?;
            let drm_mode = *info
                .modes()
                .iter()
                .find(|m| {
                    let (w, h) = m.size();
                    (w as u32, h as u32, refresh_mhz(m))
                        == (mode.width, mode.height, mode.refresh_mhz)
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("{} no longer offers {}", d.connector, mode_label(mode))
                })?;
            let res = c.resource_handles()?;
            let crtc = {
                let mut used = crate::lock(&card.crtcs);
                let current = info
                    .current_encoder()
                    .and_then(|e| c.get_encoder(e).ok())
                    .and_then(|e| e.crtc());
                let mut candidates: Vec<drm::control::crtc::Handle> = current.into_iter().collect();
                for e in info.encoders() {
                    if let Ok(e) = c.get_encoder(*e) {
                        candidates.extend(res.filter_crtcs(e.possible_crtcs()));
                    }
                }
                let crtc = candidates
                    .into_iter()
                    .find(|h| !used.contains(&u32::from(*h)))
                    .ok_or_else(|| {
                        anyhow::anyhow!("no free display controller for {}", d.connector)
                    })?;
                used.insert(u32::from(crtc));
                crtc
            };
            let mut sink = KmsSink {
                name: format!("Display: {}", d.connector),
                card: card.clone(),
                crtc,
                size: (mode.width, mode.height),
                bufs: Vec::new(),
                next: 1,
                drawn: vec![None; 3],
                scaled: Vec::new(),
                pair: None,
                pairing: None,
                paired_flips: (0, 0),
            };
            for _ in 0..3 {
                let mut b = c.create_dumb_buffer(
                    (mode.width, mode.height),
                    drm::buffer::DrmFourcc::Xrgb8888,
                    32,
                )?;
                c.map_dumb_buffer(&mut b)?.fill(0);
                let fb = match c.add_framebuffer(&b, 24, 32) {
                    Ok(fb) => fb,
                    Err(e) => {
                        let _ = c.destroy_dumb_buffer(b);
                        return Err(e.into());
                    }
                };
                sink.bufs.push((b, fb));
            }
            c.set_crtc(
                crtc,
                Some(sink.bufs[0].1),
                (0, 0),
                &[handle],
                Some(drm_mode),
            )
            .map_err(|e| {
                anyhow::anyhow!("cannot set {} on {}: {e}", mode_label(mode), d.connector)
            })?;
            Ok(sink)
        }
    }

    impl Drop for KmsSink {
        fn drop(&mut self) {
            use drm::control::Device;
            if let Some((pair, role)) = self.pair.take() {
                pair.release(role);
                tracing::debug!(display = %self.name, together = self.paired_flips.0, alone = self.paired_flips.1, "fill/key flips");
            }
            let c = &self.card.card;
            // Removing the framebuffer on screen turns the output off (black).
            for (b, fb) in self.bufs.drain(..) {
                let _ = c.destroy_framebuffer(fb);
                let _ = c.destroy_dumb_buffer(b);
            }
            crate::lock(&self.card.crtcs).remove(&u32::from(self.crtc));
        }
    }

    impl FrameSink for KmsSink {
        fn name(&self) -> &str {
            &self.name
        }

        fn send(&mut self, frame: &Frame<'_>) -> anyhow::Result<()> {
            use drm::control::Device;
            frame.validate()?;
            let (dw, dh) = self.size;
            let (x, y, w, h) = letterbox(frame.width, frame.height, dw, dh);
            let src: &[u8] = if (w, h) == (frame.width, frame.height) {
                frame.pixels
            } else {
                self.scaled =
                    crate::outputs::scale_rgba(frame.pixels, frame.width, frame.height, w, h);
                &self.scaled
            };
            let i = self.next;
            self.next = (self.next + 1) % self.bufs.len();
            let c = &self.card.card;
            let (buf, fb) = &mut self.bufs[i];
            let pitch = drm::buffer::Buffer::pitch(buf) as usize;
            {
                let mut map = c.map_dumb_buffer(buf)?;
                if self.drawn[i] != Some((x, y, w, h)) {
                    map.fill(0);
                    self.drawn[i] = Some((x, y, w, h));
                }
                write_xrgb(&mut map, pitch, src, (x, y, w, h));
            }
            // At most one flip is pending: wait (up to ~100 ms) for the previous one. The fill
            // and key of a screen flip together (a quarter frame's wait for the partner).
            let mine = (self.crtc, *fb);
            let (card, name) = (&self.card, &self.name);
            rejoin_pair(&mut self.pair, &card.path, self.pairing.as_ref(), name);
            match &self.pair {
                Some((pair, role)) => {
                    let r = frame.frame_rate;
                    let wait = Duration::from_nanos(
                        250_000_000 * r.den.max(1) as u64 / r.num.max(1) as u64,
                    );
                    let together = pair
                        .flip(*role, frame.timecode, mine, wait, |f| page_flip(card, f))
                        .map_err(|e| anyhow::anyhow!("display {name}: {e:#}"))?;
                    if together {
                        self.paired_flips.0 += 1;
                    } else {
                        self.paired_flips.1 += 1;
                        if self.paired_flips.1.is_power_of_two() {
                            tracing::debug!(display = %name, alone = self.paired_flips.1, "a fill/key half flipped without its partner");
                        }
                    }
                }
                None => {
                    page_flip(card, mine).map_err(|e| anyhow::anyhow!("display {name}: {e:#}"))?
                }
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// E03-3: fill on A, key on B; the operator swaps them one output at a time. A (now
        /// key) finds the role taken and flips alone, but joins as soon as B gives it up.
        #[test]
        fn a_half_that_found_its_role_taken_joins_when_it_is_free() {
            let card = Path::new("/nonexistent/e03-3-card");
            let pairing = |key| Pairing {
                screen: "e03-3-screen".into(),
                part: String::new(),
                key,
            };
            let a_fill = join_pair(card, &pairing(false)).expect("fill");
            let b_key = join_pair(card, &pairing(true)).expect("key");
            // Edit 1 rebuilds A as the key: B still holds it.
            a_fill.0.release(a_fill.1);
            let mut a = join_pair(card, &pairing(true));
            assert!(a.is_none(), "the key role is still B's");
            rejoin_pair(&mut a, card, Some(&pairing(true)), "A");
            assert!(a.is_none());
            // Edit 2 rebuilds B as the fill: the key role is free now.
            b_key.0.release(b_key.1);
            let b = join_pair(card, &pairing(false)).expect("fill");
            rejoin_pair(&mut a, card, Some(&pairing(true)), "A");
            let (pa, role) = a.expect("A joined the pair");
            assert!(Arc::ptr_eq(&pa, &b.0) && role == 1);
        }

        /// F03-2: on a tiled screen each tile's fill and key make a pair of their own, whatever
        /// order the four halves open in.
        #[test]
        fn each_tile_has_its_own_pair() {
            let card = Path::new("/nonexistent/f03-2-card");
            let screen = model::ScreenId::new();
            let half = |tile: u32, key| {
                let b = OutputBinding {
                    screen,
                    tile: Some(tile),
                    target: if key {
                        OutputTarget::Key
                    } else {
                        OutputTarget::Fill
                    },
                    ..OutputBinding::default()
                };
                Pairing::of(&b).unwrap()
            };
            let fill0 = join_pair(card, &half(0, false)).expect("tile 0 fill");
            let key1 = join_pair(card, &half(1, true)).expect("tile 1 key");
            let fill1 =
                join_pair(card, &half(1, false)).expect("tile 1's fill finds its role free");
            let key0 = join_pair(card, &half(0, true)).expect("tile 0's key finds its role free");
            assert!(
                Arc::ptr_eq(&fill0.0, &key0.0),
                "tile 0's halves flip together"
            );
            assert!(
                Arc::ptr_eq(&fill1.0, &key1.0),
                "tile 1's halves flip together"
            );
            assert!(!Arc::ptr_eq(&fill0.0, &fill1.0));
        }

        /// Hardware probe, run by hand (`video` group, no desktop on the card):
        /// `cargo test -p vesper-server --lib kms_probe -- --ignored --nocapture`. Lists the
        /// connectors, checks that this process becomes the DRM master, and shows a grey
        /// picture for two seconds on every connected display.
        #[test]
        #[ignore = "drives real displays"]
        fn kms_probe() {
            let kms = Kms::default();
            let list = kms.list();
            assert!(!list.is_empty(), "no DRM connectors");
            for d in &list {
                eprintln!(
                    "{} connected={} model={:?} modes={} preferred={:?}",
                    d.connector,
                    d.connected,
                    d.model,
                    d.modes.len(),
                    d.preferred.map(mode_label)
                );
            }
            // As in the server: the renderer (the shared GPU device) is up first, and must not
            // have taken the card's master role itself.
            let _r = render::Renderer::new(render::RendererOptions::default()).expect("GPU");
            let card = master(&list[0].card).expect("DRM master of the card");
            eprintln!("DRM master of {}: yes", card.path.display());
            for d in list.iter().filter(|d| d.connected) {
                let m = d.preferred.unwrap();
                let mut sink = KmsSink::open(d, m).expect("modeset");
                let px = vec![128u8; 640 * 360 * 4];
                let f = Frame {
                    width: 640,
                    height: 360,
                    pixels: &px,
                    alpha: model::AlphaMode::Opaque,
                    frame_rate: model::FrameRate::FPS30,
                    timecode: 0,
                };
                let t = std::time::Instant::now();
                for _ in 0..120 {
                    sink.send(&f).expect("flip");
                }
                eprintln!("{}: 120 frames in {:?}", d.connector, t.elapsed());
            }
            // A disconnected connector fails cleanly.
            if let Some(d) = list.iter().find(|d| !d.connected) {
                let e = KmsSink::open(
                    d,
                    DisplayMode {
                        width: 1920,
                        height: 1080,
                        refresh_mhz: 60000,
                    },
                )
                .err()
                .expect("no modes on a disconnected connector");
                eprintln!("{}: {e}", d.connector);
            }
        }

        /// A04-2: on a box with connectors on two cards (an iGPU and a graphics card), the
        /// display IDs carry the card; on a one-card box they stay the bare names.
        #[test]
        fn display_ids_are_unique_across_cards() {
            let root = std::env::temp_dir().join(format!("vesper-sysfs-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let add = |name: &str, id: u32, status: &str| {
                let d = root.join(name);
                std::fs::create_dir_all(&d).unwrap();
                std::fs::write(d.join("connector_id"), format!("{id}\n")).unwrap();
                std::fs::write(d.join("status"), format!("{status}\n")).unwrap();
            };
            add("card0-HDMI-A-1", 90, "disconnected");
            add("card0-DP-1", 95, "disconnected");
            add("card1-HDMI-A-1", 101, "connected");
            add("card1-Writeback-1", 110, "unknown");
            std::fs::create_dir_all(root.join("renderD128")).unwrap();
            let conns = connectors_in(&root);
            assert_eq!(
                display_ids(&conns),
                ["card0-HDMI-A-1", "card0-DP-1", "card1-HDMI-A-1"]
            );
            assert_eq!(conns[2].card, Path::new("/dev/dri/card1"));
            assert!(conns[2].connected);
            let one_card: Vec<Connector> = conns.into_iter().take(2).collect();
            assert_eq!(display_ids(&one_card), ["HDMI-A-1", "DP-1"]);
            assert!(display_ids(&[]).is_empty());
            let _ = std::fs::remove_dir_all(&root);
        }

        #[test]
        fn edid_names_are_read() {
            let mut e = vec![0u8; 128];
            e[0..8].copy_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0]);
            // "XYZ": X=24, Y=25, Z=26.
            let v: u16 = (24 << 10) | (25 << 5) | 26;
            e[8..10].copy_from_slice(&v.to_be_bytes());
            e[72..77].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
            e[77..90].copy_from_slice(b"Test Panel\n  ");
            assert_eq!(
                edid_names(&e),
                (Some("XYZ".into()), Some("Test Panel".into()))
            );
            assert_eq!(edid_names(&[0u8; 10]), (None, None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_and_key_flip_together_or_alone() {
        let pair = Arc::new(FlipPair::<u32>::new());
        // Alone: no partner, a flip at once.
        assert!(pair.claim(0) && !pair.claim(0), "one fill per pair");
        let flips = Mutex::new(Vec::new());
        let rec = |f: u32| {
            crate::lock(&flips).push(f);
            Ok(())
        };
        assert!(!pair.flip(0, 1, 10, Duration::from_secs(5), rec).unwrap());
        assert_eq!(crate::lock(&flips).pop(), Some(10));
        // Paired: both halves of tick 2 are flipped by whoever comes last, the fill first.
        assert!(pair.claim(1));
        let seen = Mutex::new(Vec::new());
        std::thread::scope(|s| {
            let (pair, seen) = (&pair, &seen);
            let key = s.spawn(move || {
                pair.flip(1, 2, 21, Duration::from_secs(5), |f: u32| {
                    crate::lock(seen).push(("key", f));
                    Ok(())
                })
            });
            std::thread::sleep(Duration::from_millis(50));
            let fill = pair.flip(0, 2, 20, Duration::from_secs(5), |f: u32| {
                crate::lock(seen).push(("fill", f));
                Ok(())
            });
            assert!(fill.unwrap() && key.join().unwrap().unwrap());
        });
        assert_eq!(*crate::lock(&seen), vec![("fill", 20), ("fill", 21)]);
        // A partner on another tick (or stalled) is not waited for longer than `wait`.
        let t = std::time::Instant::now();
        assert!(!pair.flip(1, 3, 31, Duration::from_millis(30), rec).unwrap());
        assert!(t.elapsed() >= Duration::from_millis(30) && t.elapsed() < Duration::from_secs(2));
        assert!(!pair.flip(0, 4, 40, Duration::from_millis(30), rec).unwrap());
        assert_eq!(*crate::lock(&flips), vec![31, 40]);
        // A released role frees its place.
        pair.release(1);
        assert!(pair.claim(1));
    }

    /// E03-2: when the half that comes second flips both, each half gets the result of its
    /// own flip, and a failed fill flip does not skip the key's.
    #[test]
    fn each_half_of_a_pair_gets_its_own_flip_result() {
        let pair = Arc::new(FlipPair::<u32>::new());
        assert!(pair.claim(0) && pair.claim(1));
        let flipped = Mutex::new(Vec::new());
        // The fill's CRTC (20) fails; the key (21) arrives second and flips both.
        let flip = |f: u32| {
            crate::lock(&flipped).push(f);
            if f == 20 {
                anyhow::bail!("fill CRTC gone")
            }
            Ok(())
        };
        std::thread::scope(|s| {
            let (pair, flip) = (&pair, &flip);
            let fill = s.spawn(move || pair.flip(0, 7, 20, Duration::from_secs(5), flip));
            std::thread::sleep(Duration::from_millis(50));
            let key = pair.flip(1, 7, 21, Duration::from_secs(5), flip);
            assert!(key.unwrap(), "the key flipped, together");
            let e = fill.join().unwrap().unwrap_err();
            assert!(format!("{e:#}").contains("fill CRTC gone"), "{e:#}");
        });
        assert_eq!(
            *crate::lock(&flipped),
            vec![20, 21],
            "the key flip was not skipped"
        );
        // The other way round: the fill comes second, the key's flip fails.
        let flip = |f: u32| {
            if f == 31 {
                anyhow::bail!("key CRTC gone")
            }
            Ok(())
        };
        std::thread::scope(|s| {
            let (pair, flip) = (&pair, &flip);
            let key = s.spawn(move || pair.flip(1, 8, 31, Duration::from_secs(5), flip));
            std::thread::sleep(Duration::from_millis(50));
            assert!(pair.flip(0, 8, 30, Duration::from_secs(5), flip).unwrap());
            assert!(key.join().unwrap().is_err());
        });
    }

    #[test]
    fn frames_are_letterboxed_into_xrgb() {
        assert_eq!(letterbox(1920, 1080, 1920, 1080), (0, 0, 1920, 1080));
        assert_eq!(letterbox(640, 360, 1280, 720), (0, 0, 1280, 720));
        assert_eq!(letterbox(1920, 1080, 1024, 768), (0, 96, 1024, 576));
        assert_eq!(letterbox(1440, 1080, 1920, 1080), (240, 0, 1440, 1080));
        // A 2×1 red/green frame at (1, 1) of a 4×3 buffer with a 20-byte pitch.
        let mut dst = vec![0u8; 20 * 3];
        write_xrgb(
            &mut dst,
            20,
            &[255, 0, 0, 255, 0, 128, 0, 255],
            (1, 1, 2, 1),
        );
        assert_eq!(&dst[24..32], &[0, 0, 255, 255, 0, 128, 0, 255]);
        assert!(dst[..24].iter().all(|b| *b == 0) && dst[32..].iter().all(|b| *b == 0));
    }

    #[test]
    fn modes_resolve_to_what_the_display_offers() {
        let d = FakeDisplays::display("HDMI-A-1", "Panel", true);
        let m = |w, h, r| DisplayMode {
            width: w,
            height: h,
            refresh_mhz: r,
        };
        assert_eq!(d.resolve_mode(None), Some(m(1920, 1080, 60000)));
        assert_eq!(
            d.resolve_mode(Some(m(1280, 720, 60000))),
            Some(m(1280, 720, 60000))
        );
        // Same size, nearest refresh.
        assert_eq!(
            d.resolve_mode(Some(m(1920, 1080, 49999))),
            Some(m(1920, 1080, 50000))
        );
        // Not offered: the preferred mode.
        assert_eq!(
            d.resolve_mode(Some(m(3840, 2160, 30000))),
            Some(m(1920, 1080, 60000))
        );
        assert_eq!(mode_label(m(1920, 1080, 59940)), "1920×1080 @ 59.94 Hz");
        assert_eq!(mode_label(m(1280, 720, 60000)), "1280×720 @ 60 Hz");
    }

    #[test]
    fn selectors_match_connector_then_name_then_index() {
        let list = vec![
            Display {
                index: 0,
                ..FakeDisplays::display("HDMI-A-1", "Panel", false)
            },
            Display {
                index: 1,
                ..FakeDisplays::display("DP-1", "Panel", true)
            },
        ];
        let sel = |c: Option<&str>, n: Option<&str>, i: Option<u32>| DisplaySelector {
            connector: c.map(Into::into),
            name: n.map(Into::into),
            index: i,
        };
        assert_eq!(
            find(&list, &sel(Some("hdmi-a-1"), None, None))
                .unwrap()
                .index,
            0
        );
        // A connector that is gone: the model name, connected first.
        assert_eq!(
            find(&list, &sel(Some("DP-9"), Some("panel"), None))
                .unwrap()
                .index,
            1
        );
        assert_eq!(
            find(&list, &sel(None, None, Some(1))).unwrap().connector,
            "DP-1"
        );
        assert!(find(&list, &sel(Some("DP-9"), None, None)).is_none());
    }

    /// A04-2: a selector saved with a bare connector name (`HDMI-A-1`) on a box whose
    /// displays now carry their card finds the one display it can mean.
    #[test]
    fn bare_connector_selectors_match_displays_listed_with_their_card() {
        let d = |i, c: &str, m: &str, on| Display {
            index: i,
            ..FakeDisplays::display(c, m, on)
        };
        let list = vec![
            d(0, "card0-HDMI-A-1", "", false),
            d(1, "card0-DP-1", "Panel", true),
            d(2, "card1-HDMI-A-1", "Projector", true),
        ];
        let sel = |c: &str, n: Option<&str>| DisplaySelector {
            connector: Some(c.into()),
            name: n.map(Into::into),
            index: None,
        };
        let found = |c, n| find(&list, &sel(c, n)).map(|d| d.connector.as_str());
        assert_eq!(found("card1-HDMI-A-1", None), Some("card1-HDMI-A-1"));
        assert_eq!(found("card0-HDMI-A-1", None), Some("card0-HDMI-A-1"));
        // The only DP-1; of the two HDMI-A-1, the connected one, or the chosen model.
        assert_eq!(found("DP-1", None), Some("card0-DP-1"));
        assert_eq!(found("HDMI-A-1", None), Some("card1-HDMI-A-1"));
        assert_eq!(found("hdmi-a-1", Some("projector")), Some("card1-HDMI-A-1"));
        // Saved with its card, now on a one-card box.
        let one = vec![d(0, "HDMI-A-1", "", true)];
        assert_eq!(
            find(&one, &sel("card1-HDMI-A-1", None)).map(|d| d.index),
            Some(0)
        );
        assert_eq!(bare_connector("card12-DP-3"), "DP-3");
        assert_eq!(bare_connector("cards-DP-3"), "cards-DP-3");
        assert_eq!(bare_connector(r"\\.\DISPLAY2"), r"\\.\DISPLAY2");
        // One connector, saved bare and listed with its card; two cards' stay apart.
        assert!(same_connector("HDMI-A-1", "card1-HDMI-A-1"));
        assert!(same_connector("card1-hdmi-a-1", "HDMI-A-1"));
        assert!(same_connector("card0-DP-1", "CARD0-dp-1"));
        assert!(!same_connector("card0-HDMI-A-1", "card1-HDMI-A-1"));
        assert!(!same_connector("HDMI-A-1", "HDMI-A-2"));
        // Two disconnected namesakes and no model: nothing to go on.
        let two = vec![d(0, "card0-DP-2", "", false), d(1, "card1-DP-2", "", false)];
        assert!(find(&two, &sel("DP-2", None)).is_none());
        // E06-3: a bare selector is one display, the one it resolves to; when nothing
        // resolves, the names decide.
        let same = |a, b| same_display(&list, &sel(a, None), &sel(b, None));
        assert!(same("HDMI-A-1", "card1-HDMI-A-1"));
        assert!(!same("HDMI-A-1", "card0-HDMI-A-1"));
        assert!(!same("card0-HDMI-A-1", "HDMI-A-1"));
        assert!(same("card0-hdmi-a-1", "CARD0-HDMI-A-1"));
        assert!(!same("card0-HDMI-A-1", "card1-HDMI-A-1"));
        assert!(same_display(
            &[],
            &sel("HDMI-A-1", None),
            &sel("card0-HDMI-A-1", None)
        ));
        assert!(same_display(
            &two,
            &sel("DP-2", None),
            &sel("card1-DP-2", None)
        ));
    }
}

#[cfg(test)]
mod session_tests {
    /// A04-3: session 0 (the SYSTEM startup task, services) has no visible desktop.
    #[test]
    fn session_zero_has_no_visible_desktop() {
        let p = super::session_problem(Some(0)).expect("a problem");
        assert!(
            p.contains("session 0") && p.contains("Show screens on this PC"),
            "{p}"
        );
        assert_eq!(super::session_problem(Some(1)), None);
        assert_eq!(super::session_problem(None), None);
        // As Oriel shows it: display outputs unavailable, with the reason.
        let t = crate::config::transports("window", Some(&p));
        let d = t
            .iter()
            .find(|t| t.kind == "display")
            .expect("display transport");
        assert!(!d.available && d.detail.as_deref().unwrap_or("").contains("session 0"));
    }
}
