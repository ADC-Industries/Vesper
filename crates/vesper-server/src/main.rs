//! `vesper`: the headless render/playout server.
//!
//! ```text
//! vesper --data <dir> [--log-file <file>] [--listen 0.0.0.0:5555] [--no-trust-localhost] [--no-mdns]
//!        [--png-dir <dir> [--png-every 30]] [--preview <screen>] [--idn-prefix <text>]
//!        [--no-bindings] [--software] [--software-decode] [--fonts <dir>]...
//!        [--ui-dir <dir>] [--scripture-dir <dir>] [--audio-device <name>]
//!        [--no-site-manager] [--site-discovery-port 5959] [--site-api-port 29202]
//!        [--codex-channel-port <port>] [--codex-cache-gib 32]
//! ```
//!
//! The control API (REST + WebSocket under `/api/v1/`) is documented in
//! `crates/protocol/README.md`. Remote clients pair with the 6-digit code the server logs.
//!
//! On Windows `--data` defaults to `%ProgramData%\Vesper`.
//!
//! Logs go to stderr (or `--log-file`); set `RUST_LOG` (e.g. `RUST_LOG=vesper_server=debug,info`) to change the level.
//! Stops cleanly on Ctrl-C / SIGTERM.

use clap::Parser;

/// Rust's allocations go to mimalloc, which gives freed memory back to the system (glibc's
/// arenas held it free; see `vesper_server::heap`). C libraries keep glibc's `malloc`.
#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;
use std::net::SocketAddr;
use std::path::PathBuf;
use vesper_server::outputs::DevOutputs;
use vesper_server::{ServerConfig, start};

/// `--displays`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum DisplaysArg {
    Auto,
    Kms,
    Window,
    None,
}

/// Vesper: the headless render/playout server.
#[derive(Parser, Debug)]
#[command(name = "vesper", version, about)]
struct Args {
    /// Data directory (portable/ and machine/ folders, as written by pro-import). On Windows
    /// the default is %ProgramData%\Vesper.
    #[arg(long, required = cfg!(not(windows)))]
    data: Option<PathBuf>,
    /// Write the log to this file instead of stderr (the previous run's log is kept as
    /// <FILE>.old). For a server with no console, such as the Windows scheduled task.
    #[arg(long, value_name = "FILE")]
    log_file: Option<PathBuf>,
    /// Control API listen address (HTTP + WebSocket).
    #[arg(long, default_value = "0.0.0.0:5555")]
    listen: SocketAddr,
    /// Require pairing for requests from this machine too (127.0.0.1 / ::1).
    #[arg(long)]
    no_trust_localhost: bool,
    /// Do not advertise the server over mDNS.
    #[arg(long)]
    no_mdns: bool,
    /// Where display outputs (monitors and projectors on this computer) come from: `window`
    /// (a borderless window per monitor on the desktop: Windows, Linux with a desktop
    /// session), `kms` (Linux without a desktop: driven directly), `none`, or `auto` (windows
    /// in a desktop session, else KMS).
    #[arg(long, value_enum, default_value = "auto")]
    displays: DisplaysArg,
    /// Do not list or drive the monitors attached to this box (same as `--displays none`).
    #[arg(long)]
    no_displays: bool,
    /// Write PNG snapshots of every screen into <dir>/<screen name>/.
    #[arg(long, value_name = "DIR")]
    png_dir: Option<PathBuf>,
    /// With --png-dir: write every Nth frame.
    #[arg(long, value_name = "N", default_value_t = 30)]
    png_every: u64,
    /// Open an ffplay preview window for this screen (name or ID). Needs a display.
    #[arg(long, value_name = "SCREEN")]
    preview: Option<String>,
    /// Also send every screen as an IDN Open Protocol source (compatible with NDI®) named
    /// "<prefix> <screen name>".
    #[arg(long, value_name = "TEXT")]
    idn_prefix: Option<String>,
    /// Ignore the machine's output bindings (only the CLI outputs run).
    #[arg(long)]
    no_bindings: bool,
    /// Force the software renderer (lavapipe).
    #[arg(long)]
    software: bool,
    /// Decode videos in software (no VA-API). `VESPER_VIDEO_DECODE=software` does the same.
    #[arg(long)]
    software_decode: bool,
    /// Extra font file or directory (repeatable).
    #[arg(long, value_name = "DIR")]
    fonts: Vec<PathBuf>,
    /// Built control UI to serve at http://<server>:5555/ (default: apps/control/dist under
    /// the working directory, when it exists).
    #[arg(long, value_name = "DIR")]
    ui_dir: Option<PathBuf>,
    /// The bundled Bible translations (`*.vbible`). Default: `scripture` next to the UI dir
    /// (`/usr/share/vesper/scripture` with the package), else `target/scripture` under the
    /// working directory (`scripts/fetch-scripture.sh`), when it exists.
    #[arg(long, value_name = "DIR")]
    scripture_dir: Option<PathBuf>,
    /// Audio output device (a sound card's name, on Linux also an ALSA name such as `hw:1,0`;
    /// `default`; `""` = none), instead of the machine's audio device configuration.
    #[arg(long, value_name = "NAME")]
    audio_device: Option<String>,
    /// Never run a Site Manager here (testing). A server that isn't in another server's site
    /// (`POST /api/v1/site/join`) is then in no site and can't pair clients.
    #[arg(long)]
    no_site_manager: bool,
    /// The site's discovery server port (IDN: 5959).
    #[arg(long, value_name = "PORT", default_value_t = vesper_server::site::SiteConfig::default().discovery_port)]
    site_discovery_port: u16,
    /// The Site Manager's HTTPS API port (IDN: 29202; IDN Voice takes the next one).
    #[arg(long, value_name = "PORT", default_value_t = vesper_server::site::SiteConfig::default().api_port)]
    site_api_port: u16,
    /// Largest media upload into the Codex, GiB.
    #[arg(long, value_name = "GIB", default_value_t = 64)]
    max_upload_gib: u64,
    /// Media probing and thumbnail threads (low priority).
    #[arg(long, value_name = "N", default_value_t = 1)]
    media_workers: usize,
    /// Never prepare new media automatically (conversion to the house format, loudness,
    /// poster frames); "Prepare" in Oriel still works.
    #[arg(long)]
    no_media_prep: bool,
    /// The UDP port of the box-to-Codex link (IDN's channel) on this box. Default: a free one
    /// the site's directory tells the other machines. Set a fixed one where firewalls sit
    /// between buildings (VPN routed, without NAT) and open it for UDP.
    #[arg(long, value_name = "PORT", default_value_t = 0)]
    codex_channel_port: u16,
    /// A show box's media cache cap, GiB (the media its playlists and bins use are kept).
    #[arg(long, value_name = "GIB", default_value_t = 32)]
    codex_cache_gib: u64,
    /// A settings snapshot of this box at least this often, hours (on the Codex box).
    #[arg(long, value_name = "HOURS", default_value_t = 24)]
    backup_interval_hours: u64,
    /// An update signing public key (minisign `.pub` file) the site's update channel accepts
    /// packages from, besides the compiled-in release key. Repeatable.
    #[arg(long, value_name = "FILE")]
    update_key: Vec<PathBuf>,
    /// LibreOffice's `soffice` for the slide pictures of PowerPoint imports (default: found
    /// on the PATH or where LibreOffice installs; also `VESPER_SOFFICE`).
    #[arg(long, value_name = "FILE", conflicts_with = "no_slide_pictures")]
    soffice: Option<PathBuf>,
    /// PowerPoint imports make no slide pictures (what isn't editable is approximated).
    #[arg(long)]
    no_slide_pictures: bool,
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("installing the SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => tracing::info!("Ctrl-C received, shutting down"),
            _ = term.recv() => tracing::info!("SIGTERM received, shutting down"),
        }
    }
    #[cfg(not(unix))]
    {
        // Without a console to send it (the Windows scheduled task), the handler may not
        // install: run until the process is ended rather than stop at once.
        if ctrl_c.await.is_err() {
            std::future::pending::<()>().await;
        }
        tracing::info!("Ctrl-C received, shutting down");
    }
}

/// The data directory when `--data` is not given (only allowed on Windows).
fn default_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("ProgramData")
            .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
            .join("Vesper")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/var/lib/vesper")
    }
}

/// Opens `--log-file` for a new run: the last run's log becomes `<file>.old`.
fn open_log_file(path: &std::path::Path) -> anyhow::Result<std::fs::File> {
    use anyhow::Context as _;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    if path.exists() {
        let mut old = path.as_os_str().to_owned();
        old.push(".old");
        let _ = std::fs::rename(path, old);
    }
    std::fs::File::create(path).with_context(|| format!("opening the log file {}", path.display()))
}

fn main() -> anyhow::Result<()> {
    // glibc's malloc for the C libraries (FFmpeg, whisper, drivers): see `heap`. Settings only,
    // no thread: first, so the child processes below (whisper's worker above all) have it too.
    vesper_server::heap::tune_glibc();
    // `vesper --idn-captions-worker …` (hidden): IDN's captions worker, whisper in a process of
    // its own below normal priority, started by this server's captions engine (see
    // `vesper_server::captions`). Before any thread exists: the priority it sets is
    // inherited only by threads started after it. Returns at once in every other case.
    idn_captions::worker::serve_if_asked();
    // `vesper media-convert --config FILE` (hidden): one conversion of media preparation, in a
    // child process the server starts (see `vesper_codex::prep::child`).
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref()
        == Some(std::ffi::OsStr::new(
            vesper_server::codex::prep::child::SUBCOMMAND,
        ))
    {
        std::process::exit(vesper_server::codex::prep::child::main(args.collect()));
    }
    // Before any thread exists (setting the environment is unsafe once threads run).
    vesper_server::vulkan_encode_off();
    run()
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    let a = Args::parse();
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,wgpu_core=warn,wgpu_hal=warn,naga=warn".into());
    match &a.log_file {
        Some(path) => {
            let file = open_log_file(path)?;
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .init();
        }
        None => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
            .init(),
    }
    if let Some(tuned) = vesper_server::heap::glibc_tuning() {
        tracing::info!("glibc malloc for the C libraries: {tuned}");
    }
    let mut renderer = render::RendererOptions {
        font_dirs: a.fonts.clone(),
        ..render::RendererOptions::default()
    };
    if a.software_decode {
        media::set_decode_mode(media::DecodeMode::Software);
    }
    tracing::info!(
        ffmpeg = %media::ffmpeg_version(),
        decode = ?media::decode_mode(),
        gpu = ?media::hardware_decoder(),
        "video decoding (IDN media, system FFmpeg)"
    );
    if a.software {
        renderer.adapter = render::AdapterPreference::Software;
    }
    let ui_dir = a.ui_dir.clone().or_else(|| {
        let d = PathBuf::from("apps/control/dist");
        d.join("index.html").is_file().then_some(d)
    });
    let scripture_dir = a.scripture_dir.clone().or_else(|| {
        let next_to_ui = ui_dir
            .as_ref()
            .and_then(|u| u.parent())
            .map(|p| p.join("scripture"));
        next_to_ui
            .into_iter()
            .chain([PathBuf::from("target/scripture")])
            .find(|d| d.is_dir())
    });
    let update_keys = a
        .update_key
        .iter()
        .map(|f| {
            std::fs::read_to_string(f)
                .map_err(|e| anyhow::anyhow!("can't read the update key {}: {e}", f.display()))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let config = ServerConfig {
        update_keys,
        // Vesper's releases on GitHub for the site's update channel (off in Site ▸ Updates).
        github_api: Some(vesper_releases::github::DEFAULT_API.to_string()),
        github_tick: std::time::Duration::from_secs(60),
        data_dir: a.data.unwrap_or_else(default_data_dir),
        listen: a.listen,
        dev: DevOutputs {
            png_dir: a.png_dir,
            png_every: a.png_every,
            preview: a.preview,
            idn_prefix: a.idn_prefix,
            no_bindings: a.no_bindings,
        },
        renderer,
        trust_localhost: !a.no_trust_localhost,
        mdns: !a.no_mdns,
        displays: {
            use vesper_server::displays::{Choice, source};
            match (a.no_displays, a.displays) {
                (true, _) | (_, DisplaysArg::None) => Some(source(Choice::None)),
                (_, DisplaysArg::Kms) => Some(source(Choice::Kms)),
                (_, DisplaysArg::Window) => Some(source(Choice::Window)),
                (_, DisplaysArg::Auto) => None,
            }
        },
        ui_dir: ui_dir.clone(),
        ws_ping: std::time::Duration::from_secs(20),
        mosaic_poll: vesper_server::mosaic::DEFAULT_POLL,
        mosaic_logo_settle: vesper_server::mosaic::LOGO_SETTLE,
        // Machine translation: OPUS-MT from its publisher, the real engine.
        translation: Default::default(),
        slide_pictures: match (a.no_slide_pictures, a.soffice) {
            (true, _) => pptx_import::render::Renderer::Off,
            (_, Some(p)) => pptx_import::render::Renderer::Program(p),
            _ => pptx_import::render::Renderer::Auto,
        },
        // This machine's IDN Node (which Node is this Vesper's; the site's server's routers
        // and logo push), when one runs here.
        mosaic_nodes: Some(std::sync::Arc::new(vesper_server::mosaic::IdnNodes::new(
            true,
        ))),
        audio_device: a.audio_device,
        scripture_dir,
        site: vesper_server::site::SiteConfig {
            available: !a.no_site_manager,
            discovery_port: a.site_discovery_port,
            api_port: a.site_api_port,
            announce: !a.no_mdns,
            ..Default::default()
        },
        // whisper in a worker process of this program (`vesper --idn-captions-worker`).
        captions: vesper_server::captions::CaptionsOptions::this_program(),
        codex: vesper_server::codex::CodexOptions {
            max_upload_bytes: a.max_upload_gib.saturating_mul(1 << 30),
            media_workers: a.media_workers.clamp(1, 8),
            media_prep: !a.no_media_prep,
            // Conversions in a child process of this program (`vesper media-convert`).
            converter: vesper_server::codex::prep::child::Converter::this_program(),
            channel_port: a.codex_channel_port,
            cache_bytes: a.codex_cache_gib.max(1).saturating_mul(1 << 30),
            backups: vesper_server::codex::backups::BackupOptions {
                interval: std::time::Duration::from_secs(
                    a.backup_interval_hours.clamp(1, 24 * 365) * 3600,
                ),
                ..Default::default()
            },
            ..Default::default()
        },
    };
    let server = start(config).await?;
    // Freed heap goes back to the system (glibc; see `heap`).
    #[cfg(feature = "mimalloc")]
    vesper_server::heap::rust_on_mimalloc();
    vesper_server::heap::start_trimmer();
    shutdown_signal().await;
    server.shutdown().await?;
    Ok(())
}
