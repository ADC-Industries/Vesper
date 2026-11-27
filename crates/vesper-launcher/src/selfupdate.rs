//! The Launcher's own update (`docs/LAUNCHER-PLAN.md`, round 3). The Launcher can't wait for
//! its own new version while the installer replaces and restarts it, so a **helper** does: a
//! copy of the running Launcher (`self-update/vesper-launcher-update`, so nothing that stops
//! `vesper-launcher` stops it), started outside the Launcher's service (Linux: a transient
//! systemd unit, `systemd-run`; Windows: a one-time scheduled task as SYSTEM, "Vesper Launcher
//! Update"). The helper:
//!
//! 1. installs the new package (`dpkg -i`, or the setup `/S`, which ends the Launcher's task);
//! 2. restarts the Launcher (`systemctl restart vesper-launcher`; Windows: runs its task);
//! 3. waits for the Launcher's health check (`GET /launcher/v1/version` answering the new
//!    version) for the health timeout;
//! 4. healthy: records `installed`; otherwise installs the kept previous package again,
//!    restarts it, and records `rolledBack` (or `failed`, with why).
//!
//! The outcome is written to `self-update/result.json`; the Launcher that starts (new or old)
//! finds its update marked under way in the manifest and records the outcome from there
//! ([`finish`]).

use crate::config::{Config, InstallKind, ProductConfig};
use crate::launcher::Launcher;
use protocol::UpdateResult;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The helper's scheduled task on Windows.
pub const WINDOWS_TASK: &str = "Vesper Launcher Update";

/// What the helper is asked to do (`self-update/request.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// The Launcher's configuration file.
    pub config: PathBuf,
    /// The Launcher's product ID.
    pub product: String,
    /// The version installed now.
    pub from: Option<String>,
    /// The version to install.
    pub to: String,
    /// Its checked package.
    pub package: PathBuf,
    /// The package to go back to `from` with, if there is one.
    pub back: Option<PathBuf>,
}

/// What the helper did (`self-update/result.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// The version it installed.
    pub to: String,
    /// Its result so far.
    pub result: UpdateResult,
    /// Why it went back, or failed.
    pub message: String,
    /// It finished (the Launcher it left is healthy, or nothing more can be done).
    pub done: bool,
    /// When (RFC 3339).
    pub at: String,
}

/// The helper's folder in the state folder.
pub fn dir(state: &Path) -> PathBuf {
    state.join("self-update")
}

fn result_path(state: &Path) -> PathBuf {
    dir(state).join("result.json")
}

/// The helper's last outcome, if any.
pub fn read_outcome(state: &Path) -> Option<Outcome> {
    crate::setup::read(&result_path(state)).ok().flatten()
}

fn write_outcome(state: &Path, o: &Outcome) {
    if let Err(e) = crate::setup::write(&result_path(state), o, false) {
        tracing::error!("can't write the self-update's outcome: {e}");
    }
}

/// Starts the helper for `req` (the Launcher keeps running until the helper restarts it).
pub fn spawn(cfg: &Config, req: &Request) -> Result<(), String> {
    let d = dir(&cfg.state_dir);
    std::fs::create_dir_all(&d).map_err(|e| format!("can't make {}: {e}", d.display()))?;
    let _ = std::fs::remove_file(result_path(&cfg.state_dir));
    // Linux: the running program itself, even when an upgrade by hand already replaced its file.
    let exe = if cfg!(target_os = "linux") && Path::new("/proc/self/exe").exists() {
        PathBuf::from("/proc/self/exe")
    } else {
        std::env::current_exe().map_err(|e| format!("the Launcher's program: {e}"))?
    };
    let helper = d.join(format!(
        "vesper-launcher-update{}",
        std::env::consts::EXE_SUFFIX
    ));
    let tmp = d.join(".helper");
    std::fs::copy(&exe, &tmp).map_err(|e| format!("can't copy the Launcher: {e}"))?;
    std::fs::rename(&tmp, &helper).map_err(|e| format!("can't copy the Launcher: {e}"))?;
    let req_path = d.join("request.json");
    crate::setup::write(&req_path, req, false)
        .map_err(|e| format!("can't write {}: {e}", req_path.display()))?;
    start_helper(cfg, &helper, &req_path)
}

#[cfg(windows)]
fn start_helper(cfg: &Config, helper: &Path, req: &Path) -> Result<(), String> {
    let run = |args: &[&str]| {
        std::process::Command::new(&cfg.system.schtasks)
            .args(args)
            .output()
            .map_err(|e| format!("schtasks didn't start: {e}"))
            .and_then(|o| {
                if o.status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "schtasks failed: {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    ))
                }
            })
    };
    let tr = format!(
        "\"{}\" self-update --request \"{}\"",
        helper.display(),
        req.display()
    );
    run(&[
        "/Create",
        "/F",
        "/TN",
        WINDOWS_TASK,
        "/SC",
        "ONCE",
        "/ST",
        "00:00",
        "/RU",
        "SYSTEM",
        "/RL",
        "HIGHEST",
        "/TR",
        &tr,
    ])?;
    run(&["/Run", "/TN", WINDOWS_TASK])
}

#[cfg(not(windows))]
fn start_helper(_cfg: &Config, helper: &Path, req: &Path) -> Result<(), String> {
    let mut cmd = if Path::new("/run/systemd/system").is_dir() {
        // Its own transient unit: a restart or stop of vesper-launcher.service doesn't reach it.
        let unit = format!(
            "vesper-launcher-update-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        );
        let mut c = std::process::Command::new("systemd-run");
        c.args([
            "--unit",
            &unit,
            "--collect",
            "--quiet",
            "--service-type=exec",
        ])
        .arg(helper);
        c
    } else {
        // No systemd (a container, a dev machine): a process of its own group.
        let mut c = std::process::Command::new(helper);
        std::os::unix::process::CommandExt::process_group(&mut c, 0);
        c
    };
    cmd.arg("self-update")
        .arg("--request")
        .arg(req)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("the self-update helper didn't start: {e}"))?;
    // systemd-run returns once the unit runs; a direct helper is left running.
    if Path::new("/run/systemd/system").is_dir() {
        let s = child.wait().map_err(|e| e.to_string())?;
        if !s.success() {
            return Err(format!("systemd-run failed ({s})"));
        }
    } else {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    Ok(())
}

/// Restarts the Launcher after its package was installed (the helper, blocking).
fn restart(cfg: &Config, p: &ProductConfig) -> Result<(), String> {
    let (program, args): (&Path, Vec<String>) = match &p.install {
        InstallKind::Deb { units } => {
            let unit = units
                .first()
                .cloned()
                .unwrap_or_else(|| "vesper-launcher.service".into());
            (&cfg.system.systemctl, vec!["restart".into(), unit])
        }
        InstallKind::Nsis { task, .. } => {
            let task = task.clone().unwrap_or_else(|| "Vesper Launcher".into());
            // The installer ended and started the task again; running it again is harmless
            // (it is already running then) and starts it if the installer didn't.
            let _ = std::process::Command::new(&cfg.system.schtasks)
                .args(["/Run", "/TN", &task])
                .output();
            return Ok(());
        }
        InstallKind::Dir { .. } => return Ok(()),
    };
    let o = std::process::Command::new(program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{} didn't start: {e}", program.display()))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(format!(
            "restarting the Launcher failed ({}): {}",
            o.status,
            String::from_utf8_lossy(&o.stderr).trim()
        ))
    }
}

/// The helper (`vesper-launcher self-update --request FILE`, blocking): installs, restarts,
/// checks, and goes back when the new Launcher isn't healthy. Returns the outcome it wrote.
pub fn run_helper(request: &Path) -> anyhow::Result<Outcome> {
    let req: Request = crate::setup::read(request)?
        .ok_or_else(|| anyhow::anyhow!("{} is missing", request.display()))?;
    let cfg = Config::load(&req.config)?;
    let p = cfg
        .products
        .iter()
        .find(|p| p.product == req.product)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no product {} in the configuration", req.product))?;
    let state = cfg.state_dir.clone();
    let timeout = Duration::from_secs(cfg.health_timeout_secs);
    let outcome = |result, message: String, done| Outcome {
        to: req.to.clone(),
        result,
        message,
        done,
        at: crate::launcher::now(),
    };
    let tried = crate::install::run_system_installer(&p.install, &req.package, &[], &cfg.dpkg)
        .and_then(|()| restart(&cfg, &p))
        .and_then(|()| crate::health::wait_healthy(&p.health_url, &req.to, timeout));
    let why = match tried {
        Ok(()) => {
            let o = outcome(UpdateResult::Installed, String::new(), true);
            write_outcome(&state, &o);
            return Ok(o);
        }
        Err(why) => why,
    };
    tracing::warn!("the Launcher's update to {}: {why}", req.to);
    let (Some(from), Some(back)) = (req.from.clone(), req.back.clone()) else {
        // Nothing to go back to: whatever runs now stays.
        let _ = restart(&cfg, &p);
        let o = outcome(
            UpdateResult::Failed,
            format!("{why}; there is no kept package of the version before to go back to"),
            true,
        );
        write_outcome(&state, &o);
        return Ok(o);
    };
    // Said before the old Launcher starts, so it finds why.
    write_outcome(
        &state,
        &outcome(
            UpdateResult::RolledBack,
            format!("{why}; going back to {from}"),
            false,
        ),
    );
    let back = crate::install::run_system_installer(&p.install, &back, &[], &cfg.dpkg)
        .and_then(|()| restart(&cfg, &p))
        .and_then(|()| crate::health::wait_healthy(&p.health_url, &from, timeout));
    let o = match back {
        Ok(()) => outcome(
            UpdateResult::RolledBack,
            format!("{why}; {from} is back"),
            true,
        ),
        Err(e) => outcome(
            UpdateResult::Failed,
            format!("{why}; going back to {from} failed too: {e}"),
            true,
        ),
    };
    write_outcome(&state, &o);
    Ok(o)
}

/// At the Launcher's start, its own update found under way (`from` → `to`): waits for the
/// helper's last word (at most the health timeout and a minute), then says what happened.
/// `(result, message, the version running now)`.
pub async fn finish(l: &Launcher, to: &str) -> (UpdateResult, String) {
    let state = l.config().state_dir.clone();
    let deadline = Instant::now() + Duration::from_secs(l.config().health_timeout_secs.max(1) + 60);
    let mut last = None;
    while Instant::now() < deadline {
        last = read_outcome(&state);
        if last.as_ref().is_some_and(|o| o.done && o.to == to) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let running_new = crate::health::same_version(crate::VERSION, to);
    match last.filter(|o| o.to == to) {
        Some(o) if o.result == UpdateResult::Installed && running_new => {
            (UpdateResult::Installed, String::new())
        }
        Some(o) if o.result != UpdateResult::Installed => (o.result, o.message),
        _ if running_new => (UpdateResult::Installed, String::new()),
        _ => (
            UpdateResult::Failed,
            format!(
                "the Launcher's update to {to} didn't finish (its helper stopped): {} is running",
                crate::VERSION
            ),
        ),
    }
}
