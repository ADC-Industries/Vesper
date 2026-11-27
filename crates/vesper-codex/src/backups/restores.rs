//! Restores queued for show boxes (`<data>/codex/backups/restores/<id>.json`, on the Codex
//! box).
//!
//! The Codex never calls a show box: a restore of another box's settings is queued here, and
//! the box takes it when it next asks the Codex ([`super::remote`]; a show box asks at each
//! poll). A restore goes `queued` → `applying` (the box made its `beforeRestore` snapshot and
//! got the files) → `applied` or `failed`; a queued one `expired` when the box didn't ask for
//! it in time ([`super::BackupOptions::restore_expiry`]), or `cancelled` by an admin. One
//! that stays `applying` past [`APPLY_TIMEOUT`] without the box's report is `failed`.
//! Finished restores are kept for [`KEEP_FINISHED`] (the REST answer can be read back), then
//! removed.

use protocol::{QueuedRestore, RestoreChange, RestoreState, SnapshotSummary};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A box that took a restore and said nothing for this long: it failed.
pub const APPLY_TIMEOUT: Duration = Duration::from_secs(3600);
/// Finished restores are listed this long.
pub const KEEP_FINISHED: Duration = Duration::from_secs(30 * 86_400);

/// A queued restore as the Codex keeps it: the wire view plus what only the Codex uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// What the REST routes answer.
    pub restore: QueuedRestore,
    /// The target box's device fingerprint in the site (from its backup set): only that
    /// device takes the restore.
    pub fingerprint: String,
    /// The items it changes (from the Codex's preparation, while `applying`).
    #[serde(default)]
    pub changes: Vec<RestoreChange>,
    /// The box's `beforeRestore` snapshot (while `applying`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<SnapshotSummary>,
    /// The server ID the box takes over (a replacement box).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopt: Option<uuid::Uuid>,
    /// When it went `applying`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applying_since: Option<String>,
}

impl Job {
    /// Still to be done (queued or being applied).
    pub fn open(&self) -> bool {
        matches!(
            self.restore.state,
            RestoreState::Queued | RestoreState::Applying
        )
    }
}

/// The folder of queued restores.
pub struct Queue {
    dir: PathBuf,
}

fn rfc(t: SystemTime) -> String {
    crate::auth::rfc3339(t, true)
}

/// Moves a job on as time passes: a queued one past its expiry is `expired`; one applying
/// for longer than [`APPLY_TIMEOUT`] is `failed`. Whether it changed.
pub fn age(job: &mut Job, now: SystemTime) -> bool {
    let now_s = rfc(now);
    match job.restore.state {
        RestoreState::Queued if job.restore.expires_at <= now_s => {
            job.restore.state = RestoreState::Expired;
            job.restore.error = Some(format!(
                "{} didn't ask the Codex for it before {}: nothing was restored",
                job.restore.target_name, job.restore.expires_at
            ));
            job.restore.updated_at = now_s;
            true
        }
        RestoreState::Applying
            if job
                .applying_since
                .as_ref()
                .is_some_and(|t| *t <= rfc(now.checked_sub(APPLY_TIMEOUT).unwrap_or(now))) =>
        {
            job.restore.state = RestoreState::Failed;
            job.restore.error = Some(format!(
                "{} took the restore but didn't report how it went",
                job.restore.target_name
            ));
            job.restore.updated_at = now_s;
            true
        }
        _ => false,
    }
}

impl Queue {
    /// The queue under the Codex folder `codex`.
    pub fn new(codex: &Path) -> Queue {
        Queue {
            dir: codex.join("backups").join("restores"),
        }
    }

    fn path(&self, id: &str) -> Option<PathBuf> {
        let id: uuid::Uuid = id.parse().ok()?;
        Some(self.dir.join(format!("{id}.json")))
    }

    /// One job.
    pub fn get(&self, id: &str) -> Option<Job> {
        let b = std::fs::read(self.path(id)?).ok()?;
        serde_json::from_slice(&b).ok()
    }

    /// Writes a job.
    pub fn put(&self, job: &Job) -> anyhow::Result<()> {
        let path = self
            .path(&job.restore.id)
            .ok_or_else(|| anyhow::anyhow!("a restore's id is a UUID"))?;
        crate::store::write_atomic(&path, &serde_json::to_vec_pretty(job)?)
    }

    /// The snapshots that open jobs (queued or applying) restore: never pruned before the box
    /// takes them. Read as they are (not aged, nothing written).
    pub fn pinned(&self) -> Vec<model::SnapshotId> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        rd.flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .filter_map(|e| {
                let job: Job = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
                job.open().then_some(job.restore.snapshot)
            })
            .collect()
    }

    /// Every job, newest first, aged to `now` (changed ones written back; finished ones older
    /// than [`KEEP_FINISHED`] removed).
    pub fn all(&self, now: SystemTime) -> Vec<Job> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let forget = rfc(now.checked_sub(KEEP_FINISHED).unwrap_or(now));
        let mut out = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(mut job) = std::fs::read(&p)
                .ok()
                .and_then(|b| serde_json::from_slice::<Job>(&b).ok())
            else {
                continue;
            };
            if age(&mut job, now)
                && let Err(e) = self.put(&job)
            {
                tracing::warn!("settings restore {}: {e:#}", job.restore.id);
            }
            if !job.open() && job.restore.updated_at < forget {
                let _ = std::fs::remove_file(&p);
                continue;
            }
            out.push(job);
        }
        out.sort_by(|a, b| {
            b.restore
                .queued_at
                .cmp(&a.restore.queued_at)
                .then_with(|| b.restore.id.cmp(&a.restore.id))
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{BackupSetId, ChangeBy, SnapshotId};

    fn job(state: RestoreState, expires: SystemTime) -> Job {
        Job {
            restore: QueuedRestore {
                id: uuid::Uuid::new_v4().to_string(),
                set: BackupSetId::new(),
                snapshot: SnapshotId::new(),
                target: "box".into(),
                target_name: "BOX".into(),
                items: None,
                adopt_server_id: false,
                state,
                queued_at: rfc(SystemTime::now()),
                updated_at: rfc(SystemTime::now()),
                expires_at: rfc(expires),
                by: ChangeBy::default(),
                error: None,
                result: None,
            },
            fingerprint: "fp".into(),
            changes: Vec::new(),
            before: None,
            adopt: None,
            applying_since: None,
        }
    }

    #[test]
    fn queued_restores_expire_and_silent_boxes_fail() {
        let now = SystemTime::now();
        let mut j = job(RestoreState::Queued, now + Duration::from_secs(60));
        assert!(!age(&mut j, now));
        assert!(age(&mut j, now + Duration::from_secs(61)));
        assert_eq!(j.restore.state, RestoreState::Expired);
        assert!(j.restore.error.as_deref().unwrap().contains("BOX"));
        assert!(!j.open());

        let mut j = job(RestoreState::Applying, now);
        j.applying_since = Some(rfc(now));
        assert!(!age(&mut j, now + Duration::from_secs(60)));
        assert!(age(&mut j, now + APPLY_TIMEOUT + Duration::from_secs(1)));
        assert_eq!(j.restore.state, RestoreState::Failed);

        // Kept on disk, listed newest first; finished ones go after a while.
        let dir = std::env::temp_dir().join(format!("codex-restores-{}", uuid::Uuid::new_v4()));
        let q = Queue::new(&dir);
        let open = job(RestoreState::Queued, now + Duration::from_secs(3600));
        let mut done = job(RestoreState::Applied, now);
        done.restore.updated_at = rfc(now - KEEP_FINISHED - Duration::from_secs(1));
        q.put(&open).unwrap();
        q.put(&done).unwrap();
        let all = q.all(now);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].restore.id, open.restore.id);
        assert!(q.get(&done.restore.id).is_none());
        assert!(q.get("not-a-uuid").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
