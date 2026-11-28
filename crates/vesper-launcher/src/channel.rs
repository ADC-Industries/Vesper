//! The site's update channel (on the site's Vesper server, `/api/v1/updates/…`): the Launcher
//! reports its status there and gets back the actions queued for it in Oriel, and downloads
//! the packages it is told to install. A pull: the machine needs no inbound port.

use crate::config::ChannelConfig;
use crate::launcher::{Launcher, LauncherError};
use protocol::{LauncherAction, LauncherActions, LauncherReport, UpdateOutcome, UpdatePackage};
use std::sync::Arc;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);
/// A download may take a while on a slow LAN (per read, not in total).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// A client of the site's channel.
#[derive(Clone)]
pub struct Channel {
    /// The site's server.
    pub url: String,
    token: Option<String>,
}

impl Channel {
    /// For a configured channel.
    pub fn new(c: &ChannelConfig) -> Self {
        Channel {
            url: c.url.trim_end_matches('/').to_string(),
            token: c.token.clone(),
        }
    }

    fn auth(&self) -> Option<String> {
        self.token.as_ref().map(|t| format!("Bearer {t}"))
    }

    fn call(&self, method: &str, path: &str, body: Option<&[u8]>) -> anyhow::Result<Vec<u8>> {
        let auth = self.auth();
        let mut headers = vec![("Content-Type", "application/json")];
        if let Some(a) = &auth {
            headers.push(("Authorization", a));
        }
        let r = crate::http::request(
            method,
            &format!("{}/api/v1{path}", self.url),
            &headers,
            body,
            TIMEOUT,
        )?;
        anyhow::ensure!(
            (200..300).contains(&r.status),
            "the site's server answered {} to {method} {path}: {}",
            r.status,
            String::from_utf8_lossy(&r.body).trim()
        );
        Ok(r.body)
    }

    /// Reports; the actions queued for this machine.
    pub fn report(&self, r: &LauncherReport) -> anyhow::Result<Vec<LauncherAction>> {
        let body = serde_json::to_vec(r)?;
        let b = self.call(
            "PUT",
            &format!("/updates/launchers/{}", r.machine_id),
            Some(&body),
        )?;
        Ok(serde_json::from_slice::<LauncherActions>(&b)?.actions)
    }

    /// An IDN join code for this machine's IDN Node, made by the site's server
    /// (`POST /updates/launchers/{machine}/idn-join`, with this machine's Launcher key).
    pub fn idn_join_code(&self, machine: &str) -> anyhow::Result<protocol::LauncherJoinCode> {
        let b = self.call(
            "POST",
            &format!("/updates/launchers/{machine}/idn-join"),
            Some(b"{}"),
        )?;
        Ok(serde_json::from_slice(&b)?)
    }

    /// The channel's packages.
    pub fn packages(&self) -> anyhow::Result<Vec<UpdatePackage>> {
        Ok(serde_json::from_slice(&self.call(
            "GET",
            "/updates/packages",
            None,
        )?)?)
    }

    /// Downloads a package and its signature into the Launcher's store (verified there).
    pub fn fetch(&self, l: &Launcher, p: &UpdatePackage) -> Result<(), LauncherError> {
        let io = |e: anyhow::Error| LauncherError::Io(format!("download failed: {e}"));
        let sig = self
            .call(
                "GET",
                &format!("/updates/packages/{}/signature", p.id),
                None,
            )
            .map_err(io)?;
        let sig = String::from_utf8(sig).map_err(|e| LauncherError::Io(e.to_string()))?;
        let tmp = l
            .config()
            .state_dir
            .join(format!(".download-{}", uuid::Uuid::new_v4()));
        let auth = self.auth();
        let headers: Vec<(&str, &str)> =
            auth.iter().map(|a| ("Authorization", a.as_str())).collect();
        let r = crate::http::download(
            &format!("{}/api/v1/updates/packages/{}/file", self.url, p.id),
            &headers,
            &tmp,
            DOWNLOAD_TIMEOUT,
        )
        .map_err(io)
        .and_then(|_| l.add_package(&tmp, &p.file_name, &sig).map(|_| ()));
        let _ = std::fs::remove_file(&tmp);
        r
    }
}

/// Runs an action: an update installs the stored package, downloading it from the channel
/// first when it isn't stored yet. The package of the version installed now (for an update)
/// or of the previous one (a rollback) is downloaded too when the channel has it and it isn't
/// stored (a version installed outside the Launcher), so the Launcher can go back to it.
pub async fn run_action(
    l: &Arc<Launcher>,
    a: &LauncherAction,
) -> Result<UpdateOutcome, LauncherError> {
    match a {
        LauncherAction::Rollback { product } => {
            if let Some(prev) = l.installed(product).previous {
                keep_package(l, product, &prev).await;
            }
            l.rollback(product).await
        }
        LauncherAction::Update { product, version } => {
            l.newer_than_installed(product, version)?;
            let version = fetch(l, product, version).await?;
            update_fetched(l, product, &version).await
        }
        LauncherAction::Install { product, version } => {
            let added = crate::github::prepare_add(l, product)?;
            let r = async {
                let version = fetch(l, product, version).await?;
                update_fetched(l, product, &version).await
            }
            .await;
            if added && !matches!(&r, Ok(o) if o.result == protocol::UpdateResult::Installed) {
                l.forget_added(product);
            }
            r
        }
    }
}

async fn update_fetched(
    l: &Arc<Launcher>,
    product: &str,
    version: &str,
) -> Result<UpdateOutcome, LauncherError> {
    fetch_needed(l, product, version).await;
    if let Some(cur) = l.installed(product).version {
        keep_package(l, product, &cur).await;
    }
    l.update(product, version).await
}

/// Makes sure `product` `version` is in the Launcher's store: stored already, else downloaded
/// from the site's channel, else (no channel, the channel doesn't have it or can't be reached)
/// from GitHub when checking it is on. The version as stored (a GitHub release's version is
/// the signed one there).
async fn fetch(l: &Arc<Launcher>, product: &str, version: &str) -> Result<String, LauncherError> {
    if l.stored_package(product, version).is_some() {
        return Ok(version.to_string());
    }
    let (l2, product2, version2) = (l.clone(), product.to_string(), version.to_string());
    tokio::task::spawn_blocking(move || {
        let from_channel = match l2.channel() {
            None => Err(LauncherError::Invalid(format!(
                "{product2} {version2} isn't stored here and no update channel is configured"
            ))),
            Some(c) => c
                .packages()
                .map_err(|e| LauncherError::Io(e.to_string()))
                .and_then(|pkgs| {
                    let p = pkgs
                        .iter()
                        .find(|p| {
                            p.product == product2
                                && p.version == version2
                                && p.platform == l2.platform()
                        })
                        .ok_or_else(|| {
                            LauncherError::Invalid(format!(
                                "the site's update channel has no {product2} {version2} for {}",
                                l2.platform()
                            ))
                        })?;
                    c.fetch(&l2, p).map(|()| version2.clone())
                }),
        };
        match from_channel {
            Ok(v) => Ok(v),
            // A refused package stays refused: never fetched again elsewhere.
            Err(e @ LauncherError::Signature(_)) => Err(e),
            Err(e) if l2.config().github.enabled => {
                tracing::info!("{product2} {version2}: {e}; trying GitHub");
                l2.fetch_from_github(&product2, &version2)
                    .map_err(|g| match g {
                        LauncherError::Signature(m) => LauncherError::Signature(m),
                        g => LauncherError::Io(format!("{e}; GitHub: {g}")),
                    })
            }
            Err(e) => Err(e),
        }
    })
    .await
    .map_err(|e| LauncherError::Io(e.to_string()))?
}

/// Downloads from the site's channel the packages of other products `product` `version`'s
/// package needs at a version not installed here (the newest meeting each bound), so they are
/// installed with it in one dpkg run (best effort: without them, the update is refused saying
/// what it needs).
async fn fetch_needed(l: &Arc<Launcher>, product: &str, version: &str) {
    let (Ok(p), Some(pkg)) = (l.product(product), l.stored_package(product, version)) else {
        return;
    };
    let l2 = l.clone();
    let r = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let needs = l2.deb_needs(&p, &pkg)?;
        let needs: Vec<_> = needs
            .into_iter()
            .filter(|(name, bound)| l2.stored_meeting(name, bound.as_ref()).is_none())
            .collect();
        if needs.is_empty() {
            return Ok(());
        }
        let c = l2.channel().ok_or("no update channel")?;
        let pkgs = c.packages().map_err(|e| e.to_string())?;
        for (name, bound) in needs {
            let best = pkgs
                .iter()
                .filter(|c| c.product == name && c.platform == l2.platform())
                .filter(|c| {
                    bound
                        .as_ref()
                        .is_none_or(|(op, v)| crate::install::satisfies(&c.version, op, v))
                })
                .max_by(|a, b| crate::package::compare_versions(&a.version, &b.version));
            if let Some(b) = best {
                c.fetch(&l2, b).map_err(|e| e.to_string())?;
                tracing::info!(
                    "{name} {}: downloaded, to install with {}",
                    b.version,
                    p.product
                );
            }
        }
        Ok(())
    })
    .await;
    match r {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("{product} {version}: the packages it needs: {e}"),
        Err(e) => tracing::warn!("{e}"),
    }
}

/// Downloads `product` `version` from the site's channel when it isn't stored and the channel
/// has it (best effort: without it, that version can't be gone back to).
async fn keep_package(l: &Arc<Launcher>, product: &str, version: &str) {
    if l.stored_package(product, version).is_some() {
        return;
    }
    let Some(c) = l.channel() else { return };
    let (l2, product2, version2) = (l.clone(), product.to_string(), version.to_string());
    let r = tokio::task::spawn_blocking(move || {
        let pkgs = c.packages().map_err(|e| LauncherError::Io(e.to_string()))?;
        match pkgs
            .iter()
            .find(|p| p.product == product2 && p.version == version2 && p.platform == l2.platform())
        {
            Some(p) => c.fetch(&l2, p).map(|()| true),
            None => Ok(false),
        }
    })
    .await;
    match r {
        Ok(Ok(true)) => tracing::info!("{product} {version}: downloaded, to go back to"),
        Ok(Ok(false)) => {}
        Ok(Err(e)) => tracing::warn!("{product} {version}: can't download it to go back to: {e}"),
        Err(e) => tracing::warn!("{e}"),
    }
}

/// Reports to the channel every `interval_secs` (and right after each action, and as soon as
/// the machine joined a site), running the actions it gets back, and notes the newer versions
/// the channel has; health checks run every `check_interval_secs`. The channel is the site
/// joined in the wizard (with this machine's Launcher key), else the configuration file's.
/// Never returns.
pub async fn run(l: Arc<Launcher>) {
    let check = Duration::from_secs(l.config().check_interval_secs.max(1));
    let report_every = Duration::from_secs(
        l.config()
            .channel
            .as_ref()
            .map_or(30, |c| c.interval_secs.max(1)),
    );
    let mut last_report: Option<std::time::Instant> = None;
    let mut last_url: Option<String> = None;
    loop {
        l.check().await;
        // No church server (or none for a long time): GitHub, when a check is due.
        let l2 = l.clone();
        tokio::spawn(async move { l2.check_github().await });
        // The bundled IDN Node stays Vesper's to update (its marker, its service).
        l.idn_node_guard().await;
        // The machine's chosen roles (a run at a time, in the background: a join can wait).
        let l2 = l.clone();
        tokio::spawn(async move { l2.act_on_roles().await });
        let channel = l.channel();
        if channel.as_ref().map(|c| &c.url) != last_url.as_ref() {
            // Joined (or left) meanwhile: report at once.
            last_url = channel.as_ref().map(|c| c.url.clone());
            last_report = None;
        }
        if let Some(c) = &channel
            && last_report.is_none_or(|t| t.elapsed() >= report_every)
        {
            last_report = Some(std::time::Instant::now());
            let (c2, l2) = (c.clone(), l.clone());
            let actions = tokio::task::spawn_blocking(move || {
                let r = c2.report(&l2.report());
                if r.is_ok() {
                    let newer = c2.packages().ok().map(|p| l2.newer(&p));
                    l2.reported(None, newer);
                }
                r
            })
            .await;
            if let Ok(Err(e)) = &actions {
                l.reported(Some(e.to_string()), None);
            }
            match actions {
                Ok(Ok(actions)) => {
                    for a in &actions {
                        match run_action(&l, a).await {
                            Ok(o) => tracing::info!("{a:?}: {:?} {}", o.result, o.message),
                            Err(e) => {
                                tracing::warn!("{a:?}: {e}");
                                if let LauncherAction::Update { product, version }
                                | LauncherAction::Install { product, version } = a
                                {
                                    l.record_failure(product, version, &e.to_string());
                                }
                            }
                        }
                    }
                    if !actions.is_empty() {
                        // Report the outcome straight away.
                        last_report = None;
                        continue;
                    }
                }
                Ok(Err(e)) => tracing::warn!("can't report to the site's update channel: {e}"),
                Err(e) => tracing::warn!("{e}"),
            }
        }
        tokio::time::sleep(check.min(report_every)).await;
    }
}
