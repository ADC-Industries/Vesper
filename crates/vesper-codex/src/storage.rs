//! Storage usage (`GET /codex/storage`) and which media library items the documents use.
//!
//! **References.** A library item is in use while a document refers to it: a media asset of a
//! collection, of a collection in the collection trash, or a document in the item trash (a
//! `library` location in it; a relink's record of an earlier location doesn't count).
//!
//! **In use marks** ([`refresh_marks`]): the Codex marks every item in use in the media library
//! (holder [`super::library::HOLDER`], `set`: all of them at once), at start and after writes
//! that change media references (debounced; [`crate::Server::library`]'s `marks_due`). IDN
//! refuses to delete a marked item unless a person types its name back. Items nothing uses
//! stay in the library unmarked; the Codex never deletes library items itself (removing media
//! is the owner's action, through the library's delete and trash). This replaces the blob
//! collection of the old store (`media-gc.json`).
//!
//! **Thumbnails** of files the Codex no longer has (neither in the library nor in the old
//! store) are removed by the hourly upkeep ([`clean_thumbs`]).

use super::blobs;
use super::disk;
use crate::Shared;
use model::{CollectionId, TrashEntry};
use protocol::{CodexStorage, CollectionStorage, DiskUsage, FileUsage, LibraryUsage, TrashUsage};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Every library item named in a JSON document (`{"type": "library", "id", …}`; not inside a
/// relink's record).
pub fn library_in_json(v: &Value, out: &mut HashSet<String>) {
    match v {
        Value::Object(m) => {
            if m.get("type").and_then(Value::as_str) == Some("library")
                && let Some(id) = m.get("id").and_then(Value::as_str)
            {
                out.insert(id.to_string());
            }
            for (k, v) in m {
                if k != "relinks" {
                    library_in_json(v, out);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|v| library_in_json(v, out)),
        _ => {}
    }
}

/// Every managed media file of the old store named in a JSON document (`{"type": "managed",
/// "sha256", …}`), as `<sha256>.<ext>`.
pub fn managed_in_json(v: &Value, out: &mut HashSet<String>) {
    match v {
        Value::Object(m) => {
            if m.get("type").and_then(Value::as_str) == Some("managed")
                && let Some(sha) = m.get("sha256").and_then(Value::as_str)
            {
                let ext = m.get("extension").and_then(Value::as_str).unwrap_or("");
                out.insert(blobs::blob_name(sha, ext));
            }
            m.values().for_each(|v| managed_in_json(v, out));
        }
        Value::Array(a) => a.iter().for_each(|v| managed_in_json(v, out)),
        _ => {}
    }
}

/// The library items a collection's assets use.
pub fn collection_items(c: &crate::data::CollectionContent) -> HashSet<String> {
    c.assets
        .values()
        .filter_map(|a| match &a.location {
            model::AssetLocation::Library { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}

/// Who uses which library item.
#[derive(Default)]
pub struct References {
    /// Per live collection.
    pub collections: HashMap<CollectionId, HashSet<String>>,
    /// Collections in the collection trash and documents in the item trash.
    pub trashed: HashSet<String>,
}

impl References {
    /// Used by any document.
    pub fn all(&self) -> BTreeSet<String> {
        let mut all: BTreeSet<String> = self.trashed.iter().cloned().collect();
        for s in self.collections.values() {
            all.extend(s.iter().cloned());
        }
        all
    }

    /// Item → the live collections that use it.
    pub fn users(&self) -> HashMap<String, BTreeSet<CollectionId>> {
        let mut out: HashMap<String, BTreeSet<CollectionId>> = HashMap::new();
        for (c, ids) in &self.collections {
            for id in ids {
                out.entry(id.clone()).or_default().insert(*c);
            }
        }
        out
    }
}

/// The library items named by the media asset files of a collection folder, read raw:
/// documents that didn't load (a newer schema, a broken file) still keep their media.
pub fn scan_assets(collection_dir: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let dir = collection_dir
        .join("portable")
        .join(crate::data::kind_dir(model::DocumentKind::MediaAsset));
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        if let Some(v) = std::fs::read(&p)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            library_in_json(&v, &mut out);
        }
    }
    out
}

/// Reads every reference: live collections (their assets, and their asset files read raw),
/// deleted collections (from disk), the item trash's entries (`entries`, with their
/// documents). Blocking.
pub fn references(sh: &Shared, entries: &[TrashEntry]) -> References {
    let codex = sh.codex();
    let mut r = References::default();
    for c in &codex.collections {
        let id = c.collection.id;
        let mut set = scan_assets(&codex.layout.collection(id));
        if let Some(content) = sh.collection_content(id) {
            set.extend(collection_items(&content));
        }
        r.collections.insert(id, set);
    }
    for d in &codex.deleted {
        r.trashed.extend(scan_assets(
            &codex.layout.deleted_collection(d.collection.id),
        ));
    }
    for e in entries {
        for doc in e.document.iter().chain(e.subtree.iter()) {
            library_in_json(doc, &mut r.trashed);
        }
    }
    r
}

/// Marks every library item a document uses as in use by the Codex, and nothing else (see the
/// module docs). The number of items marked. Blocking; the caller holds the Codex writer, so
/// no write changes the references meanwhile.
pub fn refresh_marks(sh: &Shared, entries: &[TrashEntry]) -> Result<usize, String> {
    let refs = references(sh, entries);
    sh.library().set_marks(&refs.all(), refs.users())
}

/// The old store's managed media files on disk: name → (path, bytes). Hidden and temporary
/// files aside.
pub fn list_blobs(dirs: &[PathBuf]) -> BTreeMap<String, (PathBuf, u64)> {
    let mut out = BTreeMap::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || blobs::parse_blob_name(&name).is_none() {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            if m.is_file() {
                out.entry(name).or_insert((e.path(), m.len()));
            }
        }
    }
    out
}

fn usage<'a>(files: impl Iterator<Item = &'a u64>) -> FileUsage {
    let mut u = FileUsage::default();
    for b in files {
        u.files += 1;
        u.bytes += b;
    }
    u
}

/// `GET /codex/storage`: the library's figures (IDN's `space`) and per collection usage from
/// its catalog. Blocking (reads the disk).
pub fn report(sh: &Shared, entries: &[TrashEntry]) -> CodexStorage {
    let codex = sh.codex();
    let refs = references(sh, entries);
    let server = sh.library().server();
    let catalog = server.as_ref().map(|s| s.catalog()).unwrap_or_default();
    let space = server.as_ref().map(|s| s.space()).unwrap_or(Value::Null);
    let num = |k: &str| space.get(k).and_then(Value::as_u64).unwrap_or(0);
    let sizes: HashMap<&str, u64> = catalog
        .items
        .iter()
        .map(|i| (i.id.as_str(), i.size))
        .collect();
    let size = |id: &String| sizes.get(id.as_str()).copied();
    let old = list_blobs(&blobs::media_dirs(sh));
    // How many collections use each item.
    let mut users: HashMap<&String, usize> = HashMap::new();
    for set in refs.collections.values() {
        for n in set {
            *users.entry(n).or_default() += 1;
        }
    }
    let mut collections = Vec::new();
    for c in &codex.collections {
        let id = c.collection.id;
        let Some(content) = sh.collection_content(id) else {
            continue;
        };
        let used = refs.collections.get(&id).cloned().unwrap_or_default();
        let mut managed = HashSet::new();
        for a in content.assets.values() {
            if let Some(n) = blobs::managed_name(&a.location) {
                managed.insert(n);
            }
        }
        let old_bytes: u64 = managed
            .iter()
            .filter_map(|n| old.get(n).map(|(_, b)| *b))
            .sum();
        let media_bytes = used.iter().filter_map(size).sum::<u64>() + old_bytes;
        let unique_media_bytes = used
            .iter()
            .filter(|n| users.get(n) == Some(&1) && !refs.trashed.contains(*n))
            .filter_map(size)
            .sum();
        let missing = content
            .assets
            .keys()
            .filter(|a| content.asset_path(**a).is_err())
            .count() as u32;
        collections.push(CollectionStorage {
            collection: id,
            name: c.collection.name.clone(),
            documents: super::writer::counts(&content).total(),
            document_bytes: disk::dir_usage(&codex.layout.collection(id).join("portable")).0,
            media_bytes,
            unique_media_bytes,
            assets: content.assets.len() as u32,
            missing,
        });
    }
    let all = refs.all();
    let live: HashSet<&String> = refs.collections.values().flatten().collect();
    let items = || catalog.items.iter();
    let shared = usage(
        items()
            .filter(|i| users.get(&i.id).copied().unwrap_or(0) > 1)
            .map(|i| &i.size),
    );
    let unreferenced = usage(items().filter(|i| !all.contains(&i.id)).map(|i| &i.size));
    let trash_media: u64 = items()
        .filter(|i| refs.trashed.contains(&i.id) && !live.contains(&i.id))
        .map(|i| i.size)
        .sum();
    let disk_space = disk::space(&codex.layout.codex()).unwrap_or_default();
    let opts = &sh.uploads().options;
    let keep_free = disk_space
        .reserve(opts.keep_free_percent, opts.keep_free_bytes)
        .max(num("keep_free"));
    let (item_bytes, _) = disk::dir_usage(&codex.layout.codex().join("trash"));
    let (collection_bytes, _) = disk::dir_usage(&codex.layout.collection_trash());
    let (tb, tn) = disk::dir_usage(&blobs::thumbs_dir(&codex.layout.codex()));
    let (ub, un) = sh.uploads().usage(sh);
    CodexStorage {
        disk: DiskUsage {
            total: disk_space.total,
            available: disk_space.available,
            keep_free,
        },
        collections,
        library: LibraryUsage {
            bytes: num("library_bytes"),
            items: num("items"),
            trash_items: num("trash"),
            trash_days: num("trash_days"),
        },
        media: usage(items().map(|i| &i.size)),
        shared,
        unreferenced,
        managed: usage(old.values().map(|(_, b)| b)),
        trash: TrashUsage {
            items: entries.len() as u64,
            item_bytes,
            collections: codex.deleted.len() as u64,
            collection_bytes,
            media_bytes: trash_media,
        },
        thumbs: FileUsage {
            files: tn,
            bytes: tb,
        },
        uploads: FileUsage {
            files: un,
            bytes: ub,
        },
    }
}

/// Removes thumbnails of media the Codex no longer has (by SHA-256: neither a library item
/// nor a managed file of the old store). The number removed. Blocking.
pub fn clean_thumbs(sh: &Shared) -> usize {
    // (While the library is closed nothing can be told: nothing goes.)
    if !sh.library().is_open() {
        return 0;
    }
    let mut shas: HashSet<String> = sh
        .library()
        .items()
        .into_values()
        .filter_map(|f| f.sha256)
        .collect();
    shas.extend(
        list_blobs(&blobs::media_dirs(sh))
            .keys()
            .filter_map(|n| blobs::parse_blob_name(n).map(|(s, _)| s)),
    );
    let mut removed = 0;
    if let Ok(rd) = std::fs::read_dir(blobs::thumbs_dir(&sh.codex().layout.codex())) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(sha) = name.strip_suffix(".jpg").and_then(|n| n.rsplit_once('-')) else {
                continue;
            };
            if !shas.contains(sha.0) && std::fs::remove_file(e.path()).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn locations_anywhere_in_a_document() {
        let doc = json!({
            "id": "x",
            "location": {"type": "managed", "sha256": "abc", "extension": "png"},
            "sections": [{"slides": [{"fill": {"location": {"type": "managed", "sha256": "def"}}}]}],
            "other": {"type": "linked", "sha256": "nope"}
        });
        let mut out = HashSet::new();
        managed_in_json(&doc, &mut out);
        let mut v: Vec<_> = out.into_iter().collect();
        v.sort();
        assert_eq!(v, ["abc.png", "def"]);
        let doc = json!({
            "location": {"type": "library", "id": "0011223344556677", "sha256": "x"},
            "relinks": [{"previous": {"type": "library", "id": "8899aabbccddeeff"}}],
            "document": {"location": {"type": "library", "id": "1111222233334444"}},
        });
        let mut out = HashSet::new();
        library_in_json(&doc, &mut out);
        let mut v: Vec<_> = out.into_iter().collect();
        v.sort();
        assert_eq!(v, ["0011223344556677", "1111222233334444"]);
    }
}
