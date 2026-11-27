//! Vesper Launcher's tray app (`docs/LAUNCHER-PLAN.md`): a tray icon in the Launcher's status
//! colour (green: all good, amber: an update or a start under way, red: needs attention, grey:
//! the service isn't answering) with "Open", "Diagnostics" and "Quit", and a small window that shows the
//! Launcher service's own page (`http://127.0.0.1:5556/`: the product cards and the first-run
//! wizard). It opens by itself while the machine isn't set up yet.
//!
//! It talks only to the local Launcher service (loopback), as a program on the machine: it
//! asks `GET /launcher/v1/overview` every two seconds. The page in the window is the
//! service's, same-origin with its API; it gets no Tauri commands.

// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

/// The Launcher service's local API (`VESPER_LAUNCHER_API` overrides it, for tests).
fn api_addr() -> SocketAddr {
    std::env::var("VESPER_LAUNCHER_API")
        .ok()
        .and_then(|a| a.parse().ok())
        .unwrap_or_else(|| "127.0.0.1:5556".parse().expect("an address"))
}

/// `GET /launcher/v1/overview` from the service, as a program on this machine (no browser
/// headers). `None`: it isn't answering.
fn overview(addr: SocketAddr) -> Option<serde_json::Value> {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        s,
        "GET /launcher/v1/overview HTTP/1.0\r\nHost: {addr}\r\nUser-Agent: vesper-launcher-tray\r\n\r\n"
    )
    .ok()?;
    let mut b = Vec::new();
    s.read_to_end(&mut b).ok()?;
    let t = String::from_utf8_lossy(&b);
    let (head, body) = t.split_once("\r\n\r\n")?;
    head.split_whitespace()
        .nth(1)
        .filter(|c| *c == "200")
        .and_then(|_| serde_json::from_str(body).ok())
}

/// The tray icon: a filled circle in the status colour, 32×32.
fn dot(light: &str) -> Image<'static> {
    let (r, g, b) = match light {
        "ok" => (0x5f, 0xcb, 0x82),
        "updating" => (0xf5, 0xa5, 0x24),
        "attention" => (0xf0, 0x6b, 0x6b),
        _ => (0x6a, 0x6e, 0x78),
    };
    const N: usize = 32;
    let mut rgba = vec![0u8; N * N * 4];
    let c = (N as f32 - 1.0) / 2.0;
    for y in 0..N {
        for x in 0..N {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            // Anti-aliased edge, radius 13.
            let a = (13.5 - d).clamp(0.0, 1.0);
            let i = (y * N + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[r, g, b, (a * 255.0) as u8]);
        }
    }
    Image::new_owned(rgba, N as u32, N as u32)
}

fn tooltip(light: &str) -> &'static str {
    match light {
        "ok" => "Vesper Launcher: all good",
        "updating" => "Vesper Launcher: working…",
        "attention" => "Vesper Launcher: needs attention",
        _ => "Vesper Launcher: the service isn't answering",
    }
}

/// Opens the window on the Launcher's diagnostics page (`idn doctor` in plain language, "Copy
/// report").
fn diagnostics(app: &AppHandle) {
    let url = format!("http://{}/#diagnostics", api_addr()).parse::<tauri::Url>();
    if let (Some(w), Ok(url)) = (app.get_webview_window("main"), url) {
        let _ = w.navigate(url);
    }
    show(app);
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Follows the service: the tray's colour; the window shows the service's page once it
/// answers (and opens by itself on the first run).
fn watch(app: AppHandle) {
    let addr = api_addr();
    let page: tauri::Url = format!("http://{addr}/").parse().expect("a URL");
    let mut last_light = String::new();
    let mut on_page = false;
    let mut opened = false;
    loop {
        let o = overview(addr);
        let light = o
            .as_ref()
            .and_then(|o| o["light"].as_str())
            .unwrap_or("off")
            .to_string();
        if light != last_light {
            if let Some(t) = app.tray_by_id("main") {
                let _ = t.set_icon(Some(dot(&light)));
                let _ = t.set_tooltip(Some(tooltip(&light)));
            }
            last_light = light;
        }
        if let (Some(o), Some(w)) = (&o, app.get_webview_window("main")) {
            if !on_page {
                on_page = w.navigate(page.clone()).is_ok();
            }
            if !opened && o["setup"]["firstRun"].as_bool() == Some(true) {
                opened = true;
                show(&app);
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open Vesper Launcher", true, None::<&str>)?;
            let diag = MenuItem::with_id(app, "diagnostics", "Diagnostics", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit the tray icon", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &diag, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(dot("off"))
                .tooltip(tooltip("off"))
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show(app),
                    "diagnostics" => diagnostics(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = e
                    {
                        show(tray.app_handle());
                    }
                })
                .build(app)?;
            let handle = app.handle().clone();
            std::thread::spawn(move || watch(handle));
            Ok(())
        })
        // Closing the window hides it: the tray icon stays (Quit is in its menu).
        .on_window_event(|w, e| {
            if let WindowEvent::CloseRequested { api, .. } = e {
                let _ = w.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running the Vesper Launcher tray app");
}
