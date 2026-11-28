//! The site's update channel (G10, `docs/LAUNCHER-PLAN.md`): signed product packages an admin
//! uploaded, served to the site's Vesper Launchers, and what each Launcher last reported with
//! the actions queued for it in Oriel (Site ▸ Updates).
//!
//! Packages live in `<data>/updates/packages/<id>/` (the file, `package.minisig`,
//! `package.json`). A package is accepted only when its minisign signature holds for one of
//! the server's update keys ([`vesper_launcher::sign::RELEASE_KEYS`] and `--update-key`), and
//! its product, version and platform come from the signed trusted comment. The Launchers
//! verify every package again before installing it: the channel is a convenience, not the
//! gate.
//!
//! Launchers are kept in memory: each reports every 30 seconds, so the list is back within a
//! report interval after a restart. Queued actions are given out once, with the next report.

use protocol::{
    ApiError, AvailableUpdate, ErrorCode, LauncherAction, LauncherEntry, LauncherReport,
    UpdatePackage,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use vesper_launcher::package::{self, compare_versions};
use vesper_launcher::sign::{self, Digests, PublicKey};

/// A Launcher that hasn't reported for this long is shown offline.
pub const OFFLINE_AFTER: Duration = Duration::from_secs(90);
/// Launchers that haven't reported for this long are forgotten.
const FORGET_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);
/// The most Launchers kept (a site has a handful of machines).
pub const MAX_LAUNCHERS: usize = 256;
/// The most products a Launcher may report.
pub const MAX_PRODUCTS: usize = 32;
/// The signature file of a stored package.
const SIGNATURE: &str = "package.minisig";
/// The description of a stored package.
const META: &str = "package.json";

struct Known {
    report: LauncherReport,
    seen: Instant,
    pending: Vec<LauncherAction>,
}

/// The update channel.
pub struct Updates {
    dir: PathBuf,
    keys: Vec<PublicKey>,
    packages: Mutex<Vec<UpdatePackage>>,
    launchers: Mutex<BTreeMap<String, Known>>,
}

fn api(code: ErrorCode, m: impl Into<String>) -> ApiError {
    ApiError::new(code, m)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Updates {
    /// Opens `<data>/updates/` with the compiled-in keys and `extra_keys` (minisign text).
    pub fn open(data_dir: &Path, extra_keys: &[String]) -> anyhow::Result<Self> {
        let mut texts: Vec<String> = sign::RELEASE_KEYS.iter().map(|s| s.to_string()).collect();
        texts.extend(extra_keys.iter().cloned());
        let keys = sign::parse_keys(&texts).map_err(|e| anyhow::anyhow!("update key: {e}"))?;
        let dir = data_dir.join("updates").join("packages");
        let mut packages = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                match std::fs::read(e.path().join(META))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<UpdatePackage>(&b).ok())
                {
                    Some(p) => packages.push(p),
                    // An upload that didn't finish.
                    None => {
                        let _ = std::fs::remove_dir_all(e.path());
                    }
                }
            }
        }
        Ok(Updates {
            dir,
            keys,
            packages: Mutex::new(packages),
            launchers: Mutex::new(BTreeMap::new()),
        })
    }

    /// The update keys packages are checked with.
    pub fn keys(&self) -> &[PublicKey] {
        &self.keys
    }

    /// The folder uploads are written to before they are checked.
    pub fn staging(&self) -> std::io::Result<PathBuf> {
        let d = self.dir.join(".incoming");
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    /// Every package, by product and platform, newest first.
    pub fn list(&self) -> Vec<UpdatePackage> {
        let mut v = lock(&self.packages).clone();
        v.sort_by(|a, b| {
            (&a.product, &a.platform)
                .cmp(&(&b.product, &b.platform))
                .then_with(|| compare_versions(&b.version, &a.version))
        });
        v
    }

    /// A package and the paths of its file and signature.
    pub fn get(&self, id: &str) -> Option<(UpdatePackage, PathBuf, PathBuf)> {
        let p = lock(&self.packages).iter().find(|p| p.id == id).cloned()?;
        let dir = self.dir.join(&p.id);
        Some((p.clone(), dir.join(&p.file_name), dir.join(SIGNATURE)))
    }

    /// Checks an uploaded file (`tmp`, with `digests`) and its signature, and stores it.
    /// Refused: a signature that doesn't hold for one of the server's keys (`400`), a package
    /// that is already there (`409`).
    pub fn add(
        &self,
        tmp: &Path,
        digests: &Digests,
        file_name: &str,
        sig_text: &str,
    ) -> Result<UpdatePackage, ApiError> {
        self.add_from(tmp, digests, file_name, sig_text, None)
    }

    /// [`Updates::add`], noting where the package came from (`github`; `None`: an upload).
    pub fn add_from(
        &self,
        tmp: &Path,
        digests: &Digests,
        file_name: &str,
        sig_text: &str,
        source: Option<&str>,
    ) -> Result<UpdatePackage, ApiError> {
        let v = package::verify(&self.keys, digests, sig_text).map_err(|e| {
            api(
                ErrorCode::BadRequest,
                format!("the package was refused: {e}"),
            )
        })?;
        let file_name = Path::new(file_name)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty() && !n.starts_with('.') && *n != SIGNATURE && *n != META)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}-{}", v.meta.product, v.meta.version));
        let p = UpdatePackage {
            id: digests.sha256[..16].to_string(),
            product: v.meta.product.clone(),
            version: v.meta.version.clone(),
            platform: v.meta.platform.clone(),
            file_name,
            size: digests.size,
            sha256: digests.sha256.clone(),
            key_id: v.key_id.clone(),
            trusted_comment: v.trusted_comment.clone(),
            uploaded_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            source: source.map(str::to_string),
        };
        let mut all = lock(&self.packages);
        if let Some(o) = all.iter().find(|o| {
            o.id == p.id
                || (o.product == p.product && o.version == p.version && o.platform == p.platform)
        }) {
            return Err(api(
                ErrorCode::Conflict,
                format!(
                    "{} {} for {} is already in the channel",
                    o.product, o.version, o.platform
                ),
            ));
        }
        let dir = self.dir.join(&p.id);
        let store = || -> std::io::Result<()> {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join(SIGNATURE), sig_text)?;
            std::fs::rename(tmp, dir.join(&p.file_name))?;
            // Written last: a folder without it is an unfinished upload.
            std::fs::write(dir.join(META), serde_json::to_vec_pretty(&p)?)?;
            Ok(())
        };
        if let Err(e) = store() {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(api(
                ErrorCode::Internal,
                format!("can't store the package: {e}"),
            ));
        }
        all.push(p.clone());
        Ok(p)
    }

    /// Deletes a package; `false` if there is none.
    pub fn delete(&self, id: &str) -> bool {
        let mut all = lock(&self.packages);
        let Some(i) = all.iter().position(|p| p.id == id) else {
            return false;
        };
        all.remove(i);
        let _ = std::fs::remove_dir_all(self.dir.join(id));
        true
    }

    /// A Launcher's report; the actions queued for it (given out once). Refused (`400`): a
    /// report listing more than [`MAX_PRODUCTS`] products; (`409`) a new machine when
    /// [`MAX_LAUNCHERS`] are known.
    pub fn report(&self, r: LauncherReport) -> Result<Vec<LauncherAction>, ApiError> {
        if r.products.len() > MAX_PRODUCTS {
            return Err(api(
                ErrorCode::BadRequest,
                format!("a Launcher reports at most {MAX_PRODUCTS} products"),
            ));
        }
        let mut all = lock(&self.launchers);
        all.retain(|_, k| k.seen.elapsed() < FORGET_AFTER);
        if !all.contains_key(&r.machine_id) && all.len() >= MAX_LAUNCHERS {
            return Err(api(
                ErrorCode::Conflict,
                format!("the site already knows {MAX_LAUNCHERS} Launchers"),
            ));
        }
        let k = all.entry(r.machine_id.clone()).or_insert_with(|| Known {
            report: r.clone(),
            seen: Instant::now(),
            pending: Vec::new(),
        });
        k.report = r;
        k.seen = Instant::now();
        Ok(std::mem::take(&mut k.pending))
    }

    fn entry(&self, k: &Known, packages: &[UpdatePackage]) -> LauncherEntry {
        let updates = k
            .report
            .products
            .iter()
            .filter_map(|p| {
                let newest = packages
                    .iter()
                    .filter(|c| c.product == p.product && c.platform == k.report.platform)
                    .max_by(|a, b| compare_versions(&a.version, &b.version))?;
                let newer = p.version.as_deref().is_none_or(|v| {
                    compare_versions(&newest.version, v) == std::cmp::Ordering::Greater
                });
                newer.then(|| AvailableUpdate {
                    product: p.product.clone(),
                    version: newest.version.clone(),
                    package: newest.id.clone(),
                })
            })
            .collect();
        LauncherEntry {
            report: k.report.clone(),
            last_seen_seconds: k.seen.elapsed().as_secs(),
            online: k.seen.elapsed() < OFFLINE_AFTER,
            pending: k.pending.clone(),
            updates,
        }
    }

    /// Every Launcher that reported, by machine name.
    pub fn launchers(&self) -> Vec<LauncherEntry> {
        let packages = self.list();
        let all = lock(&self.launchers);
        let mut v: Vec<LauncherEntry> = all.values().map(|k| self.entry(k, &packages)).collect();
        v.sort_by(|a, b| {
            a.report
                .machine_name
                .to_lowercase()
                .cmp(&b.report.machine_name.to_lowercase())
                .then_with(|| a.report.machine_id.cmp(&b.report.machine_id))
        });
        v
    }

    /// Queues an action for a machine's Launcher (`404` unknown machine or product; `409` no
    /// such package for its platform, nothing to roll back to). The same action isn't queued
    /// twice.
    pub fn queue(&self, machine: &str, a: LauncherAction) -> Result<LauncherEntry, ApiError> {
        let packages = self.list();
        let mut all = lock(&self.launchers);
        let k = all.get_mut(machine).ok_or_else(|| {
            api(
                ErrorCode::NotFound,
                "no Launcher with that machine ID has reported",
            )
        })?;
        let product = match &a {
            LauncherAction::Update { product, .. }
            | LauncherAction::Rollback { product }
            | LauncherAction::Install { product, .. } => product,
        };
        if let LauncherAction::Install { .. } = &a {
            // "Add app": a product the machine doesn't run yet. Its package comes from the
            // channel, else the Launcher fetches it from GitHub (the API handler made sure one
            // of them has it).
            if k.report
                .products
                .iter()
                .any(|p| &p.product == product && p.version.is_some())
            {
                return Err(api(
                    ErrorCode::Conflict,
                    format!("{} already has {product}", k.report.machine_name),
                ));
            }
            if !k.pending.contains(&a) {
                k.pending.push(a);
            }
            return Ok(self.entry(k, &packages));
        }
        let p = k
            .report
            .products
            .iter()
            .find(|p| &p.product == product)
            .ok_or_else(|| {
                api(
                    ErrorCode::NotFound,
                    format!("{} doesn't manage {product:?}", k.report.machine_name),
                )
            })?;
        match &a {
            LauncherAction::Update { version, .. } => {
                if !packages.iter().any(|c| {
                    &c.product == product
                        && &c.version == version
                        && c.platform == k.report.platform
                }) {
                    return Err(api(
                        ErrorCode::Conflict,
                        format!(
                            "the channel has no {product} {version} for {}",
                            k.report.platform
                        ),
                    ));
                }
            }
            LauncherAction::Install { .. } => {}
            LauncherAction::Rollback { .. } => {
                if p.previous.is_none() {
                    return Err(api(
                        ErrorCode::Conflict,
                        format!(
                            "{} has no previous version of {} to go back to",
                            k.report.machine_name, p.name
                        ),
                    ));
                }
            }
        }
        if !k.pending.contains(&a) {
            k.pending.push(a);
        }
        Ok(self.entry(k, &packages))
    }
}
