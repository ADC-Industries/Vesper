//! The backups on disk (`<data>/codex/backups/`): sets, snapshots and content-addressed blobs.
//! Every file is written atomically; a blob is written before the snapshot naming it, so a
//! crash leaves at most a blob nothing names (removed by the next [`Store::gc`]).

use model::{BackupSet, BackupSetId, SettingsSnapshot, SnapshotId};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A blob name: 64 lowercase hex digits.
pub fn valid_sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `<data>/codex/backups`.
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The backups of the Codex in `codex` (`<data>/codex`).
    pub fn new(codex: &Path) -> Store {
        Store {
            root: codex.join("backups"),
        }
    }

    /// `backups/sets`.
    pub fn sets_dir(&self) -> PathBuf {
        self.root.join("sets")
    }

    /// `backups/snapshots/<set>`.
    pub fn snapshots_dir(&self, set: BackupSetId) -> PathBuf {
        self.root.join("snapshots").join(set.to_string())
    }

    /// `backups/blobs`.
    pub fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs")
    }

    fn set_path(&self, id: BackupSetId) -> PathBuf {
        self.sets_dir().join(format!("{id}.json"))
    }

    fn snapshot_path(&self, set: BackupSetId, id: SnapshotId) -> PathBuf {
        self.snapshots_dir(set).join(format!("{id}.json"))
    }

    /// `backups/blobs/<sha256>`.
    pub fn blob_path(&self, sha: &str) -> PathBuf {
        self.blobs_dir().join(sha)
    }

    /// Every set (unreadable files are skipped with a warning), by product, then name.
    pub fn sets(&self) -> Vec<BackupSet> {
        let mut out: Vec<BackupSet> = json_files(&self.sets_dir())
            .into_iter()
            .filter_map(|p| read_json(&p))
            .collect();
        out.sort_by(|a, b| {
            a.product
                .cmp(&b.product)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then(a.id.cmp(&b.id))
        });
        out
    }

    /// One set.
    pub fn set(&self, id: BackupSetId) -> Option<BackupSet> {
        read_json(&self.set_path(id))
    }

    /// Writes a set.
    pub fn put_set(&self, set: &BackupSet) -> anyhow::Result<()> {
        crate::store::write_atomic(&self.set_path(set.id), &serde_json::to_vec_pretty(set)?)
    }

    /// A set's snapshots, newest (highest `seq`) first.
    pub fn snapshots(&self, set: BackupSetId) -> Vec<SettingsSnapshot> {
        let mut out: Vec<SettingsSnapshot> = json_files(&self.snapshots_dir(set))
            .into_iter()
            .filter_map(|p| read_json::<SettingsSnapshot>(&p))
            .filter(|s| s.set == set)
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.seq));
        out
    }

    /// One snapshot.
    pub fn snapshot(&self, set: BackupSetId, id: SnapshotId) -> Option<SettingsSnapshot> {
        read_json::<SettingsSnapshot>(&self.snapshot_path(set, id)).filter(|s| s.set == set)
    }

    /// Writes a snapshot (its blobs must be there already).
    pub fn put_snapshot(&self, s: &SettingsSnapshot) -> anyhow::Result<()> {
        crate::store::write_atomic(
            &self.snapshot_path(s.set, s.id),
            &serde_json::to_vec_pretty(s)?,
        )
    }

    /// Removes a snapshot.
    pub fn remove_snapshot(&self, set: BackupSetId, id: SnapshotId) -> std::io::Result<()> {
        match std::fs::remove_file(self.snapshot_path(set, id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Stores a blob under its SHA-256 (once).
    pub fn put_blob(&self, sha: &str, bytes: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(valid_sha(sha), "{sha:?} is not a SHA-256");
        let path = self.blob_path(sha);
        if path.is_file() {
            return Ok(());
        }
        crate::store::write_atomic(&path, bytes)
    }

    /// A blob's bytes, checked against its SHA-256 (`None`: missing or damaged).
    pub fn blob(&self, sha: &str) -> Option<Vec<u8>> {
        if !valid_sha(sha) {
            return None;
        }
        let bytes = std::fs::read(self.blob_path(sha)).ok()?;
        if sha256(&bytes) != sha {
            tracing::warn!(blob = sha, "backup blob damaged (its SHA-256 differs)");
            return None;
        }
        Some(bytes)
    }

    /// Removes every blob no snapshot of any set names; how many went.
    pub fn gc(&self) -> usize {
        let mut used = HashSet::new();
        for set in self.sets() {
            for s in self.snapshots(set.id) {
                used.extend(s.items.into_iter().map(|i| i.sha256));
            }
        }
        let Ok(rd) = std::fs::read_dir(self.blobs_dir()) else {
            return 0;
        };
        let mut removed = 0;
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            // Temporary files of a write in progress (`.<sha>.<rnd>.tmp`) are left alone.
            if !valid_sha(&name) || used.contains(&name) {
                continue;
            }
            match std::fs::remove_file(e.path()) {
                Ok(()) => removed += 1,
                Err(err) => tracing::warn!(blob = name, "removing an unused backup blob: {err}"),
            }
        }
        removed
    }
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    out.sort();
    out
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(file = %path.display(), "backups: unreadable file skipped: {e}");
            None
        }
    }
}
