//! Acting on the machine's roles (`docs/LAUNCHER-PLAN.md`, round 3): the roles chosen in the
//! wizard are made true on the machine, and a role taken away is undone.
//!
//! | Role | Made true | Undone |
//! |---|---|---|
//! | presentation server | the Vesper server's service enabled and started (`vesper.service`, or `vesper-display.service` when that one is enabled; Windows: the "Vesper" task) | disabled and stopped |
//! | display box (Mosaic) | no Vesper server (disabled and stopped); the bundled IDN Node installed as the system's service (`idn-node install`); joined to the site with a code the site's server makes for this machine ([`crate::channel::Channel::idn_join_code`], so nobody types a second code); its display service switched on (`apps-display=on` and `apps-display-boot=on` on Linux: from boot, headless; `apps-display=on` on Windows, which needs a signed-in session) | the display service off; the Node's service removed when the Launcher installed it (`idn-node uninstall` keeps its state) |
//!
//! Only roles chosen in the wizard (or with `PUT /launcher/v1/setup`) are acted on, never the
//! configuration file's default, so an admin's own service setup isn't touched until someone
//! picks a role. Each step is recorded in `roles.json` as it succeeds, so a run that failed
//! (no site yet, the server unreachable) is retried from where it stopped, every minute and as
//! soon as the machine joins the site. A role whose undo has begun leaves `applied` at once, and
//! a Node service the Launcher installed is removed whenever the display role isn't wanted, so a
//! run that stopped partway never leaves the machine half set up when the roles change back.

use crate::config::InstallKind;
use crate::launcher::{Launcher, LauncherError, lock};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long one program the roles run may take (`idn-node join` waits for the site).
const STEP_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a failed run waits before it is tried again.
pub const RETRY_AFTER: Duration = Duration::from_secs(60);

/// How the bundled IDN Node is updated, as the diagnostics page says (H04-2 / R3-2).
const UPDATED_BY: &str = if cfg!(windows) {
    "it comes with the Launcher's installer: updating the Launcher from Site ▸ Updates updates it"
} else {
    "Site ▸ Updates: the Launcher installs newer vesper-idn-node packages"
};

/// The IDN Node's service is put back on the bundled `idn-node` at most this often (another
/// product fighting over it isn't restarted every check).
pub const GUARD_EVERY: Duration = Duration::from_secs(600);

/// What the Windows display box needs, shown with its role.
pub const WINDOWS_AUTOLOGON_NOTE: &str = "On Windows the display service runs in a signed-in session: set this PC to sign in to its display account automatically (Windows' own auto sign-in, or Sysinternals Autologon) and leave it signed in.";

/// `roles.json`: what the Launcher has made true.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RolesFile {
    /// The roles made true on the machine.
    #[serde(default)]
    pub applied: Vec<String>,
    /// The IDN Node's service was installed by the Launcher (removed again when the display
    /// role goes).
    #[serde(default)]
    pub node_installed: bool,
    /// The run towards these roles (`target`), with the steps done so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RoleRun>,
}

/// A run towards a set of roles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleRun {
    /// The roles it makes true.
    pub target: Vec<String>,
    /// `applying`, `done` or `failed`.
    pub state: String,
    /// The steps, in order, with how each went.
    pub steps: Vec<RoleStep>,
    /// Notes for people (what the machine still needs).
    #[serde(default)]
    pub notes: Vec<String>,
    /// When it last changed (RFC 3339).
    pub at: String,
}

/// One step of a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleStep {
    /// Its ID (`serverOn`, `serverOff`, `nodeInstall`, `nodeJoin`, `displayOn`, `displayOff`,
    /// `nodeUninstall`).
    pub id: String,
    /// What it does, for people.
    pub label: String,
    /// `pending`, `done`, `failed` or `skipped`.
    pub state: String,
    /// What went wrong, or what was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// The roles as the wizard and the window show them (`SetupView::role_status`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RolesView {
    /// The roles made true on the machine.
    pub applied: Vec<String>,
    /// The last run (or the one under way).
    pub run: Option<RoleRun>,
}

/// A step of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The Vesper server's service enabled and started.
    ServerOn,
    /// The Vesper server's service stopped and disabled.
    ServerOff,
    /// `idn-node install`.
    NodeInstall,
    /// A join code from the site's server, then `idn-node join CODE`.
    NodeJoin,
    /// The IDN Node's display service on.
    DisplayOn,
    /// The IDN Node's display service off.
    DisplayOff,
    /// `idn-node uninstall` (only when the Launcher installed it).
    NodeUninstall,
}

impl Step {
    fn id(self) -> &'static str {
        match self {
            Step::ServerOn => "serverOn",
            Step::ServerOff => "serverOff",
            Step::NodeInstall => "nodeInstall",
            Step::NodeJoin => "nodeJoin",
            Step::DisplayOn => "displayOn",
            Step::DisplayOff => "displayOff",
            Step::NodeUninstall => "nodeUninstall",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Step::ServerOn => "Start the Vesper server (and at every boot)",
            Step::ServerOff => "Stop the Vesper server here (a display box runs none)",
            Step::NodeInstall => "Install the IDN Node service",
            Step::NodeJoin => "Join the IDN Node to the site",
            Step::DisplayOn if cfg!(windows) => "Switch the display service on",
            Step::DisplayOn => "Switch the display service on (from boot, no desktop)",
            Step::DisplayOff => "Switch the display service off",
            Step::NodeUninstall => "Remove the IDN Node service",
        }
    }
}

/// The steps from `applied` to `target`: what goes is undone first, then what comes is made
/// true.
pub fn plan(applied: &[String], target: &[String], node_installed: bool) -> Vec<Step> {
    let has = |v: &[String], r: &str| v.iter().any(|x| x == r);
    let mut steps = Vec::new();
    // A Node service the Launcher installed while the display role isn't wanted (a run towards
    // it stopped partway, or its undo stopped partway) is undone too.
    if (has(applied, "display") || node_installed) && !has(target, "display") {
        steps.push(Step::DisplayOff);
        if node_installed {
            steps.push(Step::NodeUninstall);
        }
    }
    if has(applied, "presentation") && !has(target, "presentation") {
        steps.push(Step::ServerOff);
    }
    if has(target, "presentation") && !has(applied, "presentation") {
        steps.push(Step::ServerOn);
    }
    if has(target, "display") && !has(applied, "display") {
        if !steps.contains(&Step::ServerOff) {
            steps.push(Step::ServerOff);
        }
        steps.extend([Step::NodeInstall, Step::NodeJoin, Step::DisplayOn]);
    }
    steps
}

/// The roles that are acted on (the others are only recorded): sorted.
fn acted(roles: &[String]) -> Vec<String> {
    let mut v: Vec<String> = roles
        .iter()
        .filter(|r| matches!(r.as_str(), "presentation" | "display"))
        .cloned()
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Runs `program` with `args` (at most [`STEP_TIMEOUT`]); its output, or why it failed.
async fn run(program: &Path, args: &[&str]) -> Result<String, String> {
    run_shown(program, args, &args.join(" ")).await
}

/// [`run`], with `shown_args` in place of the arguments in what it says (a secret kept out of
/// the logs, `roles.json` and the report).
async fn run_shown(program: &Path, args: &[&str], shown_args: &str) -> Result<String, String> {
    let name = program
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let shown = format!("{name} {shown_args}");
    let out = tokio::time::timeout(
        STEP_TIMEOUT,
        tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        format!(
            "`{shown}` didn't finish within {} s",
            STEP_TIMEOUT.as_secs()
        )
    })?
    .map_err(|e| format!("`{shown}` didn't start: {e}"))?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if out.status.success() {
        Ok(text(&out.stdout))
    } else {
        let err = text(&out.stderr);
        let err = if err.is_empty() {
            text(&out.stdout)
        } else {
            err
        };
        let tail: Vec<&str> = err.lines().rev().take(3).collect();
        Err(format!(
            "`{shown}` failed ({}): {}",
            out.status,
            tail.into_iter().rev().collect::<Vec<_>>().join(" / ")
        ))
    }
}

/// The host of a server URL (`http://host:port` → `host`, IPv6 without brackets).
pub fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let hostport = rest.split('/').next()?;
    let host = if let Some(v6) = hostport.strip_prefix('[') {
        v6.split(']').next()?.to_string()
    } else {
        hostport
            .rsplit_once(':')
            .map_or(hostport, |(h, _)| h)
            .to_string()
    };
    (!host.is_empty()).then_some(host)
}

impl Launcher {
    /// The roles as the wizard shows them.
    pub fn roles_view(&self) -> RolesView {
        let f = lock(&self.roles_file);
        RolesView {
            applied: f.applied.clone(),
            run: f.run.clone(),
        }
    }

    fn save_roles(&self, f: &RolesFile) {
        let path = self.config().state_dir.join("roles.json");
        if let Err(e) = crate::setup::write(&path, f, false) {
            tracing::error!("can't save {}: {e}", path.display());
        }
    }

    /// Roles are to be acted on again now (the machine joined the site, the roles changed).
    pub fn roles_changed(&self) {
        *lock(&self.roles_retry) = None;
    }

    /// Makes the machine's chosen roles true, if they aren't (one run at a time; a failed run
    /// is tried again after [`RETRY_AFTER`] or when [`Launcher::roles_changed`] is called).
    pub async fn act_on_roles(self: &Arc<Self>) {
        let Ok(_busy) = self.roles_busy.try_lock() else {
            return;
        };
        // Only roles someone chose (the wizard, PUT /launcher/v1/setup).
        let Some(chosen) = self.chosen_roles() else {
            return;
        };
        let target = acted(&chosen);
        let (applied, node_installed, mut run) = {
            let f = lock(&self.roles_file);
            (acted(&f.applied), f.node_installed, f.run.clone())
        };
        let steps = plan(&applied, &target, node_installed);
        if steps.is_empty() {
            // Nothing to do. A run that stopped partway towards other roles is over: what it
            // undid is no longer in `applied` (so going back redoes it), and it isn't retried.
            if run.as_ref().is_some_and(|r| r.state != "done") {
                let mut f = lock(&self.roles_file);
                f.run = None;
                f.applied = target;
                let f2 = f.clone();
                drop(f);
                self.save_roles(&f2);
            }
            *lock(&self.roles_retry) = None;
            return;
        }
        if let Some(t) = *lock(&self.roles_retry)
            && Instant::now() < t
        {
            return;
        }
        // A run towards the same roles keeps the steps it finished.
        let same = run.as_ref().is_some_and(|r| r.target == target);
        if !same {
            run = None;
        }
        let mut r = run.unwrap_or_else(|| RoleRun {
            target: target.clone(),
            state: "applying".into(),
            steps: steps
                .iter()
                .map(|s| RoleStep {
                    id: s.id().into(),
                    label: s.label().into(),
                    state: "pending".into(),
                    message: None,
                })
                .collect(),
            notes: Vec::new(),
            at: crate::launcher::now(),
        });
        r.state = "applying".into();
        if target.iter().any(|t| t == "display") && cfg!(windows) {
            r.notes = vec![WINDOWS_AUTOLOGON_NOTE.into()];
        }
        self.record_run(&r);
        tracing::info!(?applied, ?target, "acting on the machine's roles");
        let mut failed = false;
        for s in &steps {
            let i = r
                .steps
                .iter()
                .position(|x| x.id == s.id())
                .unwrap_or_else(|| {
                    r.steps.push(RoleStep {
                        id: s.id().into(),
                        label: s.label().into(),
                        state: "pending".into(),
                        message: None,
                    });
                    r.steps.len() - 1
                });
            if matches!(r.steps[i].state.as_str(), "done" | "skipped") {
                continue;
            }
            match self.step(*s).await {
                Ok(note) => {
                    r.steps[i].state = if note.as_deref().is_some_and(|n| n.starts_with('-')) {
                        "skipped".into()
                    } else {
                        "done".into()
                    };
                    r.steps[i].message = note.map(|n| n.trim_start_matches('-').to_string());
                    tracing::info!("roles: {}: done", s.label());
                    // A role whose undo has begun is no longer made true: a later run (back
                    // to it, or on) plans from what the machine really has.
                    let undone = match s {
                        Step::DisplayOff | Step::NodeUninstall => Some("display"),
                        Step::ServerOff if !target.iter().any(|t| t == "presentation") => {
                            Some("presentation")
                        }
                        _ => None,
                    };
                    if let Some(role) = undone {
                        let mut f = lock(&self.roles_file);
                        if f.applied.iter().any(|a| a == role) {
                            f.applied.retain(|a| a != role);
                            let f2 = f.clone();
                            drop(f);
                            self.save_roles(&f2);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("roles: {}: {e}", s.label());
                    r.steps[i].state = "failed".into();
                    r.steps[i].message = Some(e);
                    failed = true;
                }
            }
            r.at = crate::launcher::now();
            self.record_run(&r);
            if failed {
                break;
            }
        }
        if failed {
            r.state = "failed".into();
            *lock(&self.roles_retry) = Some(Instant::now() + RETRY_AFTER);
        } else {
            r.state = "done".into();
            lock(&self.roles_file).applied = target.clone();
        }
        r.at = crate::launcher::now();
        self.record_run(&r);
    }

    /// The bundled IDN Node stays Vesper's to update ([`crate::idn_node`]): its marker put back
    /// when missing, and (Linux and Windows) the system's IDN Node service put back on the bundled
    /// `idn-node` when the Launcher installed it and another program replaced it, with a
    /// warning (the log, the diagnostics page).
    pub async fn idn_node_guard(self: &Arc<Self>) {
        use crate::idn_node::{self, Marker};
        let sys = self.config().system.clone();
        let dir = sys.idn_dir.clone();
        let marker = tokio::task::spawn_blocking(move || idn_node::ensure_marker(&dir))
            .await
            .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())));
        let warn = |w: String| {
            tracing::warn!("{w}");
            lock(&self.idn_guard).warning = Some((crate::launcher::now(), w));
        };
        match &marker {
            Ok(Marker::Restored(had)) => {
                warn(format!(
                    "the bundled IDN Node's {} {}: put back (Vesper updates this IDN Node, not the Site Manager)",
                    idn_node::MARKER,
                    match had {
                        Some(p) => format!("named {p}"),
                        None => "was missing".into(),
                    }
                ));
            }
            Ok(Marker::Kept) => {}
            Ok(Marker::NoNode) => {
                // No bundled Node (the package removed): nothing to show.
                let mut g = lock(&self.idn_guard);
                *g = crate::launcher::IdnGuard {
                    put_back: g.put_back,
                    ..Default::default()
                };
            }
            Err(e) => {
                // Said once (H04-3: also when it fails from the first check on), again when
                // the reason changes.
                let e = e.to_string();
                if lock(&self.idn_guard).marker_error.as_deref() != Some(e.as_str()) {
                    warn(format!(
                        "the bundled IDN Node's {} can't be written: {e}",
                        idn_node::MARKER
                    ));
                }
            }
        }
        {
            let mut g = lock(&self.idn_guard);
            g.node = !matches!(marker, Ok(Marker::NoNode));
            g.marker_error = marker.as_ref().err().map(|e| e.to_string());
        }
        let installed = lock(&self.roles_file).node_installed;
        let service = if installed {
            self.idn_node_service(&sys).await
        } else {
            None
        };
        // H04-4 / R3-4: flagged while the service runs another program, not after.
        lock(&self.idn_guard).replaced = service.as_ref().is_some_and(|s| !s.ours);
        let node = sys.idn_program("idn-node");
        let since = lock(&self.idn_guard).put_back.map(|t| t.elapsed());
        let action = |role_run| {
            idn_node::guard_action(
                installed,
                service.as_ref(),
                node.is_file(),
                since,
                GUARD_EVERY,
                role_run,
            )
        };
        if action(false) != idn_node::GuardAction::PutBack {
            return;
        }
        // Not while the roles are being made true (they install and remove the service); the
        // run's lock is held while the service is put back. Taken only when it would be put
        // back, so a role run is never kept waiting by an idle check.
        let busy = self.roles_busy.try_lock();
        if action(busy.is_err()) != idn_node::GuardAction::PutBack {
            return;
        }
        let Some(service) = service else {
            return;
        };
        let program = service.shown;
        lock(&self.idn_guard).put_back = Some(Instant::now());
        match run(&node, &["install"]).await {
            Ok(_) => {
                lock(&self.idn_guard).replaced = false;
                warn(format!(
                    "the IDN Node's service ran {program} instead of Vesper's bundled IDN Node (another IDN install replaced it): Vesper's was put back. Remove the other IDN Node {} from this display box; Vesper updates its IDN Node",
                    if cfg!(windows) {
                        "install (IDN Media)"
                    } else {
                        "package"
                    }
                ))
            }
            Err(e) => warn(format!(
                "the IDN Node's service runs {program} instead of Vesper's bundled IDN Node (another IDN install replaced it), and Vesper's couldn't be put back: {e}"
            )),
        }
    }

    /// The program the system's IDN Node service runs (`None`: no service, or it can't be
    /// read): Linux, its systemd unit's `ExecStart`; Windows, the service manager's image path
    /// of the service `idn-node`.
    async fn idn_node_service(
        &self,
        sys: &crate::config::SystemConfig,
    ) -> Option<crate::idn_node::ServiceProgram> {
        use crate::idn_node::{self, ServiceProgram};
        #[cfg(windows)]
        {
            let image = tokio::task::spawn_blocking(|| {
                idn_node::windows_service_image(idn_node::WINDOWS_SERVICE)
            })
            .await
            .ok()?;
            let image = match image {
                Ok(i) => i?,
                Err(e) => {
                    tracing::debug!("the IDN Node's service can't be read: {e}");
                    return None;
                }
            };
            let program = idn_node::windows_image_program(&image)?;
            Some(ServiceProgram {
                ours: idn_node::windows_is_ours(&program, &sys.idn_dir),
                shown: program,
            })
        }
        #[cfg(not(windows))]
        {
            let program = idn_node::service_program(&sys.idn_node_unit)?;
            Some(ServiceProgram {
                ours: idn_node::is_ours(&program, &sys.idn_dir),
                shown: program.display().to_string(),
            })
        }
    }

    /// The bundled IDN Node for the diagnostics page: who updates it, and the guard's last
    /// warning (`None`: no bundled Node here). Flagged while its marker can't be written or
    /// the Node's service runs another program (as of the last check), not for what was put
    /// right since (H04-4 / R3-4).
    pub fn idn_node_view(&self) -> Option<(String, bool)> {
        let g = lock(&self.idn_guard).clone();
        if !g.node {
            return None;
        }
        let mut text = match &g.marker_error {
            None => format!(
                "updates managed by Vesper, not the Site Manager ({})",
                UPDATED_BY
            ),
            Some(e) => format!("its updates marker can't be written ({e})"),
        };
        if g.replaced {
            text.push_str("; its service runs another IDN Node");
        }
        if let Some((at, w)) = &g.warning {
            text.push_str(&format!("; last: {at}: {w}"));
        }
        Some((text, g.marker_error.is_some() || g.replaced))
    }

    fn record_run(&self, r: &RoleRun) {
        let mut f = lock(&self.roles_file);
        f.run = Some(r.clone());
        let f2 = f.clone();
        drop(f);
        self.save_roles(&f2);
    }

    /// One step: `Ok(note)` (a note starting with `-`: nothing to do, skipped).
    async fn step(self: &Arc<Self>, s: Step) -> Result<Option<String>, String> {
        let sys = self.config().system.clone();
        let node = sys.idn_program("idn-node");
        let node_missing = || {
            if cfg!(windows) {
                format!(
                    "the IDN Node isn't installed with the Launcher here ({} is missing): run the Vesper Launcher installer again",
                    node.display()
                )
            } else {
                format!(
                    "the IDN Node isn't installed here ({} is missing): install the vesper-idn-node package",
                    node.display()
                )
            }
        };
        match s {
            Step::ServerOn | Step::ServerOff => self.server_service(s == Step::ServerOn).await,
            Step::NodeInstall => {
                if !node.is_file() {
                    return Err(node_missing());
                }
                run(&node, &["install"]).await?;
                lock(&self.roles_file).node_installed = true;
                let f = lock(&self.roles_file).clone();
                self.save_roles(&f);
                Ok(None)
            }
            Step::NodeJoin => {
                if !node.is_file() {
                    return Err(node_missing());
                }
                let Some(site) = self.site() else {
                    return Err("join this machine to the site first (the setup's second step): its IDN Node then joins with a code from the site's server".into());
                };
                let channel = self.channel().ok_or("no site to join")?;
                let machine = self.machine_id().to_string();
                let code = tokio::task::spawn_blocking(move || channel.idn_join_code(&machine))
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| format!("the site's server didn't make a join code: {e}"))?;
                let host =
                    url_host(&site.server).ok_or_else(|| format!("{} has no host", site.server))?;
                run(&node, &["set", &format!("server={host}")]).await?;
                // The code is a secret until it's used: never in what the step says.
                run_shown(&node, &["join", &code.code], "join ******").await?;
                Ok(Some(format!("joined through {}", site.server_name)))
            }
            Step::DisplayOn | Step::DisplayOff => {
                if !node.is_file() {
                    return if s == Step::DisplayOff {
                        Ok(Some("-the IDN Node isn't installed".into()))
                    } else {
                        Err(node_missing())
                    };
                }
                let on = if s == Step::DisplayOn { "on" } else { "off" };
                // Windows: the service in the signed-in session. Linux: headless from boot,
                // which needs both settings on (the IDN maintainers' answer of 2026-11-26, as
                // Mosaic sends them); off means both off.
                run(&node, &["set", &format!("apps-display={on}")]).await?;
                if !cfg!(windows) {
                    run(&node, &["set", &format!("apps-display-boot={on}")]).await?;
                }
                Ok(None)
            }
            Step::NodeUninstall => {
                if !node.is_file() {
                    return Ok(Some("-the IDN Node isn't installed".into()));
                }
                run(&node, &["uninstall"]).await?;
                lock(&self.roles_file).node_installed = false;
                let f = lock(&self.roles_file).clone();
                self.save_roles(&f);
                Ok(None)
            }
        }
    }

    /// The Vesper server's service on (enabled and started) or off (stopped and disabled).
    async fn server_service(self: &Arc<Self>, on: bool) -> Result<Option<String>, String> {
        let Some(p) = self
            .config()
            .products
            .iter()
            .find(|p| p.product == protocol::PRODUCT_ID)
            .cloned()
        else {
            return if on {
                Err(if cfg!(windows) {
                    "the Vesper server isn't installed on this PC: install it (vesper-server setup) first".into()
                } else {
                    "the Vesper server isn't installed on this machine: install the vesper package first".into()
                })
            } else {
                Ok(Some("-no Vesper server on this machine".into()))
            };
        };
        let sys = self.config().system.clone();
        match &p.install {
            InstallKind::Deb { units } => {
                if units.is_empty() {
                    return Err("no systemd unit is configured for the Vesper server".into());
                }
                if on {
                    let mut unit = units[0].clone();
                    for u in units {
                        if run(&sys.systemctl, &["is-enabled", "--quiet", u])
                            .await
                            .is_ok()
                        {
                            unit = u.clone();
                            break;
                        }
                    }
                    run(&sys.systemctl, &["enable", "--now", &unit]).await?;
                } else {
                    let mut args = vec!["disable", "--now"];
                    args.extend(units.iter().map(String::as_str));
                    run(&sys.systemctl, &args).await?;
                }
            }
            InstallKind::Nsis { task, .. } => {
                let t = task
                    .as_deref()
                    .ok_or("no scheduled task is configured for the Vesper server")?;
                if on {
                    run(&sys.schtasks, &["/Change", "/TN", t, "/ENABLE"]).await?;
                    run(&sys.schtasks, &["/Run", "/TN", t]).await?;
                } else {
                    let _ = run(&sys.schtasks, &["/End", "/TN", t]).await;
                    run(&sys.schtasks, &["/Change", "/TN", t, "/DISABLE"]).await?;
                }
            }
            InstallKind::Dir { .. } => {
                let r = if on {
                    self.start_product(&p.product).await
                } else {
                    self.stop_product(&p.product).await
                };
                r.map_err(|e: LauncherError| e.to_string())?;
                return Ok(None);
            }
        }
        self.hold(&p.product, !on);
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(r: &[&str]) -> Vec<String> {
        r.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plans_make_roles_true_and_undo_what_goes() {
        use Step::*;
        assert_eq!(plan(&[], &v(&["presentation"]), false), vec![ServerOn]);
        assert_eq!(
            plan(&[], &v(&["display"]), false),
            vec![ServerOff, NodeInstall, NodeJoin, DisplayOn]
        );
        assert_eq!(
            plan(&v(&["presentation"]), &v(&["display"]), false),
            vec![ServerOff, NodeInstall, NodeJoin, DisplayOn]
        );
        assert_eq!(
            plan(&v(&["display"]), &v(&["presentation"]), true),
            vec![DisplayOff, NodeUninstall, ServerOn]
        );
        assert_eq!(
            plan(&v(&["display"]), &v(&["presentation"]), false),
            vec![DisplayOff, ServerOn]
        );
        assert_eq!(plan(&v(&["presentation"]), &[], false), vec![ServerOff]);
        assert!(plan(&v(&["display"]), &v(&["display"]), true).is_empty());
        // A Node service left from a run that stopped partway goes with the role.
        assert_eq!(plan(&[], &[], true), vec![DisplayOff, NodeUninstall]);
        assert_eq!(
            plan(&[], &v(&["presentation"]), true),
            vec![DisplayOff, NodeUninstall, ServerOn]
        );
        assert_eq!(acted(&v(&["mixer", "display"])), v(&["display"]));
    }

    /// A join code never shows in what a failed step says (the journal, `roles.json`, the
    /// report).
    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_join_doesnt_show_its_code() {
        let e = run_shown(Path::new("/bin/false"), &["join", "482913"], "join ******")
            .await
            .unwrap_err();
        assert!(e.contains("join ******"), "{e}");
        assert!(!e.contains("482913"), "{e}");
    }

    #[test]
    fn server_hosts_come_out_of_urls() {
        assert_eq!(
            url_host("http://192.0.2.10:5555").as_deref(),
            Some("192.0.2.10")
        );
        assert_eq!(
            url_host("http://main.local:5555").as_deref(),
            Some("main.local")
        );
        assert_eq!(
            url_host("http://[2001:db8::1]:5555").as_deref(),
            Some("2001:db8::1")
        );
        assert_eq!(url_host("http://:5555"), None);
    }
}
