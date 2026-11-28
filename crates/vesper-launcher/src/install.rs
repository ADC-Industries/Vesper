//! Installing a stored package, per [`InstallKind`]: `apt-get install ./<package>.deb` (Linux), the NSIS installer run
//! silently (Windows), or a versioned folder whose executable the Launcher runs itself
//! ([`Supervisor`], restarted after a crash with back-off).

use crate::config::{DpkgConfig, InstallKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// Runs a system installer for `kind` (blocking). For [`InstallKind::Dir`] nothing runs here:
/// the Launcher copies and starts the executable ([`dir_install`], [`Supervisor`]).
///
/// A `.deb` is installed with apt (`apt-get install ./<package>.deb`, [`APT_INSTALL`]), so
/// the system packages it needs come along from the machine's repositories (the package lists
/// are refreshed once when apt can't find or fetch one). It waits until no other dpkg or apt
/// run holds dpkg's lock (an install the Launcher started before it restarted, unattended
/// upgrades), for at most `dpkg.lock_timeout_secs`, and is installed without questions
/// ([`APT_ENV`]): a configuration file the admin edited is kept (the package's new one is left
/// beside it as `.dpkg-dist`), an older version is installed when asked (a rollback), and
/// nothing is ever removed (apt stops instead). A failure says what apt said
/// ([`apt_failure`]).
///
/// `with`: more `.deb`s installed in the same run (the packages of other products this one's
/// package needs at a newer version: [`crate::Launcher`]'s companions).
pub fn run_system_installer(
    kind: &InstallKind,
    package: &Path,
    with: &[PathBuf],
    dpkg: &DpkgConfig,
) -> Result<(), String> {
    let deb = matches!(kind, InstallKind::Deb { .. });
    let mut cmd = match kind {
        InstallKind::Deb { .. } => {
            let mut c = std::process::Command::new(&dpkg.apt);
            c.args(APT_INSTALL)
                .args(with.iter().map(|p| local_deb(p)))
                .arg(local_deb(package))
                .envs(APT_ENV);
            c
        }
        InstallKind::Nsis { args, .. } => {
            let mut c = std::process::Command::new(package);
            c.arg("/S").args(args);
            c
        }
        InstallKind::Dir { .. } => return Ok(()),
    };
    cmd.stdin(std::process::Stdio::null());
    let deadline = Instant::now() + Duration::from_secs(dpkg.lock_timeout_secs);
    let mut refreshed = false;
    loop {
        if deb && !wait_dpkg_idle(dpkg, deadline) {
            return Err(format!(
                "another install (dpkg or apt) was still running after {} s",
                dpkg.lock_timeout_secs
            ));
        }
        let out = cmd
            .output()
            .map_err(|e| format!("the installer didn't start: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let (stdout, stderr) = (
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        if !deb {
            let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
            return Err(format!(
                "the installer failed ({}): {}",
                out.status,
                tail.into_iter().rev().collect::<Vec<_>>().join(" / ")
            ));
        }
        let text = format!("{stdout}\n{stderr}");
        if LOCKED.iter().any(|m| text.contains(m)) && Instant::now() < deadline {
            // Another dpkg or apt took the lock in between: wait for it again.
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        if !refreshed && STALE_LISTS.iter().any(|m| text.contains(m)) {
            // A needed package isn't in (or can't be fetched with) the package lists apt has:
            // refreshed once, then tried again.
            refreshed = true;
            let updated = std::process::Command::new(&dpkg.apt)
                .args(["update", "-q"])
                .envs(APT_ENV)
                .stdin(std::process::Stdio::null())
                .output();
            match updated {
                Ok(u) if u.status.success() => continue,
                Ok(u) => tracing::warn!(
                    "apt-get update failed ({}): {}",
                    u.status,
                    String::from_utf8_lossy(&u.stderr).trim()
                ),
                Err(e) => tracing::warn!("apt-get update didn't start: {e}"),
            }
        }
        return Err(format!(
            "apt couldn't install it ({}): {}",
            out.status,
            apt_failure(&stdout, &stderr)
        ));
    }
}

/// `apt-get`'s arguments before the packages: install without asking (`-y`), quietly (no
/// progress lines), an older version too (a rollback), never removing a package to make room
/// (it stops instead: a package that `Breaks:` the Launcher must not take it away), and keep a
/// configuration file the admin edited (`--force-confdef --force-confold`) instead of asking
/// (with no one to answer, dpkg would stop there and leave the package half-configured).
pub const APT_INSTALL: [&str; 9] = [
    "install",
    "-y",
    "-q",
    "--allow-downgrades",
    "--no-remove",
    "-o",
    "Dpkg::Options::=--force-confdef",
    "-o",
    "Dpkg::Options::=--force-confold",
];

/// The environment of an apt run: no questions (debconf, apt-listchanges), Ubuntu's
/// `needrestart` left out (it would ask, or restart services, the Launcher among them), and
/// apt's messages in English (read here and shown in the failure).
pub const APT_ENV: [(&str, &str); 4] = [
    ("DEBIAN_FRONTEND", "noninteractive"),
    ("APT_LISTCHANGES_FRONTEND", "none"),
    ("NEEDRESTART_SUSPEND", "1"),
    ("LC_ALL", "C.UTF-8"),
];

/// What dpkg and apt say when another run holds the lock.
const LOCKED: [&str; 3] = [
    "locked by another process",
    "Could not get lock",
    "Unable to acquire the dpkg frontend lock",
];

/// What apt says when its package lists are out of date (or were never fetched).
const STALE_LISTS: [&str; 4] = [
    "Unable to fetch some archives",
    "is not installable",
    "has no installation candidate",
    "404  Not Found",
];

/// A path apt reads as a package file, not a package name (it has a `/`).
fn local_deb(p: &Path) -> PathBuf {
    if p.is_absolute() || p.starts_with(".") {
        p.to_path_buf()
    } else {
        Path::new(".").join(p)
    }
}

/// apt's output, trimmed to what says why an install failed: its errors (`E:`), the
/// unmet-dependency lines and dpkg's errors, else its last lines; at most 8 lines and 600
/// characters, joined with ` / `.
pub fn apt_failure(stdout: &str, stderr: &str) -> String {
    let lines: Vec<&str> = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .filter(|l| !l.is_empty())
        // apt's note that it read a root-only file as root: not why it failed.
        .filter(|l| !l.starts_with("N: Download is performed unsandboxed"))
        .collect();
    let telling: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| {
            l.starts_with("E:")
                || l.starts_with("dpkg: error")
                || l.contains("Depends:")
                || l.contains("Breaks:")
                || l.contains("Conflicts:")
        })
        .collect();
    let pick = if telling.is_empty() { &lines } else { &telling };
    let tail = &pick[pick.len().saturating_sub(8)..];
    let mut text = tail.join(" / ");
    if text.chars().count() > 600 {
        let cut: String = text.chars().take(599).collect();
        text = format!("{cut}…");
    }
    if text.is_empty() {
        "it said nothing".into()
    } else {
        text
    }
}

/// A dependency of a `.deb`: the package's name and its version bound (`>=`, `<<`, …).
pub type DebDepend = (String, Option<(String, String)>);

/// The dependencies on single packages of a `Depends:` field (alternatives `a | b` are left to
/// dpkg; an architecture qualifier `:any` is dropped).
pub fn parse_depends(field: &str) -> Vec<DebDepend> {
    field
        .split(',')
        .map(str::trim)
        .filter(|d| !d.is_empty() && !d.contains('|'))
        .filter_map(|d| {
            let (name, bound) = match d.split_once('(') {
                Some((n, rest)) => {
                    let inner = rest.trim_end_matches(')').trim();
                    let op_len = inner
                        .find(|c: char| !matches!(c, '<' | '>' | '='))
                        .unwrap_or(inner.len());
                    let (op, v) = inner.split_at(op_len);
                    (
                        n.trim(),
                        Some((op.trim().to_string(), v.trim().to_string())),
                    )
                }
                None => (d, None),
            };
            let name = name.split(':').next().unwrap_or(name).trim();
            (!name.is_empty()).then(|| (name.to_string(), bound))
        })
        .collect()
}

/// Whether version `have` meets bound `op` `want` (`>=`, `<=`, `>>`, `<<`, `=`).
pub fn satisfies(have: &str, op: &str, want: &str) -> bool {
    use std::cmp::Ordering::*;
    let o = crate::package::compare_versions(have, want);
    match op {
        ">=" | ">" => o != Less,
        "<=" | "<" => o != Greater,
        ">>" => o == Greater,
        "<<" => o == Less,
        _ => o == Equal,
    }
}

/// A `.deb`'s dependencies (`dpkg-deb -f PACKAGE Depends`, blocking). One dpkg-deb can't read
/// has none here: apt says what is wrong with it.
pub fn deb_depends(dpkg: &DpkgConfig, package: &Path) -> Vec<DebDepend> {
    match std::process::Command::new(&dpkg.deb)
        .arg("-f")
        .arg(package)
        .arg("Depends")
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(out) if out.status.success() => parse_depends(&String::from_utf8_lossy(&out.stdout)),
        Ok(out) => {
            tracing::debug!(
                "dpkg-deb can't read {}: {}",
                package.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
            Vec::new()
        }
        Err(e) => {
            tracing::debug!("dpkg-deb didn't start: {e}");
            Vec::new()
        }
    }
}

/// The version of Debian package `package` dpkg has installed (blocking): `None` when it
/// isn't installed (or only its configuration files are left), or dpkg can't be asked.
pub fn deb_installed_version(dpkg: &DpkgConfig, package: &str) -> Option<String> {
    let out = std::process::Command::new(&dpkg.query)
        .args(["-W", "-f=${db:Status-Status}\t${Version}", package])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (status, version) = text.trim().split_once('\t')?;
    (status == "installed" && !version.is_empty()).then(|| version.to_string())
}

/// Waits until no dpkg or apt run holds dpkg's lock, at most until `deadline` (blocking);
/// whether it is free.
pub fn wait_dpkg_idle(dpkg: &DpkgConfig, deadline: Instant) -> bool {
    loop {
        if !dpkg_locked(&dpkg.admin_dir) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Whether a dpkg or apt run holds dpkg's lock (`lock-frontend` or `lock` in `admin_dir`):
/// asked with an open-file-description lock test, which sees the lock of any process
/// (this one's too) and takes nothing.
#[cfg(target_os = "linux")]
pub fn dpkg_locked(admin_dir: &Path) -> bool {
    use std::os::fd::AsRawFd;
    ["lock-frontend", "lock"].iter().any(|name| {
        let Ok(f) = std::fs::File::open(admin_dir.join(name)) else {
            return false;
        };
        // SAFETY: a plain C struct; all zeroes is a valid value.
        let mut l: libc::flock = unsafe { std::mem::zeroed() };
        l.l_type = libc::F_RDLCK as libc::c_short;
        l.l_whence = libc::SEEK_SET as libc::c_short;
        // SAFETY: `f` is an open file and `l` a valid `flock` it may write to.
        let r = unsafe { libc::fcntl(f.as_raw_fd(), libc::F_OFD_GETLK, &mut l) };
        r == 0 && l.l_type != libc::F_UNLCK as libc::c_short
    })
}

/// Whether a dpkg or apt run holds dpkg's lock (only Linux has dpkg).
#[cfg(not(target_os = "linux"))]
pub fn dpkg_locked(_admin_dir: &Path) -> bool {
    false
}

/// apt's cached download of Debian package `package` `version`, if it is there
/// (`<package>_<version>_<arch>.deb`, an epoch's `:` written `%3a`).
pub fn apt_cached(dpkg: &DpkgConfig, package: &str, version: &str) -> Option<PathBuf> {
    let prefix = format!("{package}_{}_", version.replace(':', "%3a"));
    std::fs::read_dir(&dpkg.apt_archives)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".deb"))
        })
}

/// Starts (`start`) or stops a system-run product (blocking): Linux, its systemd unit (to
/// start: the first enabled of `units`, else the first; to stop: every one running); Windows,
/// its scheduled task (enabled and run; or ended and disabled, since its trigger repeats every
/// minute, as `vesper-task.ps1 -Stop` does).
pub fn service(kind: &InstallKind, start: bool) -> Result<(), String> {
    let run = |program: &str, args: &[&str]| -> Result<std::process::Output, String> {
        std::process::Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("{program} didn't start: {e}"))
    };
    let check = |o: std::process::Output, what: &str| -> Result<(), String> {
        if o.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{what} failed ({}): {}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            ))
        }
    };
    match kind {
        InstallKind::Deb { units } => {
            if units.is_empty() {
                return Err("no systemd unit is configured for it".into());
            }
            if start {
                let unit = units
                    .iter()
                    .find(|u| {
                        run("systemctl", &["is-enabled", "--quiet", u.as_str()])
                            .is_ok_and(|o| o.status.success())
                    })
                    .unwrap_or(&units[0]);
                check(
                    run("systemctl", &["start", unit.as_str()])?,
                    &format!("systemctl start {unit}"),
                )
            } else {
                for u in units {
                    let active = run("systemctl", &["is-active", "--quiet", u.as_str()])
                        .is_ok_and(|o| o.status.success());
                    if active {
                        check(
                            run("systemctl", &["stop", u.as_str()])?,
                            &format!("systemctl stop {u}"),
                        )?;
                    }
                }
                Ok(())
            }
        }
        InstallKind::Nsis { task, .. } => {
            let Some(task) = task else {
                return Err("no scheduled task is configured for it".into());
            };
            let t = task.as_str();
            if start {
                check(
                    run("schtasks", &["/Change", "/TN", t, "/ENABLE"])?,
                    "enabling the task",
                )?;
                check(run("schtasks", &["/Run", "/TN", t])?, "starting the task")
            } else {
                // Ended even when it isn't running (an error then), and disabled.
                let _ = run("schtasks", &["/End", "/TN", t]);
                check(
                    run("schtasks", &["/Change", "/TN", t, "/DISABLE"])?,
                    "disabling the task",
                )
            }
        }
        InstallKind::Dir { .. } => Ok(()),
    }
}

/// Copies a folder-kind package (the executable) to `<root>/<version>/<name>`; its path.
pub fn dir_install(root: &Path, version: &str, package: &Path) -> std::io::Result<PathBuf> {
    let dir = root.join(version);
    std::fs::create_dir_all(&dir)?;
    let name = package.file_name().unwrap_or_else(|| "product".as_ref());
    let exe = dir.join(name);
    if !exe.exists() {
        let tmp = dir.join(".installing");
        std::fs::copy(package, &tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, &exe)?;
    }
    Ok(exe)
}

/// The back-off before restart `n` (counted from 1: 1 s, 2 s, 4 s, … at most 60 s).
pub fn backoff(n: u32) -> Duration {
    Duration::from_secs((1u64 << n.saturating_sub(1).min(6)).min(60))
}

/// A process the Launcher runs and restarts after a crash.
pub struct Supervisor {
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Supervisor {
    /// Starts `exe` with `args`; its output goes to `log`. Each restart after a crash adds one
    /// to `restarts`.
    pub fn start(exe: PathBuf, args: Vec<String>, log: PathBuf, restarts: Arc<AtomicU32>) -> Self {
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut failures = 0u32;
            loop {
                let started = Instant::now();
                let child = open_log(&log).and_then(|(out, err)| {
                    tokio::process::Command::new(&exe)
                        .args(&args)
                        .stdin(std::process::Stdio::null())
                        .stdout(out)
                        .stderr(err)
                        .kill_on_drop(true)
                        .spawn()
                });
                let mut child = match child {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("{} didn't start: {e}", exe.display());
                        failures += 1;
                        tokio::select! {
                            _ = tokio::time::sleep(backoff(failures)) => continue,
                            _ = stopped.changed() => return,
                        }
                    }
                };
                tokio::select! {
                    status = child.wait() => {
                        tracing::warn!("{} exited ({status:?}); restarting", exe.display());
                        // A run of a minute or more starts the back-off again.
                        if started.elapsed() >= Duration::from_secs(60) {
                            failures = 0;
                        }
                        failures += 1;
                        restarts.fetch_add(1, Ordering::Relaxed);
                        tokio::select! {
                            _ = tokio::time::sleep(backoff(failures)) => {}
                            _ = stopped.changed() => return,
                        }
                    }
                    _ = stopped.changed() => {
                        let _ = child.kill().await;
                        return;
                    }
                }
            }
        });
        Supervisor { stop, task }
    }

    /// Stops the process and waits until it is gone.
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

fn open_log(log: &Path) -> std::io::Result<(std::process::Stdio, std::process::Stdio)> {
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)?;
    let g = f.try_clone()?;
    Ok((f.into(), g.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depends_and_bounds() {
        let d = parse_depends(
            "libc6 (>= 2.43), vesper-launcher (>= 0.2.0), systemd, a | b, libfoo:any (<< 3)",
        );
        assert_eq!(
            d,
            vec![
                (
                    "libc6".to_string(),
                    Some((">=".to_string(), "2.43".to_string()))
                ),
                (
                    "vesper-launcher".to_string(),
                    Some((">=".to_string(), "0.2.0".to_string()))
                ),
                ("systemd".to_string(), None),
                (
                    "libfoo".to_string(),
                    Some(("<<".to_string(), "3".to_string()))
                ),
            ]
        );
        assert!(satisfies("0.2.0-1", ">=", "0.2.0"));
        assert!(!satisfies("0.1.5-2", ">=", "0.2.0"));
        assert!(satisfies("0.1.0-2", "<<", "0.2.0"));
        assert!(!satisfies("0.2.0", ">>", "0.2.0"));
        assert!(satisfies("0.2.0", "=", "0.2.0"));
    }

    #[test]
    fn apts_failure_is_trimmed_to_why() {
        let stdout = "Reading package lists...\nBuilding dependency tree...\n\nThe following packages have unmet dependencies:\n vesper : Depends: libexample1 but it is not installable\n";
        let stderr = "N: Download is performed unsandboxed as root\nE: Unable to correct problems, you have held broken packages.\n";
        assert_eq!(
            apt_failure(stdout, stderr),
            "vesper : Depends: libexample1 but it is not installable / E: Unable to correct problems, you have held broken packages."
        );
        // Nothing telling: its last lines.
        let many: String = (0..20).map(|i| format!("line {i}\n")).collect();
        let t = apt_failure(&many, "");
        assert!(t.starts_with("line 12 / ") && t.ends_with("line 19"), "{t}");
        assert_eq!(apt_failure("", "\n"), "it said nothing");
        let long = format!("E: {}", "x".repeat(2000));
        assert_eq!(apt_failure("", &long).chars().count(), 600);
    }

    #[test]
    fn a_package_path_apt_reads_as_a_file() {
        assert_eq!(local_deb(Path::new("/s/p.deb")), Path::new("/s/p.deb"));
        assert_eq!(local_deb(Path::new("p.deb")), Path::new("./p.deb"));
        assert_eq!(local_deb(Path::new("./p.deb")), Path::new("./p.deb"));
        assert_eq!(local_deb(Path::new("d/p.deb")), Path::new("./d/p.deb"));
    }

    #[test]
    fn back_off_grows_to_a_minute() {
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(2), Duration::from_secs(2));
        assert_eq!(backoff(6), Duration::from_secs(32));
        assert_eq!(backoff(9), Duration::from_secs(60));
    }
}
