//! Writing documents into the data directory safely.
//!
//! - **Atomic:** a document is written to a temporary file in the same directory
//!   (`.<id>.json.<random>.tmp`), flushed to disk (`fsync`), then renamed over
//!   `<id>.json`, and the directory is synced. A crash or a full disk leaves either the old or the
//!   new document, never a partial one; a leftover temporary file is ignored by the loader
//!   (it only reads `*.json`) and removed by the next write to that directory.
//! - **Never downgrade:** before a document is overwritten or deleted, the stored one is read
//!   and checked with [`model::ensure_writable`]: a document written by a newer schema is
//!   refused, because this server would lose what it does not understand.
//! - **Batches** ([`apply`]) check every target first and only then write, in order.

use crate::data::kind_dir;
use model::{Document, DocumentKind};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One change to the data directory.
#[derive(Clone, Debug)]
pub struct DocWrite {
    /// `portable` or `machine`.
    pub section: &'static str,
    /// Document kind (its directory).
    pub kind: DocumentKind,
    /// Document ID (its file name).
    pub id: uuid::Uuid,
    /// New content (`None` = delete).
    pub doc: Option<Value>,
}

impl DocWrite {
    /// Writes `doc` (its kind, ID and section come from the [`Document`] impl).
    pub fn put<T: Document>(doc: &T) -> anyhow::Result<DocWrite> {
        Ok(DocWrite {
            section: section_of::<T>(),
            kind: T::KIND,
            id: doc.id(),
            doc: Some(serde_json::to_value(doc)?),
        })
    }

    /// Deletes the document of kind `T` with this ID.
    pub fn delete<T: Document>(id: uuid::Uuid) -> DocWrite {
        DocWrite {
            section: section_of::<T>(),
            kind: T::KIND,
            id,
            doc: None,
        }
    }

    /// The file this write targets.
    pub fn path(&self, root: &Path) -> PathBuf {
        doc_path(root, self.section, self.kind, self.id)
    }
}

fn section_of<T: Document>() -> &'static str {
    if T::portable() { "portable" } else { "machine" }
}

/// `<root>/<section>/<kind>/<id>.json`.
pub fn doc_path(root: &Path, section: &str, kind: DocumentKind, id: uuid::Uuid) -> PathBuf {
    root.join(section)
        .join(kind_dir(kind))
        .join(format!("{id}.json"))
}

/// Why a write was refused.
#[derive(Debug)]
pub enum StoreError {
    /// The stored document comes from a newer schema.
    TooNew(String),
    /// I/O or encoding failure.
    Io(anyhow::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::TooNew(m) => write!(f, "{m}"),
            StoreError::Io(e) => write!(f, "{e:#}"),
        }
    }
}

/// Fails when the document stored at `path` must not be overwritten (newer schema). A missing
/// file is fine; an unreadable or invalid one may be replaced.
pub fn check_writable(path: &Path) -> Result<(), StoreError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(StoreError::Io(anyhow::anyhow!(
                "reading {}: {e}",
                path.display()
            )));
        }
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Ok(());
    };
    model::ensure_writable(&v).map_err(|e| {
        StoreError::TooNew(format!(
            "{} was written by a newer version of the server and is not changed: {e}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ))
    })
}

/// Writes `bytes` to `path` atomically (temporary file in the same directory, fsync, rename,
/// directory fsync).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{} has no file name", path.display()))?
        .to_string_lossy()
        .to_string();
    remove_stale_temps(dir, &name);
    let mut rnd = [0u8; 6];
    getrandom::fill(&mut rnd).map_err(|e| anyhow::anyhow!("random: {e}"))?;
    let suffix: String = rnd.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = dir.join(format!(".{name}.{suffix}.tmp"));
    let result = (|| -> anyhow::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    sync_dir(dir);
    Ok(())
}

/// Removes temporary files a crashed write of `name` left behind.
fn remove_stale_temps(dir: &Path, name: &str) {
    let prefix = format!(".{name}.");
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let f = e.file_name().to_string_lossy().to_string();
            if f.starts_with(&prefix) && f.ends_with(".tmp") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn sync_dir(dir: &Path) {
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

/// The schema version a document is written with, set in it ([`model::version_for`]: 3 for
/// one holding a media library location, so older servers refuse it rather than lose the
/// location; else 2).
pub fn stamp_version(doc: &mut Value) {
    if doc.is_object() {
        doc["schemaVersion"] = Value::from(model::version_for(doc));
    }
}

/// Applies a batch: checks every target with [`check_writable`] first, then writes / deletes
/// in order. Documents are written pretty-printed (diff- and backup-friendly), with the schema
/// version they need ([`stamp_version`]).
///
/// All or nothing: when one write fails, the ones before it are undone (each file put back as
/// it was), so the caller's "not saved" is true on disk too. Only when undoing fails as well
/// (the error says so) is part of the batch on disk.
pub fn apply(root: &Path, writes: &[DocWrite]) -> Result<(), StoreError> {
    apply_batch(root, writes).map_err(|(e, _)| e)
}

/// [`apply`], saying on a failure whether part of the batch stayed on disk (undoing failed).
pub fn apply_batch(root: &Path, writes: &[DocWrite]) -> Result<(), (StoreError, bool)> {
    let mut before: Vec<Option<Vec<u8>>> = Vec::with_capacity(writes.len());
    for w in writes {
        let path = w.path(root);
        check_writable(&path).map_err(|e| (e, false))?;
        before.push(match std::fs::read(&path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err((
                    StoreError::Io(anyhow::anyhow!("reading {}: {e}", path.display())),
                    false,
                ));
            }
        });
    }
    for (i, w) in writes.iter().enumerate() {
        if let Err(e) = apply_one(root, w) {
            let stuck = writes[..i]
                .iter()
                .zip(&before)
                .rev()
                .filter(|(w, old)| restore(&w.path(root), old.as_deref()).is_err())
                .count();
            return Err(if stuck == 0 {
                (e, false)
            } else {
                (
                    StoreError::Io(anyhow::anyhow!(
                        "{e}; {stuck} earlier writes of this batch could not be undone"
                    )),
                    true,
                )
            });
        }
    }
    Ok(())
}

/// Puts a file back as it was before a batch (`None`: it wasn't there).
fn restore(path: &Path, old: Option<&[u8]>) -> anyhow::Result<()> {
    match old {
        Some(b) => write_atomic(path, b),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        },
    }
}

fn apply_one(root: &Path, w: &DocWrite) -> Result<(), StoreError> {
    let path = w.path(root);
    match &w.doc {
        Some(v) => {
            let mut v = v.clone();
            stamp_version(&mut v);
            let mut bytes = serde_json::to_vec_pretty(&v).map_err(|e| StoreError::Io(e.into()))?;
            bytes.push(b'\n');
            write_atomic(&path, &bytes)
                .map_err(|e| StoreError::Io(e.context(format!("writing {}", path.display()))))?;
        }
        None => match std::fs::remove_file(&path) {
            Ok(()) => {
                if let Some(d) = path.parent() {
                    sync_dir(d);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(StoreError::Io(anyhow::anyhow!(
                    "deleting {}: {e}",
                    path.display()
                )));
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn atomic_write_replaces_and_ignores_partial_temps() {
        let d = tmp("atomic");
        let p = d.join("a.json");
        write_atomic(&p, b"{\"v\":1}").unwrap();
        // A crashed write leaves a partial temp file: the document is untouched and the
        // next write cleans it up.
        std::fs::write(d.join(".a.json.dead.tmp"), b"{\"v\":").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"{\"v\":1}");
        write_atomic(&p, b"{\"v\":2}").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"{\"v\":2}");
        let left: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "{left:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_batch_that_fails_partway_is_undone() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = tmp("partial");
        let put = |kind, id: u128, v: i32| DocWrite {
            section: "portable",
            kind,
            id: uuid::Uuid::from_u128(id),
            doc: Some(serde_json::json!({"schemaVersion": 2, "v": v})),
        };
        let a = put(DocumentKind::Presentation, 1, 1);
        let b = put(DocumentKind::Timer, 2, 1);
        apply(&d, &[a, b]).unwrap();
        let (pa, pb) = (
            DocWrite::delete::<model::Presentation>(uuid::Uuid::from_u128(1)).path(&d),
            DocWrite::delete::<model::Timer>(uuid::Uuid::from_u128(2)).path(&d),
        );
        let (old_a, old_b) = (std::fs::read(&pa).unwrap(), std::fs::read(&pb).unwrap());
        // The timer's folder can't be changed: the batch's second step fails.
        let tdir = pb.parent().unwrap().to_path_buf();
        std::fs::set_permissions(&tdir, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(tdir.join("probe"), b"x").is_ok() {
            // (Running as root: permissions don't stop it; nothing to show.)
            std::fs::set_permissions(&tdir, std::fs::Permissions::from_mode(0o755)).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            return;
        }
        let batch = [
            put(DocumentKind::Presentation, 1, 2),
            put(DocumentKind::Presentation, 3, 2),
            DocWrite::delete::<model::Timer>(uuid::Uuid::from_u128(2)),
        ];
        assert!(apply(&d, &batch).is_err());
        std::fs::set_permissions(&tdir, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Nothing of it is on disk: the first write put back, the new file gone.
        assert_eq!(std::fs::read(&pa).unwrap(), old_a);
        assert_eq!(std::fs::read(&pb).unwrap(), old_b);
        assert!(!batch[1].path(&d).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn newer_documents_are_never_overwritten() {
        let d = tmp("newer");
        let p = d.join("x.json");
        std::fs::write(&p, br#"{"schemaVersion": 999, "id": "x"}"#).unwrap();
        assert!(matches!(check_writable(&p), Err(StoreError::TooNew(_))));
        std::fs::write(&p, br#"{"schemaVersion": 1, "id": "x"}"#).unwrap();
        assert!(check_writable(&p).is_ok());
        assert!(check_writable(&d.join("missing.json")).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}
