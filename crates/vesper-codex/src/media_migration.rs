//! Moving the old managed media store (`codex/media/<sha256>.<ext>`) into the Codex's media
//! library ([`super::library`]), on the box that hosts the Codex, at every load while there is
//! something to move (old installs; old-layout media that show up later, such as an import's).
//!
//! 1. `codex/library-migration.json` is written with `state: moving` (and the blobs taken in
//!    so far, `imported`: blob file name → `{id, sha256}` of its library item).
//! 2. Each blob file is moved to `codex/library-import/<blob>/<name>` (same filesystem), `name`
//!    being the file's original name where a document knows it (the asset's original relative
//!    path, else its name with the blob's extension; else the blob's own name), then taken into
//!    the library folder of the first collection that uses it (by its name; the top of the
//!    library for media nothing uses) with IDN's `import` (moved; IDN checks its SHA-256). The
//!    same bytes already in the library: that item, and the staged file goes (it is a copy).
//!    Media the library doesn't take (an ending neither IDN nor Vesper knows as media) stay
//!    where they are, managed, and are said in the log; a later start takes them in once the
//!    library takes their ending (the migration runs while such a file is left).
//! 3. Every media asset document (collections, deleted collections) and every item trash entry
//!    is rewritten: a `managed` location of a blob taken in becomes `library {id, sha256}`.
//!    Documents of a newer schema are never rewritten (reported). Only the `location` fields
//!    change (and the document's `schemaVersion`: 3, [`model::version_for`]); a relink's
//!    record of an earlier location stays as it was.
//! 4. `state: done` once no blob is left to take in and every document naming one taken in
//!    is rewritten (a failed write, a document of a newer schema: tried again at the next
//!    start); `codex/media-gc.json` (the old blob collection's list) goes.
//!
//! **Resumable:** a crash leaves a blob in `codex/media/` (moved again), in its staging folder
//! (taken in from there), or already in the library with its staging folder empty (found again
//! by its SHA-256 or its place in the library); the imported map is saved after each blob,
//! before its staging folder goes, and documents are rewritten from it every run, so a rerun
//! finishes the job. A blob a document names that is nowhere else is looked for in the
//! library by its SHA-256.
//! Importing reads each file once for its SHA-256, on a thread of its own at low priority.

use super::library::{MediaLibrary, folder_for, takes};
use super::{Layout, blobs, issue};
use crate::data::LoadIssue;
use model::{CollectionId, DocumentKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// A blob taken into the library.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Imported {
    /// The library item's ID.
    pub id: String,
    /// Its SHA-256 (as IDN worked it out).
    pub sha256: String,
}

/// `codex/library-migration.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMigration {
    /// Schema version.
    pub schema_version: u32,
    /// `moving` or `done` (empty before the first run).
    #[serde(default)]
    pub state: String,
    /// When it began (RFC 3339 UTC).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub started_at: String,
    /// When it last finished.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub finished_at: String,
    /// Blob file name → the library item it became.
    #[serde(default)]
    pub imported: BTreeMap<String, Imported>,
}

/// What a run did.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Blobs taken into the library.
    pub imported: usize,
    /// Documents rewritten.
    pub rewritten: usize,
    /// Blobs left in the old store (the library doesn't take them, or taking them in failed).
    pub left: usize,
}

fn now() -> String {
    crate::auth::rfc3339(std::time::SystemTime::now(), true)
}

fn state_path(layout: &Layout) -> PathBuf {
    layout.codex().join("library-migration.json")
}

fn save(layout: &Layout, st: &LibraryMigration) {
    let r = serde_json::to_vec_pretty(st)
        .map_err(anyhow::Error::from)
        .and_then(|b| crate::store::write_atomic(&state_path(layout), &b));
    if let Err(e) = r {
        tracing::warn!("saving the media library migration's state: {e:#}");
    }
}

/// Blob files in the old store, by name.
fn blob_files(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.') && blobs::parse_blob_name(n).is_some())
        .collect();
    v.sort();
    v
}

/// Staging folders (`library-import/<blob>/`) left by an earlier run.
fn staged(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| blobs::parse_blob_name(n).is_some())
        .collect();
    v.sort();
    v
}

/// Where a blob goes: its file name and library folder.
#[derive(Clone, Debug, Default)]
struct Place {
    name: Option<String>,
    folder: String,
}

/// The managed location in `v` (a media asset document's `location`), as a blob name.
fn managed_blob(loc: &Value) -> Option<String> {
    if loc.get("type")?.as_str()? != "managed" {
        return None;
    }
    let sha = loc.get("sha256")?.as_str()?;
    let ext = loc.get("extension").and_then(Value::as_str).unwrap_or("");
    Some(blobs::blob_name(sha, ext))
}

/// The file name a media asset document suggests for its blob (`ext` its extension).
fn suggested_name(doc: &Value, ext: &str) -> Option<String> {
    let from_origin = doc
        .pointer("/origin/relativePath")
        .and_then(Value::as_str)
        .map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p))
        .and_then(plain_file_name)
        .filter(|n| blobs::split_name(n).1 == ext && takes(n));
    from_origin.or_else(|| {
        let name = doc.get("name")?.as_str()?.trim();
        let n = plain_file_name(&format!("{name}.{ext}"))?;
        (!name.is_empty() && blobs::split_name(&n).1 == ext && takes(&n)).then_some(n)
    })
}

/// `name` made a plain file name on every platform: no path separators (it can't leave its
/// staging folder), control characters or characters Windows refuses, not hidden, no
/// Windows device name. `None` when nothing usable is left (the blob's own name is used).
pub(super) fn plain_file_name(name: &str) -> Option<String> {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"/\<>:"|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s
        .trim()
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' '])
        .to_string();
    let stem = s
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    let s = if device { format!("_{s}") } else { s };
    (!s.is_empty() && s.len() <= 200).then_some(s)
}

/// Every JSON file in `dir`.
fn json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    v.sort();
    v
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// A collection folder's name (its `collection.json`, read raw).
fn collection_name(dir: &Path) -> Option<String> {
    read_json(&dir.join("collection.json"))?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

fn collection_dirs(parent: &Path) -> Vec<(CollectionId, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut v: Vec<(CollectionId, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Some((e.file_name().to_str()?.parse().ok()?, e.path())))
        .collect();
    v.sort_by_key(|(id, _)| id.0);
    v
}

/// Every document that may hold a location: (file, the collection it belongs to). Live
/// collections first, then deleted ones, then the item trash.
fn documents(layout: &Layout) -> Vec<(PathBuf, Option<CollectionId>)> {
    let kind = crate::data::kind_dir(DocumentKind::MediaAsset);
    let mut out = Vec::new();
    for parent in [layout.collections(), layout.collection_trash()] {
        for (id, dir) in collection_dirs(&parent) {
            for f in json_files(&dir.join("portable").join(&kind)) {
                out.push((f, Some(id)));
            }
        }
    }
    for f in json_files(&layout.codex().join("trash")) {
        let c = read_json(&f).and_then(|v| v.get("collection")?.as_str()?.parse().ok());
        out.push((f, c));
    }
    out
}

/// Replaces managed locations of imported blobs in `doc` (its `location` fields; a trash
/// entry's documents' too; never a relink's record): whether anything changed.
fn rewrite(doc: &mut Value, imported: &BTreeMap<String, Imported>) -> bool {
    let mut changed = false;
    match doc {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if k == "location" && x.is_object() {
                    if let Some(b) = managed_blob(x)
                        && let Some(i) = imported.get(&b)
                    {
                        *x = serde_json::json!({"type": "library", "id": i.id, "sha256": i.sha256});
                        changed = true;
                    }
                } else if k != "relinks" {
                    changed |= rewrite(x, imported);
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                changed |= rewrite(x, imported);
            }
        }
        _ => {}
    }
    changed
}

/// Where each blob goes, from the documents that use it.
fn places(layout: &Layout) -> HashMap<String, Place> {
    let mut names: HashMap<CollectionId, String> = HashMap::new();
    for parent in [layout.collections(), layout.collection_trash()] {
        for (id, dir) in collection_dirs(&parent) {
            names.insert(id, collection_name(&dir).unwrap_or_default());
        }
    }
    let mut out: HashMap<String, Place> = HashMap::new();
    for (file, collection) in documents(layout) {
        let Some(v) = read_json(&file) else { continue };
        let docs: Vec<&Value> = match v.get("document") {
            // A trash entry: its documents.
            Some(d) => std::iter::once(d)
                .chain(
                    v.get("subtree")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten(),
                )
                .collect(),
            None => vec![&v],
        };
        for d in docs {
            let Some(b) = d.get("location").and_then(managed_blob) else {
                continue;
            };
            if out.contains_key(&b) {
                continue;
            }
            let ext = blobs::parse_blob_name(&b)
                .map(|(_, e)| e)
                .unwrap_or_default();
            out.insert(
                b,
                Place {
                    name: suggested_name(d, &ext),
                    folder: collection
                        .map(|c| folder_for(names.get(&c).map_or("", String::as_str), c))
                        .unwrap_or_default(),
                },
            );
        }
    }
    out
}

/// Takes the old store's blobs into the library and rewrites the documents (see the module
/// docs). Nothing happens while the library isn't open. `note`: the documents rewritten in
/// live collections are noted for their change logs (show boxes may follow them). Blocking.
pub fn run(
    layout: &Layout,
    library: &MediaLibrary,
    note: bool,
    issues: &mut Vec<LoadIssue>,
) -> anyhow::Result<Outcome> {
    let mut out = Outcome::default();
    let Some(server) = library.server() else {
        return Ok(out);
    };
    let media = layout.media();
    let staging = layout.codex().join("library-import");
    let mut st: LibraryMigration = read_json(&state_path(layout))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let (files, left_staged) = (blob_files(&media), staged(&staging));
    // (A document's name for a blob keeps its ending: one the library doesn't take by its own
    // name it doesn't take at all.)
    let todo = files.iter().filter(|b| takes(b)).count();
    if todo == 0 && left_staged.is_empty() && st.state == "done" {
        return Ok(out);
    }
    if st.state != "moving" {
        st.schema_version = model::SCHEMA_VERSION;
        st.state = "moving".into();
        st.started_at = now();
        st.finished_at.clear();
        save(layout, &st);
        tracing::info!(
            blobs = files.len(),
            "Codex: moving the old media store into the media library"
        );
    }
    let places = places(layout);
    let mut names: Vec<String> = left_staged;
    for f in &files {
        if !names.contains(f) {
            names.push(f.clone());
        }
    }
    // Imports on a thread of their own at low priority: IDN reads each file whole.
    let imported = std::thread::scope(|s| {
        s.spawn(|| {
            super::disk::lower_thread_priority();
            let mut done = Vec::new();
            let mut left = 0usize;
            for b in &names {
                let place = places.get(b).cloned().unwrap_or_default();
                match take_in(&server, library, &media, &staging, b, &place) {
                    Ok(Some(imp)) => {
                        st.imported.insert(b.clone(), imp);
                        done.push(b.clone());
                        // Saved before its staging folder goes: a stop now finds it again
                        // (the folder left), or by this state.
                        save(layout, &st);
                        let _ = std::fs::remove_dir(staging.join(b));
                    }
                    Ok(None) => left += 1,
                    Err(e) => {
                        left += 1;
                        tracing::warn!(blob = %b, "Codex: taking a media file into the library failed (tried again at the next start): {e}");
                    }
                }
            }
            (done.len(), left)
        })
        .join()
    })
    .map_err(|_| anyhow::anyhow!("the media library migration stopped"))?;
    (out.imported, out.left) = imported;
    // Blobs a document still names that are neither in the old store, nor staged, nor in the
    // state (taken in by a run whose state was lost): found again in the library by their
    // SHA-256.
    for b in places.keys() {
        if st.imported.contains_key(b) || media.join(b).is_file() || staging.join(b).is_dir() {
            continue;
        }
        let Some((sha, _)) = blobs::parse_blob_name(b) else {
            continue;
        };
        if let Some(imp) = library.by_sha(&sha).and_then(|(id, _)| {
            let sha256 = library.item(&id)?.sha256?;
            Some(Imported { id, sha256 })
        }) {
            tracing::info!(blob = %b, "Codex: a media file taken into the library earlier found again by its SHA-256");
            st.imported.insert(b.clone(), imp);
        }
    }
    save(layout, &st);
    // Documents still to be rewritten (a write failed, a newer schema): the state stays
    // `moving`, so the next start tries them again.
    let mut unwritten = 0usize;
    // Documents of live collections rewritten: noted for their change logs (show boxes).
    let mut noted: BTreeMap<CollectionId, Vec<super::log::NewChange>> = BTreeMap::new();
    let live = layout.collections();
    // The documents, from everything taken in so far (also what an interrupted run took in).
    for (file, collection) in documents(layout) {
        let Some(mut v) = read_json(&file) else {
            continue;
        };
        if !rewrite(&mut v, &st.imported) {
            continue;
        }
        // (A library location: the version older servers refuse; a trash entry's documents
        // each, and the entry.)
        if let Some(d) = v.get_mut("document") {
            crate::store::stamp_version(d);
        }
        if let Some(Value::Array(sub)) = v.get_mut("subtree") {
            sub.iter_mut().for_each(crate::store::stamp_version);
        }
        if let Err(crate::store::StoreError::TooNew(m)) = crate::store::check_writable(&file) {
            issues.push(issue(
                &file,
                format!(
                    "its media moved into the media library, but this server can't rewrite where it finds them (until a server that reads it does, they are missing): {m}"
                ),
                true,
            ));
            unwritten += 1;
            continue;
        }
        crate::store::stamp_version(&mut v);
        let written = serde_json::to_vec_pretty(&v)
            .map_err(anyhow::Error::from)
            .and_then(|b| crate::store::write_atomic(&file, &b));
        match written {
            Ok(()) => {
                out.rewritten += 1;
                if let Some(c) = collection.filter(|_| note && file.starts_with(&live))
                    && let Some(id) = file.file_stem().and_then(|n| n.to_str()?.parse().ok())
                {
                    noted.entry(c).or_default().push(super::log::NewChange {
                        kind: DocumentKind::MediaAsset,
                        id,
                        change: model::DocChange::Changed,
                        revision: None,
                    });
                }
            }
            Err(e) => {
                unwritten += 1;
                tracing::warn!(file = %file.display(), "rewriting a media location (tried again at the next start): {e:#}");
            }
        }
    }
    let by = model::ChangeBy {
        client: "migration".into(),
        ..model::ChangeBy::default()
    };
    for (c, changes) in noted {
        let log = super::log::path(&layout.collection(c));
        if let Err(e) = super::log::note_pending(&log, &changes, &by) {
            tracing::warn!(collection = %c, "noting rewritten media for the change log: {e}");
        }
    }
    let remaining = blob_files(&media).into_iter().filter(|b| takes(b)).count();
    if remaining == 0 && staged(&staging).is_empty() && unwritten == 0 {
        st.state = "done".into();
        st.finished_at = now();
        save(layout, &st);
        let _ = std::fs::remove_file(layout.codex().join("media-gc.json"));
        let _ = std::fs::remove_dir(&staging);
    }
    if out.imported > 0 || out.rewritten > 0 {
        tracing::info!(
            imported = out.imported,
            rewritten = out.rewritten,
            left = out.left,
            "Codex: the old media store moved into the media library"
        );
    }
    Ok(out)
}

/// Takes one blob in (see the module docs). `Ok(None)`: the library doesn't take it (left in
/// the old store).
fn take_in(
    server: &idn_open_protocol::library::server::LibraryServer,
    library: &MediaLibrary,
    media: &Path,
    staging: &Path,
    blob: &str,
    place: &Place,
) -> std::io::Result<Option<Imported>> {
    let (sha, _) = blobs::parse_blob_name(blob).unwrap_or_default();
    let name = place
        .name
        .clone()
        .filter(|n| takes(n))
        .unwrap_or_else(|| blob.to_string());
    let dir = staging.join(blob);
    let src = media.join(blob);
    // A staged file of an earlier run (whatever its name), else the blob.
    let mut file = std::fs::read_dir(&dir)
        .ok()
        .and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.is_file()));
    if file.is_none() && !src.is_file() {
        // Taken in by a run that stopped before saving: found again by its bytes or place
        // (its staging folder goes once the state names it).
        let found = library.by_sha(&sha).map(|(id, _)| id).or_else(|| {
            let p = if place.folder.is_empty() {
                name.clone()
            } else {
                format!("{}/{name}", place.folder)
            };
            server.id_of(&p)
        });
        let found = found.and_then(|id| {
            let sha = library.item(&id)?.sha256?;
            Some(Imported { id, sha256: sha })
        });
        if found.is_none() {
            let _ = std::fs::remove_dir(&dir);
        }
        return Ok(found);
    }
    if file.is_none() {
        if !takes(&name) {
            tracing::info!(blob = %blob, "Codex: the media library doesn't take this file (its ending): it stays in the old store");
            return Ok(None);
        }
        std::fs::create_dir_all(&dir)?;
        let to = dir.join(&name);
        std::fs::rename(&src, &to)?;
        file = Some(to);
    }
    let Some(file) = file else { return Ok(None) };
    match library.import(&file, &place.folder) {
        Ok((id, sha256, existed)) => {
            if existed {
                // The same bytes were in the library already: this copy goes.
                std::fs::remove_file(&file)?;
            }
            // (Its staging folder goes once the state names it: the caller.)
            Ok(Some(Imported { id, sha256 }))
        }
        Err(e) => {
            // Back where it was, for the next try.
            if !src.exists() {
                let _ = std::fs::rename(&file, &src);
                let _ = std::fs::remove_dir(&dir);
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_locations_are_rewritten() {
        let mut imported = BTreeMap::new();
        imported.insert(
            "abc.png".to_string(),
            Imported {
                id: "0011223344556677".into(),
                sha256: "f".repeat(64),
            },
        );
        let mut doc = json!({
            "location": {"type": "managed", "sha256": "abc", "extension": "png"},
            "relinks": [{"previous": {"type": "managed", "sha256": "abc", "extension": "png"}}],
        });
        assert!(rewrite(&mut doc, &imported));
        assert_eq!(
            doc["location"],
            json!({"type": "library", "id": "0011223344556677", "sha256": "f".repeat(64)})
        );
        assert_eq!(doc["relinks"][0]["previous"]["type"], "managed");
        assert!(!rewrite(&mut doc, &imported), "once");
        let mut entry = json!({"document": {"location": {"type": "managed", "sha256": "abc", "extension": "png"}},
            "subtree": [{"location": {"type": "managed", "sha256": "zzz", "extension": "png"}}]});
        assert!(rewrite(&mut entry, &imported));
        assert_eq!(entry["document"]["location"]["type"], "library");
        assert_eq!(entry["subtree"][0]["location"]["type"], "managed");
        assert_eq!(
            suggested_name(
                &json!({"name": "Sunrise", "origin": {"relativePath": "a/Sunrise.PNG"}}),
                "png"
            ),
            Some("Sunrise.PNG".into())
        );
        assert_eq!(
            suggested_name(&json!({"name": "Sunrise"}), "png"),
            Some("Sunrise.png".into())
        );
        assert_eq!(suggested_name(&json!({"name": "X"}), "xyz"), None);
    }
}
