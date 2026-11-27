//! The collection trash: deleted collections, kept intact until they are restored or deleted
//! for good. No expiry (the 30-day expiry is the item trash's, [`super::trash`]).
//!
//! ```text
//! codex/collection-trash/<id>/                  the collection's folder, moved here unchanged
//!   collection.json  portable/  changes.jsonl     (everything it had)
//!   deleted.json                                CollectionDeletion {deletedAt, by}
//! ```
//!
//! **Deleting** writes `deleted.json` into `collections/<id>/`, then renames the folder into
//! `collection-trash/` (one rename on the same filesystem). **Restoring** removes `deleted.json`
//! first, then renames the folder back. Each step is resumable at the next load
//! ([`finish_moves`]): a folder in `collections/` holding `deleted.json` finishes its delete, a
//! folder in `collection-trash/` without one finishes its restore. **Deleting for good** removes
//! `collection.json` first (a folder without it is no collection), then the rest. The Codex
//! writer serializes all of it.

use super::{Layout, read_doc};
use model::{ChangeBy, Collection, CollectionDeletion, CollectionId};
use std::path::Path;

/// The deletion record's file name, inside the collection's folder.
pub const DELETION: &str = "deleted.json";

/// A collection in the collection trash, as loaded.
#[derive(Clone, Debug)]
pub struct DeletedState {
    /// Its `collection.json` (for one written by a newer server: its ID and name only).
    pub collection: Collection,
    /// Its `deleted.json`.
    pub deletion: CollectionDeletion,
}

fn ids_in(dir: &Path) -> Vec<CollectionId> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ids: Vec<CollectionId> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .collect();
    ids.sort_by_key(|id| id.0);
    ids
}

/// Finishes a delete or a restore that a crash interrupted (see the module docs). Problems are
/// logged; the folder stays where it is.
pub fn finish_moves(layout: &Layout) {
    for id in ids_in(&layout.collections()) {
        let dir = layout.collection(id);
        if dir.join(DELETION).is_file() {
            let to = layout.deleted_collection(id);
            if to.exists() {
                tracing::warn!(collection = %id, "a deleted collection is in both folders: left in place");
                continue;
            }
            match move_dir(&dir, &to) {
                Ok(()) => tracing::info!(collection = %id, "finished deleting a collection"),
                Err(e) => tracing::warn!(collection = %id, "finishing a collection's delete: {e}"),
            }
        }
    }
    for id in ids_in(&layout.collection_trash()) {
        let dir = layout.deleted_collection(id);
        if !dir.join(DELETION).is_file() && dir.join("collection.json").is_file() {
            let to = layout.collection(id);
            if to.exists() {
                tracing::warn!(collection = %id, "a restored collection is in both folders: left in place");
                continue;
            }
            match move_dir(&dir, &to) {
                Ok(()) => tracing::info!(collection = %id, "finished restoring a collection"),
                Err(e) => tracing::warn!(collection = %id, "finishing a collection's restore: {e}"),
            }
        }
    }
}

fn move_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(from, to)
}

/// Every collection in the collection trash, newest deletion first.
pub fn load(layout: &Layout) -> Vec<DeletedState> {
    let mut out: Vec<DeletedState> = ids_in(&layout.collection_trash())
        .into_iter()
        .filter_map(|id| {
            let dir = layout.deleted_collection(id);
            let collection = match read_doc::<Collection>(&dir.join("collection.json")) {
                Ok(Some(c)) if c.id == id => c,
                Ok(_) => return None,
                Err((e, too_new)) => {
                    tracing::warn!(collection = %id, too_new, "deleted collection: {e}");
                    let name = std::fs::read(dir.join("collection.json"))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .and_then(|v| v.get("name")?.as_str().map(str::to_string))
                        .unwrap_or_default();
                    Collection {
                        id,
                        name,
                        ..Collection::default()
                    }
                }
            };
            let deletion = std::fs::read(dir.join(DELETION))
                .ok()
                .and_then(|b| serde_json::from_slice::<CollectionDeletion>(&b).ok())
                .unwrap_or_default();
            Some(DeletedState {
                collection,
                deletion,
            })
        })
        .collect();
    out.sort_by(|a, b| b.deletion.deleted_at.cmp(&a.deletion.deleted_at));
    out
}

/// Moves a collection into the collection trash.
pub fn delete(layout: &Layout, id: CollectionId, by: &ChangeBy, at: &str) -> anyhow::Result<()> {
    let dir = layout.collection(id);
    let to = layout.deleted_collection(id);
    anyhow::ensure!(
        !to.exists(),
        "the collection trash already holds a collection with this ID"
    );
    let deletion = CollectionDeletion {
        schema_version: model::SCHEMA_VERSION,
        deleted_at: at.to_string(),
        by: by.clone(),
    };
    crate::store::write_atomic(&dir.join(DELETION), &serde_json::to_vec_pretty(&deletion)?)?;
    if let Err(e) = move_dir(&dir, &to) {
        // Still a collection: take the record back out.
        let _ = std::fs::remove_file(dir.join(DELETION));
        return Err(e.into());
    }
    Ok(())
}

/// Moves a collection back out of the collection trash.
pub fn restore(layout: &Layout, id: CollectionId) -> anyhow::Result<()> {
    let dir = layout.deleted_collection(id);
    let to = layout.collection(id);
    anyhow::ensure!(
        !to.exists(),
        "a collection with this ID is already in the Codex"
    );
    // Only a collection that loads goes back: a damaged one stays here, listed, where it can
    // still be deleted for good (moved, it would be in neither place).
    match read_doc::<Collection>(&dir.join("collection.json")) {
        Ok(Some(c)) if c.id == id => {}
        // A newer server's: loaded as it is (not rewritten).
        Err((_, true)) => {}
        Ok(Some(c)) => anyhow::bail!("its collection.json names another collection ({})", c.id),
        Ok(None) => anyhow::bail!("its collection.json is missing"),
        Err((e, false)) => anyhow::bail!("its collection.json can't be read: {e}"),
    }
    let record = std::fs::read(dir.join(DELETION)).ok();
    match std::fs::remove_file(dir.join(DELETION)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    if let Err(e) = move_dir(&dir, &to) {
        if let Some(r) = record {
            let _ = crate::store::write_atomic(&dir.join(DELETION), &r);
        }
        return Err(e.into());
    }
    Ok(())
}

/// Deletes a collection of the collection trash for good: its documents, change log and every
/// other file in its folder.
pub fn purge(layout: &Layout, id: CollectionId) -> std::io::Result<()> {
    let dir = layout.deleted_collection(id);
    match std::fs::remove_file(dir.join("collection.json")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::remove_dir_all(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(tag: &str) -> Layout {
        let dir = std::env::temp_dir().join(format!("codex-ctrash-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("codex/collections")).unwrap();
        Layout::new(&dir)
    }

    fn collection(layout: &Layout, name: &str) -> CollectionId {
        let c = Collection {
            name: name.into(),
            ..Collection::default()
        };
        super::super::write_doc(&layout.collection_json(c.id), &c).unwrap();
        let p = layout.collection(c.id).join("portable/presentation");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("x.json"), b"{}").unwrap();
        c.id
    }

    #[test]
    fn delete_restore_and_purge_keep_the_folder_intact_until_the_end() {
        let l = layout("cycle");
        let id = collection(&l, "Youth");
        let by = ChangeBy {
            client: "Booth".into(),
            ..ChangeBy::default()
        };
        delete(&l, id, &by, "2026-11-21T10:00:00Z").unwrap();
        assert!(!l.collection(id).exists());
        let d = l.deleted_collection(id);
        assert!(d.join("portable/presentation/x.json").is_file());
        let listed = load(&l);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].collection.name, "Youth");
        assert_eq!(listed[0].deletion.by.client, "Booth");

        restore(&l, id).unwrap();
        assert!(
            l.collection(id)
                .join("portable/presentation/x.json")
                .is_file()
        );
        assert!(!l.collection(id).join(DELETION).exists());
        assert!(load(&l).is_empty());

        delete(&l, id, &by, "2026-11-21T10:00:00Z").unwrap();
        purge(&l, id).unwrap();
        assert!(!d.exists() && !l.collection(id).exists());
        std::fs::remove_dir_all(l.data()).ok();
    }

    #[test]
    fn a_damaged_collection_stays_in_the_collection_trash() {
        let l = layout("damaged");
        let id = collection(&l, "Choir");
        let by = ChangeBy::default();
        delete(&l, id, &by, "2026-11-21T10:00:00Z").unwrap();
        let d = l.deleted_collection(id);
        std::fs::write(d.join("collection.json"), b"{\"id\": ").unwrap();
        let e = restore(&l, id).unwrap_err();
        assert!(format!("{e:#}").contains("can't be read"), "{e:#}");
        // Where it was, as it was: listed, and it can still be deleted for good.
        assert!(!l.collection(id).exists());
        assert!(d.join(DELETION).is_file());
        assert_eq!(load(&l).len(), 1);
        purge(&l, id).unwrap();
        assert!(!d.exists());
        std::fs::remove_dir_all(l.data()).ok();
    }

    #[test]
    fn interrupted_moves_finish_at_the_next_load() {
        let l = layout("resume");
        let deleting = collection(&l, "Half deleted");
        std::fs::write(l.collection(deleting).join(DELETION), b"{}").unwrap();
        let restoring = collection(&l, "Half restored");
        move_dir(&l.collection(restoring), &l.deleted_collection(restoring)).unwrap();
        finish_moves(&l);
        assert!(l.deleted_collection(deleting).join(DELETION).is_file());
        assert!(!l.collection(deleting).exists());
        assert!(l.collection(restoring).join("collection.json").is_file());
        assert!(!l.deleted_collection(restoring).exists());
        std::fs::remove_dir_all(l.data()).ok();
    }
}
