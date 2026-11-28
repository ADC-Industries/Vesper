//! The Launcher and Vesper's releases on GitHub (`docs/LAUNCHER-PLAN.md`, "Releases from
//! GitHub").
//!
//! - **Direct mode.** A Launcher with no church server (not joined, no channel configured, or
//!   no report reached the site's server for [`GithubConfig::direct_after_secs`], a day by
//!   default) checks GitHub itself for its installed products and its own updates, every
//!   [`GithubConfig::interval_hours`] (failures back off from 5 minutes; a rate limit is waited
//!   out). What it finds shows as "Update available" like the channel's; the update downloads
//!   the package and its signature from GitHub and verifies them exactly as a channel package
//!   (the Launcher's keys, the trusted comment's product, version and platform), then installs
//!   with the same rules (health check, automatic rollback).
//! - **Add app.** [`offered`] filters the catalogue (the site's channel and GitHub) to the
//!   products this machine can run; [`template`] is how an added product is installed and
//!   checked; the install itself is a [`protocol::LauncherAction::Install`].

use crate::config::{InstallKind, ProductConfig};
use crate::launcher::{Launcher, LauncherError};
use protocol::{CatalogueProduct, GithubReleases, GithubSettings, ReleaseChannel};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vesper_releases::github::{self, GitHub, Release, Schedule};

/// Checking GitHub (the configuration file's `github`).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubConfig {
    /// Check at all (the off switch).
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The repository (`owner/name`).
    #[serde(default = "default_repository")]
    pub repository: String,
    /// `stable` or `beta`.
    #[serde(default)]
    pub channel: ReleaseChannel,
    /// Hours between checks.
    #[serde(default = "default_interval")]
    pub interval_hours: u32,
    /// GitHub's API (tests: a fake).
    #[serde(default = "default_api")]
    pub api: String,
    /// The church server counts as unreachable once no report reached it for this long: the
    /// Launcher then checks GitHub itself.
    #[serde(default = "default_direct_after")]
    pub direct_after_secs: u64,
}

fn yes() -> bool {
    true
}
fn default_repository() -> String {
    github::DEFAULT_REPOSITORY.into()
}
fn default_interval() -> u32 {
    6
}
fn default_api() -> String {
    github::DEFAULT_API.into()
}
fn default_direct_after() -> u64 {
    24 * 3600
}

impl Default for GithubConfig {
    fn default() -> Self {
        GithubConfig {
            enabled: true,
            repository: default_repository(),
            channel: ReleaseChannel::Stable,
            interval_hours: default_interval(),
            api: default_api(),
            direct_after_secs: default_direct_after(),
        }
    }
}

impl GithubConfig {
    fn interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.interval_hours.clamp(1, 168)) * 3600)
    }

    fn settings(&self) -> GithubSettings {
        GithubSettings {
            enabled: self.enabled,
            repository: self.repository.clone(),
            channel: self.channel,
            interval_hours: self.interval_hours,
        }
    }
}

/// The direct mode as the window shows it (`Overview::github`).
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubView {
    /// The Launcher checks GitHub itself now (no church server, or it has been unreachable).
    pub direct: bool,
    /// What the checks found.
    pub status: GithubReleases,
}

#[derive(Default)]
struct State {
    schedule: Schedule,
    status: GithubReleases,
    client: Option<Arc<GitHub>>,
    last_error: Option<String>,
    /// Newer versions on GitHub for this machine's products.
    available: BTreeMap<String, String>,
}

/// The Launcher's GitHub client and what it found.
#[derive(Default)]
pub struct Direct {
    state: Mutex<State>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Direct {
    /// The client (made once).
    pub fn client(&self, cfg: &GithubConfig) -> Result<Arc<GitHub>, LauncherError> {
        let mut s = lock(&self.state);
        if let Some(c) = &s.client {
            return Ok(c.clone());
        }
        let ua = format!("vesper-launcher/{}", crate::VERSION);
        let c = Arc::new(
            GitHub::new(&cfg.api, &cfg.repository, &ua)
                .map_err(|e| LauncherError::Invalid(e.to_string()))?,
        );
        s.client = Some(c.clone());
        Ok(c)
    }

    /// Newer versions on GitHub, by product (as of the last check in direct mode).
    pub fn available(&self) -> BTreeMap<String, String> {
        lock(&self.state).available.clone()
    }

    /// What the window shows.
    pub fn view(&self, cfg: &GithubConfig, direct: bool) -> GithubView {
        let s = lock(&self.state);
        let mut status = s.status.clone();
        status.settings = cfg.settings();
        GithubView { direct, status }
    }
}

/// How an added product is installed and checked on `platform` (`None`: it can't be added
/// there by the Launcher).
pub fn template(product: &str, platform: &str) -> Option<ProductConfig> {
    let linux = platform.starts_with("linux-");
    let windows = platform.starts_with("windows-");
    let (install, health) = match product {
        "vesper" if linux => (
            InstallKind::Deb {
                units: vec!["vesper.service".into(), "vesper-display.service".into()],
            },
            "http://127.0.0.1:5555/api/v1/server",
        ),
        "vesper" if windows => (
            InstallKind::Nsis {
                args: Vec::new(),
                task: Some("Vesper".into()),
            },
            "http://127.0.0.1:5555/api/v1/server",
        ),
        // Programs, not services: up while dpkg has them.
        "vesper-idn-node" if linux => (
            InstallKind::Deb {
                units: vec!["idn-node.service".into()],
            },
            "",
        ),
        "vesper-oriel" if linux => (InstallKind::Deb { units: Vec::new() }, ""),
        _ => return None,
    };
    Some(ProductConfig {
        product: product.into(),
        name: github::product_name(product),
        install,
        health_url: health.into(),
        log_file: None,
        is_self: false,
    })
}

/// Of a catalogue, what can be added to a machine on `platform` with `roles` that has the
/// products `have` installed: a product it has no package for, that it already has, the
/// Launcher itself and its tray (they come with the Launcher), the Vesper server on a display
/// box (a display box runs no Vesper server), and what the Launcher can't install there
/// ([`template`]: Oriel on Windows, a program with no health check whose installed version
/// the Launcher can't read yet) are left out.
pub fn offered(
    cat: &[CatalogueProduct],
    platform: &str,
    roles: &[String],
    have: &[String],
) -> Vec<CatalogueProduct> {
    cat.iter()
        .filter(|c| c.platforms.iter().any(|p| p == platform))
        .filter(|c| !have.contains(&c.product))
        .filter(|c| template(&c.product, platform).is_some())
        .filter(|c| !(c.product == protocol::PRODUCT_ID && roles.iter().any(|r| r == "display")))
        .cloned()
        .collect()
}

impl Launcher {
    /// Whether the Launcher checks GitHub itself now: checking is on, and there is no church
    /// server (not joined, no channel configured) or none was reached for
    /// [`GithubConfig::direct_after_secs`].
    pub fn github_direct(&self) -> bool {
        let g = &self.config().github;
        if !g.enabled {
            return false;
        }
        if self.channel().is_none() {
            return true;
        }
        let after = Duration::from_secs(g.direct_after_secs);
        self.last_report_ok().unwrap_or(self.started()).elapsed() >= after
    }

    /// Checks GitHub when in direct mode and a check is due (in the background of the
    /// report loop; blocking work on its own thread).
    pub async fn check_github(self: &Arc<Self>) {
        let cfg = self.config().github.clone();
        if !self.github_direct() {
            return;
        }
        {
            let mut s = lock(&self.direct.state);
            if s.status.checking || !s.schedule.due(cfg.interval()) {
                return;
            }
            s.status.checking = true;
        }
        let l = self.clone();
        let r = tokio::task::spawn_blocking(move || l.check_github_now())
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        let mut s = lock(&self.direct.state);
        s.status.checking = false;
        s.status.last_check = Some(crate::launcher::now());
        s.schedule.attempted(r.is_ok());
        match r {
            Ok(available) => {
                s.status.last_success = s.status.last_check.clone();
                s.status.error = None;
                if s.last_error.take().is_some() {
                    tracing::info!("GitHub can be reached again");
                }
                for (p, v) in &available {
                    tracing::info!("GitHub has {p} {v} for this machine");
                }
                s.available = available;
            }
            Err(e) => {
                if s.last_error.as_deref() != Some(e.as_str()) {
                    tracing::warn!("checking GitHub for updates: {e}");
                }
                s.last_error = Some(e.clone());
                s.status.error = Some(e);
            }
        }
        let wait = s.schedule.wait(cfg.interval());
        s.status.next_check = Some(
            (chrono::Utc::now() + chrono::Duration::from_std(wait).unwrap_or_default())
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
    }

    /// One check: the newest release in the channel of each of this machine's products, when
    /// newer than the installed version.
    fn check_github_now(&self) -> Result<BTreeMap<String, String>, String> {
        let cfg = &self.config().github;
        let gh = self.direct.client(cfg).map_err(|e| e.to_string())?;
        let releases = gh.releases().map_err(|e| e.to_string())?;
        let mut out = BTreeMap::new();
        for p in self.products() {
            if github::line_of(&p.product).is_none() {
                continue;
            }
            let Some(c) = gh
                .latest(&releases, &p.product, self.platform(), cfg.channel)
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            let newer = self.installed(&p.product).version.is_none_or(|cur| {
                vesper_releases::package::compare_versions(&c.version, &cur).is_gt()
            });
            if newer && self.installed(&p.product).version.is_some() {
                out.insert(p.product.clone(), c.version.clone());
            }
        }
        Ok(out)
    }

    /// The releases on GitHub (blocking).
    pub fn github_releases(&self) -> Result<(Arc<GitHub>, Vec<Release>), LauncherError> {
        let cfg = &self.config().github;
        if !cfg.enabled {
            return Err(LauncherError::Invalid(
                "checking GitHub for releases is off on this machine".into(),
            ));
        }
        let gh = self.direct.client(cfg)?;
        let rs = gh
            .releases()
            .map_err(|e| LauncherError::Io(e.to_string()))?;
        Ok((gh, rs))
    }

    /// Downloads `product` `version` for this machine from GitHub into the Launcher's store,
    /// verified (blocking). The version as the package is signed (the IDN Node's release
    /// `0.3.8-beta18` is signed `0.3.8~beta18`).
    pub fn fetch_from_github(&self, product: &str, version: &str) -> Result<String, LauncherError> {
        let (gh, rs) = self.github_releases()?;
        let c = gh
            .find(&rs, product, version, self.platform())
            .map_err(|e| LauncherError::Io(e.to_string()))?
            .ok_or_else(|| {
                LauncherError::Invalid(format!(
                    "GitHub has no {product} {version} for {}",
                    self.platform()
                ))
            })?;
        let staging = self.config().state_dir.join(".github");
        let d = gh
            .download(&c, &staging, self.keys())
            .map_err(|e| match e {
                github::CheckError::Refused(m) => LauncherError::Signature(m),
                e => LauncherError::Io(e.to_string()),
            })?;
        let r = self
            .add_package(&d.path, &d.file_name, &d.signature)
            .map(|v| v.meta.version);
        let _ = std::fs::remove_file(&d.path);
        r
    }

    /// The products that can be added here, with their newest versions: the site's channel
    /// (when there is one) and GitHub (when checking it is on) (blocking).
    pub fn catalogue(&self) -> Result<Vec<CatalogueProduct>, LauncherError> {
        let mut problems = Vec::new();
        let mut cat = Vec::new();
        if self.config().github.enabled {
            match self.github_releases().and_then(|(gh, rs)| {
                github::catalogue(&gh, &rs).map_err(|e| LauncherError::Io(e.to_string()))
            }) {
                Ok(c) => cat = c,
                Err(e) => problems.push(format!("GitHub: {e}")),
            }
        }
        if let Some(c) = self.channel() {
            match c.packages() {
                Ok(p) => github::merge_channel(&mut cat, &p),
                Err(e) => problems.push(format!("the site's server: {e}")),
            }
        }
        if cat.is_empty() && !problems.is_empty() {
            return Err(LauncherError::Io(problems.join("; ")));
        }
        let have: Vec<String> = self
            .products()
            .iter()
            .filter(|p| self.installed(&p.product).version.is_some())
            .map(|p| p.product.clone())
            .collect();
        let mut v = offered(&cat, self.platform(), &self.roles(), &have);
        v.sort_by_key(|c| c.name.to_lowercase());
        Ok(v)
    }
}

/// A product being added: the Launcher manages it from now (taken back if its install fails).
pub(crate) fn prepare_add(l: &Launcher, product: &str) -> Result<bool, LauncherError> {
    if l.product(product).is_ok() {
        if l.installed(product).version.is_some() {
            return Err(LauncherError::Invalid(format!(
                "{} is already installed here",
                github::product_name(product)
            )));
        }
        return Ok(false);
    }
    let t = template(product, l.platform()).ok_or_else(|| {
        LauncherError::Invalid(format!(
            "{} can't be added to this machine ({})",
            github::product_name(product),
            l.platform()
        ))
    })?;
    if product == protocol::PRODUCT_ID && l.roles().iter().any(|r| r == "display") {
        return Err(LauncherError::Invalid(
            "a display box runs no Vesper server: change its role first".into(),
        ));
    }
    l.add_product(t)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(product: &str, platforms: &[&str]) -> CatalogueProduct {
        CatalogueProduct {
            product: product.into(),
            name: github::product_name(product),
            stable: Some("0.2.0".into()),
            beta: None,
            platforms: platforms.iter().map(|s| s.to_string()).collect(),
            sources: vec!["github".into()],
        }
    }

    #[test]
    fn only_what_the_machine_can_run_is_offered() {
        let cat = vec![
            entry("vesper", &["linux-x86_64", "windows-x86_64"]),
            entry("vesper-launcher", &["linux-x86_64", "windows-x86_64"]),
            entry("vesper-launcher-tray", &["linux-x86_64"]),
            entry("vesper-idn-node", &["linux-x86_64"]),
            entry(
                "vesper-oriel",
                &["linux-x86_64", "linux-x86_64-appimage", "windows-x86_64"],
            ),
        ];
        let ids = |v: Vec<CatalogueProduct>| -> Vec<String> {
            v.into_iter().map(|c| c.product).collect()
        };
        let none: Vec<String> = Vec::new();
        assert_eq!(
            ids(offered(&cat, "linux-x86_64", &none, &none)),
            ["vesper", "vesper-idn-node", "vesper-oriel"]
        );
        // Windows: the server only (the Node comes with the Launcher, Oriel installs per user).
        assert_eq!(
            ids(offered(&cat, "windows-x86_64", &none, &none)),
            ["vesper"]
        );
        // A display box: no Vesper server; what is installed isn't offered again.
        let display = vec!["display".to_string()];
        let have = vec!["vesper-idn-node".to_string()];
        assert_eq!(
            ids(offered(&cat, "linux-x86_64", &display, &have)),
            ["vesper-oriel"]
        );
        assert!(offered(&cat, "linux-aarch64", &none, &none).is_empty());
    }

    #[test]
    fn added_products_are_installed_as_their_packages_install_them() {
        let p = template("vesper", "linux-x86_64").unwrap();
        assert!(matches!(&p.install, InstallKind::Deb { units } if units[0] == "vesper.service"));
        assert_eq!(p.health_url, "http://127.0.0.1:5555/api/v1/server");
        let p = template("vesper", "windows-x86_64").unwrap();
        assert!(matches!(&p.install, InstallKind::Nsis { task: Some(t), .. } if t == "Vesper"));
        assert_eq!(
            template("vesper-oriel", "linux-x86_64").unwrap().health_url,
            ""
        );
        assert!(template("vesper-oriel", "windows-x86_64").is_none());
        assert!(template("vesper-launcher", "linux-x86_64").is_none());
    }
}
