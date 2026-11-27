//! A collection's change log: `codex/collections/<id>/changes.jsonl`.
//!
//! One JSON line per document added, changed or removed ([`ChangeEntry`]), appended by the
//! Codex writer after the change is on disk. `seq` grows by one per entry and is never reused:
//! the next one follows the newest entry, or the floor when the log is empty.
//!
//! **Compaction** runs when the log passes [`MAX_ENTRIES`] entries or its oldest entry is
//! [`MAX_AGE`] old: entries older than [`MAX_AGE`] go, and at most [`KEEP_ENTRIES`] of the
//! newest stay. The log then starts with a header line `{"floor": N}`: the changes up to `N`
//! are gone, and a reader asking from before `N` gets `resync: true` (refetch everything).
//! The rewrite is atomic (temporary file, rename).
//!
//! A torn last line (a crash while appending) is skipped when reading and dropped by the next
//! compaction.
//!
//! **Pending changes** (`changes.pending.jsonl` next to the log): changes on disk that aren't
//! in the log yet. The writer notes a batch's changes there before it writes the documents and
//! clears them once they are appended, so a stop in between, or an append that fails (a full
//! disk), leaves them noted; the start's migrations note what they move in or rewrite. They
//! are appended before anything else at the next append, and at every start, so show boxes
//! following the log fetch them. (A change noted for a write that then didn't happen is
//! harmless: a follower fetches the document as it is.)

use model::{ChangeBy, ChangeEntry, DocChange, DocumentKind};
use protocol::CollectionChanges;
use serde::{Deserialize, Serialize};
use std::io::{Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Compaction is due past this many entries.
pub const MAX_ENTRIES: usize = 10_000;
/// Entries kept (at most) by a compaction.
pub const KEEP_ENTRIES: usize = 5_000;
/// Entries older than this go at a compaction (and make one due).
pub const MAX_AGE: Duration = Duration::from_secs(90 * 86_400);
/// The most entries one `changes?since=` answer carries.
pub const MAX_LIMIT: usize = 1000;

/// The log's file in a collection's folder.
pub fn path(collection_dir: &Path) -> PathBuf {
    collection_dir.join("changes.jsonl")
}

/// The header line of a compacted log.
#[derive(Serialize, Deserialize)]
struct Header {
    floor: u64,
}

/// One change to log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewChange {
    /// The document's kind.
    pub kind: DocumentKind,
    /// The document's ID.
    pub id: uuid::Uuid,
    /// What happened.
    pub change: DocChange,
    /// Its revision after the change.
    pub revision: Option<String>,
}

/// What the writer keeps of a log between appends.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogState {
    /// The newest `seq` (the floor when empty).
    pub head: u64,
    /// The changes up to here are gone.
    pub floor: u64,
    /// Entries in the file.
    pub count: usize,
    /// When the oldest entry was made (RFC 3339 UTC).
    pub oldest: Option<String>,
}

/// A log's contents: its floor and entries, oldest first.
pub fn read(path: &Path) -> std::io::Result<(u64, Vec<ChangeEntry>)> {
    let bytes = match std::fs::read(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, Vec::new())),
        Err(e) => return Err(e),
    };
    let mut floor = 0;
    let mut entries: Vec<ChangeEntry> = Vec::new();
    // Line by line as bytes: a torn line (even inside a character) spoils only itself.
    for (i, line) in bytes.split(|b| *b == b'\n').enumerate() {
        let line = line.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if i == 0
            && let Ok(h) = serde_json::from_slice::<Header>(line)
        {
            floor = h.floor;
            continue;
        }
        match serde_json::from_slice::<ChangeEntry>(line) {
            // Never out of order (a hand-edited file): such a line is skipped.
            Ok(e) if e.seq > entries.last().map_or(floor, |l| l.seq) => entries.push(e),
            Ok(e) => {
                tracing::warn!(file = %path.display(), seq = e.seq, "change log: entry out of order, skipped")
            }
            Err(e) => {
                tracing::warn!(file = %path.display(), line = i + 1, "change log: bad line skipped: {e}")
            }
        }
    }
    Ok((floor, entries))
}

/// The state of a log on disk.
pub fn state(path: &Path) -> std::io::Result<LogState> {
    let (floor, entries) = read(path)?;
    Ok(LogState {
        head: entries.last().map_or(floor, |e| e.seq),
        floor,
        count: entries.len(),
        oldest: entries.first().map(|e| e.at.clone()),
    })
}

fn rfc3339(t: SystemTime) -> String {
    crate::auth::rfc3339(t, true)
}

/// Appends `changes` (seq from `state.head + 1`), then compacts when due (a failed compaction
/// is only logged: the entries are appended). Returns the entries.
pub fn append(
    path: &Path,
    state: &mut LogState,
    changes: &[NewChange],
    by: &ChangeBy,
    now: SystemTime,
) -> std::io::Result<Vec<ChangeEntry>> {
    if changes.is_empty() {
        return Ok(Vec::new());
    }
    let at = rfc3339(now);
    let entries: Vec<ChangeEntry> = changes
        .iter()
        .enumerate()
        .map(|(i, c)| ChangeEntry {
            seq: state.head + 1 + i as u64,
            at: at.clone(),
            kind: c.kind,
            id: c.id,
            change: c.change,
            revision: c.revision.clone(),
            by: by.clone(),
        })
        .collect();
    let mut text = String::new();
    for e in &entries {
        text.push_str(&serde_json::to_string(e).map_err(std::io::Error::other)?);
        text.push('\n');
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    // A torn last line (a crash while appending) must not swallow the first new line.
    let mut last = [0u8; 1];
    let torn = f.metadata()?.len() > 0 && {
        f.seek(std::io::SeekFrom::End(-1))?;
        f.read_exact(&mut last)?;
        last[0] != b'\n'
    };
    if torn {
        f.write_all(b"\n")?;
    }
    f.write_all(text.as_bytes())?;
    f.sync_data()?;
    state.head += entries.len() as u64;
    state.count += entries.len();
    if state.oldest.is_none() {
        state.oldest = Some(at);
    }
    // The entries are in the log now: a compaction that fails (a nearly full disk) must not
    // make the caller think they aren't, or they'd be appended again (F08-2). It is tried
    // again at the next append.
    if let Err(e) = compact_if_due(path, state, now) {
        tracing::warn!(file = %path.display(), "compacting the change log failed (tried again at the next change): {e}");
    }
    Ok(entries)
}

/// Compacts the log when it is due (see the module docs). Returns whether it did.
pub fn compact_if_due(path: &Path, state: &mut LogState, now: SystemTime) -> std::io::Result<bool> {
    let cutoff = rfc3339(now.checked_sub(MAX_AGE).unwrap_or(SystemTime::UNIX_EPOCH));
    let old = state.oldest.as_ref().is_some_and(|o| *o < cutoff);
    if state.count <= MAX_ENTRIES && !old {
        return Ok(false);
    }
    let (floor, entries) = read(path)?;
    let mut keep: Vec<ChangeEntry> = entries.into_iter().filter(|e| e.at >= cutoff).collect();
    let mut new_floor = floor.max(state.floor);
    if keep.len() > KEEP_ENTRIES {
        keep.drain(..keep.len() - KEEP_ENTRIES);
    }
    // The floor: the newest seq that is gone.
    new_floor = new_floor.max(keep.first().map_or(state.head, |e| e.seq - 1));
    let mut text =
        serde_json::to_string(&Header { floor: new_floor }).map_err(std::io::Error::other)?;
    text.push('\n');
    for e in &keep {
        text.push_str(&serde_json::to_string(e).map_err(std::io::Error::other)?);
        text.push('\n');
    }
    crate::store::write_atomic(path, text.as_bytes()).map_err(std::io::Error::other)?;
    tracing::info!(file = %path.display(), floor = new_floor, kept = keep.len(), "change log compacted");
    state.floor = new_floor;
    state.count = keep.len();
    state.oldest = keep.first().map(|e| e.at.clone());
    state.head = state.head.max(new_floor);
    Ok(true)
}

/// The pending changes' file of the log at `path` (see the module docs).
pub fn pending_path(path: &Path) -> PathBuf {
    path.with_file_name("changes.pending.jsonl")
}

/// One line of the pending file.
#[derive(Serialize, Deserialize)]
struct PendingLine {
    #[serde(flatten)]
    change: NewChange,
    by: ChangeBy,
}

/// Notes `changes` as pending for the log at `path` (appended, synced). The file's length
/// before, to take the note back with [`unnote`].
pub fn note_pending(path: &Path, changes: &[NewChange], by: &ChangeBy) -> std::io::Result<u64> {
    let p = pending_path(path);
    let before = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    if changes.is_empty() {
        return Ok(before);
    }
    let mut text = String::new();
    for c in changes {
        let line = PendingLine {
            change: c.clone(),
            by: by.clone(),
        };
        text.push_str(&serde_json::to_string(&line).map_err(std::io::Error::other)?);
        text.push('\n');
    }
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written at the end, but not opened in append mode: on Windows an append-only handle
    // can't be truncated, and a failed note must be taken back (E04-2).
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&p)?;
    let before = f.metadata()?.len();
    let written = (|| {
        // A torn last line (a write that stopped part-way) must not swallow the first new one.
        let mut last = [0u8; 1];
        let torn = before > 0 && {
            f.seek(std::io::SeekFrom::End(-1))?;
            f.read_exact(&mut last)?;
            last[0] != b'\n'
        };
        f.seek(std::io::SeekFrom::Start(before))?;
        if torn {
            f.write_all(b"\n")?;
        }
        f.write_all(text.as_bytes())?;
        f.sync_data()
    })();
    if let Err(e) = written {
        // Nothing half-noted is left behind.
        if let Err(t) = f.set_len(before) {
            tracing::warn!(file = %p.display(), "taking back a failed note: {t}");
        }
        return Err(e);
    }
    Ok(before)
}

/// Takes back a note of [`note_pending`] (the batch didn't change anything after all).
pub fn unnote(path: &Path, len: u64) {
    let p = pending_path(path);
    let r = if len == 0 {
        std::fs::remove_file(&p)
    } else {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&p)
            .and_then(|f| f.set_len(len))
    };
    if let Err(e) = r
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::debug!(file = %p.display(), "taking back pending changes: {e}");
    }
}

/// The pending changes of the log at `path`, in order (a torn or bad line skipped; none when
/// the file can't be read).
pub fn pending(path: &Path) -> Vec<(NewChange, ChangeBy)> {
    read_pending(path).unwrap_or_default()
}

/// The pending changes of the log at `path`, in order: a torn or bad line is skipped (even one
/// torn inside a character); a file that can't be read is an error (none when there is none).
fn read_pending(path: &Path) -> std::io::Result<Vec<(NewChange, ChangeBy)>> {
    let p = pending_path(path);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for (i, line) in bytes.split(|b| *b == b'\n').enumerate() {
        let line = line.trim_ascii();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<PendingLine>(line) {
            Ok(l) => out.push((l.change, l.by)),
            Err(e) => {
                tracing::warn!(file = %p.display(), line = i + 1, "pending changes: bad line skipped: {e}")
            }
        }
    }
    Ok(out)
}

/// Appends the pending changes (each run of the same author as one append), then `changes`;
/// the pending file goes once they are in. Nothing is appended when there is nothing to.
pub fn append_with_pending(
    path: &Path,
    state: &mut LogState,
    changes: &[NewChange],
    by: &ChangeBy,
    now: SystemTime,
) -> std::io::Result<Vec<ChangeEntry>> {
    let mut runs: Vec<(ChangeBy, Vec<NewChange>)> = Vec::new();
    // A pending file that can't be read is kept (an error): its changes must reach the log.
    let had_pending = pending_path(path).exists();
    let noted = read_pending(path)?;
    for (c, b) in noted {
        match runs.last_mut() {
            Some((lb, cs)) if *lb == b => cs.push(c),
            _ => runs.push((b, vec![c])),
        }
    }
    if !changes.is_empty() {
        runs.push((by.clone(), changes.to_vec()));
    }
    let mut out = Vec::new();
    for (b, cs) in &runs {
        out.extend(append(path, state, cs, b, now)?);
    }
    if had_pending {
        let p = pending_path(path);
        match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                // Its changes are in the log now: emptied, they aren't appended again.
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&p)
                    .and_then(|f| f.set_len(0))
                    .map_err(|_| e)?;
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The changes after `since`, at most `limit` (see [`CollectionChanges`]).
pub fn since(path: &Path, since: u64, limit: usize) -> std::io::Result<CollectionChanges> {
    let (floor, entries) = read(path)?;
    let head = entries.last().map_or(floor, |e| e.seq);
    if since < floor || since > head {
        return Ok(CollectionChanges {
            from: floor,
            to: head,
            changes: Vec::new(),
            resync: true,
        });
    }
    let changes: Vec<ChangeEntry> = entries
        .into_iter()
        .filter(|e| e.seq > since)
        .take(limit.clamp(1, MAX_LIMIT))
        .collect();
    Ok(CollectionChanges {
        from: since,
        to: changes.last().map_or(head, |e| e.seq),
        changes,
        resync: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("codex-log-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        path(&dir)
    }

    fn change(n: u128) -> NewChange {
        NewChange {
            kind: DocumentKind::Presentation,
            id: uuid::Uuid::from_u128(n),
            change: DocChange::Changed,
            revision: Some(format!("{n:016x}")),
        }
    }

    fn by() -> ChangeBy {
        ChangeBy {
            client: "Booth".into(),
            client_id: Some("k1".into()),
            machine: "m1".into(),
        }
    }

    const DAY: Duration = Duration::from_secs(86_400);

    #[test]
    fn seq_grows_by_one_and_survives_a_reopen() {
        let p = temp("seq");
        let now = SystemTime::now();
        let mut st = state(&p).unwrap();
        assert_eq!(st, LogState::default());
        let a = append(&p, &mut st, &[change(1), change(2)], &by(), now).unwrap();
        assert_eq!(a.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(a[0].by, by());
        // Reopened: the state comes from the file, seq continues.
        let mut st = state(&p).unwrap();
        assert_eq!((st.head, st.floor, st.count), (2, 0, 2));
        let b = append(&p, &mut st, &[change(3)], &by(), now).unwrap();
        assert_eq!(b[0].seq, 3);
        assert!(append(&p, &mut st, &[], &by(), now).unwrap().is_empty());

        let all = since(&p, 0, 100).unwrap();
        assert_eq!((all.from, all.to, all.resync), (0, 3, false));
        assert_eq!(all.changes.len(), 3);
        assert_eq!(all.changes[2].id, uuid::Uuid::from_u128(3));
        let some = since(&p, 2, 100).unwrap();
        assert_eq!(some.changes.iter().map(|e| e.seq).collect::<Vec<_>>(), [3]);
        let none = since(&p, 3, 100).unwrap();
        assert!(none.changes.is_empty() && !none.resync);
        assert_eq!(none.to, 3);
        // A page: `to` is the last one sent.
        let page = since(&p, 0, 2).unwrap();
        assert_eq!((page.to, page.changes.len()), (2, 2));
        // From the future (another log, or one that was lost): resync.
        let ahead = since(&p, 9, 100).unwrap();
        assert!(ahead.resync && ahead.changes.is_empty());
        assert_eq!((ahead.from, ahead.to), (0, 3));
    }

    /// F08-2: a compaction that fails after the append (here: the folder can't take the
    /// rewritten log) doesn't fail the append, so nothing is appended twice; it compacts at the
    /// next append that can.
    #[cfg(unix)]
    #[test]
    fn a_failed_compaction_does_not_fail_the_append() {
        use std::os::unix::fs::PermissionsExt;
        let p = temp("compact-fail");
        let dir = p.parent().unwrap().to_path_buf();
        let then = SystemTime::now();
        let mut st = state(&p).unwrap();
        append(&p, &mut st, &[change(1)], &by(), then).unwrap();
        // 100 days later the first entry is due to go: compaction is due at every append.
        let later = then + DAY * 100;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let blocked = std::fs::File::create(dir.join("probe")).is_err();
        let r = append(&p, &mut st, &[change(2)], &by(), later);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        if !blocked {
            eprintln!("SKIPPED: this user can write a read-only folder (root)");
            return;
        }
        let e = r.expect("the append itself worked");
        assert_eq!(e[0].seq, 2);
        let seqs = |p: &Path| read(p).unwrap().1.iter().map(|e| e.seq).collect::<Vec<_>>();
        assert_eq!(seqs(&p), [1, 2], "not compacted yet, each appended once");
        // Writable again: the next append compacts (the 100-day-old entry goes).
        append(&p, &mut st, &[change(3)], &by(), later).unwrap();
        assert_eq!(seqs(&p), [2, 3]);
        assert_eq!(read(&p).unwrap().0, 1, "the floor");
    }

    #[test]
    fn pending_changes_are_appended_first_and_then_cleared() {
        let p = temp("pending");
        let now = SystemTime::now();
        let mut st = state(&p).unwrap();
        // Nothing pending: an empty append appends nothing.
        assert!(
            append_with_pending(&p, &mut st, &[], &by(), now)
                .unwrap()
                .is_empty()
        );
        // A batch noted, then taken back (it changed nothing).
        let len = note_pending(&p, &[change(9)], &by()).unwrap();
        unnote(&p, len);
        assert!(pending(&p).is_empty());
        // Noted by two authors (a stop before the append; a migration).
        let migration = ChangeBy {
            client: "migration".into(),
            ..ChangeBy::default()
        };
        note_pending(&p, &[change(1), change(2)], &by()).unwrap();
        note_pending(&p, &[change(3)], &migration).unwrap();
        assert_eq!(pending(&p).len(), 3);
        let a = append_with_pending(&p, &mut st, &[change(4)], &by(), now).unwrap();
        assert_eq!(a.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3, 4]);
        assert_eq!(a[2].by, migration);
        assert_eq!(a[3].id, uuid::Uuid::from_u128(4));
        assert!(!pending_path(&p).exists(), "cleared once in the log");
        assert_eq!(since(&p, 0, 100).unwrap().changes.len(), 4);
    }

    #[test]
    fn a_torn_pending_note_loses_nothing_after_it() {
        let p = temp("torn-pending");
        let now = SystemTime::now();
        let mut st = state(&p).unwrap();
        let regie = ChangeBy {
            client: "Régie".into(),
            ..ChangeBy::default()
        };
        // A note that stopped part-way, inside the "é" of the client's name.
        note_pending(&p, &[change(1)], &regie).unwrap();
        let full = std::fs::read(pending_path(&p)).unwrap();
        let cut = full.iter().position(|b| *b == 0xC3).unwrap() + 1;
        std::fs::write(pending_path(&p), &full[..cut]).unwrap();
        // The next batch's note starts on a line of its own, and is read and logged.
        note_pending(&p, &[change(2)], &regie).unwrap();
        let noted = pending(&p);
        assert_eq!(noted.len(), 1, "only the torn line is lost");
        assert_eq!(noted[0].0.id, uuid::Uuid::from_u128(2));
        let a = append_with_pending(&p, &mut st, &[], &by(), now).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].id, uuid::Uuid::from_u128(2));
        assert_eq!(a[0].by, regie);
        assert!(!pending_path(&p).exists());
        // A torn log line inside a character spoils only itself.
        let log = std::fs::read(&p).unwrap();
        let mut torn = log.clone();
        torn.extend_from_slice(&full[..cut]);
        std::fs::write(&p, &torn).unwrap();
        assert_eq!(since(&p, 0, 100).unwrap().changes.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_pending_file_that_cant_be_read_is_kept() {
        use std::os::unix::fs::PermissionsExt as _;
        let p = temp("unreadable-pending");
        let mut st = state(&p).unwrap();
        note_pending(&p, &[change(1)], &by()).unwrap();
        let pp = pending_path(&p);
        std::fs::set_permissions(&pp, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&pp).is_ok() {
            return; // (root reads it anyway)
        }
        assert!(append_with_pending(&p, &mut st, &[change(2)], &by(), SystemTime::now()).is_err());
        assert!(pp.exists(), "its changes aren't dropped");
        std::fs::set_permissions(&pp, std::fs::Permissions::from_mode(0o644)).unwrap();
        let a = append_with_pending(&p, &mut st, &[change(2)], &by(), SystemTime::now()).unwrap();
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn a_torn_line_is_skipped_and_the_next_append_starts_clean() {
        let p = temp("torn");
        let now = SystemTime::now();
        let mut st = state(&p).unwrap();
        append(&p, &mut st, &[change(1)], &by(), now).unwrap();
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(br#"{"seq":2,"at":"20"#).unwrap();
        drop(f);
        let mut st = state(&p).unwrap();
        assert_eq!(st.head, 1);
        append(&p, &mut st, &[change(2)], &by(), now).unwrap();
        let (_, entries) = read(&p).unwrap();
        assert_eq!(entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn compaction_by_count_keeps_the_newest_and_a_floor() {
        let p = temp("count");
        let now = SystemTime::now();
        let mut st = state(&p).unwrap();
        let batch: Vec<NewChange> = (0..1000).map(change).collect();
        for _ in 0..10 {
            append(&p, &mut st, &batch, &by(), now).unwrap();
        }
        assert_eq!(
            (st.head, st.count, st.floor),
            (10_000, 10_000, 0),
            "not due yet"
        );
        append(&p, &mut st, &[change(1)], &by(), now).unwrap();
        assert_eq!(st.head, 10_001);
        assert_eq!(st.count, KEEP_ENTRIES);
        assert_eq!(st.floor, 10_001 - KEEP_ENTRIES as u64);
        assert_eq!(state(&p).unwrap(), st, "the file says the same");
        // Seq continues after a compaction.
        let e = append(&p, &mut st, &[change(2)], &by(), now).unwrap();
        assert_eq!(e[0].seq, 10_002);
        // Below the floor: resync; at the floor: everything after it.
        let r = since(&p, 100, 10).unwrap();
        assert!(r.resync && r.changes.is_empty());
        assert_eq!((r.from, r.to), (st.floor, 10_002));
        let r = since(&p, st.floor, MAX_LIMIT).unwrap();
        assert!(!r.resync);
        assert_eq!(r.changes[0].seq, st.floor + 1);
    }

    #[test]
    fn compaction_by_age_drops_old_entries_even_all_of_them() {
        let p = temp("age");
        let now = SystemTime::now();
        let mut st = state(&p).unwrap();
        append(&p, &mut st, &[change(1), change(2)], &by(), now - 100 * DAY).unwrap();
        // Not due by itself until checked again (the oldest is past 90 days now).
        assert!(compact_if_due(&p, &mut st, now).unwrap());
        assert_eq!((st.head, st.floor, st.count), (2, 2, 0));
        assert_eq!(state(&p).unwrap(), st);
        let r = since(&p, 0, 10).unwrap();
        assert!(r.resync);
        assert_eq!((r.from, r.to), (2, 2));
        assert!(!since(&p, 2, 10).unwrap().resync);
        // The next entry continues after the floor.
        append(&p, &mut st, &[change(3)], &by(), now - 10 * DAY).unwrap();
        append(&p, &mut st, &[change(4)], &by(), now).unwrap();
        assert_eq!(st.head, 4);
        assert!(!compact_if_due(&p, &mut st, now).unwrap(), "nothing old");
        // 95 days on, only the newer one stays.
        assert!(compact_if_due(&p, &mut st, now + 85 * DAY).unwrap());
        assert_eq!((st.head, st.floor, st.count), (4, 3, 1));
        let r = since(&p, 3, 10).unwrap();
        assert_eq!(r.changes.iter().map(|e| e.seq).collect::<Vec<_>>(), [4]);
    }
}
