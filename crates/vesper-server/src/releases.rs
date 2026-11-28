//! Vesper's releases on GitHub, brought into the site's update channel ([`crate::updates`];
//! `docs/PACKAGING.md`, "Releases").
//!
//! Every [`GithubSettings::interval_hours`] (6 by default; failures back off from 5 minutes,
//! a rate limit is waited out) the server lists the releases of the configured repository
//! (unauthenticated API, with the last list's ETag), and for each product and platform the
//! site has (what its Launchers report, the server itself, and what "Add app" asked for)
//! takes the newest release in the channel (stable, or beta with pre-releases). A package the
//! channel doesn't have yet is downloaded with its signature and verified with the channel's
//! update keys (the trusted comment must name the product, version and platform), then added
//! as if an admin uploaded it, with `github` as its source. Installing stays an admin's action
//! (or a machine's install window), with the Launcher's rules (never while live, rollback).
//! Of the packages that came from GitHub, the two newest per product and platform are kept.
//!
//! The settings live in `<data>/updates/github.json`. The server checks only with an API
//! address ([`crate::ServerConfig::github_api`]: the `vesper` binary passes GitHub's; tests a
//! fake or none, so a test server never reaches the internet).

use crate::updates::Updates;
use protocol::{
    ApiError, CatalogueProduct, ErrorCode, GithubReleases, GithubSettings, UpdatePackage,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vesper_releases::github::{self, CheckError, GitHub, Schedule};

/// The most packages listed as brought in.
const MAX_ADDED: usize = 20;
/// Packages from GitHub kept per product and platform.
const KEEP: usize = 2;

/// A product and platform "Add app" asked for (kept until a package for it is in the channel).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wanted {
    product: String,
    platform: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    #[serde(default)]
    settings: GithubSettings,
    #[serde(default)]
    wanted: Vec<Wanted>,
}

#[derive(Default)]
struct State {
    saved: Saved,
    status: GithubReleases,
    schedule: Schedule,
    client: Option<Arc<GitHub>>,
    catalogue: Vec<CatalogueProduct>,
    last_error: Option<String>,
    /// Checks asked for ("Check now", new settings, "Add app") and the last one a check that
    /// ended had seen: `checking` stays on until a check started after the last request ends.
    requested: u64,
    done: u64,
}

impl State {
    fn ask(&mut self) {
        self.requested += 1;
        self.status.checking = true;
        self.schedule.reset();
    }
}

/// The server's GitHub release checker.
pub struct Releases {
    file: PathBuf,
    api: Option<String>,
    platform: String,
    state: Mutex<State>,
    wake: tokio::sync::Notify,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn interval(s: &GithubSettings) -> Duration {
    Duration::from_secs(u64::from(s.interval_hours.clamp(1, 168)) * 3600)
}

impl Releases {
    /// Reads `<data>/updates/github.json`; checks GitHub at `api` (`None`: never).
    pub fn open(data_dir: &std::path::Path, api: Option<String>) -> Self {
        let file = data_dir.join("updates").join("github.json");
        let saved: Saved = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Releases {
            file,
            api,
            platform: vesper_launcher::package::this_platform(),
            state: Mutex::new(State {
                saved,
                ..State::default()
            }),
            wake: tokio::sync::Notify::new(),
        }
    }

    fn save(&self, saved: &Saved) -> Result<(), ApiError> {
        let write = || -> std::io::Result<()> {
            if let Some(d) = self.file.parent() {
                std::fs::create_dir_all(d)?;
            }
            let tmp = self.file.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(saved)?)?;
            std::fs::rename(&tmp, &self.file)
        };
        write().map_err(|e| {
            ApiError::new(
                ErrorCode::Internal,
                format!("can't save the GitHub settings: {e}"),
            )
        })
    }

    /// What the checks found, with the settings.
    pub fn status(&self) -> GithubReleases {
        let s = lock(&self.state);
        let mut st = s.status.clone();
        st.settings = s.saved.settings.clone();
        if !st.settings.enabled || self.api.is_none() {
            st.next_check = None;
        }
        st
    }

    /// Changes the settings (`400`: a repository that isn't `owner/name`, an interval outside
    /// 1–168 hours) and checks at once when on.
    pub fn set(&self, settings: GithubSettings) -> Result<GithubReleases, ApiError> {
        if !github::valid_repository(&settings.repository) {
            return Err(ApiError::new(
                ErrorCode::BadRequest,
                "the repository is owner/name, as on GitHub",
            ));
        }
        if !(1..=168).contains(&settings.interval_hours) {
            return Err(ApiError::new(
                ErrorCode::BadRequest,
                "check every 1 to 168 hours",
            ));
        }
        {
            let mut s = lock(&self.state);
            let mut saved = s.saved.clone();
            if saved.settings.repository != settings.repository {
                s.client = None;
                s.catalogue.clear();
            }
            saved.settings = settings;
            self.save(&saved)?;
            let on = saved.settings.enabled;
            s.saved = saved;
            s.status.error = None;
            if on && self.api.is_some() {
                s.ask();
            }
        }
        self.wake.notify_one();
        Ok(self.status())
    }

    /// Checks at once (`409` when checking is off).
    pub fn check_now(&self) -> Result<GithubReleases, ApiError> {
        if !lock(&self.state).saved.settings.enabled {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "checking GitHub is off: turn it on first",
            ));
        }
        if self.api.is_some() {
            lock(&self.state).ask();
        }
        self.wake.notify_one();
        Ok(self.status())
    }

    /// "Add app" asked for `product` on `platform`: looked for from the next check (at once).
    pub fn want(&self, product: &str, platform: &str) {
        let w = Wanted {
            product: product.into(),
            platform: platform.into(),
        };
        {
            let mut s = lock(&self.state);
            if !s.saved.wanted.contains(&w) {
                let mut saved = s.saved.clone();
                saved.wanted.push(w);
                if let Err(e) = self.save(&saved) {
                    tracing::warn!("{}", e.message);
                }
                s.saved = saved;
            }
            if !s.saved.settings.enabled || self.api.is_none() {
                return;
            }
            s.ask();
        }
        self.wake.notify_one();
    }

    /// The products GitHub offers (as of the last check), with the channel's.
    pub fn catalogue(&self, packages: &[UpdatePackage]) -> Vec<CatalogueProduct> {
        let mut cat = lock(&self.state).catalogue.clone();
        github::merge_channel(&mut cat, packages);
        cat
    }

    /// Checks when due, until `stop`; wakes up every `tick` and when the settings change.
    pub async fn run(
        self: Arc<Self>,
        shared: Arc<crate::Shared>,
        tick: Duration,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) {
        if self.api.is_none() {
            return;
        }
        loop {
            let due = {
                let mut s = lock(&self.state);
                let due = s.saved.settings.enabled
                    && (s.requested > s.done || s.schedule.due(interval(&s.saved.settings)));
                if due {
                    s.status.checking = true;
                }
                due.then_some(s.requested)
            };
            if let Some(req) = due {
                let shared = shared.clone();
                let me = self.clone();
                let r = tokio::task::spawn_blocking(move || me.check(&shared.updates))
                    .await
                    .unwrap_or_else(|e| Err(CheckError::Other(e.to_string())));
                self.finish(r, req);
            }
            tokio::select! {
                _ = tokio::time::sleep(tick) => {}
                _ = self.wake.notified() => {}
                _ = stop.wait_for(|s| *s) => break,
            }
        }
    }

    fn finish(&self, r: Result<Vec<String>, CheckError>, req: u64) {
        let mut s = lock(&self.state);
        s.done = req;
        s.status.checking = s.requested > s.done;
        s.status.last_check = Some(now());
        let ok = r.is_ok();
        s.schedule.attempted(ok);
        match r {
            Ok(added) => {
                s.status.last_success = s.status.last_check.clone();
                s.status.error = None;
                if s.last_error.take().is_some() {
                    tracing::info!("GitHub can be reached again");
                }
                for a in added.into_iter().rev() {
                    s.status.added.insert(0, a);
                }
                s.status.added.truncate(MAX_ADDED);
            }
            Err(e) => {
                let m = e.to_string();
                // Logged once per kind of problem, not at every try.
                if s.last_error.as_deref() != Some(m.as_str()) {
                    tracing::warn!("checking GitHub for Vesper releases: {m}");
                }
                s.last_error = Some(m.clone());
                s.status.error = Some(m);
            }
        }
        let wait = s.schedule.wait(interval(&s.saved.settings));
        s.status.next_check = Some(
            (chrono::Utc::now() + chrono::Duration::from_std(wait).unwrap_or_default())
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
    }

    /// The products and platforms to look for.
    fn targets(&self, updates: &Updates) -> Vec<(String, String)> {
        let mut t: Vec<(String, String)> =
            vec![(protocol::PRODUCT_ID.into(), self.platform.clone())];
        for e in updates.launchers() {
            for p in &e.report.products {
                t.push((p.product.clone(), e.report.platform.clone()));
            }
        }
        for w in &lock(&self.state).saved.wanted {
            t.push((w.product.clone(), w.platform.clone()));
        }
        t.retain(|(p, _)| github::line_of(p).is_some());
        t.sort();
        t.dedup();
        t
    }

    /// One check (blocking): the packages brought in (`<product> <version> for <platform>`).
    pub fn check(&self, updates: &Updates) -> Result<Vec<String>, CheckError> {
        let (api, settings) = {
            let s = lock(&self.state);
            (self.api.clone(), s.saved.settings.clone())
        };
        let Some(api) = api else {
            return Ok(Vec::new());
        };
        let gh = {
            let mut s = lock(&self.state);
            match &s.client {
                Some(c) => c.clone(),
                None => {
                    let ua = format!("vesper/{}", env!("CARGO_PKG_VERSION"));
                    let c = Arc::new(GitHub::new(&api, &settings.repository, &ua)?);
                    s.client = Some(c.clone());
                    c
                }
            }
        };
        let releases = gh.releases()?;
        match github::catalogue(&gh, &releases) {
            Ok(c) => lock(&self.state).catalogue = c,
            Err(e) => tracing::warn!("GitHub's catalogue: {e}"),
        }
        let mut added = Vec::new();
        let mut problems = Vec::new();
        for (product, platform) in self.targets(updates) {
            let c = match gh.latest(&releases, &product, &platform, settings.channel) {
                Ok(Some(c)) => c,
                Ok(None) => continue,
                Err(e @ CheckError::RateLimited { .. }) => return Err(e),
                Err(e) => {
                    problems.push(e.to_string());
                    continue;
                }
            };
            let have = updates.list().iter().any(|p| {
                p.product == product
                    && p.platform == platform
                    && github::same_version(&p.version, &c.version)
            });
            if have {
                self.got(&product, &platform);
                continue;
            }
            let staging = updates
                .staging()
                .map_err(|e| CheckError::Other(format!("can't stage a download: {e}")))?;
            match gh.download(&c, &staging, updates.keys()) {
                Ok(d) => {
                    let r = updates.add_from(
                        &d.path,
                        &d.verified.digests,
                        &d.file_name,
                        &d.signature,
                        Some("github"),
                    );
                    let _ = std::fs::remove_file(&d.path);
                    match r {
                        Ok(p) => {
                            tracing::info!(
                                "{} {} for {}: brought in from GitHub ({})",
                                p.product,
                                p.version,
                                p.platform,
                                c.tag
                            );
                            added.push(format!("{} {} for {}", p.product, p.version, p.platform));
                            self.got(&product, &platform);
                            self.prune(updates, &product, &platform);
                        }
                        Err(e) => problems.push(e.message),
                    }
                }
                Err(e @ CheckError::RateLimited { .. }) => return Err(e),
                Err(e) => {
                    tracing::warn!("{product} {} for {platform} from GitHub: {e}", c.version);
                    problems.push(format!("{product} {} for {platform}: {e}", c.version));
                }
            }
        }
        if problems.is_empty() {
            Ok(added)
        } else {
            {
                let mut s = lock(&self.state);
                for a in added.into_iter().rev() {
                    s.status.added.insert(0, a);
                }
                s.status.added.truncate(MAX_ADDED);
            }
            Err(CheckError::Other(problems.join("; ")))
        }
    }

    /// A wanted product is in the channel now.
    fn got(&self, product: &str, platform: &str) {
        let mut s = lock(&self.state);
        if let Some(i) = s
            .saved
            .wanted
            .iter()
            .position(|w| w.product == product && w.platform == platform)
        {
            let mut saved = s.saved.clone();
            saved.wanted.remove(i);
            if let Err(e) = self.save(&saved) {
                tracing::warn!("{}", e.message);
            }
            s.saved = saved;
        }
    }

    /// Keeps the [`KEEP`] newest packages from GitHub of a product and platform.
    fn prune(&self, updates: &Updates, product: &str, platform: &str) {
        // `list` is newest first per product and platform.
        let old: Vec<String> = updates
            .list()
            .into_iter()
            .filter(|p| {
                p.product == product
                    && p.platform == platform
                    && p.source.as_deref() == Some("github")
            })
            .skip(KEEP)
            .map(|p| p.id)
            .collect();
        for id in old {
            updates.delete(&id);
        }
    }
}
