//! `vesper-launcher`: the Vesper Launcher service (see the library's docs and
//! `docs/LAUNCHER-PLAN.md`).

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use vesper_launcher::{Config, Launcher};

/// Vesper Launcher: installs, updates and monitors the Vesper products on this machine.
#[derive(Parser)]
#[command(version)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    /// The configuration file (JSON).
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Verify a package against the configured keys and print what it is, then exit.
    #[arg(long, value_name = "PACKAGE", requires = "signature")]
    verify: Option<PathBuf>,
    /// The package's signature (`.minisig`) for --verify.
    #[arg(long, value_name = "FILE")]
    signature: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Joins this machine to the site (a headless box): the running Launcher service pairs
    /// with the site's server, with its pairing code or an admin's approval in Oriel.
    Join {
        /// The pairing code the site's server shows (Oriel ▸ Site); without it, an admin
        /// approves this machine.
        code: Option<String>,
        /// The site's server's name or address (default: the only Vesper server found on the
        /// network).
        #[arg(long, value_name = "HOST")]
        server: Option<String>,
        /// The machine's role, set first: `presentation` or `display` (a Mosaic display box).
        #[arg(long, value_parser = ["presentation", "display"])]
        role: Option<String>,
        /// Don't wait for an admin's approval.
        #[arg(long)]
        no_wait: bool,
        /// The Launcher service's local API.
        #[arg(long, value_name = "ADDRESS", default_value = vesper_launcher::config::DEFAULT_LISTEN)]
        api: String,
    },
    /// The self-update helper (started by the Launcher itself).
    #[command(hide = true)]
    SelfUpdate {
        /// Its request file.
        #[arg(long, value_name = "FILE")]
        request: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();
    match args.command {
        Some(Command::Join {
            code,
            server,
            role,
            no_wait,
            api,
        }) => {
            let a = vesper_launcher::cli::JoinArgs {
                api,
                server,
                code,
                role,
                wait: !no_wait,
                wait_for: std::time::Duration::from_secs(11 * 60),
            };
            return tokio::task::spawn_blocking(move || {
                vesper_launcher::cli::join(&a, &mut std::io::stdout())
            })
            .await?;
        }
        Some(Command::SelfUpdate { request }) => {
            let o = tokio::task::spawn_blocking(move || {
                vesper_launcher::selfupdate::run_helper(&request)
            })
            .await??;
            tracing::info!(
                "the Launcher's update to {}: {:?} {}",
                o.to,
                o.result,
                o.message
            );
            return Ok(());
        }
        None => {}
    }
    let path = args
        .config
        .unwrap_or_else(vesper_launcher::config::default_config_file);
    let cfg = Config::load(&path)?;
    let launcher = Launcher::open(cfg)?;
    if let (Some(pkg), Some(sig)) = (args.verify, args.signature) {
        let d = vesper_launcher::sign::Hasher::file(&pkg)?;
        let sig = std::fs::read_to_string(sig)?;
        let v = vesper_launcher::package::verify(launcher.keys(), &d, &sig)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        println!(
            "{} {} for {}, signed with key {}",
            v.meta.product, v.meta.version, v.meta.platform, v.key_id
        );
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind(launcher.config().listen).await?;
    tracing::info!(
        "Vesper Launcher {} on {}",
        vesper_launcher::VERSION,
        listener.local_addr()?
    );
    let app = vesper_launcher::api::router(launcher.clone());
    // The local API answers (and SIGTERM is heard) while an interrupted update is finished,
    // which can take a while; then the reports and health checks start.
    let l = launcher.clone();
    let reporter = tokio::spawn(async move {
        l.start_products().await;
        vesper_launcher::channel::run(l).await
    });
    let app = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
    let serve = axum::serve(listener, app).with_graceful_shutdown(async {
        shutdown_signal().await;
    });
    serve.await?;
    reporter.abort();
    launcher.shutdown().await;
    Ok(())
}

/// Ctrl+C, or SIGTERM (systemd stopping the service).
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("a SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
