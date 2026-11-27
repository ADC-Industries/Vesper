//! Migration of a standalone install into its own Codex, and of old-layout documents that show
//! up later (for example `pro-import` output written into the data folder).
//!
//! A data folder from before the Codex keeps the collection kinds in `<data>/portable/<kind>/`
//! and managed media in `<data>/media/`. On the first start without `codex/`:
//!
//! 1. `codex/migration.json` is written with `state: moving` and the new collection's ID, so a
//!    crash at any later step resumes with the same collection at the next start.
//! 2. `codex/codex.json` and `codex/collections/<id>/collection.json` (named after the server)
//!    are written when missing.
//! 3. Each collection kind directory is moved to `codex/collections/<id>/portable/<kind>/`, and
//!    `media/` to `codex/media/`, with same-filesystem renames. Each step is idempotent: a
//!    source that is gone (and a destination that is there) is skipped.
//! 4. `machine/codex.json` is written when missing: role `host`, the new collection active.
//! 5. `migration.json` is marked `done`.
//!
//! When the destination of a step already exists (a resumed step that had begun merging, or
//! old-layout documents that appeared after the migration), the source's entries are moved one
//! by one into it. A document is never moved over one written by a newer schema: it stays where
//! it is and is reported. Documents are only renamed, never rewritten, so a too-new document
//! moves unchanged.
//!
//! Once the Codex exists, every load also moves old-layout collection documents and media into
//! the active collection the same way.

use super::{Layout, issue, read_doc, read_json, write_doc};
use crate::data::{COLLECTION_KINDS, LoadIssue};
use model::{CodexConfig, CodexInfo, CodexRole, Collection, CollectionId, DocumentKind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `codex/migration.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationState {
    /// Schema version.
    pub schema_version: u32,
    /// Where the migration stands.
    pub state: MigrationStep,
    /// The collection the data folder's content moves into.
    pub collection: CollectionId,
    /// When it began (RFC 3339 UTC).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub started_at: String,
    /// When it finished.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub finished_at: String,
}

/// Where a migration stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrationStep {
    /// Moving files; resumed at the next start after a crash.
    Moving,
    /// Finished.
    Done,
}

/// What [`run`] did.
#[derive(Clone, Debug)]
pub struct Outcome {
    /// The active collection.
    pub active: CollectionId,
    /// This box's Codex settings as used.
    pub config: CodexConfig,
    /// A migration (new or resumed) finished in this run.
    pub migrated: bool,
    /// Entries moved in this run (documents, kind directories and media files).
    pub moved: usize,
    /// Problems: documents left in the old layout, unreadable Codex files.
    pub issues: Vec<LoadIssue>,
}

fn now() -> String {
    crate::auth::rfc3339(std::time::SystemTime::now(), true)
}

fn read_state(layout: &Layout) -> Result<Option<MigrationState>, (String, bool)> {
    // `migration.json` is versioned like a document (any kind: only the version is checked).
    match read_json(&layout.migration_json(), DocumentKind::CodexInfo)? {
        None => Ok(None),
        Some(v) => serde_json::from_value(v)
            .map(Some)
            .map_err(|e| (format!("invalid migration state: {e}"), false)),
    }
}

fn write_state(layout: &Layout, state: &MigrationState) -> anyhow::Result<()> {
    crate::store::write_atomic(&layout.migration_json(), &serde_json::to_vec_pretty(state)?)
}

/// Sets up the Codex of the data folder `layout` when needed (see the module docs) and returns
/// the active collection. `server_name` names a new collection.
pub fn run(layout: &Layout, server_name: &str) -> anyhow::Result<Outcome> {
    let mut issues = Vec::new();
    let mut moved = 0;
    let mut migrated = false;
    let codex_existed = layout.codex().is_dir();
    // A newer server's migration state: nothing is moved; the Codex is used as it is.
    let mut frozen = false;
    let state = match read_state(layout) {
        Ok(s) => s,
        Err((error, too_new)) => {
            issues.push(issue(&layout.migration_json(), error, too_new));
            frozen = too_new;
            None
        }
    };
    let state = match state {
        Some(s) => Some(s),
        None if !codex_existed => {
            let s = MigrationState {
                schema_version: model::SCHEMA_VERSION,
                state: MigrationStep::Moving,
                collection: CollectionId::new(),
                started_at: now(),
                finished_at: String::new(),
            };
            tracing::info!(
                collection = %s.collection,
                "moving the data folder's content into its own Codex"
            );
            std::fs::create_dir_all(layout.codex())?;
            write_state(layout, &s)?;
            Some(s)
        }
        None => None,
    };

    if let Some(mut s) = state.filter(|s| s.state == MigrationStep::Moving) {
        ensure_codex(layout)?;
        ensure_collection(layout, s.collection, server_name)?;
        moved += move_old_layout(layout, s.collection, true, &mut issues)?.0;
        ensure_config(layout, s.collection, &mut issues)?;
        s.state = MigrationStep::Done;
        s.finished_at = now();
        write_state(layout, &s)?;
        migrated = true;
        tracing::info!(collection = %s.collection, moved, "the data folder's content is now in its Codex");
    }

    // Every load: the active collection (one is made when there is none), then old-layout
    // documents and media that showed up since.
    ensure_codex(layout)?;
    let (active, config) = active_collection(layout, server_name, &mut issues)?;
    let late = if frozen {
        0
    } else {
        let (n, docs) = move_old_layout(layout, active, false, &mut issues)?;
        if codex_existed && !docs.is_empty() {
            // (Into a collection show boxes may follow: they fetch them.)
            note_moved(layout, active, &docs);
        }
        n
    };
    if late > 0 {
        tracing::info!(collection = %active, moved = late, "moved old-layout documents and media into the active collection");
    }
    moved += late;
    Ok(Outcome {
        active,
        config,
        migrated,
        moved,
        issues,
    })
}

/// Writes `codex/codex.json` when missing.
fn ensure_codex(layout: &Layout) -> anyhow::Result<()> {
    if !layout.codex_json().exists() {
        let info = CodexInfo {
            created_at: now(),
            ..CodexInfo::default()
        };
        write_doc(&layout.codex_json(), &info)?;
    }
    Ok(())
}

/// Writes `collections/<id>/collection.json` when missing.
fn ensure_collection(layout: &Layout, id: CollectionId, name: &str) -> anyhow::Result<()> {
    let path = layout.collection_json(id);
    if !path.exists() {
        let c = Collection {
            id,
            name: if name.trim().is_empty() {
                protocol::PRODUCT_NAME.to_string()
            } else {
                name.trim().to_string()
            },
            sort_key: model::sort_key_between(None, None),
            created_at: now(),
            ..Collection::default()
        };
        write_doc(&path, &c)?;
    }
    Ok(())
}

/// Writes `machine/codex.json` when missing: role host, `active` active.
fn ensure_config(
    layout: &Layout,
    active: CollectionId,
    issues: &mut Vec<LoadIssue>,
) -> anyhow::Result<()> {
    match read_doc::<CodexConfig>(&layout.config_json()) {
        Ok(Some(_)) => {}
        Ok(None) => write_doc(
            &layout.config_json(),
            &CodexConfig {
                role: CodexRole::Host,
                collection: Some(active),
                ..CodexConfig::default()
            },
        )?,
        Err((error, too_new)) => issues.push(issue(&layout.config_json(), error, too_new)),
    }
    Ok(())
}

/// The collections that have a `collection.json`, in folder order.
fn collection_ids(layout: &Layout) -> Vec<CollectionId> {
    let Ok(rd) = std::fs::read_dir(layout.collections()) else {
        return Vec::new();
    };
    let mut ids: Vec<CollectionId> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .filter(|id| layout.collection_json(*id).is_file())
        .collect();
    ids.sort();
    ids
}

/// The active collection from `machine/codex.json`. Without one (or when it names a collection
/// that isn't there) the first collection becomes active, or a new one named after the server;
/// `machine/codex.json` is then written, unless it comes from a newer schema.
fn active_collection(
    layout: &Layout,
    server_name: &str,
    issues: &mut Vec<LoadIssue>,
) -> anyhow::Result<(CollectionId, CodexConfig)> {
    let ids = collection_ids(layout);
    let (config, writable) = match read_doc::<CodexConfig>(&layout.config_json()) {
        Ok(c) => (c, true),
        Err((error, too_new)) => {
            issues.push(issue(&layout.config_json(), error, too_new));
            (None, !too_new)
        }
    };
    if let Some(c) = &config
        && let Some(id) = c.collection.filter(|id| ids.contains(id))
    {
        return Ok((id, c.clone()));
    }
    let id = match ids.first() {
        Some(id) => *id,
        None => {
            let id = CollectionId::new();
            ensure_collection(layout, id, server_name)?;
            id
        }
    };
    let config = CodexConfig {
        collection: Some(id),
        ..config.unwrap_or_default()
    };
    if writable {
        write_doc(&layout.config_json(), &config)?;
    }
    Ok((id, config))
}

/// How moves go: a failure an error (the first migration: its state resumes it) or a load
/// issue (the moves at every later load: one stray entry never stops a start); and the
/// documents moved.
struct Moves<'a> {
    strict: bool,
    issues: &'a mut Vec<LoadIssue>,
    /// Document files moved in (their new paths).
    docs: Vec<PathBuf>,
}

impl Moves<'_> {
    /// A failed step: the error when strict, else an issue (the entry stays where it is).
    fn failed(&mut self, path: &Path, e: anyhow::Error) -> anyhow::Result<()> {
        if self.strict {
            return Err(e);
        }
        self.issues
            .push(issue(path, format!("left in the old layout: {e:#}"), false));
        Ok(())
    }
}

/// Moves the old layout's collection kinds and media into collection `id` (see the module
/// docs). The number of entries moved, and the documents moved in.
fn move_old_layout(
    layout: &Layout,
    id: CollectionId,
    strict: bool,
    issues: &mut Vec<LoadIssue>,
) -> anyhow::Result<(usize, Vec<PathBuf>)> {
    let mut moved = 0;
    let mut m = Moves {
        strict,
        issues,
        docs: Vec::new(),
    };
    let dest = layout.collection(id).join("portable");
    for kind in COLLECTION_KINDS {
        let src = layout.old_kind_dir(*kind);
        moved += move_dir(&src, &dest.join(crate::data::kind_dir(*kind)), true, &mut m)?;
    }
    moved += move_dir(&layout.old_media(), &layout.media(), false, &mut m)?;
    Ok((moved, m.docs))
}

/// Notes the documents moved into collection `id` as pending changes of its change log
/// (logged at the start, see [`super::log`]): show boxes following it fetch them.
fn note_moved(layout: &Layout, id: CollectionId, docs: &[PathBuf]) {
    let changes: Vec<super::log::NewChange> = docs
        .iter()
        .filter_map(|p| {
            let kind_dir = p.parent()?.file_name()?.to_str()?;
            let kind = COLLECTION_KINDS
                .iter()
                .find(|k| crate::data::kind_dir(**k) == kind_dir)?;
            let id = p.file_stem()?.to_str()?.parse().ok()?;
            Some(super::log::NewChange {
                kind: *kind,
                id,
                change: model::DocChange::Added,
                revision: None,
            })
        })
        .collect();
    let by = model::ChangeBy {
        client: "migration".into(),
        ..model::ChangeBy::default()
    };
    if let Err(e) =
        super::log::note_pending(&super::log::path(&layout.collection(id)), &changes, &by)
    {
        tracing::warn!(collection = %id, "noting the moved documents for the change log: {e}");
    }
}

/// The document files in `dir` (one kind's folder).
fn doc_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
        .collect()
}

/// Moves directory `src` to `dst`: one rename when `dst` doesn't exist, else entry by entry
/// (documents never over a newer schema's). Removes `src` once empty. A missing `src` is
/// nothing to do. The number of entries moved.
fn move_dir(src: &Path, dst: &Path, documents: bool, m: &mut Moves) -> anyhow::Result<usize> {
    if !src.is_dir() {
        return Ok(0);
    }
    if !dst.exists() {
        let renamed = dst
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::rename(src, dst))
            .map_err(|e| anyhow::anyhow!("moving {} to {}: {e}", src.display(), dst.display()));
        if let Err(e) = renamed {
            m.failed(src, e)?;
            return Ok(0);
        }
        sync_parents(src, dst);
        if documents {
            m.docs.extend(doc_files(dst));
        }
        return Ok(1);
    }
    let mut moved = 0;
    let entries = match std::fs::read_dir(src) {
        Ok(rd) => rd,
        Err(e) => {
            m.failed(src, anyhow::anyhow!("reading {}: {e}", src.display()))?;
            return Ok(0);
        }
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for from in entries {
        let Some(name) = from.file_name() else {
            continue;
        };
        let to = dst.join(name);
        if documents
            && let Err(crate::store::StoreError::TooNew(msg)) = crate::store::check_writable(&to)
        {
            m.issues.push(issue(
                &from,
                format!("left in the old layout, not moved into the Codex: {msg}"),
                true,
            ));
            continue;
        }
        if to.is_dir() && from.is_dir() {
            moved += move_dir(&from, &to, documents, m)?;
            continue;
        }
        if to.exists() && to.is_dir() != from.is_dir() {
            m.issues.push(issue(
                &from,
                format!("left in the old layout: {} is in the way", to.display()),
                false,
            ));
            continue;
        }
        if let Err(e) = std::fs::rename(&from, &to) {
            m.failed(
                &from,
                anyhow::anyhow!("moving {} to {}: {e}", from.display(), to.display()),
            )?;
            continue;
        }
        if documents && to.is_file() {
            m.docs.push(to);
        }
        moved += 1;
    }
    sync_parents(src, dst);
    sync_dir(dst);
    // Fails (and stays) while it still holds entries left behind.
    let _ = std::fs::remove_dir(src);
    Ok(moved)
}

fn sync_parents(src: &Path, dst: &Path) {
    for p in [src.parent(), dst.parent()].into_iter().flatten() {
        sync_dir(p);
    }
}

fn sync_dir(dir: &Path) {
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("codex-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn moves(issues: &mut Vec<LoadIssue>) -> Moves<'_> {
        Moves {
            strict: true,
            issues,
            docs: Vec::new(),
        }
    }

    fn doc(version: u32, name: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"schemaVersion": version, "name": name})).unwrap()
    }

    #[test]
    fn a_whole_directory_moves_in_one_rename_and_a_missing_one_is_skipped() {
        let dir = temp("rename");
        let (src, dst) = (
            dir.join("old/presentation"),
            dir.join("new/portable/presentation"),
        );
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.json"), doc(2, "a")).unwrap();
        let mut issues = Vec::new();
        assert_eq!(
            move_dir(&src, &dst, true, &mut moves(&mut issues)).unwrap(),
            1
        );
        assert!(!src.exists() && dst.join("a.json").is_file());
        // Again: nothing to do.
        assert_eq!(
            move_dir(&src, &dst, true, &mut moves(&mut issues)).unwrap(),
            0
        );
        assert!(issues.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_merge_never_replaces_a_newer_document() {
        let dir = temp("merge");
        let (src, dst) = (dir.join("old"), dir.join("new"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("same.json"), doc(2, "import")).unwrap();
        std::fs::write(dst.join("same.json"), doc(1, "stored")).unwrap();
        std::fs::write(src.join("newer.json"), doc(2, "import")).unwrap();
        std::fs::write(dst.join("newer.json"), doc(999, "from a newer server")).unwrap();
        std::fs::write(src.join("fresh.json"), doc(2, "fresh")).unwrap();
        let mut issues = Vec::new();
        assert_eq!(
            move_dir(&src, &dst, true, &mut moves(&mut issues)).unwrap(),
            2
        );
        assert_eq!(
            std::fs::read(dst.join("same.json")).unwrap(),
            doc(2, "import")
        );
        assert_eq!(
            std::fs::read(dst.join("fresh.json")).unwrap(),
            doc(2, "fresh")
        );
        assert_eq!(
            std::fs::read(dst.join("newer.json")).unwrap(),
            doc(999, "from a newer server"),
            "never overwritten"
        );
        assert_eq!(
            std::fs::read(src.join("newer.json")).unwrap(),
            doc(2, "import"),
            "left where it was"
        );
        assert_eq!(issues.len(), 1);
        assert!(issues[0].too_new && issues[0].file.ends_with("newer.json"));
        // The source stays while something is left in it.
        assert!(src.is_dir());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn media_merges_by_name() {
        let dir = temp("media");
        let (src, dst) = (dir.join("media"), dir.join("codex/media"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("abc.png"), b"png").unwrap();
        std::fs::write(dst.join("abc.png"), b"png").unwrap();
        std::fs::write(src.join("def.mp4"), b"mp4").unwrap();
        let mut issues = Vec::new();
        assert_eq!(
            move_dir(&src, &dst, false, &mut moves(&mut issues)).unwrap(),
            2
        );
        assert!(!src.exists(), "emptied and removed");
        assert_eq!(std::fs::read(dst.join("def.mp4")).unwrap(), b"mp4");
        assert!(issues.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Later moves (every load): an entry that can't be moved is a load issue, not a failed
    /// start (R2-3); the documents moved in are noted for the change log (R2-4).
    #[cfg(unix)]
    #[test]
    fn a_stray_entry_never_stops_a_load_and_moves_are_noted_for_the_log() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp("late");
        std::fs::create_dir_all(dir.join("portable")).unwrap();
        let layout = Layout::new(&dir);
        let first = run(&layout, "Booth").unwrap();
        let active = first.active;
        let kind = |k| crate::data::kind_dir(k);
        let dest = |k| layout.collection(active).join("portable").join(kind(k));
        // A presentation folder the service can't change (made by another user).
        let stuck = layout.old_kind_dir(DocumentKind::Presentation);
        std::fs::create_dir_all(&stuck).unwrap();
        std::fs::create_dir_all(dest(DocumentKind::Presentation)).unwrap();
        let p = uuid::Uuid::new_v4();
        std::fs::write(stuck.join(format!("{p}.json")), doc(2, "stuck")).unwrap();
        std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(stuck.join("probe"), b"x").is_ok() {
            // (Running as root: nothing to show.)
            std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::remove_dir_all(&dir).ok();
            return;
        }
        // A timer that moves in.
        let timers = layout.old_kind_dir(DocumentKind::Timer);
        std::fs::create_dir_all(&timers).unwrap();
        let t = uuid::Uuid::new_v4();
        std::fs::write(timers.join(format!("{t}.json")), doc(2, "timer")).unwrap();

        let out = run(&layout, "Booth");
        std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = out.expect("the load goes on");
        assert_eq!(out.active, active);
        assert!(
            out.issues
                .iter()
                .any(|i| i.file.ends_with(&format!("{p}.json"))
                    && i.error.contains("left in the old layout")),
            "{:?}",
            out.issues
        );
        assert!(
            dest(DocumentKind::Timer)
                .join(format!("{t}.json"))
                .is_file()
        );
        let pending =
            super::super::log::pending(&super::super::log::path(&layout.collection(active)));
        assert_eq!(pending.len(), 1, "{pending:?}");
        assert_eq!(
            (pending[0].0.kind, pending[0].0.id),
            (DocumentKind::Timer, t)
        );
        assert_eq!(pending[0].1.client, "migration");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_new_data_folder_gets_one_collection_named_after_the_server() {
        let dir = temp("fresh");
        std::fs::create_dir_all(dir.join("portable")).unwrap();
        let layout = Layout::new(&dir);
        let out = run(&layout, "  Booth  ").unwrap();
        assert!(out.migrated && out.issues.is_empty());
        let c: Collection = read_doc(&layout.collection_json(out.active))
            .unwrap()
            .unwrap();
        assert_eq!(c.name, "Booth");
        let cfg: CodexConfig = read_doc(&layout.config_json()).unwrap().unwrap();
        assert_eq!(
            (cfg.role, cfg.collection),
            (CodexRole::Host, Some(out.active))
        );
        // A second run changes nothing.
        let again = run(&layout, "Other").unwrap();
        assert_eq!(again.active, out.active);
        assert!(!again.migrated && again.moved == 0);
        std::fs::remove_dir_all(&dir).ok();
    }
}
