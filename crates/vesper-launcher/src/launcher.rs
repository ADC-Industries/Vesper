//! The Launcher: the manifest of installed products, the package store, updates with automatic
//! rollback, health checks and the report.
//!
//! State folder:
//!
//! | Path | What |
//! |---|---|
//! | `machine-id` | the machine's stable ID |
//! | `manifest.json` | installed and previous version per product, the last update, an update in progress |
//! | `packages/<product>/<version>/` | the package and its `package.minisig` (current and previous kept) |
//! | `products/<product>/<version>/` | folder-kind products' executables |
//! | `logs/<product>.log` | folder-kind products' output |

use crate::config::{Config, InstallKind, ProductConfig};
use crate::install::{self, Supervisor};
use crate::package::{self, Verified};
use crate::setup::{JoinState, PairAnswer, SetupFile, SiteKey};
use crate::sign::{self, Hasher, PublicKey};
use protocol::{LauncherProduct, LauncherReport, ProductState, UpdateOutcome, UpdateResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The signature file next to a stored package.
pub const SIGNATURE_FILE: &str = "package.minisig";

/// What the manifest keeps per product.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    /// The installed version.
    #[serde(default)]
    pub version: Option<String>,
    /// The kept previous version.
    #[serde(default)]
    pub previous: Option<String>,
    /// The last update or rollback.
    #[serde(default)]
    pub last_update: Option<UpdateOutcome>,
    /// An update or rollback under way: saved before the installer runs, cleared when its
    /// outcome is recorded. Still there when the Launcher starts, it was interrupted (a
    /// restart, a power cut): the Launcher finishes it ([`Launcher::start_products`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_progress: Option<InProgress>,
}

/// An update or rollback under way (see [`Installed::in_progress`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InProgress {
    /// The version installed before (`None`: a first install).
    pub from: Option<String>,
    /// The version being installed.
    pub to: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Manifest {
    products: BTreeMap<String, Installed>,
}

/// Why an action was refused before anything changed.
#[derive(Debug)]
pub enum LauncherError {
    /// No such product here.
    UnknownProduct(String),
    /// The request can't be done (no previous version, package missing, …).
    Invalid(String),
    /// The package's signature or description was refused.
    Signature(String),
    /// Something failed on this machine.
    Io(String),
}

impl std::fmt::Display for LauncherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LauncherError::UnknownProduct(p) => write!(f, "this machine doesn't manage {p:?}"),
            LauncherError::Invalid(m) | LauncherError::Signature(m) | LauncherError::Io(m) => {
                f.write_str(m)
            }
        }
    }
}

impl std::error::Error for LauncherError {}

fn io(e: impl std::fmt::Display) -> LauncherError {
    LauncherError::Io(e.to_string())
}

struct Runtime {
    state: ProductState,
    /// Stopped by hand (Stop in the tray): shown `Stopped`, not `Failed`, until started.
    held: bool,
    restarts: Arc<AtomicU32>,
    supervisor: Option<Supervisor>,
    /// Since when it is expected to come up (started, or seen down after running): it shows
    /// `Starting` for the health timeout, then `Failed`.
    down_since: Option<Instant>,
}

/// A machine's Launcher.
pub struct Launcher {
    cfg: Config,
    keys: Vec<PublicKey>,
    machine_id: String,
    platform: String,
    manifest: Mutex<Manifest>,
    runtimes: Mutex<BTreeMap<String, Runtime>>,
    /// One update or rollback at a time.
    busy: tokio::sync::Mutex<()>,
    /// `setup.json` ([`crate::setup`]).
    setup: Mutex<SetupFile>,
    /// `site.json`: the site joined and this machine's Launcher key.
    site: Mutex<Option<SiteKey>>,
    /// A join waiting for approval, or why the last one ended (when not joined).
    joining: Mutex<JoinState>,
    /// Newer versions the site's channel has for this machine, by product (as of the last
    /// report).
    available: Mutex<BTreeMap<String, String>>,
    /// The last report to the channel: when, and what went wrong.
    last_report: Mutex<Option<(String, Option<String>)>>,
    /// `roles.json`: the roles made true on the machine ([`crate::roles`]).
    pub(crate) roles_file: Mutex<crate::roles::RolesFile>,
    /// One run of the roles at a time.
    pub(crate) roles_busy: tokio::sync::Mutex<()>,
    /// A failed run of the roles is tried again from then.
    pub(crate) roles_retry: Mutex<Option<Instant>>,
    /// What the bundled IDN Node's guard did last ([`crate::idn_node`]): when (RFC 3339), what,
    /// and when it last put the Node's service back.
    pub(crate) idn_guard: Mutex<IdnGuard>,
}

/// What the bundled IDN Node's guard did last ([`crate::idn_node`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct IdnGuard {
    /// The last check found a bundled `idn-node` (a row on the diagnostics page).
    pub node: bool,
    /// Why the marker couldn't be read or written at the last check (`None`: it names Vesper).
    pub marker_error: Option<String>,
    /// The last check found the Launcher's IDN Node service running another program (and it
    /// wasn't put back then).
    pub replaced: bool,
    /// The last warning: when (RFC 3339) and what (cleared when there is no bundled Node).
    pub warning: Option<(String, String)>,
    /// When the Node's service was last put back (at most every [`crate::roles::GUARD_EVERY`]).
    pub put_back: Option<Instant>,
}

/// The site's update channel as the tray shows it (`GET /launcher/v1/overview`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelView {
    /// The site's server reports go to (`None`: no channel, not joined).
    pub server: Option<String>,
    /// The last report (RFC 3339).
    pub last_report: Option<String>,
    /// What went wrong with it.
    pub problem: Option<String>,
}

/// The first-run setup as the wizard shows it (`GET /launcher/v1/setup`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupView {
    /// The wizard wasn't finished yet: the tray opens it.
    pub first_run: bool,
    /// The machine's roles.
    pub roles: Vec<String>,
    /// The roles offered: `{id, name, available}` (the others are "later").
    pub available_roles: Vec<serde_json::Value>,
    /// Joining the site.
    pub join: JoinState,
    /// The roles made true on the machine, and the last run towards the chosen ones.
    pub role_status: crate::roles::RolesView,
    /// The machine's name, as the site sees it.
    pub machine_name: String,
    /// Its ID.
    pub machine_id: String,
}

/// Everything the tray window shows (`GET /launcher/v1/overview`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    /// The report: products, versions, states, health.
    pub status: LauncherReport,
    /// Newer versions in the site's channel, by product (only newer than the installed one).
    pub available: BTreeMap<String, String>,
    /// How each product is installed, by product: `deb` and `nsis` (the system runs it, and
    /// Launch starts it whatever version is known), `dir` (the Launcher runs its installed
    /// folder), `self` (the Launcher itself: no Launch or Stop), or `package` (a package of
    /// programs, the bundled IDN Node: updated only, no Launch or Stop).
    pub kinds: BTreeMap<String, &'static str>,
    /// The site's channel.
    pub channel: ChannelView,
    /// The first-run setup.
    pub setup: SetupView,
    /// The tray icon's colour: `ok`, `updating` (an update, a rollback or a start under way)
    /// or `attention` (a product failed, the channel can't be reached, the join failed, setting
    /// up the machine's roles failed).
    pub light: &'static str,
}

/// The tray icon's colour for `o` (see [`Overview::light`]).
fn light(status: &LauncherReport, channel: &ChannelView, setup: &SetupView) -> &'static str {
    use ProductState as S;
    let states = || status.products.iter().map(|p| p.state);
    let join_problem = matches!(&setup.join, JoinState::NotJoined { problem: Some(_) });
    let roles_failed = setup
        .role_status
        .run
        .as_ref()
        .is_some_and(|r| r.state == "failed");
    if states().any(|s| s == S::Failed) || channel.problem.is_some() || join_problem || roles_failed
    {
        "attention"
    } else if states().any(|s| matches!(s, S::Updating | S::RollingBack | S::Starting)) {
        "updating"
    } else {
        "ok"
    }
}

pub(crate) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl Launcher {
    /// Opens the state folder (made if missing) and reads the keys.
    pub fn open(cfg: Config) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(&cfg.state_dir)?;
        let mut texts: Vec<String> = sign::RELEASE_KEYS.iter().map(|s| s.to_string()).collect();
        texts.extend(cfg.keys.iter().cloned());
        if let Some(dir) = &cfg.key_dir
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for e in entries.flatten() {
                if e.path().extension().is_some_and(|x| x == "pub") {
                    texts.push(std::fs::read_to_string(e.path())?);
                }
            }
        }
        let keys = sign::parse_keys(&texts).map_err(|e| anyhow::anyhow!("update key: {e}"))?;
        if keys.is_empty() {
            tracing::warn!("no update signing key is configured: every package will be refused");
        }
        let id_file = cfg.state_dir.join("machine-id");
        let machine_id = match std::fs::read_to_string(&id_file) {
            Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => {
                let id = uuid::Uuid::new_v4().to_string();
                std::fs::write(&id_file, &id)?;
                id
            }
        };
        let manifest = match std::fs::read(cfg.state_dir.join("manifest.json")) {
            Ok(b) => serde_json::from_slice(&b)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Manifest::default(),
            Err(e) => return Err(e.into()),
        };
        let runtimes = cfg
            .products
            .iter()
            .map(|p| {
                (
                    p.product.clone(),
                    Runtime {
                        state: ProductState::Stopped,
                        held: false,
                        restarts: Arc::new(AtomicU32::new(0)),
                        supervisor: None,
                        down_since: None,
                    },
                )
            })
            .collect();
        let platform = cfg.platform.clone().unwrap_or_else(package::this_platform);
        let setup =
            crate::setup::read(&crate::setup::setup_path(&cfg.state_dir))?.unwrap_or_default();
        let site = crate::setup::read(&crate::setup::site_path(&cfg.state_dir))?;
        let roles_file = crate::setup::read(&cfg.state_dir.join("roles.json"))?.unwrap_or_default();
        let l = Arc::new(Launcher {
            cfg,
            keys,
            machine_id,
            platform,
            manifest: Mutex::new(manifest),
            runtimes: Mutex::new(runtimes),
            busy: tokio::sync::Mutex::new(()),
            setup: Mutex::new(setup),
            site: Mutex::new(site),
            joining: Mutex::new(JoinState::NotJoined { problem: None }),
            available: Mutex::new(BTreeMap::new()),
            last_report: Mutex::new(None),
            roles_file: Mutex::new(roles_file),
            roles_busy: tokio::sync::Mutex::new(()),
            roles_retry: Mutex::new(None),
            idn_guard: Mutex::new(IdnGuard::default()),
        });
        for p in &l.cfg.products {
            if matches!(p.install, InstallKind::Deb { .. })
                && let Some(v) = install::deb_installed_version(&l.cfg.dpkg, &p.product)
            {
                l.adopt(&p.product, &v);
            }
        }
        Ok(l)
    }

    /// Takes `version` as `product`'s installed version when the manifest says otherwise: it
    /// was installed outside the Launcher (with apt, the Windows installer, by hand). Not while
    /// an update of it is under way, and not when the two only differ by a packaging revision
    /// (a package signed `0.2.0` that dpkg calls `0.2.0-1` is the same build). The version it
    /// replaces is kept as the previous one.
    fn adopt(&self, product: &str, version: &str) {
        let mut m = lock(&self.manifest);
        let i = m.products.entry(product.to_string()).or_default();
        let same = i.version.as_deref().is_some_and(|have| {
            crate::health::same_version(have, version) || crate::health::same_version(version, have)
        });
        if i.in_progress.is_some() || same {
            return;
        }
        let was = i.version.take();
        if was.is_some() {
            i.previous = was.clone();
        }
        i.version = Some(version.to_string());
        match &was {
            Some(w) => tracing::info!("{product}: {version} is installed (was {w} here)"),
            None => {
                tracing::info!("{product}: {version} is installed (installed outside the Launcher)")
            }
        }
        if let Err(e) = self.save_manifest(&m) {
            tracing::error!("can't save the manifest: {e}");
        }
    }

    /// Whether `product` is run by the system (a `deb` or `nsis` product), not by the Launcher.
    fn system_run(&self, product: &str) -> bool {
        self.product(product)
            .is_ok_and(|p| !matches!(p.install, InstallKind::Dir { .. }))
    }

    /// The machine's name, as the site sees it.
    pub fn machine_name(&self) -> String {
        self.cfg
            .machine_name
            .clone()
            .unwrap_or_else(|| gethostname::gethostname().to_string_lossy().into_owned())
    }

    /// The machine's roles: chosen in the wizard, else the configuration file's.
    /// Without either, a machine with the Vesper server's product is a presentation server.
    pub fn roles(&self) -> Vec<String> {
        let chosen = lock(&self.setup).roles.clone();
        chosen.unwrap_or_else(|| {
            if self.cfg.roles.is_empty()
                && self
                    .cfg
                    .products
                    .iter()
                    .any(|p| p.product == protocol::PRODUCT_ID)
            {
                vec!["presentation".into()]
            } else {
                self.cfg.roles.clone()
            }
        })
    }

    /// The roles chosen in the wizard (or with `PUT /launcher/v1/setup`): `None` when nobody
    /// chose any (the configuration file's are only reported, not acted on).
    pub fn chosen_roles(&self) -> Option<Vec<String>> {
        lock(&self.setup).roles.clone()
    }

    /// Marks `product` stopped on purpose (`held`: shown `Stopped`, not `Failed`) or not.
    pub(crate) fn hold(&self, product: &str, held: bool) {
        if let Some(r) = lock(&self.runtimes).get_mut(product) {
            r.held = held;
            if held {
                r.state = ProductState::Stopped;
                r.down_since = None;
            } else {
                r.state = ProductState::Starting;
                r.down_since = Some(Instant::now());
            }
        }
    }

    /// The site's update channel: the site joined (with this machine's Launcher key), else
    /// the configuration file's.
    pub fn channel(&self) -> Option<crate::channel::Channel> {
        let interval = self.cfg.channel.as_ref().map_or(30, |c| c.interval_secs);
        match &*lock(&self.site) {
            Some(s) => Some(crate::channel::Channel::new(
                &crate::config::ChannelConfig {
                    url: s.server.clone(),
                    token: Some(s.token.clone()),
                    interval_secs: interval,
                },
            )),
            None => self.cfg.channel.as_ref().map(crate::channel::Channel::new),
        }
    }

    /// The site joined, if any.
    pub fn site(&self) -> Option<SiteKey> {
        lock(&self.site).clone()
    }

    fn join_state(&self) -> JoinState {
        match &*lock(&self.site) {
            Some(s) => JoinState::Joined {
                server: s.server.clone(),
                server_name: s.server_name.clone(),
                key_id: s.key_id.clone(),
                joined_at: s.joined_at.clone(),
            },
            None => lock(&self.joining).clone(),
        }
    }

    /// The first-run setup.
    pub fn setup_view(&self) -> SetupView {
        // (Not inside the literal: its temporaries, the lock, would live through `roles()`.)
        let first_run = !lock(&self.setup).done;
        SetupView {
            first_run,
            roles: self.roles(),
            available_roles: crate::setup::ROLES
                .iter()
                .map(
                    |(id, name, now)| serde_json::json!({"id": id, "name": name, "available": now}),
                )
                .collect(),
            join: self.join_state(),
            role_status: self.roles_view(),
            machine_name: self.machine_name(),
            machine_id: self.machine_id.clone(),
        }
    }

    /// Sets the roles and/or the wizard's end (saved in `setup.json`).
    pub fn set_setup(
        &self,
        roles: Option<Vec<String>>,
        done: Option<bool>,
    ) -> Result<(), LauncherError> {
        if let Some(r) = &roles
            && let Some(bad) = r
                .iter()
                .find(|r| !crate::setup::ROLES.iter().any(|(id, _, _)| id == r))
        {
            return Err(LauncherError::Invalid(format!("{bad:?} isn't a role")));
        }
        if let Some(r) = &roles
            && r.iter().any(|x| x == "display")
            && r.iter().any(|x| x == "presentation")
        {
            // Mosaic (docs/MOSAIC-PLAN.md): a display box runs the IDN Node's display service
            // and no Vesper server.
            return Err(LauncherError::Invalid(
                "a display box isn't also a presentation server: it runs no Vesper server".into(),
            ));
        }
        let mut s = lock(&self.setup);
        let mut next = s.clone();
        if let Some(mut r) = roles {
            r.dedup();
            next.roles = Some(r);
        }
        if let Some(d) = done {
            next.done = d;
        }
        crate::setup::write(&crate::setup::setup_path(&self.cfg.state_dir), &next, false)
            .map_err(io)?;
        *s = next;
        drop(s);
        self.roles_changed();
        Ok(())
    }

    /// Joins the site at `server` (with its pairing code, or waiting for an admin's
    /// approval: asked again every two seconds in the background).
    pub async fn join(
        self: &Arc<Self>,
        server: &str,
        code: Option<String>,
    ) -> Result<JoinState, LauncherError> {
        let url = crate::setup::server_url(server).map_err(LauncherError::Invalid)?;
        if lock(&self.site).is_some() {
            return Err(LauncherError::Invalid(
                "this machine is in a site already: leave it first".into(),
            ));
        }
        self.cancel_join();
        let (name, id) = (self.machine_name(), self.machine_id.clone());
        let code = code.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
        let url2 = url.clone();
        let (server_name, answer) = tokio::task::spawn_blocking(move || {
            let server_name = crate::setup::server_name(&url2)?;
            Ok::<_, String>((
                server_name,
                crate::setup::pair(&url2, &name, &id, code.as_deref()),
            ))
        })
        .await
        .map_err(io)?
        .map_err(LauncherError::Invalid)?;
        self.took_answer(&url, &server_name, answer)?;
        if let JoinState::Waiting { .. } = self.join_state() {
            let l = self.clone();
            tokio::spawn(async move { l.wait_for_approval().await });
        }
        Ok(self.join_state())
    }

    /// Takes the site's answer to the pairing.
    fn took_answer(
        &self,
        url: &str,
        server_name: &str,
        a: PairAnswer,
    ) -> Result<(), LauncherError> {
        match a {
            PairAnswer::Key(token, key_id) => {
                let site = SiteKey {
                    server: url.to_string(),
                    server_name: server_name.to_string(),
                    token,
                    key_id,
                    joined_at: now(),
                };
                crate::setup::write(&crate::setup::site_path(&self.cfg.state_dir), &site, true)
                    .map_err(io)?;
                tracing::info!(server = %url, key = %site.key_id, "joined the site: this machine's Launcher key is stored");
                *lock(&self.site) = Some(site);
                *lock(&self.joining) = JoinState::NotJoined { problem: None };
                *lock(&self.last_report) = None;
                // A display box's IDN Node joins now.
                self.roles_changed();
            }
            PairAnswer::Waiting(pairing_id, expires_in_seconds) => {
                tracing::info!(server = %url, "joining the site: waiting for an admin's approval");
                *lock(&self.joining) = JoinState::Waiting {
                    server: url.to_string(),
                    server_name: server_name.to_string(),
                    pairing_id,
                    expires_in_seconds,
                };
            }
            PairAnswer::No(why) => {
                tracing::warn!(server = %url, "joining the site: {why}");
                *lock(&self.joining) = JoinState::NotJoined { problem: Some(why) };
            }
        }
        Ok(())
    }

    /// Asks for the waiting pairing's answer every two seconds until there is one.
    async fn wait_for_approval(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let JoinState::Waiting {
                server,
                server_name,
                pairing_id,
                ..
            } = lock(&self.joining).clone()
            else {
                return;
            };
            let (s2, id2) = (server.clone(), pairing_id.clone());
            let a = tokio::task::spawn_blocking(move || crate::setup::pairing_status(&s2, &id2))
                .await
                .ok()
                .flatten();
            let Some(a) = a else { continue };
            // Cancelled meanwhile, or another join started.
            if !matches!(&*lock(&self.joining), JoinState::Waiting { pairing_id: p, .. } if *p == pairing_id)
            {
                return;
            }
            if let Err(e) = self.took_answer(&server, &server_name, a) {
                tracing::error!("joining the site: {e}");
                *lock(&self.joining) = JoinState::NotJoined {
                    problem: Some(e.to_string()),
                };
            }
        }
    }

    /// Takes a waiting join back.
    pub fn cancel_join(&self) {
        let prev = std::mem::replace(
            &mut *lock(&self.joining),
            JoinState::NotJoined { problem: None },
        );
        if let JoinState::Waiting {
            server, pairing_id, ..
        } = prev
        {
            std::thread::spawn(move || crate::setup::withdraw(&server, &pairing_id));
        }
    }

    /// Leaves the site here: forgets its Launcher key (an admin revokes it in Site ▸ API
    /// keys; a new join replaces it). A waiting join is taken back.
    pub fn leave(&self) -> Result<(), LauncherError> {
        self.cancel_join();
        let path = crate::setup::site_path(&self.cfg.state_dir);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
        if let Some(s) = lock(&self.site).take() {
            tracing::info!(server = %s.server, key = %s.key_id, "left the site here: the Launcher key is forgotten");
        }
        lock(&self.available).clear();
        *lock(&self.last_report) = None;
        Ok(())
    }

    /// Notes a report to the channel (its error, if any) and the newer versions it has.
    pub fn reported(&self, problem: Option<String>, available: Option<BTreeMap<String, String>>) {
        *lock(&self.last_report) = Some((now(), problem));
        if let Some(a) = available {
            *lock(&self.available) = a;
        }
    }

    /// Of the channel's packages, the newest version for this machine of each product newer
    /// than the installed one.
    pub fn newer(&self, pkgs: &[protocol::UpdatePackage]) -> BTreeMap<String, String> {
        let mut out: BTreeMap<String, String> = BTreeMap::new();
        for p in pkgs.iter().filter(|p| p.platform == self.platform) {
            if self.product(&p.product).is_err() {
                continue;
            }
            let cur = self.installed(&p.product).version;
            if cur
                .as_deref()
                .is_some_and(|c| package::compare_versions(&p.version, c).is_le())
            {
                continue;
            }
            if cur.is_none() && self.system_run(&p.product) {
                // Installed outside the Launcher, its version not known yet: nothing is
                // offered (it could be older than the one installed).
                continue;
            }
            let better = out
                .get(&p.product)
                .is_none_or(|v| package::compare_versions(&p.version, v).is_gt());
            if better {
                out.insert(p.product.clone(), p.version.clone());
            }
        }
        out
    }

    /// Everything the tray window shows.
    pub fn overview(&self) -> Overview {
        let (last_report, problem) = lock(&self.last_report)
            .clone()
            .map_or((None, None), |(at, p)| (Some(at), p));
        let server = lock(&self.site)
            .as_ref()
            .map(|s| s.server.clone())
            .or_else(|| self.cfg.channel.as_ref().map(|c| c.url.clone()));
        let (status, setup) = (self.report(), self.setup_view());
        let channel = ChannelView {
            server,
            last_report,
            problem,
        };
        // Only versions newer than the installed one (an update made here since the last
        // report installed it).
        let mut available = lock(&self.available).clone();
        available.retain(|product, v| {
            self.installed(product)
                .version
                .is_none_or(|cur| package::compare_versions(v, &cur).is_gt())
        });
        let kinds = self
            .cfg
            .products
            .iter()
            .map(|p| {
                let kind = match p.install {
                    _ if p.is_self => "self",
                    // A package of programs (no Launch or Stop).
                    _ if p.health_url.is_empty() => "package",
                    InstallKind::Deb { .. } => "deb",
                    InstallKind::Nsis { .. } => "nsis",
                    InstallKind::Dir { .. } => "dir",
                };
                (p.product.clone(), kind)
            })
            .collect();
        Overview {
            light: light(&status, &channel, &setup),
            status,
            available,
            kinds,
            channel,
            setup,
        }
    }

    /// Starts `product` (Launch in the tray): its system service or scheduled task, or the
    /// installed folder's executable.
    pub async fn start_product(&self, product: &str) -> Result<(), LauncherError> {
        let p = self.product(product)?.clone();
        if p.is_self {
            return Err(LauncherError::Invalid("the Launcher is running".into()));
        }
        if p.health_url.is_empty() {
            return Err(LauncherError::Invalid(format!(
                "{} is a package of programs: there is nothing to launch",
                p.name
            )));
        }
        let _busy = self.busy.lock().await;
        if let Some(r) = lock(&self.runtimes).get_mut(product) {
            r.held = false;
        }
        match &p.install {
            InstallKind::Dir { .. } => {
                let v = self
                    .installed(product)
                    .version
                    .ok_or_else(|| LauncherError::Invalid(format!("{} isn't installed", p.name)))?;
                let pkg = self.verify_stored(product, &v)?;
                self.install_version(&p, &v, &pkg)
                    .await
                    .map_err(LauncherError::Io)?;
            }
            kind => {
                let kind = kind.clone();
                tokio::task::spawn_blocking(move || install::service(&kind, true))
                    .await
                    .map_err(io)?
                    .map_err(LauncherError::Io)?;
            }
        }
        self.set_state(product, ProductState::Starting);
        tracing::info!("{product}: started by hand");
        Ok(())
    }

    /// Stops `product` (Stop in the tray); it shows `Stopped` until started again.
    pub async fn stop_product(&self, product: &str) -> Result<(), LauncherError> {
        let p = self.product(product)?.clone();
        if p.is_self {
            return Err(LauncherError::Invalid(
                "the Launcher doesn't stop itself from here (systemctl stop vesper-launcher, or its scheduled task)".into(),
            ));
        }
        if p.health_url.is_empty() {
            return Err(LauncherError::Invalid(format!(
                "{} is a package of programs: there is nothing to stop",
                p.name
            )));
        }
        let _busy = self.busy.lock().await;
        match &p.install {
            InstallKind::Dir { .. } => {
                self.stop_supervisor(product).await;
            }
            kind => {
                let kind = kind.clone();
                tokio::task::spawn_blocking(move || install::service(&kind, false))
                    .await
                    .map_err(io)?
                    .map_err(LauncherError::Io)?;
            }
        }
        if let Some(r) = lock(&self.runtimes).get_mut(product) {
            r.held = true;
            r.state = ProductState::Stopped;
            r.down_since = None;
        }
        tracing::info!("{product}: stopped by hand");
        Ok(())
    }

    /// The configuration.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// The trusted public keys.
    pub fn keys(&self) -> &[PublicKey] {
        &self.keys
    }

    /// The machine's ID.
    pub fn machine_id(&self) -> &str {
        &self.machine_id
    }

    /// The platform its packages must be for.
    pub fn platform(&self) -> &str {
        &self.platform
    }

    pub(crate) fn product(&self, product: &str) -> Result<&ProductConfig, LauncherError> {
        self.cfg
            .products
            .iter()
            .find(|p| p.product == product)
            .ok_or_else(|| LauncherError::UnknownProduct(product.to_string()))
    }

    /// What the manifest says about `product`.
    pub fn installed(&self, product: &str) -> Installed {
        lock(&self.manifest)
            .products
            .get(product)
            .cloned()
            .unwrap_or_default()
    }

    fn save_manifest(&self, m: &Manifest) -> Result<(), LauncherError> {
        let path = self.cfg.state_dir.join("manifest.json");
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(m).map_err(io)?).map_err(io)?;
        std::fs::rename(&tmp, &path).map_err(io)
    }

    fn set_installed(&self, product: &str, f: impl FnOnce(&mut Installed)) {
        let mut m = lock(&self.manifest);
        f(m.products.entry(product.to_string()).or_default());
        if let Err(e) = self.save_manifest(&m) {
            tracing::error!("can't save the manifest: {e}");
        }
    }

    fn set_state(&self, product: &str, state: ProductState) {
        if let Some(r) = lock(&self.runtimes).get_mut(product) {
            r.state = state;
            r.down_since = (state == ProductState::Starting).then(Instant::now);
        }
    }

    fn package_dir(&self, product: &str, version: &str) -> PathBuf {
        self.cfg
            .state_dir
            .join("packages")
            .join(product)
            .join(version)
    }

    /// The stored package of `product` `version` (the file other than its signature).
    pub fn stored_package(&self, product: &str, version: &str) -> Option<PathBuf> {
        std::fs::read_dir(self.package_dir(product, version))
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_name().is_some_and(|n| n != SIGNATURE_FILE) && p.is_file())
    }

    /// Verifies a downloaded or uploaded package (`file`, named `file_name`) with its signature
    /// text and moves it into the store. Refused: a bad signature, another platform, a product
    /// this machine doesn't manage.
    pub fn add_package(
        &self,
        file: &Path,
        file_name: &str,
        sig_text: &str,
    ) -> Result<Verified, LauncherError> {
        let digests = Hasher::file(file).map_err(io)?;
        let v = package::verify(&self.keys, &digests, sig_text)
            .map_err(|e| LauncherError::Signature(e.0))?;
        self.product(&v.meta.product)?;
        if v.meta.platform != self.platform {
            return Err(LauncherError::Invalid(format!(
                "the package is for {}, this machine is {}",
                v.meta.platform, self.platform
            )));
        }
        let name = Path::new(file_name)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty() && *n != SIGNATURE_FILE && !n.starts_with('.'))
            .unwrap_or("package");
        let dir = self.package_dir(&v.meta.product, &v.meta.version);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(io)?;
        }
        std::fs::create_dir_all(&dir).map_err(io)?;
        std::fs::write(dir.join(SIGNATURE_FILE), sig_text).map_err(io)?;
        if std::fs::rename(file, dir.join(name)).is_err() {
            std::fs::copy(file, dir.join(name)).map_err(io)?;
        }
        Ok(v)
    }

    /// Checks a stored package again (the store is on this machine's disk, but checked again
    /// before every install).
    fn verify_stored(&self, product: &str, version: &str) -> Result<PathBuf, LauncherError> {
        let file = self.stored_package(product, version).ok_or_else(|| {
            LauncherError::Invalid(format!("{product} {version} isn't downloaded"))
        })?;
        let sig = std::fs::read_to_string(self.package_dir(product, version).join(SIGNATURE_FILE))
            .map_err(io)?;
        let digests = Hasher::file(&file).map_err(io)?;
        let v = package::verify(&self.keys, &digests, &sig)
            .map_err(|e| LauncherError::Signature(e.0))?;
        if v.meta.product != product || v.meta.version != version {
            return Err(LauncherError::Signature(format!(
                "the stored package is signed as {} {}",
                v.meta.product, v.meta.version
            )));
        }
        Ok(file)
    }

    /// Installs and starts `version` (blocking parts off the async threads).
    async fn install_version(
        &self,
        p: &ProductConfig,
        version: &str,
        package: &Path,
    ) -> Result<(), String> {
        match &p.install {
            InstallKind::Dir { args } => {
                let root = self.cfg.state_dir.join("products").join(&p.product);
                let exe = install::dir_install(&root, version, package)
                    .map_err(|e| format!("can't install: {e}"))?;
                let log = self.log_path(p);
                let restarts = self.stop_supervisor(&p.product).await;
                let sup = Supervisor::start(exe, args.clone(), log, restarts);
                if let Some(r) = lock(&self.runtimes).get_mut(&p.product) {
                    r.supervisor = Some(sup);
                }
                Ok(())
            }
            kind => {
                let with = if p.is_self {
                    Vec::new()
                } else {
                    self.deb_companions(p, package)?
                };
                let (kind, package) = (kind.clone(), package.to_path_buf());
                let dpkg = self.cfg.dpkg.clone();
                tokio::task::spawn_blocking(move || {
                    install::run_system_installer(&kind, &package, &with, &dpkg)
                })
                .await
                .map_err(|e| e.to_string())?
            }
        }
    }

    async fn stop_supervisor(&self, product: &str) -> Arc<AtomicU32> {
        let (sup, restarts) = {
            let mut rt = lock(&self.runtimes);
            match rt.get_mut(product) {
                Some(r) => (r.supervisor.take(), r.restarts.clone()),
                None => (None, Arc::new(AtomicU32::new(0))),
            }
        };
        if let Some(s) = sup {
            s.stop().await;
        }
        restarts
    }

    async fn healthy(&self, p: &ProductConfig, version: &str) -> Result<(), String> {
        let (url, version) = (p.health_url.clone(), version.to_string());
        if url.is_empty() {
            // A package of programs: healthy once dpkg has the version installed.
            let (dpkg, name) = (self.cfg.dpkg.clone(), p.product.clone());
            let have =
                tokio::task::spawn_blocking(move || install::deb_installed_version(&dpkg, &name))
                    .await
                    .map_err(|e| e.to_string())?;
            return match have {
                Some(v) if crate::health::same_version(&version, &v) => Ok(()),
                Some(v) => Err(format!("dpkg has {v} installed, not {version}")),
                None => Err(format!("dpkg doesn't have {version} installed")),
            };
        }
        let timeout = Duration::from_secs(self.cfg.health_timeout_secs);
        tokio::task::spawn_blocking(move || crate::health::wait_healthy(&url, &version, timeout))
            .await
            .map_err(|e| e.to_string())?
    }

    /// Installs stored `version` of `product`; if it isn't healthy within the health timeout,
    /// the previous version is installed again. The outcome is kept in the manifest.
    pub async fn update(
        &self,
        product: &str,
        version: &str,
    ) -> Result<UpdateOutcome, LauncherError> {
        let p = self.product(product)?.clone();
        let _busy = self.busy.lock().await;
        if p.is_self && self.installed(product).in_progress.is_some() {
            return Err(LauncherError::Invalid(
                "an update of the Launcher is under way: it restarts to finish it".into(),
            ));
        }
        self.newer_than_installed(product, version)?;
        let before = self.installed(product);
        let package = self.verify_stored(product, version)?;
        if !p.is_self {
            // Refused before anything is installed when a product it needs isn't at hand.
            self.deb_companions(&p, &package)
                .map_err(|e| LauncherError::Invalid(format!("{} {version}: {e}", p.name)))?;
        }
        Ok(self
            .install_update(&p, &before, version, Ok(Some(package)))
            .await)
    }

    /// The products `p`'s `.deb` (`package`) needs at a version this machine doesn't have
    /// (another product managed here, named in its `Depends`, such as the server's
    /// `vesper-launcher (>= …)`): `(product, bound)`. Blocking (dpkg-deb, dpkg-query).
    pub fn deb_needs(
        &self,
        p: &ProductConfig,
        package: &Path,
    ) -> Result<Vec<install::DebDepend>, String> {
        if !matches!(p.install, InstallKind::Deb { .. }) {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for (name, bound) in install::deb_depends(&self.cfg.dpkg, package) {
            let ours = self.product(&name).is_ok_and(|o| {
                o.product != p.product && matches!(o.install, InstallKind::Deb { .. })
            });
            if !ours {
                // Not a product of the Launcher's: dpkg's (and apt's) business.
                continue;
            }
            let met = install::deb_installed_version(&self.cfg.dpkg, &name).is_some_and(|have| {
                bound
                    .as_ref()
                    .is_none_or(|(op, v)| install::satisfies(&have, op, v))
            });
            if !met {
                out.push((name, bound));
            }
        }
        Ok(out)
    }

    /// The stored versions of `product`.
    fn stored_versions(&self, product: &str) -> Vec<String> {
        std::fs::read_dir(self.cfg.state_dir.join("packages").join(product))
            .map(|d| {
                d.flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The newest stored version of `product` meeting `bound`.
    pub fn stored_meeting(
        &self,
        product: &str,
        bound: Option<&(String, String)>,
    ) -> Option<String> {
        self.stored_versions(product)
            .into_iter()
            .filter(|v| self.stored_package(product, v).is_some())
            .filter(|v| bound.is_none_or(|(op, b)| install::satisfies(v, op, b)))
            .max_by(|a, b| package::compare_versions(a, b))
    }

    /// The checked packages to install with `p`'s `package` in one dpkg run: for each product
    /// it needs at a version not installed ([`Launcher::deb_needs`]), the newest stored one
    /// meeting the bound (revision 2 moved the Launcher into its own package; a server may
    /// need a newer Launcher). One isn't stored: why it can't be installed.
    fn deb_companions(&self, p: &ProductConfig, package: &Path) -> Result<Vec<PathBuf>, String> {
        let mut with = Vec::new();
        for (name, bound) in self.deb_needs(p, package)? {
            let need = match &bound {
                Some((op, v)) => format!("{name} ({op} {v})"),
                None => name.clone(),
            };
            let Some(v) = self.stored_meeting(&name, bound.as_ref()) else {
                return Err(format!(
                    "it needs {need}, which isn't installed here and isn't in the site's update channel: upload that package too"
                ));
            };
            let pkg = self.verify_stored(&name, &v).map_err(|e| e.to_string())?;
            tracing::info!(
                "{}: installing {name} {v} with it (it needs {need})",
                p.product
            );
            with.push(pkg);
        }
        Ok(with)
    }

    /// Refuses `version` of `product` as an update when it isn't newer than the installed one
    /// (the same version, or an older one: Roll back goes back to the previous version).
    pub fn newer_than_installed(&self, product: &str, version: &str) -> Result<(), LauncherError> {
        let name = &self.product(product)?.name;
        let Some(cur) = self.installed(product).version else {
            return Ok(());
        };
        match package::compare_versions(version, &cur) {
            std::cmp::Ordering::Greater => Ok(()),
            std::cmp::Ordering::Equal => Err(LauncherError::Invalid(format!(
                "{name} {version} is already installed"
            ))),
            std::cmp::Ordering::Less => Err(LauncherError::Invalid(format!(
                "{name} {cur} is installed: {version} is older, not an update (Roll back goes back to the previous version)"
            ))),
        }
    }

    /// Installs `version` of `p` (its checked `package`; `None`: installed already, only
    /// checked) over `before`; back to `before` when it isn't healthy. The update is marked in
    /// progress in the manifest until its outcome is recorded.
    async fn install_update(
        &self,
        p: &ProductConfig,
        before: &Installed,
        version: &str,
        package: Result<Option<PathBuf>, String>,
    ) -> UpdateOutcome {
        let product = p.product.as_str();
        let from = before.version.clone();
        self.set_installed(product, |i| {
            i.in_progress = Some(InProgress {
                from,
                to: version.to_string(),
            })
        });
        self.set_state(product, ProductState::Updating);
        tracing::info!("{product}: installing {version}");
        if p.is_self {
            return self.self_update(p, before, version, package).await;
        }
        let result = match package {
            Ok(Some(package)) => match self.install_version(p, version, &package).await {
                Ok(()) => self.healthy(p, version).await,
                Err(e) => Err(e),
            },
            Ok(None) => self.healthy(p, version).await,
            Err(e) => Err(e),
        };
        let outcome = match result {
            Ok(()) => {
                self.set_installed(product, |i| {
                    if before.version.is_some() {
                        i.previous = before.version.clone();
                    }
                    i.version = Some(version.to_string());
                });
                self.set_state(product, ProductState::Running);
                self.prune(product);
                UpdateOutcome {
                    from: before.version.clone(),
                    to: version.to_string(),
                    result: UpdateResult::Installed,
                    message: String::new(),
                    at: now(),
                }
            }
            Err(why) => {
                tracing::warn!("{product} {version}: {why}");
                self.restore(p, before, version, why).await
            }
        };
        self.set_installed(product, |i| {
            i.last_update = Some(outcome.clone());
            i.in_progress = None;
        });
        outcome
    }

    /// The Launcher's own update: a helper installs `version`, restarts the Launcher and goes
    /// back when it isn't healthy ([`crate::selfupdate`]). The update stays marked under way;
    /// the Launcher that starts records its outcome ([`Launcher::start_products`]).
    async fn self_update(
        &self,
        p: &ProductConfig,
        before: &Installed,
        version: &str,
        package: Result<Option<PathBuf>, String>,
    ) -> UpdateOutcome {
        let product = p.product.as_str();
        let started = match package {
            Ok(Some(package)) => match &self.cfg.path {
                Some(config) => {
                    let back = before
                        .version
                        .as_ref()
                        .and_then(|v| self.package_to_go_back(p, v).ok());
                    let req = crate::selfupdate::Request {
                        config: config.clone(),
                        product: product.to_string(),
                        from: before.version.clone(),
                        to: version.to_string(),
                        package,
                        back,
                    };
                    let cfg = self.cfg.clone();
                    tokio::task::spawn_blocking(move || crate::selfupdate::spawn(&cfg, &req))
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r)
                }
                None => Err("the Launcher's configuration file isn't known".into()),
            },
            Ok(None) => Err("the new version isn't stored".into()),
            Err(e) => Err(e),
        };
        match started {
            Ok(()) => {
                tracing::info!(
                    "{product}: the self-update helper installs {version} and restarts the Launcher"
                );
                UpdateOutcome {
                    from: before.version.clone(),
                    to: version.to_string(),
                    result: UpdateResult::Installed,
                    message: format!(
                        "the Launcher restarts into {version} to finish (it goes back by itself if {version} doesn't come up)"
                    ),
                    at: now(),
                }
            }
            Err(e) => {
                tracing::error!("{product}: {e}");
                self.set_state(product, ProductState::Running);
                let o = UpdateOutcome {
                    from: before.version.clone(),
                    to: version.to_string(),
                    result: UpdateResult::Failed,
                    message: e,
                    at: now(),
                };
                self.set_installed(product, |i| {
                    i.last_update = Some(o.clone());
                    i.in_progress = None;
                });
                o
            }
        }
    }

    /// Finishes the updates and rollbacks a stopped Launcher left under way (the installer
    /// may have replaced the product already): the new version is checked again and kept
    /// when healthy, otherwise the one before goes back. A `.deb`'s install may still be
    /// running (the Launcher's unit leaves it running when the Launcher stops): it is waited
    /// for, and a new version it installed is only checked, not installed again. The products
    /// it finished.
    async fn finish_interrupted(&self) -> Vec<String> {
        let mut done = Vec::new();
        for p in self.cfg.products.clone() {
            if self.installed(&p.product).in_progress.is_none() {
                continue;
            }
            let _busy = self.busy.lock().await;
            // (Read again under the lock: an action may have finished it meanwhile.)
            let i = self.installed(&p.product);
            let Some(ip) = i.in_progress.clone() else {
                continue;
            };
            if p.is_self {
                let (result, message) = crate::selfupdate::finish(self, &ip.to).await;
                tracing::info!("the Launcher's update to {}: {result:?} {message}", ip.to);
                self.set_installed(&p.product, |i| {
                    if result == UpdateResult::Installed {
                        if ip.from.is_some() {
                            i.previous = ip.from.clone();
                        }
                        i.version = Some(ip.to.clone());
                    } else if let Some(f) = &ip.from {
                        i.version = Some(f.clone());
                    }
                    i.last_update = Some(UpdateOutcome {
                        from: ip.from.clone(),
                        to: ip.to.clone(),
                        result,
                        message,
                        at: now(),
                    });
                    i.in_progress = None;
                });
                self.set_state(&p.product, ProductState::Running);
                done.push(p.product.clone());
                continue;
            }
            tracing::warn!(
                "{}: the update to {} was interrupted; checking it again",
                p.product,
                ip.to
            );
            let before = Installed {
                version: ip.from.clone(),
                previous: if ip.from == i.version {
                    i.previous.clone()
                } else {
                    i.version.clone()
                },
                ..i.clone()
            };
            let mut installed_already = false;
            if matches!(p.install, InstallKind::Deb { .. }) {
                self.set_state(&p.product, ProductState::Updating);
                let (dpkg, name) = (self.cfg.dpkg.clone(), p.product.clone());
                let now = tokio::task::spawn_blocking(move || {
                    let deadline = Instant::now() + Duration::from_secs(dpkg.lock_timeout_secs);
                    install::wait_dpkg_idle(&dpkg, deadline)
                        .then(|| install::deb_installed_version(&dpkg, &name))
                })
                .await
                .unwrap_or(None);
                let Some(now) = now else {
                    // Still installing: left under way, finished at the next start.
                    tracing::error!(
                        "{}: another install (dpkg or apt) is still running; the update to {} is checked at the Launcher's next start",
                        p.product,
                        ip.to
                    );
                    self.set_state(&p.product, ProductState::Failed);
                    done.push(p.product.clone());
                    continue;
                };
                installed_already = now.as_deref() == Some(ip.to.as_str());
            }
            let package = if installed_already {
                Ok(None)
            } else {
                self.verify_stored(&p.product, &ip.to)
                    .map(Some)
                    .map_err(|e| e.to_string())
            };
            let o = self.install_update(&p, &before, &ip.to, package).await;
            tracing::info!(
                "{}: interrupted update to {}: {:?}",
                p.product,
                ip.to,
                o.result
            );
            done.push(p.product.clone());
        }
        done
    }

    /// The package to go back to `version` of `p` with: the stored one, else (a `.deb`
    /// installed with apt from a repository, before the Launcher kept its packages) apt's
    /// cached download of it.
    fn package_to_go_back(
        &self,
        p: &ProductConfig,
        version: &str,
    ) -> Result<PathBuf, LauncherError> {
        match self.verify_stored(&p.product, version) {
            Ok(pkg) => Ok(pkg),
            Err(e) if self.stored_package(&p.product, version).is_none() => {
                if matches!(p.install, InstallKind::Deb { .. })
                    && let Some(cached) = install::apt_cached(&self.cfg.dpkg, &p.product, version)
                {
                    tracing::info!(
                        "{}: going back to {version} with apt's {}",
                        p.product,
                        cached.display()
                    );
                    return Ok(cached);
                }
                Err(LauncherError::Invalid(format!(
                    "{e}: {version} was installed outside the Launcher (with apt, or by hand), and its package isn't stored here{}, so there is nothing to go back to with (install {version} again by hand, or put it in the site's update channel before updating)",
                    if matches!(p.install, InstallKind::Deb { .. }) {
                        " or in apt's cache"
                    } else {
                        ""
                    }
                )))
            }
            Err(e) => Err(e),
        }
    }

    /// The new version failed: puts the one before back.
    async fn restore(
        &self,
        p: &ProductConfig,
        before: &Installed,
        version: &str,
        why: String,
    ) -> UpdateOutcome {
        let fail = |message: String| UpdateOutcome {
            from: before.version.clone(),
            to: version.to_string(),
            result: UpdateResult::Failed,
            message,
            at: now(),
        };
        let Some(old) = before.version.clone() else {
            self.set_state(&p.product, ProductState::Failed);
            return fail(format!("{why}; there is no earlier version to go back to"));
        };
        self.set_state(&p.product, ProductState::RollingBack);
        tracing::warn!("{}: rolling back to {old}", p.product);
        let back = match self.package_to_go_back(p, &old) {
            Ok(pkg) => match self.install_version(p, &old, &pkg).await {
                Ok(()) => self.healthy(p, &old).await,
                Err(e) => Err(e),
            },
            Err(e) => Err(e.to_string()),
        };
        match back {
            Ok(()) => {
                self.set_state(&p.product, ProductState::Running);
                UpdateOutcome {
                    from: Some(old.clone()),
                    to: version.to_string(),
                    result: UpdateResult::RolledBack,
                    message: format!("{why}; {old} is back"),
                    at: now(),
                }
            }
            Err(e) => {
                self.set_state(&p.product, ProductState::Failed);
                fail(format!("{why}; going back to {old} failed too: {e}"))
            }
        }
    }

    /// Records an update that failed before anything changed (a download, a bad signature), so
    /// the report shows it.
    pub fn record_failure(&self, product: &str, version: &str, message: &str) {
        if self.product(product).is_err() {
            return;
        }
        let from = self.installed(product).version;
        self.set_installed(product, |i| {
            i.last_update = Some(UpdateOutcome {
                from,
                to: version.to_string(),
                result: UpdateResult::Failed,
                message: message.to_string(),
                at: now(),
            })
        });
    }

    /// Goes back to the kept previous version of `product`.
    pub async fn rollback(&self, product: &str) -> Result<UpdateOutcome, LauncherError> {
        let p = self.product(product)?.clone();
        let _busy = self.busy.lock().await;
        let before = self.installed(product);
        let (Some(cur), Some(prev)) = (before.version.clone(), before.previous.clone()) else {
            return Err(LauncherError::Invalid(format!(
                "{} has no previous version to go back to",
                p.name
            )));
        };
        let package = self.package_to_go_back(&p, &prev)?;
        let from = Some(cur.clone());
        self.set_installed(product, |i| {
            i.in_progress = Some(InProgress {
                from,
                to: prev.clone(),
            })
        });
        self.set_state(product, ProductState::RollingBack);
        if p.is_self {
            // The Launcher's own package: the self-update helper installs it, restarts the
            // Launcher and goes back to `cur` when `prev` doesn't come up, as for an update
            // (installed here, the running Launcher would never restart into `prev`).
            let mut o = self
                .self_update(&p, &before, &prev, Ok(Some(package)))
                .await;
            if o.result == UpdateResult::Installed {
                o.message = format!("rolled back by hand: {}", o.message);
            }
            return Ok(o);
        }
        let result = match self.install_version(&p, &prev, &package).await {
            Ok(()) => self.healthy(&p, &prev).await,
            Err(e) => Err(e),
        };
        let outcome = match result {
            Ok(()) => {
                self.set_installed(product, |i| {
                    i.version = Some(prev.clone());
                    i.previous = Some(cur.clone());
                });
                self.set_state(product, ProductState::Running);
                UpdateOutcome {
                    from: Some(cur.clone()),
                    to: prev.clone(),
                    result: UpdateResult::Installed,
                    message: "rolled back by hand".into(),
                    at: now(),
                }
            }
            Err(why) => {
                // The version that was running goes back in.
                let mut o = self.restore(&p, &before, &prev, why).await;
                if o.result == UpdateResult::RolledBack {
                    o.result = UpdateResult::Failed;
                }
                o
            }
        };
        self.set_installed(product, |i| {
            i.last_update = Some(outcome.clone());
            i.in_progress = None;
        });
        Ok(outcome)
    }

    /// Keeps only the current and previous packages (and folders) of `product`.
    fn prune(&self, product: &str) {
        let i = self.installed(product);
        let keep = [i.version, i.previous];
        for base in [
            self.cfg.state_dir.join("packages").join(product),
            self.cfg.state_dir.join("products").join(product),
        ] {
            let Ok(entries) = std::fs::read_dir(&base) else {
                continue;
            };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !keep.iter().flatten().any(|k| *k == name) {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
    }

    /// At the Launcher's start: finishes interrupted updates, then starts the installed
    /// folder-kind products.
    pub async fn start_products(&self) {
        let finished = self.finish_interrupted().await;
        for p in self.cfg.products.clone() {
            if !matches!(p.install, InstallKind::Dir { .. }) || finished.contains(&p.product) {
                continue;
            }
            let Some(v) = self.installed(&p.product).version else {
                continue;
            };
            match self.verify_stored(&p.product, &v) {
                Ok(pkg) => {
                    self.set_state(&p.product, ProductState::Starting);
                    if let Err(e) = self.install_version(&p, &v, &pkg).await {
                        tracing::error!("{}: {e}", p.product);
                        self.set_state(&p.product, ProductState::Failed);
                    }
                }
                Err(e) => {
                    tracing::error!("{} {v}: {e}", p.product);
                    self.set_state(&p.product, ProductState::Failed);
                }
            }
        }
    }

    /// Stops the processes the Launcher runs.
    pub async fn shutdown(&self) {
        for p in &self.cfg.products {
            self.stop_supervisor(&p.product).await;
        }
    }

    /// Health-checks every product not being updated.
    pub async fn check(&self) {
        let busy = self.busy.try_lock().is_err();
        for p in &self.cfg.products {
            let state = lock(&self.runtimes).get(&p.product).map(|r| r.state);
            if busy
                && matches!(
                    state,
                    Some(ProductState::Updating | ProductState::RollingBack)
                )
            {
                continue;
            }
            let url = p.health_url.clone();
            let (dpkg, name) = (self.cfg.dpkg.clone(), p.product.clone());
            let probe = tokio::task::spawn_blocking(move || {
                if url.is_empty() {
                    // A package of programs: up while dpkg has it installed.
                    match install::deb_installed_version(&dpkg, &name) {
                        Some(_) => crate::health::Probe::Up(None),
                        None => crate::health::Probe::Down("not installed".into()),
                    }
                } else {
                    crate::health::probe(&url, Duration::from_secs(2))
                }
            })
            .await
            .unwrap_or(crate::health::Probe::Down(String::new()));
            // The version installed outside the Launcher (apt, the Windows installer), never
            // while an update runs: dpkg's for a `.deb`, the one it reports for an installer
            // (a newer one installed by hand over the Launcher's is taken too, so an older
            // channel package is never offered as an update).
            if let Ok(_idle) = self.busy.try_lock() {
                let found = match (&p.install, &probe) {
                    (InstallKind::Deb { .. }, _) => {
                        let (dpkg, name) = (self.cfg.dpkg.clone(), p.product.clone());
                        tokio::task::spawn_blocking(move || {
                            install::deb_installed_version(&dpkg, &name)
                        })
                        .await
                        .ok()
                        .flatten()
                    }
                    (InstallKind::Nsis { .. }, crate::health::Probe::Up(Some(v))) => {
                        Some(v.clone())
                    }
                    _ => None,
                };
                if let Some(v) = found {
                    self.adopt(&p.product, &v);
                }
            }
            let installed = self.installed(&p.product).version.is_some();
            let grace = Duration::from_secs(self.cfg.health_timeout_secs);
            let mut rt = lock(&self.runtimes);
            let Some(r) = rt.get_mut(&p.product) else {
                continue;
            };
            if matches!(r.state, ProductState::Updating | ProductState::RollingBack)
                && self.busy.try_lock().is_err()
            {
                // (An update started while it was probed.)
                continue;
            }
            match probe {
                crate::health::Probe::Up(_) => {
                    r.state = ProductState::Running;
                    r.down_since = None;
                    r.held = false;
                }
                crate::health::Probe::Down(_) if r.held => {
                    r.state = ProductState::Stopped;
                }
                crate::health::Probe::Down(_) => {
                    r.state = match r.state {
                        // Starting, or restarting after it went down: for the health
                        // timeout, then it failed.
                        ProductState::Running | ProductState::Starting => {
                            let since = *r.down_since.get_or_insert_with(Instant::now);
                            if since.elapsed() < grace {
                                ProductState::Starting
                            } else {
                                ProductState::Failed
                            }
                        }
                        _ if !installed => ProductState::Stopped,
                        _ => ProductState::Failed,
                    };
                }
            }
        }
    }

    /// The log file of a folder-kind product, or the configured one.
    pub fn log_path(&self, p: &ProductConfig) -> PathBuf {
        p.log_file.clone().unwrap_or_else(|| {
            self.cfg
                .state_dir
                .join("logs")
                .join(format!("{}.log", p.product))
        })
    }

    /// The last `lines` lines of `product`'s log.
    pub async fn logs(&self, product: &str, lines: usize) -> Result<String, LauncherError> {
        let p = self.product(product)?.clone();
        let path = self.log_path(&p);
        tokio::task::spawn_blocking(move || match (&p.install, &p.log_file) {
            (InstallKind::Deb { units }, None) if !units.is_empty() => {
                let mut c = std::process::Command::new("journalctl");
                c.args(["--no-pager", "-o", "short-iso", "-n", &lines.to_string()]);
                for u in units {
                    c.arg("-u").arg(u);
                }
                c.output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .map_err(io)
            }
            _ => match std::fs::read(&path) {
                Ok(b) => {
                    let t = String::from_utf8_lossy(&b);
                    let all: Vec<&str> = t.lines().collect();
                    Ok(all[all.len().saturating_sub(lines)..].join("\n"))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
                Err(e) => Err(io(e)),
            },
        })
        .await
        .map_err(io)?
    }

    /// The report: products, versions, states, the machine's numbers.
    pub fn report(&self) -> LauncherReport {
        let rt = lock(&self.runtimes);
        let products = self
            .cfg
            .products
            .iter()
            .map(|p| {
                let i = self.installed(&p.product);
                let r = rt.get(&p.product);
                LauncherProduct {
                    product: p.product.clone(),
                    name: p.name.clone(),
                    version: i.version,
                    previous: i.previous,
                    state: r.map(|r| r.state).unwrap_or(ProductState::Stopped),
                    restarts: r.map(|r| r.restarts.load(Ordering::Relaxed)).unwrap_or(0),
                    last_update: i.last_update,
                }
            })
            .collect();
        LauncherReport {
            machine_id: self.machine_id.clone(),
            machine_name: self.machine_name(),
            launcher_version: crate::VERSION.to_string(),
            platform: self.platform.clone(),
            roles: self.roles(),
            products,
            health: crate::health::machine(&self.cfg.state_dir),
        }
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A product that was up and then stays down shows `Starting` for the health timeout
    /// (systemd or the Launcher restarting it), then `Failed`: not `Starting` for ever.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_product_that_stays_down_fails_after_the_health_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let cfg: Config = serde_json::from_value(serde_json::json!({
            "stateDir": dir.path().join("state"),
            "keyDir": null,
            "healthTimeoutSecs": 1,
            "products": [{
                "product": "fake", "name": "Fake", "install": {"kind": "deb"},
                "healthUrl": format!("http://127.0.0.1:{port}/api/v1/server"),
            }],
        }))
        .unwrap();
        let l = Launcher::open(cfg).unwrap();
        l.set_installed("fake", |i| i.version = Some("1.0.0".into()));
        l.set_state("fake", ProductState::Running);
        let state = || l.report().products[0].state;
        l.check().await;
        assert_eq!(state(), ProductState::Starting, "just went down");
        l.check().await;
        assert_eq!(state(), ProductState::Starting, "within the health timeout");
        tokio::time::sleep(Duration::from_millis(1100)).await;
        l.check().await;
        assert_eq!(
            state(),
            ProductState::Failed,
            "still down after the health timeout"
        );
        l.check().await;
        assert_eq!(state(), ProductState::Failed);
    }
}
