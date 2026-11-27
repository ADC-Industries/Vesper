//! Scheduled exports: nightly at a set local time into a folder, encrypted with the passphrase
//! an admin stored, with retention (the newest export of each of the `keepNightly` most recent
//! days that have exports, plus the newest of each of the `keepWeekly` most recent weeks,
//! Monday to Sunday, local time). Only files named like this module names them
//! (`vesper-codex-YYYY-MM-DD-HHMMSS.vcodex`) are listed or pruned; nothing else in the folder
//! is touched.
//!
//! - **Settings** (`machine/codex-export.json`, [`CodexExportSettings`]): on/off, the folder
//!   (default `<data>/codex/exports/`), the time, retention.
//! - **The stored passphrase** (`machine/codex-export-passphrase`, mode 0600 on Unix; on
//!   Windows a protected ACL: SYSTEM, the Administrators and the server's account): set and
//!   cleared by admins, read only to make a scheduled export; never answered back, never
//!   logged, never in a settings snapshot (not on the snapshot allowlist) or an export
//!   (`machine/` isn't in one). Without one nothing runs and the status says why.
//! - **When**: every [`super::ExportOptions::tick`] the schedule is looked at; an export is
//!   due when the newest one in the folder is older than the last time slot passed (a box
//!   that was off at the time exports when it is back).

use super::{Exports, Paths, api, join_err};
use crate::Shared;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use protocol::{
    ApiError, CodexExportSettings, CodexExportStatus, ErrorCode, MEDIA_NOTE, ScheduledExport,
    UpdateExportSettings,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// File names: `<PREFIX><date-time>.<EXTENSION>`.
const PREFIX: &str = "vesper-codex-";
const STAMP: &str = "%Y-%m-%d-%H%M%S";

/// The settings (defaults when missing or unreadable).
pub fn settings(paths: &Paths) -> CodexExportSettings {
    std::fs::read(paths.settings())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// The folder exports go to.
pub fn folder(paths: &Paths, s: &CodexExportSettings) -> PathBuf {
    s.folder
        .as_deref()
        .filter(|f| !f.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.default_folder())
}

/// The stored passphrase, when set.
fn stored_passphrase(paths: &Paths) -> Option<String> {
    std::fs::read_to_string(paths.passphrase())
        .ok()
        .filter(|p| !p.is_empty())
}

/// Writes a file only this server's user may read (0600 on Unix; on Windows also SYSTEM and
/// the Administrators), atomically.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let dir = path.parent().expect("a folder");
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".codex-export-passphrase.{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        o.mode(0o600);
    }
    let r = (|| {
        let mut f = o.open(&tmp)?;
        // Windows: the data folder lets every local user read; this file gets its own
        // protected ACL before the secret goes in.
        #[cfg(windows)]
        protect(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// Windows: gives `path` a protected ACL (nothing inherited from the data folder, where every
/// local user may read): full control for SYSTEM, the Administrators and the file's owner
/// (the account the server runs as, which made it), no one else.
#[cfg(windows)]
fn protect(path: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("icacls")
        .arg(path)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:F",
            "*S-1-5-32-544:F",
            "*S-1-3-4:F",
            "/Q",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "icacls couldn't make the file private: {}",
            String::from_utf8_lossy(&out.stdout).trim()
        )))
    }
}

/// `HH:MM`, 24 h.
fn parse_time(t: &str) -> Option<NaiveTime> {
    let (h, m) = t.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

/// An export file's local time, from its name.
fn stamp_of(name: &str) -> Option<NaiveDateTime> {
    let rest = name
        .strip_prefix(PREFIX)?
        .strip_suffix(&format!(".{}", super::EXTENSION))?;
    // (`-2` and so on when two were made in the same second.)
    let stamp = rest.get(..17)?;
    NaiveDateTime::parse_from_str(stamp, STAMP).ok()
}

fn rfc3339_local(t: NaiveDateTime) -> String {
    match Local.from_local_datetime(&t).earliest() {
        Some(t) => t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        None => t.format("%Y-%m-%dT%H:%M:%S").to_string(),
    }
}

/// The scheduled exports in `dir`, newest first.
pub fn list_in(dir: &Path) -> Vec<(ScheduledExport, NaiveDateTime)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(ScheduledExport, NaiveDateTime)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let at = stamp_of(&name)?;
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            Some((
                ScheduledExport {
                    file: name,
                    created_at: rfc3339_local(at),
                    bytes: meta.len(),
                },
                at,
            ))
        })
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.file.cmp(&a.0.file)));
    out
}

/// The files to remove from `list` (newest first) to keep `nightly` days and `weekly` weeks.
pub fn prune(list: &[(String, NaiveDateTime)], nightly: u32, weekly: u32) -> Vec<String> {
    let mut keep: HashSet<&str> = HashSet::new();
    let mut newest_per = |period: &dyn Fn(NaiveDate) -> i64, count: u32| {
        let mut seen = HashSet::new();
        for (name, at) in list {
            let p = period(at.date());
            if seen.contains(&p) || seen.len() as u32 == count {
                continue;
            }
            seen.insert(p);
            keep.insert(name.as_str());
        }
    };
    newest_per(&|d| i64::from(d.num_days_from_ce()), nightly);
    newest_per(
        &|d| i64::from(d.iso_week().year()) * 100 + i64::from(d.iso_week().week()),
        weekly,
    );
    list.iter()
        .filter(|(n, _)| !keep.contains(n.as_str()))
        .map(|(n, _)| n.clone())
        .collect()
}

/// When the next scheduled export is due, given the newest one (local times).
fn next_due(
    s: &CodexExportSettings,
    every: Option<std::time::Duration>,
    newest: Option<NaiveDateTime>,
    now: NaiveDateTime,
) -> NaiveDateTime {
    if let Some(every) = every {
        let every = chrono::Duration::from_std(every).unwrap_or(chrono::Duration::seconds(60));
        return newest.map_or(now, |n| n + every);
    }
    let at = parse_time(&s.time).unwrap_or(NaiveTime::from_hms_opt(2, 0, 0).expect("a time"));
    let today = now.date().and_time(at);
    let last_slot = if now >= today {
        today
    } else {
        today - chrono::Duration::days(1)
    };
    match newest {
        Some(n) if n >= last_slot => last_slot + chrono::Duration::days(1),
        _ => now,
    }
}

impl Exports {
    /// `GET /codex/export/settings`: the settings and how scheduled exports stand.
    pub async fn status(&self, sh: &Arc<Shared>) -> Result<CodexExportStatus, ApiError> {
        super::host(sh)?;
        let paths = Paths::new(&sh.data().root);
        let every = self.opts.every;
        let (last_error, wait) = {
            let st = crate::lock(&self.state);
            (st.last_error.clone(), self.retry_wait(&st))
        };
        tokio::task::spawn_blocking(move || {
            let s = settings(&paths);
            let dir = folder(&paths, &s);
            let passphrase_set = stored_passphrase(&paths).is_some();
            let list = list_in(&dir);
            let active = s.enabled && passphrase_set;
            let problem = if !s.enabled {
                Some("scheduled exports are turned off".to_string())
            } else if !passphrase_set {
                Some("no passphrase is stored for scheduled exports: none run until an admin stores one".to_string())
            } else {
                last_error.map(|e| format!("the last scheduled export failed: {e}"))
            };
            let now = Local::now().naive_local();
            let next_at = active.then(|| {
                let newest = list.first().map(|(_, at)| *at);
                let retry = now + chrono::Duration::from_std(wait).unwrap_or_default();
                rfc3339_local(next_due(&s, every, newest, now).max(retry))
            });
            CodexExportStatus {
                folder: dir.display().to_string(),
                passphrase_set,
                active,
                problem,
                last: list.into_iter().next().map(|(e, _)| e),
                next_at,
                media_included: false,
                note: MEDIA_NOTE.into(),
                settings: s,
            }
        })
        .await
        .map_err(join_err)
    }

    /// `PUT /codex/export/settings`.
    pub async fn put_settings(
        &self,
        sh: &Arc<Shared>,
        body: UpdateExportSettings,
    ) -> Result<CodexExportStatus, ApiError> {
        super::host(sh)?;
        let paths = Paths::new(&sh.data().root);
        let mut s = settings(&paths);
        if let Some(e) = body.enabled {
            s.enabled = e;
        }
        if let Some(f) = body.folder {
            let f = f.trim().to_string();
            if f.is_empty() {
                s.folder = None;
            } else {
                let p = Path::new(&f);
                if !p.is_absolute()
                    || p.components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err(ApiError::invalid(
                        "folder",
                        "the folder must be an absolute path on the Codex box, without “..”",
                    ));
                }
                s.folder = Some(f);
            }
        }
        if let Some(t) = body.time {
            if parse_time(&t).is_none() {
                return Err(ApiError::invalid("time", "the time is HH:MM (24 h)"));
            }
            s.time = t;
        }
        if let Some(n) = body.keep_nightly {
            if !(1..=366).contains(&n) {
                return Err(ApiError::invalid(
                    "keepNightly",
                    "keep 1 to 366 nightly exports",
                ));
            }
            s.keep_nightly = n;
        }
        if let Some(n) = body.keep_weekly {
            if n > 520 {
                return Err(ApiError::invalid(
                    "keepWeekly",
                    "keep 0 to 520 weekly exports",
                ));
            }
            s.keep_weekly = n;
        }
        let path = paths.settings();
        let bytes =
            serde_json::to_vec_pretty(&s).map_err(|e| api(ErrorCode::Internal, e.to_string()))?;
        tokio::task::spawn_blocking(move || crate::store::write_atomic(&path, &bytes))
            .await
            .map_err(join_err)?
            .map_err(|e| api(ErrorCode::Internal, format!("saving failed: {e:#}")))?;
        // (Perhaps another folder: tried again at the next tick.)
        crate::lock(&self.state).failed_at = None;
        changed(sh);
        self.status(sh).await
    }

    /// `PUT /codex/export/passphrase` (`Some`) and `DELETE` (`None`).
    pub async fn set_passphrase(
        &self,
        sh: &Arc<Shared>,
        passphrase: Option<String>,
    ) -> Result<CodexExportStatus, ApiError> {
        super::host(sh)?;
        if let Some(p) = &passphrase {
            super::check_passphrase(p, "passphrase")?;
        }
        let path = Paths::new(&sh.data().root).passphrase();
        tokio::task::spawn_blocking(move || match passphrase {
            Some(p) => write_private(&path, p.as_bytes()),
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        })
        .await
        .map_err(join_err)?
        .map_err(|e| api(ErrorCode::Internal, format!("saving failed: {e}")))?;
        {
            let mut st = crate::lock(&self.state);
            st.last_error = None;
            st.failed_at = None;
        }
        changed(sh);
        self.status(sh).await
    }

    /// `GET /codex/export/scheduled`: the scheduled exports in the folder, newest first.
    pub async fn scheduled(&self, sh: &Arc<Shared>) -> Result<Vec<ScheduledExport>, ApiError> {
        super::host(sh)?;
        let paths = Paths::new(&sh.data().root);
        tokio::task::spawn_blocking(move || {
            let s = settings(&paths);
            list_in(&folder(&paths, &s))
                .into_iter()
                .map(|(e, _)| e)
                .collect()
        })
        .await
        .map_err(join_err)
    }

    /// The schedule: makes the export when one is due (on the Codex box, on, with a stored
    /// passphrase), then prunes the folder. Uploaded imports that waited too long go.
    pub async fn tick(&self, sh: &Arc<Shared>) {
        self.drop_expired_imports(sh).await;
        if !sh.codex_is_host() {
            return;
        }
        let paths = Paths::new(&sh.data().root);
        let s = settings(&paths);
        if !s.enabled {
            return;
        }
        let Some(passphrase) = stored_passphrase(&paths) else {
            return;
        };
        let dir = folder(&paths, &s);
        let now = Local::now().naive_local();
        let d = dir.clone();
        let newest = tokio::task::spawn_blocking(move || list_in(&d).first().map(|(_, at)| *at))
            .await
            .ok()
            .flatten();
        if next_due(&s, self.opts.every, newest, now) > now
            || !self.retry_wait(&crate::lock(&self.state)).is_zero()
        {
            return;
        }
        let _r = self.running.lock().await;
        let r = async {
            // A folder that can't be written (a drive unplugged, a NAS gone) fails here, before
            // the whole Codex is copied and encrypted.
            let d = dir.clone();
            tokio::task::spawn_blocking(move || probe_folder(&d))
                .await
                .map_err(join_err)?
                .map_err(|e| {
                    api(
                        ErrorCode::Internal,
                        format!("the folder {} can't be written: {e}", dir.display()),
                    )
                })?;
            let (bytes, _) = super::make(sh, passphrase, "scheduled", super::Site::Here).await?;
            let at = Local::now().naive_local();
            let (d, nightly, weekly) = (dir.clone(), s.keep_nightly, s.keep_weekly);
            tokio::task::spawn_blocking(move || -> anyhow::Result<(String, Vec<String>)> {
                let base = format!("{PREFIX}{}", at.format(STAMP));
                let mut name = format!("{base}.{}", super::EXTENSION);
                let mut n = 2;
                while d.join(&name).exists() {
                    name = format!("{base}-{n}.{}", super::EXTENSION);
                    n += 1;
                }
                crate::store::write_atomic(&d.join(&name), &bytes)?;
                let list: Vec<(String, NaiveDateTime)> = list_in(&d)
                    .into_iter()
                    .map(|(e, at)| (e.file, at))
                    .collect();
                let gone = prune(&list, nightly, weekly);
                for f in &gone {
                    if let Err(e) = std::fs::remove_file(d.join(f)) {
                        tracing::warn!(file = %f, "removing an old Codex export: {e}");
                    }
                }
                Ok((name, gone))
            })
            .await
            .map_err(join_err)?
            .map_err(|e| {
                api(
                    ErrorCode::Internal,
                    format!("writing into {}: {e:#}", dir.display()),
                )
            })
        }
        .await;
        match r {
            Ok((name, gone)) => {
                tracing::info!(file = %name, folder = %dir.display(), pruned = gone.len(), "scheduled Codex export written");
                let mut st = crate::lock(&self.state);
                st.last_error = None;
                st.failed_at = None;
            }
            Err(e) => {
                tracing::warn!(
                    retry_in_s = self.opts.retry_after.as_secs(),
                    "the scheduled Codex export failed: {}",
                    e.message
                );
                let mut st = crate::lock(&self.state);
                st.last_error = Some(e.message);
                st.failed_at = Some(std::time::Instant::now());
            }
        }
        changed(sh);
    }

    /// How long until a failed scheduled export is tried again (zero: not waiting).
    fn retry_wait(&self, st: &super::State) -> std::time::Duration {
        st.failed_at.map_or(std::time::Duration::ZERO, |t| {
            self.opts.retry_after.saturating_sub(t.elapsed())
        })
    }
}

/// Makes sure a file can be written into `dir` (made when missing): writes and removes a
/// small one.
fn probe_folder(dir: &Path) -> std::io::Result<()> {
    use std::io::Write as _;
    std::fs::create_dir_all(dir)?;
    let p = dir.join(format!(".vesper-codex-probe-{}.tmp", uuid::Uuid::new_v4()));
    let r = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&p)
        .and_then(|mut f| f.write_all(b"vesper").and_then(|()| f.sync_all()));
    let _ = std::fs::remove_file(&p);
    r
}

/// Tells clients the scheduled exports changed (`codexChanged {parts: ["exports"]}`).
fn changed(sh: &Shared) {
    crate::writer::codex_changed(sh, &[protocol::CodexPart::Exports], None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn names_parse_and_retention_keeps_nights_and_weeks() {
        assert_eq!(
            stamp_of("vesper-codex-2026-11-22-020000.vcodex"),
            Some(at("2026-11-22 02:00"))
        );
        assert!(stamp_of("vesper-codex-2026-11-22-020000-2.vcodex").is_some());
        assert!(stamp_of("notes.txt").is_none());
        assert!(stamp_of("vesper-codex-2026-11-22.vcodex").is_none());
        // Nightly exports for 40 days (two on the last day): 7 nights, then the newest of 4
        // weeks (the current week's newest is among the nights).
        let mut list: Vec<(String, NaiveDateTime)> = (0..40)
            .map(|i| {
                let t = at("2026-11-22 02:00") - chrono::Duration::days(i);
                (format!("n{i}"), t)
            })
            .collect();
        list.insert(0, ("again".into(), at("2026-11-22 09:00")));
        let gone = prune(&list, 7, 4);
        let kept: Vec<&str> = list
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| !gone.iter().any(|g| g == n))
            .collect();
        // 2026-11-22 is a Sunday: its week is Nov 16–22.
        assert_eq!(
            kept,
            vec![
                "again", "n1", "n2", "n3", "n4", "n5", "n6", "n7", "n14", "n21"
            ],
            "the newest of each of 7 days, plus the newest of the weeks before"
        );
        assert!(gone.contains(&"n0".to_string()), "the same day's older one");
        assert_eq!(prune(&list, 1, 0).len(), list.len() - 1);
    }

    #[test]
    fn due_after_the_last_slot() {
        let s = CodexExportSettings::default(); // 02:00
        let now = at("2026-11-22 10:00");
        assert_eq!(next_due(&s, None, None, now), now, "none yet: now");
        assert_eq!(
            next_due(&s, None, Some(at("2026-11-21 02:00")), now),
            now,
            "missed today's slot"
        );
        assert_eq!(
            next_due(&s, None, Some(at("2026-11-22 02:00")), now),
            at("2026-11-23 02:00")
        );
        let early = at("2026-11-22 01:00");
        assert_eq!(
            next_due(&s, None, Some(at("2026-11-21 02:01")), early),
            at("2026-11-22 02:00")
        );
        assert!(parse_time("24:00").is_none() && parse_time("7:00").is_none());
        assert_eq!(parse_time("07:30"), NaiveTime::from_hms_opt(7, 30, 0));
    }
}
