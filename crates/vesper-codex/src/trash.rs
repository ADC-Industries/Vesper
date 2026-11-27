//! The Codex trash: deleted documents, kept [`RETENTION`] so they can be restored with their
//! IDs.
//!
//! ```text
//! codex/trash/<entry>.json   TrashEntry with the document (and a folder's subtree)
//! codex/trash.jsonl          the index: one TrashEntry per line, without documents
//! ```
//!
//! A delete writes the entry's file, then appends its index line, before the document itself
//! is deleted. Restoring, purging and pruning rewrite the index atomically first, then remove
//! the files; a file the index doesn't name (a crash in between) goes at the next prune. An
//! index found damaged (a line cut short, unreadable) is rebuilt from the entries' files
//! instead, so no entry is lost with its line. The Codex writer serializes all of it.

use model::{ChangeBy, CollectionId, DocumentKind, TrashEntry};
use protocol::TrashItem;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long a deleted document stays in the trash.
pub const RETENTION: Duration = Duration::from_secs(30 * 86_400);

/// Where the trash lives.
#[derive(Clone, Debug)]
pub struct Trash {
    dir: PathBuf,
    index: PathBuf,
}

fn rfc3339(t: SystemTime) -> String {
    crate::auth::rfc3339(t, true)
}

/// A new entry for documents deleted together (`documents`: the main one first, then a
/// folder's subtree, parents first).
pub fn entry(
    collection: CollectionId,
    kind: DocumentKind,
    mut documents: Vec<serde_json::Value>,
    by: &ChangeBy,
    now: SystemTime,
) -> Option<TrashEntry> {
    if documents.is_empty() {
        return None;
    }
    for d in &mut documents {
        crate::store::stamp_version(d);
    }
    let document = documents.remove(0);
    let doc_id = document
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())?;
    let name = document
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let schema_version = documents
        .iter()
        .chain(std::iter::once(&document))
        .map(model::version_for)
        .max()
        .unwrap_or(model::SCHEMA_VERSION);
    Some(TrashEntry {
        schema_version,
        id: uuid::Uuid::new_v4(),
        collection,
        kind,
        doc_id,
        name,
        deleted_at: rfc3339(now),
        expires_at: rfc3339(now + RETENTION),
        by: by.clone(),
        document: Some(document),
        subtree_count: documents.len(),
        subtree: documents,
    })
}

/// The list form of an entry.
pub fn item(e: &TrashEntry) -> TrashItem {
    TrashItem {
        id: e.id.to_string(),
        collection: e.collection,
        kind: e.kind,
        doc_id: e.doc_id.to_string(),
        name: e.name.clone(),
        deleted_at: e.deleted_at.clone(),
        expires_at: e.expires_at.clone(),
        by: e.by.clone(),
        subtree: e.subtree.len().max(e.subtree_count) as u32,
    }
}

/// An entry as its index line keeps it (without its documents).
pub fn indexed(e: &TrashEntry) -> TrashEntry {
    TrashEntry {
        document: None,
        subtree: Vec::new(),
        subtree_count: e.subtree.len().max(e.subtree_count),
        ..e.clone()
    }
}

impl Trash {
    /// The trash of the Codex in `codex_dir`.
    pub fn new(codex_dir: &Path) -> Trash {
        Trash {
            dir: codex_dir.join("trash"),
            index: codex_dir.join("trash.jsonl"),
        }
    }

    fn file(&self, id: uuid::Uuid) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// The index, oldest first. A damaged index (a line cut short by a crash, bytes that
    /// aren't text, a read that failed) is rebuilt from the entries' files, and written so:
    /// an entry whose file is there is never taken for an orphan because its line is lost.
    pub fn load(&self) -> Vec<TrashEntry> {
        let (mut index, damaged) = match std::fs::read(&self.index) {
            Ok(bytes) => {
                let mut damaged = false;
                let index = bytes
                    .split(|b| *b == b'\n')
                    .filter(|l| !l.trim_ascii().is_empty())
                    .filter_map(|l| match serde_json::from_slice::<TrashEntry>(l) {
                        Ok(e) => Some(e),
                        Err(e) => {
                            tracing::warn!(file = %self.index.display(), "trash index: a damaged line: {e}");
                            damaged = true;
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                (index, damaged)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), false),
            Err(e) => {
                tracing::warn!(file = %self.index.display(), "trash index can't be read: {e}");
                (Vec::new(), true)
            }
        };
        if !damaged {
            return index;
        }
        // Every entry file the index lost, back in it.
        let mut found = 0;
        for id in self.orphans(&index) {
            match self.get(id) {
                Ok(e) if e.id == id => {
                    index.push(indexed(&e));
                    found += 1;
                }
                _ => {}
            }
        }
        index.sort_by(|a, b| a.deleted_at.cmp(&b.deleted_at));
        if let Err(e) = self.write_index(&index) {
            tracing::warn!(file = %self.index.display(), "rewriting the trash index: {e:#}");
        }
        tracing::warn!(
            entries = index.len(),
            found,
            "trash index was damaged: rebuilt from the trash's files"
        );
        index
    }

    /// Puts an entry in the trash: its file, then its index line (on a line of its own, should
    /// the last one have been cut short).
    pub fn put(&self, e: &TrashEntry) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        crate::store::write_atomic(&self.file(e.id), &serde_json::to_vec_pretty(e)?)?;
        let mut line = serde_json::to_string(&indexed(e))?;
        line.push('\n');
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.index)?;
        let len = f.metadata()?.len();
        if len > 0 {
            use std::io::{Read as _, Seek as _};
            let mut last = [0u8; 1];
            f.seek(std::io::SeekFrom::Start(len - 1))?;
            f.read_exact(&mut last)?;
            if last[0] != b'\n' {
                line.insert(0, '\n');
            }
        }
        f.write_all(line.as_bytes())?;
        f.sync_data()?;
        Ok(())
    }

    fn write_index(&self, keep: &[TrashEntry]) -> anyhow::Result<()> {
        let mut text = String::new();
        for e in keep {
            text.push_str(&serde_json::to_string(&indexed(e))?);
            text.push('\n');
        }
        if let Some(dir) = self.index.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::store::write_atomic(&self.index, text.as_bytes())
    }

    /// An entry with its documents.
    pub fn get(&self, id: uuid::Uuid) -> anyhow::Result<TrashEntry> {
        Ok(serde_json::from_slice(&std::fs::read(self.file(id))?)?)
    }

    /// Rewrites the index as `keep`, then removes the files of `gone`.
    pub fn remove(&self, keep: &[TrashEntry], gone: &[uuid::Uuid]) -> anyhow::Result<()> {
        self.write_index(keep)?;
        for id in gone {
            if let Err(e) = std::fs::remove_file(self.file(*id))
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(entry = %id, "removing a trash file: {e}");
            }
        }
        Ok(())
    }

    /// Files in `trash/` that no index entry names (a crash between writing and indexing).
    pub fn orphans(&self, index: &[TrashEntry]) -> Vec<uuid::Uuid> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        rd.flatten()
            .filter_map(|e| {
                let name = e.file_name();
                name.to_str()?
                    .strip_suffix(".json")?
                    .parse::<uuid::Uuid>()
                    .ok()
            })
            .filter(|id| !index.iter().any(|e| e.id == *id))
            .collect()
    }
}

/// The entries whose time is up at `now`.
pub fn expired(index: &[TrashEntry], now: SystemTime) -> Vec<uuid::Uuid> {
    let now = rfc3339(now);
    index
        .iter()
        .filter(|e| e.expires_at <= now)
        .map(|e| e.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn put_list_get_remove_and_expire() {
        let dir = std::env::temp_dir().join(format!("codex-trash-{}", uuid::Uuid::new_v4()));
        let trash = Trash::new(&dir);
        assert!(trash.load().is_empty());
        let col = CollectionId::new();
        let by = ChangeBy {
            client: "Booth".into(),
            ..ChangeBy::default()
        };
        let t0 = SystemTime::now();
        let folder = uuid::Uuid::new_v4();
        let a = entry(
            col,
            DocumentKind::PlaylistNode,
            vec![
                json!({"id": folder, "name": "Sundays"}),
                json!({"id": uuid::Uuid::new_v4(), "name": "May 4"}),
            ],
            &by,
            t0,
        )
        .unwrap();
        assert_eq!(
            (a.doc_id, a.name.as_str(), a.subtree.len()),
            (folder, "Sundays", 1)
        );
        let b = entry(
            col,
            DocumentKind::Timer,
            vec![json!({"id": uuid::Uuid::new_v4(), "name": "Countdown"})],
            &by,
            t0 + Duration::from_secs(86_400),
        )
        .unwrap();
        assert!(
            entry(
                col,
                DocumentKind::Timer,
                vec![json!({"name": "no id"})],
                &by,
                t0
            )
            .is_none()
        );
        trash.put(&a).unwrap();
        trash.put(&b).unwrap();

        // The index has no documents, the entry's file has them.
        let index = trash.load();
        assert_eq!(index.len(), 2);
        assert!(index[0].document.is_none() && index[0].subtree.is_empty());
        assert_eq!(
            item(&index[0]).subtree,
            1,
            "the index keeps the subtree's size"
        );
        let full = trash.get(a.id).unwrap();
        assert_eq!(full, a);
        assert_eq!(item(&full).subtree, 1);

        // 30 days on, only the first is due; 31 days on, both.
        assert!(expired(&index, t0 + RETENTION - Duration::from_secs(1)).is_empty());
        assert_eq!(expired(&index, t0 + RETENTION), [a.id]);
        assert_eq!(
            expired(&index, t0 + RETENTION + Duration::from_secs(86_400)).len(),
            2
        );

        trash.remove(&index[1..], &[a.id]).unwrap();
        assert_eq!(
            trash.load().iter().map(|e| e.id).collect::<Vec<_>>(),
            [b.id]
        );
        assert!(trash.get(a.id).is_err());
        // A file the index doesn't name is an orphan.
        std::fs::write(dir.join("trash").join(format!("{}.json", a.id)), b"{}").unwrap();
        assert_eq!(trash.orphans(&trash.load()), [a.id]);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn timer(name: &str, at: SystemTime) -> TrashEntry {
        entry(
            CollectionId::new(),
            DocumentKind::Timer,
            vec![json!({"id": uuid::Uuid::new_v4(), "name": name})],
            &ChangeBy::default(),
            at,
        )
        .unwrap()
    }

    #[test]
    fn a_damaged_index_loses_no_entry() {
        let dir = std::env::temp_dir().join(format!("codex-trash-{}", uuid::Uuid::new_v4()));
        let trash = Trash::new(&dir);
        let t0 = SystemTime::now();
        let a = timer("Café countdown", t0);
        trash.put(&a).unwrap();
        // A crash cuts the next line short, inside a character that isn't ASCII.
        let b = timer("Señor", t0 + Duration::from_secs(1));
        let line = serde_json::to_string(&indexed(&b)).unwrap();
        let cut = line.find('ñ').unwrap() + 1;
        crate::store::write_atomic(&trash.file(b.id), &serde_json::to_vec(&b).unwrap()).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&trash.index)
            .unwrap();
        f.write_all(&line.as_bytes()[..cut]).unwrap();
        drop(f);
        assert!(std::fs::read_to_string(&trash.index).is_err(), "not text");
        // The next delete goes on a line of its own.
        let c = timer("Third", t0 + Duration::from_secs(2));
        trash.put(&c).unwrap();

        let index = trash.load();
        assert_eq!(
            index.iter().map(|e| e.id).collect::<Vec<_>>(),
            [a.id, b.id, c.id],
            "every entry with its file, oldest first"
        );
        assert!(index.iter().all(|e| e.document.is_none()));
        assert!(trash.orphans(&index).is_empty(), "nothing to prune");
        // The index was written again, whole.
        let text = std::fs::read_to_string(&trash.index).unwrap();
        assert_eq!(text.lines().count(), 3, "{text}");
        assert_eq!(trash.load(), index);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_index_that_cant_be_read_is_rebuilt() {
        let dir = std::env::temp_dir().join(format!("codex-trash-{}", uuid::Uuid::new_v4()));
        let trash = Trash::new(&dir);
        let a = timer("One", SystemTime::now());
        trash.put(&a).unwrap();
        // (A folder where the index should be: reading it fails, as on a disk error.)
        std::fs::remove_file(&trash.index).unwrap();
        std::fs::create_dir(&trash.index).unwrap();
        let index = trash.load();
        assert_eq!(index.iter().map(|e| e.id).collect::<Vec<_>>(), [a.id]);
        assert!(trash.orphans(&index).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
