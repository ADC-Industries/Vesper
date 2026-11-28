//! # Vesper Launcher
//!
//! The small always-running service on every Vesper machine (`docs/LAUNCHER-PLAN.md`): it
//! installs and updates the Vesper products there from **signed packages** ([`sign`],
//! [`package`]), rolls an update back automatically when the new version doesn't come up
//! healthy ([`launcher`], [`health`]), restarts what it runs after a crash ([`install`]),
//! answers a local API for the tray app ([`api`]) and reports to the site's update channel on
//! the site's Vesper server, where Oriel's Site ▸ Updates shows every machine ([`channel`]).
//!
//! - [`config`]: the configuration file (products, install kind, keys, channel).
//! - [`sign`]: Ed25519 signatures in minisign's format; the compiled-in release keys.
//! - [`package`]: a package's signed product, version and platform.
//! - [`launcher`]: manifest, package store, update and rollback, health, report.
//! - [`install`]: apt (`apt-get install ./<pkg>.deb`), the silent NSIS installer, or a supervised folder install.
//! - [`health`]: product health checks and the machine's numbers.
//! - [`api`]: the local API on loopback.
//! - [`channel`]: the site's update channel client and the report loop.
//! - [`setup`]: the first-run wizard's state, joining the site, this machine's Launcher key.
//! - [`roles`]: acting on the machine's roles (the Vesper server, a display box's IDN Node).
//! - [`idn_node`]: the bundled IDN Node stays Vesper's to update (its marker, the service guard).
//! - [`diagnostics`]: the bundled `idn doctor` in plain language, and a report for support.
//! - [`github`]: Vesper's releases on GitHub when there is no church server; "Add app".
//! - [`selfupdate`]: the Launcher's own update, through a helper that outlives its restart.
//! - [`http`]: a minimal HTTP/1.0 client for the LAN.
//! - [`cli`]: `vesper-launcher join` (a headless join from the command line).

#![warn(missing_docs)]

pub mod api;
pub mod channel;
pub mod cli;
pub mod config;
pub mod diagnostics;
pub mod github;
pub mod health;
pub mod http;
pub mod idn_node;
pub mod install;
pub mod launcher;
pub mod roles;
pub mod selfupdate;
pub mod setup;

/// Package signatures and what a package is (from `vesper-releases`, shared with the server).
pub use vesper_releases::{package, sign};

pub use config::Config;
pub use launcher::Launcher;

/// The Launcher's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
