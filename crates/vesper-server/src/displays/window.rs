//! Display outputs as windows (Windows, and Linux with a desktop session: Wayland or X11): a
//! borderless window covering the chosen monitor, or a plain movable window for rehearsal
//! (`fullscreen: false`). `docs/DISPLAY-OUTPUTS-PLAN.md`, "Stage 1b".
//!
//! - **One event loop** for the process (winit allows only one), on its own thread
//!   (`with_any_thread`), started on first use and never stopped. Other threads talk to it
//!   with [`Cmd`]s through its proxy: list the monitors, open a window, close one.
//! - **Monitors** come from the loop's monitor list: the OS name is the identity a binding
//!   keeps (`\\.\DISPLAY2` on Windows, the output name such as `HDMI-A-1` under X11 and
//!   Wayland), with its position, its current mode and whether it is the main one. Windows are
//!   shown at the desktop's mode (no mode switch: a borderless window, not exclusive full
//!   screen), so the current mode is the only one listed.
//! - **Presenting**: each window gets a surface from the renderer's shared GPU device
//!   ([`render::present::WindowPresenter`]); the output's worker thread uploads each frame
//!   (the screen's final frame, the one the network outputs send) and presents it. The engine
//!   never waits for a window.
//! - **Hot-plug**: winit has no monitor-change event, so [`super::watch`] re-lists every
//!   [`super::POLL`]; a window whose monitor is gone is closed when its output is rebuilt (the
//!   binding stays and shows the monitor as not connected).

use super::{Display, DisplaySource, Pairing};
use model::DisplayMode;
use output::{Frame, FrameSink};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId, WindowLevel};

/// How long a caller waits for the event loop to answer.
const REPLY: Duration = Duration::from_secs(3);

/// A monitor as the event loop sees it.
#[derive(Clone, Debug)]
struct Monitor {
    name: String,
    position: (i32, i32),
    size: (u32, u32),
    refresh_mhz: u32,
    primary: bool,
}

/// What a window output asks the loop for.
struct OpenRequest {
    monitor: String,
    fullscreen: bool,
    title: String,
}

/// A window the loop opened, and its inner size in pixels (`w << 32 | h`), kept up to date
/// by the loop's resize events.
struct Opened {
    window: Arc<Window>,
    size: Arc<AtomicU64>,
}

enum Cmd {
    List(mpsc::Sender<Vec<Monitor>>),
    Open(OpenRequest, mpsc::Sender<anyhow::Result<Opened>>),
    Close(WindowId),
}

struct Loop {
    proxy: Mutex<EventLoopProxy<Cmd>>,
}

static LOOP: OnceLock<Result<Loop, String>> = OnceLock::new();

/// The process's event loop, started on first use.
fn event_loop() -> Result<&'static Loop, String> {
    LOOP.get_or_init(start).as_ref().map_err(Clone::clone)
}

fn start() -> Result<Loop, String> {
    // A startup task or service: windows would open on a desktop no one sees.
    #[cfg(windows)]
    if let Some(p) = super::session_problem(super::windows_session()) {
        return Err(p);
    }
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("display-windows".into())
        .spawn(move || {
            let mut b = EventLoop::<Cmd>::with_user_event();
            #[cfg(windows)]
            {
                use winit::platform::windows::EventLoopBuilderExtWindows;
                b.with_any_thread(true);
            }
            #[cfg(target_os = "linux")]
            {
                winit::platform::x11::EventLoopBuilderExtX11::with_any_thread(&mut b, true);
                winit::platform::wayland::EventLoopBuilderExtWayland::with_any_thread(&mut b, true);
            }
            let el = match b.build() {
                Ok(el) => el,
                Err(e) => {
                    let _ = tx.send(Err(format!("no desktop session to show windows on: {e}")));
                    return;
                }
            };
            el.set_control_flow(ControlFlow::Wait);
            let _ = tx.send(Ok(el.create_proxy()));
            let mut app = App::default();
            if let Err(e) = el.run_app(&mut app) {
                tracing::error!("the display windows' event loop stopped: {e}");
            } else {
                tracing::warn!("the display windows' event loop stopped");
            }
        })
        .map_err(|e| e.to_string())?;
    let proxy = rx
        .recv_timeout(REPLY * 2)
        .map_err(|_| "the display windows' event loop did not start".to_string())??;
    Ok(Loop {
        proxy: Mutex::new(proxy),
    })
}

impl Loop {
    fn send(&self, cmd: Cmd) -> anyhow::Result<()> {
        crate::lock(&self.proxy)
            .send_event(cmd)
            .map_err(|_| anyhow::anyhow!("the display windows' event loop has stopped"))
    }
}

/// The windowing system the loop runs on (it decides how an output window is placed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Windows,
    X11,
    Wayland,
}

/// A monitor's rectangle on the desktop, px: `(x, y, width, height)`.
type Rect = (i32, i32, u32, u32);

/// How an output window is made. Firm rules (owner, batch 6), for every platform that allows
/// them: it never takes keyboard focus or grabs the pointer (also when it reopens after a
/// hot-plug), hides the pointer only while the pointer is over it, covers its own monitor
/// only, and stays out of the taskbar and Alt-Tab.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Placement {
    /// Exact outer position and inner size (`None`: the compositor places it).
    rect: Option<Rect>,
    /// Ask the compositor for full screen on the monitor (Wayland only: the one way to cover
    /// an output there). Windows' own full screen in winit activates the window, X11's goes
    /// through the window manager, so neither is used.
    os_fullscreen: bool,
    decorations: bool,
    always_on_top: bool,
    /// Created active (focused). Always false.
    active: bool,
    /// Windows: `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` (no focus on click, not in the taskbar
    /// or Alt-Tab), shown without activation.
    no_activate: bool,
    /// X11: unmanaged (no window manager: no focus, no taskbar, no Alt-Tab, exact place).
    override_redirect: bool,
    /// Pointer hidden while it is over the window.
    hide_pointer: bool,
}

fn placement(platform: Platform, fullscreen: bool, monitor: Rect) -> Placement {
    let (x, y, w, h) = monitor;
    if !fullscreen {
        // A rehearsal window: an ordinary desktop window, a quarter of the monitor, near its
        // top left corner; still opened without taking focus.
        return Placement {
            rect: Some((x + 64, y + 64, (w / 2).max(320), (h / 2).max(180))),
            os_fullscreen: false,
            decorations: true,
            always_on_top: false,
            active: false,
            no_activate: false,
            override_redirect: false,
            hide_pointer: false,
        };
    }
    Placement {
        rect: (platform != Platform::Wayland).then_some(monitor),
        os_fullscreen: platform == Platform::Wayland,
        decorations: false,
        always_on_top: true,
        active: false,
        no_activate: platform == Platform::Windows,
        override_redirect: platform == Platform::X11,
        hide_pointer: true,
    }
}

/// Whether a window at `win` stays on its monitor `mon` (a pixel of slack for rounding).
fn on_monitor(win: Rect, mon: Rect) -> bool {
    let (wx, wy, ww, wh) = (win.0 as i64, win.1 as i64, win.2 as i64, win.3 as i64);
    let (mx, my, mw, mh) = (mon.0 as i64, mon.1 as i64, mon.2 as i64, mon.3 as i64);
    wx >= mx - 1 && wy >= my - 1 && wx + ww <= mx + mw + 1 && wy + wh <= my + mh + 1
}

fn platform(el: &ActiveEventLoop) -> Platform {
    #[cfg(target_os = "linux")]
    {
        use winit::platform::wayland::ActiveEventLoopExtWayland;
        if el.is_wayland() {
            return Platform::Wayland;
        }
        Platform::X11
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = el;
        Platform::Windows
    }
}

/// An open output window.
struct Open {
    window: Arc<Window>,
    size: Arc<AtomicU64>,
    /// The monitor it belongs to (by name), when it covers it.
    monitor: Option<String>,
}

#[derive(Default)]
struct App {
    windows: HashMap<WindowId, Open>,
}

fn pack((w, h): (u32, u32)) -> u64 {
    (w as u64) << 32 | h as u64
}

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

fn monitor_name(i: usize, m: &MonitorHandle) -> String {
    m.name().unwrap_or_else(|| format!("Monitor {}", i + 1))
}

fn rect_of(m: &MonitorHandle) -> Rect {
    let (p, s) = (m.position(), m.size());
    (p.x, p.y, s.width, s.height)
}

fn find_monitor(el: &ActiveEventLoop, name: &str) -> Option<MonitorHandle> {
    el.available_monitors()
        .enumerate()
        .find(|(i, m)| monitor_name(*i, m) == name)
        .map(|(_, m)| m)
}

fn monitors(el: &ActiveEventLoop) -> Vec<Monitor> {
    let primary = el.primary_monitor();
    el.available_monitors()
        .enumerate()
        .map(|(i, m)| {
            let (x, y, w, h) = rect_of(&m);
            Monitor {
                name: monitor_name(i, &m),
                position: (x, y),
                size: (w, h),
                refresh_mhz: m.refresh_rate_millihertz().unwrap_or(60_000),
                primary: primary.as_ref() == Some(&m),
            }
        })
        .collect()
}

impl App {
    fn open(&mut self, el: &ActiveEventLoop, req: OpenRequest) -> anyhow::Result<Opened> {
        let monitor = find_monitor(el, &req.monitor)
            .ok_or_else(|| anyhow::anyhow!("{} is not connected", req.monitor))?;
        let plan = placement(platform(el), req.fullscreen, rect_of(&monitor));
        let mut attrs = WindowAttributes::default()
            .with_title(req.title)
            .with_active(plan.active)
            .with_decorations(plan.decorations)
            // Shown below once it is set up (Windows: without activating it).
            .with_visible(false);
        if let Some((x, y, w, h)) = plan.rect {
            attrs = attrs
                .with_position(PhysicalPosition::new(x, y))
                .with_inner_size(PhysicalSize::new(w, h));
        }
        if plan.always_on_top {
            attrs = attrs.with_window_level(WindowLevel::AlwaysOnTop);
        }
        if plan.os_fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))));
        }
        #[cfg(windows)]
        if plan.no_activate {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs = attrs.with_skip_taskbar(true);
        }
        // An app ID / WM_CLASS, so a desktop's window rules can match the output windows.
        #[cfg(target_os = "linux")]
        {
            use winit::platform::{wayland::WindowAttributesExtWayland, x11};
            attrs = WindowAttributesExtWayland::with_name(attrs, "vesper", "vesper-display");
            attrs = x11::WindowAttributesExtX11::with_name(attrs, "vesper", "vesper-display");
            if plan.override_redirect {
                attrs = x11::WindowAttributesExtX11::with_override_redirect(attrs, true);
            }
        }
        let window = Arc::new(el.create_window(attrs)?);
        window.set_cursor_visible(!plan.hide_pointer);
        show(&window, plan.no_activate);
        let s = window.inner_size();
        let size = Arc::new(AtomicU64::new(pack((s.width, s.height))));
        self.windows.insert(
            window.id(),
            Open {
                window: window.clone(),
                size: size.clone(),
                monitor: req.fullscreen.then_some(req.monitor),
            },
        );
        Ok(Opened { window, size })
    }

    /// A full screen window that left its monitor (the OS moves windows off a monitor that
    /// is unplugged, or rearranges them on a resolution change) goes back over it, or hides
    /// when the monitor is gone: it never shows on another monitor. Its output is rebuilt
    /// by the next hot-plug poll.
    fn keep_on_monitor(&mut self, el: &ActiveEventLoop, id: WindowId) {
        let Some(o) = self.windows.get(&id) else {
            return;
        };
        let Some(name) = &o.monitor else {
            return;
        };
        let w = &o.window;
        let (Ok(pos), size) = (w.outer_position(), w.outer_size()) else {
            return;
        };
        let at = (pos.x, pos.y, size.width, size.height);
        match find_monitor(el, name) {
            Some(m) if on_monitor(at, rect_of(&m)) => {}
            Some(m) if platform(el) != Platform::Wayland => {
                let (x, y, mw, mh) = rect_of(&m);
                tracing::info!(display = %name, "the output window moved off its monitor: putting it back");
                w.set_outer_position(PhysicalPosition::new(x, y));
                let _ = w.request_inner_size(PhysicalSize::new(mw, mh));
            }
            Some(_) => {}
            None => {
                tracing::info!(display = %name, "the output window's monitor is gone: hiding it");
                w.set_visible(false);
            }
        }
    }
}

/// Shows a new window; on Windows without activating it, out of the taskbar and Alt-Tab, and
/// never activated by a click (`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`, set while it is still
/// hidden so the taskbar never lists it).
fn show(window: &Window, no_activate: bool) {
    #[cfg(windows)]
    if no_activate {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_EX_APPWINDOW,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        };
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = window.window_handle()
            && let RawWindowHandle::Win32(w) = h.as_raw()
        {
            let hwnd = w.hwnd.get() as windows_sys::Win32::Foundation::HWND;
            // SAFETY: `hwnd` is this live window's handle, used on its own (the loop's) thread.
            unsafe {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let ex = (ex | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize)
                    & !(WS_EX_APPWINDOW as isize);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            return;
        }
    }
    let _ = no_activate;
    window.set_visible(true);
}

impl ApplicationHandler<Cmd> for App {
    fn resumed(&mut self, _: &ActiveEventLoop) {}

    fn user_event(&mut self, el: &ActiveEventLoop, cmd: Cmd) {
        match cmd {
            Cmd::List(reply) => {
                let _ = reply.send(monitors(el));
            }
            Cmd::Open(req, reply) => {
                let _ = reply.send(self.open(el, req));
            }
            Cmd::Close(id) => {
                self.windows.remove(&id);
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(s) => {
                if let Some(o) = self.windows.get(&id) {
                    o.size.store(pack((s.width, s.height)), Ordering::Release);
                }
                self.keep_on_monitor(el, id);
            }
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.keep_on_monitor(el, id)
            }
            // The window belongs to its output: it closes when the output is removed or
            // changed in Oriel, not from the desktop (no accidental Alt+F4 during a service).
            WindowEvent::CloseRequested => {}
            _ => {}
        }
    }
}

/// Display outputs as windows on this desktop.
#[derive(Debug, Default)]
pub struct WindowDisplays;

impl DisplaySource for WindowDisplays {
    fn backend(&self) -> &'static str {
        "window"
    }

    /// Why windows can't be shown here, if they can't (no desktop session).
    fn problem(&self) -> Option<String> {
        event_loop().err()
    }

    fn list(&self) -> Vec<Display> {
        let Ok(el) = event_loop() else {
            return Vec::new();
        };
        let (tx, rx) = mpsc::channel();
        if el.send(Cmd::List(tx)).is_err() {
            return Vec::new();
        }
        let Ok(list) = rx.recv_timeout(REPLY) else {
            tracing::warn!("the display windows' event loop did not list the monitors");
            return Vec::new();
        };
        list.into_iter()
            .enumerate()
            .map(|(i, m)| {
                let mode = DisplayMode {
                    width: m.size.0,
                    height: m.size.1,
                    refresh_mhz: m.refresh_mhz,
                };
                Display {
                    model: friendly_name(&m.name),
                    connector: m.name,
                    index: i as u32,
                    manufacturer: None,
                    connected: true,
                    modes: vec![mode],
                    preferred: Some(mode),
                    card: Default::default(),
                    connector_id: 0,
                    position: Some(m.position),
                    primary: m.primary,
                }
            })
            .collect()
    }

    fn open(
        &self,
        display: &Display,
        _mode: DisplayMode,
        fullscreen: bool,
        pair: Option<&Pairing>,
    ) -> anyhow::Result<Box<dyn FrameSink>> {
        let el = event_loop().map_err(|e| anyhow::anyhow!(e))?;
        let (tx, rx) = mpsc::channel();
        // The fill and key of a screen are two windows, each on its own worker (no pairing:
        // each presents at its monitor's next vertical blank).
        let role = pair.map(|p| format!(" ({})", p.role())).unwrap_or_default();
        el.send(Cmd::Open(
            OpenRequest {
                monitor: display.connector.clone(),
                fullscreen,
                title: format!("Vesper: {}{role}", display.label()),
            },
            tx,
        ))?;
        let opened = rx
            .recv_timeout(REPLY)
            .map_err(|_| anyhow::anyhow!("the display windows' event loop did not answer"))??;
        let close = |e: anyhow::Error| {
            let _ = el.send(Cmd::Close(opened.window.id()));
            e
        };
        let presenter = render::present::WindowPresenter::new(
            opened.window.clone(),
            unpack(opened.size.load(Ordering::Acquire)),
        )
        .map_err(close)?;
        let info = presenter.info();
        let connector = &display.connector;
        tracing::info!(
            connector = %connector,
            fullscreen,
            adapter = %info.adapter,
            format = %info.format,
            present = %info.present_mode,
            "showing a window on the display"
        );
        Ok(Box::new(WindowSink {
            name: format!("Display: {}", display.connector),
            presenter: Healing::new(presenter),
            window: opened.window,
            size: opened.size,
        }))
    }
}

/// "Display 2" for Windows' `\\.\DISPLAY2`; other names are already readable.
fn friendly_name(name: &str) -> String {
    match name.strip_prefix(r"\\.\DISPLAY") {
        Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => format!("Display {n}"),
        _ => String::new(),
    }
}

/// Something made again when it is lost (a window's presenter after a GPU reset), retrying
/// with a back-off while making it fails (the driver may still be resetting).
struct Healing<T> {
    current: Option<T>,
    retry_at: Option<Instant>,
    delay: Duration,
}

/// First and longest wait between attempts to make a lost presenter again.
const HEAL_FIRST: Duration = Duration::from_millis(250);
const HEAL_MAX: Duration = Duration::from_secs(5);

impl<T> Healing<T> {
    fn new(t: T) -> Self {
        Healing {
            current: Some(t),
            retry_at: None,
            delay: HEAL_FIRST,
        }
    }

    /// The current one; when it is `lost` (or an earlier attempt failed) made again with
    /// `make`, at most once per back-off. `Err` while there is none.
    fn get(
        &mut self,
        now: Instant,
        lost: impl FnOnce(&T) -> bool,
        make: impl FnOnce() -> anyhow::Result<T>,
    ) -> anyhow::Result<&mut T> {
        if self.current.as_ref().is_some_and(lost) {
            // (The old one goes first: its surface holds the window.)
            self.current = None;
            self.retry_at = None;
            self.delay = HEAL_FIRST;
        }
        if self.current.is_none() {
            if self.retry_at.is_some_and(|at| now < at) {
                anyhow::bail!("waiting to present again after a GPU reset");
            }
            match make() {
                Ok(t) => {
                    self.current = Some(t);
                    self.retry_at = None;
                    self.delay = HEAL_FIRST;
                }
                Err(e) => {
                    self.retry_at = Some(now + self.delay);
                    self.delay = (self.delay * 2).min(HEAL_MAX);
                    return Err(e.context("presenting again after a GPU reset"));
                }
            }
        }
        self.current
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no presenter"))
    }
}

struct WindowSink {
    name: String,
    presenter: Healing<render::present::WindowPresenter>,
    window: Arc<Window>,
    size: Arc<AtomicU64>,
}

impl FrameSink for WindowSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn send(&mut self, frame: &Frame<'_>) -> anyhow::Result<()> {
        frame.validate()?;
        let size = unpack(self.size.load(Ordering::Acquire));
        // After a GPU reset, present from the replacement device (the old surface goes
        // first); while that can't be made the output reports failing and it is retried.
        let (window, name) = (&self.window, &self.name);
        let p = self.presenter.get(
            Instant::now(),
            |p| p.is_lost(),
            || {
                let p = render::present::WindowPresenter::new(window.clone(), size)?;
                tracing::warn!(display = %name, adapter = %p.info().adapter, "presenting again after a GPU reset");
                Ok(p)
            },
        )?;
        p.present(frame.pixels, frame.width, frame.height, size)?;
        Ok(())
    }
}

impl Drop for WindowSink {
    fn drop(&mut self) {
        // The surface goes before its window; the loop drops the window on its own thread.
        self.presenter.current = None;
        self.window.set_visible(false);
        if let Ok(el) = event_loop() {
            let _ = el.send(Cmd::Close(self.window.id()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HEAL_FIRST, Healing, Platform, on_monitor, placement};
    use std::time::{Duration, Instant};

    /// D02-2: a presenter lost in a GPU reset whose first re-make fails is made again later
    /// (with a back-off), and the output reports failing meanwhile instead of sending nothing.
    #[test]
    fn a_lost_presenter_is_retried_until_it_comes_back() {
        let t0 = Instant::now();
        let mut h = Healing::new(1);
        assert_eq!(*h.get(t0, |_| false, || unreachable!()).unwrap(), 1);
        // Lost; the driver is still resetting.
        assert!(h.get(t0, |_| true, || anyhow::bail!("no device")).is_err());
        // Within the back-off: not tried, still failing.
        let mut tried = false;
        let r = h.get(
            t0 + HEAL_FIRST / 2,
            |_| false,
            || {
                tried = true;
                Ok(2)
            },
        );
        assert!(r.is_err() && !tried);
        // Fails again: the wait doubles.
        assert!(
            h.get(t0 + HEAL_FIRST, |_| false, || anyhow::bail!("no"))
                .is_err()
        );
        assert!(h.get(t0 + HEAL_FIRST * 2, |_| false, || Ok(9)).is_err());
        // Back: presents again.
        let at = t0 + HEAL_FIRST * 3 + Duration::from_millis(1);
        assert_eq!(*h.get(at, |_| false, || Ok(3)).unwrap(), 3);
        assert_eq!(*h.get(at, |_| false, || unreachable!()).unwrap(), 3);
    }

    /// The owner's firm rules for output windows, on each platform.
    #[test]
    fn output_windows_never_take_focus_and_cover_only_their_monitor() {
        let mon = (1920, 0, 1280, 720);
        for p in [Platform::Windows, Platform::X11, Platform::Wayland] {
            let full = placement(p, true, mon);
            assert!(!full.active, "{p:?}: opens without focus");
            assert!(full.hide_pointer && !full.decorations && full.always_on_top);
            // Exactly the bound monitor, where the platform lets us place windows.
            match p {
                Platform::Wayland => assert!(full.os_fullscreen && full.rect.is_none()),
                _ => assert!(!full.os_fullscreen && full.rect == Some(mon)),
            }
            // Out of the taskbar and Alt-Tab, never activated by a click.
            assert_eq!(full.no_activate, p == Platform::Windows);
            assert_eq!(full.override_redirect, p == Platform::X11);
            // A rehearsal window: an ordinary window, still not focused, on its monitor.
            let win = placement(p, false, mon);
            assert!(!win.active && !win.hide_pointer && win.decorations && !win.always_on_top);
            assert!(!win.os_fullscreen && !win.no_activate && !win.override_redirect);
            assert!(on_monitor(win.rect.unwrap(), mon));
        }
    }

    #[test]
    fn a_window_off_its_monitor_is_noticed() {
        let mon = (1920, 0, 1920, 1080);
        assert!(on_monitor(mon, mon));
        assert!(on_monitor((1919, 0, 1921, 1080), mon));
        // Moved to the operator's monitor (unplugged projector) or spanning both.
        assert!(!on_monitor((0, 0, 1920, 1080), mon));
        assert!(!on_monitor((1000, 0, 1920, 1080), mon));
    }

    #[test]
    fn windows_display_names_read_well() {
        assert_eq!(super::friendly_name(r"\\.\DISPLAY2"), "Display 2");
        assert_eq!(super::friendly_name("HDMI-A-1"), "");
        assert_eq!(super::unpack(super::pack((1920, 1080))), (1920, 1080));
    }

    /// A real window under the desktop session this test runs in (`DISPLAY` or
    /// `WAYLAND_DISPLAY`; on this build machine: `xvfb-run`). Run by hand:
    /// `xvfb-run -s "-screen 0 1280x720x24" cargo test -p vesper-server --lib window_probe --
    /// --ignored --nocapture` (`VESPER_TEST_SOFTWARE=1` renders and presents on the software adapter, for an X
    /// server the GPU driver can't present to, such as Xvfb without DRI3).
    #[test]
    #[ignore = "needs a desktop session"]
    fn window_probe() {
        use super::*;
        let _r = render::Renderer::new(render::RendererOptions {
            adapter: if std::env::var_os("VESPER_TEST_SOFTWARE").is_some() {
                render::AdapterPreference::Software
            } else {
                render::AdapterPreference::HardwareFirst
            },
            ..Default::default()
        })
        .expect("GPU");
        let src = WindowDisplays;
        assert_eq!(src.problem(), None, "a desktop session");
        let list = src.list();
        eprintln!("{list:#?}");
        let d = list.first().expect("a monitor");
        for fullscreen in [true, false] {
            let mut sink = src
                .open(d, d.preferred.unwrap(), fullscreen, None)
                .expect("a window");
            let px = vec![200u8; 640 * 360 * 4];
            let f = Frame {
                width: 640,
                height: 360,
                pixels: &px,
                alpha: model::AlphaMode::Opaque,
                frame_rate: model::FrameRate::FPS30,
                timecode: 0,
            };
            let t = std::time::Instant::now();
            for _ in 0..60 {
                sink.send(&f).expect("present");
            }
            eprintln!("fullscreen={fullscreen}: 60 frames in {:?}", t.elapsed());
        }
    }
}
