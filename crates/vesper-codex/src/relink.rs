//! Missing media and relinking.
//!
//! **Availability** is worked out when asked (the file of the asset's location found on this
//! box, [`crate::data::CollectionContent::asset_path`]: for a library location, the item is in
//! the library now); the stored `status` is the last known one and is written only when it
//! actually changes (a search or a relink). Library items the collections use that change or
//! vanish are announced as they happen (`codexChanged` with `media`, [`super::library`]).
//!
//! Relink paths, each appending a [`RelinkRecord`] to the asset and announced with
//! `documentChanged` (through the Codex writer):
//! - **manual**: an item of the Codex's media library, a media file the Codex has by its
//!   SHA-256, a file under a media root ([`manual`]), or an upload with `CreateUpload.replaces`
//!   ([`super::uploads`]);
//! - **search** ([`search`]): for library and managed media, first the library's item with
//!   the same SHA-256 (`contentHash`: the bytes are in the library under another ID, say put
//!   back by hand); then under the media roots bound on this box: the same relative path under
//!   a bound root (`rootBinding`), else a file of the same name found once (`fileName`); for
//!   library and managed media, a file with the same SHA-256 (`contentHash`, names alike
//!   first);
//! - **client folder match** ([`match_files`]): which files of a folder the client picked are
//!   missing assets (by the end of their relative path, else by a unique name); the client
//!   uploads them with `replaces`;
//! - **bulk re-root** ([`reroot`]): every asset under a root and folder, or whose original path
//!   starts with a prefix, gets the same path under another root and folder.
//!
//! Searches read the disk on a thread of their own at low priority.

use super::blobs;
use crate::data::CollectionContent;
use model::{
    AssetLocation, MediaAsset, MediaAssetId, MediaRootId, MediaStatus, RelinkMethod, RelinkRecord,
};
use protocol::{
    ApiError, ClientFile, CollectionMedia, MatchBy, MediaAvailability, MediaMatch,
    MediaMatchReport, RelinkAmbiguous, RelinkFound, RelinkReport, RerootRequest,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

/// Files indexed by a search, at most.
const MAX_INDEXED: usize = 500_000;
/// Bytes hashed by one search, at most.
const MAX_HASHED: u64 = 50 << 30;
/// Candidates listed for an ambiguous asset, at most.
const MAX_CANDIDATES: usize = 10;

/// `/`-separated, without a leading `/` or `./`, no empty parts.
pub fn normalize(p: &str) -> String {
    p.replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// A relative path that stays below its root (no `..`, no drive letter, not even a
/// drive-relative one like `D:x`, which Windows joins as a path of its own).
pub fn safe_relative(p: &str) -> Option<String> {
    let n = normalize(p);
    let ok = !n.is_empty()
        && !n.split('/').any(|s| s == "..")
        && !n.split('/').next().is_some_and(drive_shaped)
        // On Windows a `:` anywhere names a drive or an alternate data stream.
        && !(cfg!(windows) && n.contains(':'))
        && !p.starts_with('/')
        && !p.starts_with('\\')
        && stays_below(&n);
    ok.then_some(n)
}

/// `C:`, `D:x`: a drive (or drive-relative path) on Windows, refused everywhere so a document
/// written on one box can't escape its root on another.
fn drive_shaped(s: &str) -> bool {
    let b = s.as_bytes();
    s.ends_with(':') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// Joining `p` to a folder stays inside it on this platform: only plain names (and `.`), no
/// root, drive prefix or `..`.
pub fn stays_below(p: &str) -> bool {
    use std::path::Component;
    Path::new(p)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn base_name(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}

/// `path` is `prefix` or inside it (case-insensitive; `""` is everything). The rest of `path`.
fn strip_dir<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(path);
    }
    let (pl, xl) = (path.to_lowercase(), prefix.to_lowercase());
    if pl == xl {
        return Some("");
    }
    if pl.starts_with(&format!("{xl}/")) && path.is_char_boundary(prefix.len() + 1) {
        return Some(&path[prefix.len() + 1..]);
    }
    None
}

fn join(prefix: &str, rest: &str) -> String {
    match (prefix.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_string(),
        (_, true) => prefix.to_string(),
        _ => format!("{prefix}/{rest}"),
    }
}

/// The paths an asset was known by, normalized: its linked path, its original relative path,
/// its original absolute path.
fn known_paths(a: &MediaAsset) -> Vec<String> {
    let mut out = Vec::new();
    if let AssetLocation::Linked { path, .. } | AssetLocation::ThemeAsset { path, .. } = &a.location
    {
        out.push(normalize(path));
    }
    if let Some(o) = &a.origin {
        for p in [&o.relative_path, &o.absolute_hint].into_iter().flatten() {
            let n = normalize(p);
            if !n.is_empty() && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

/// The SHA-256 of a library or managed asset's file, with its ending where known.
fn content_hash(a: &MediaAsset) -> Option<(String, Option<String>)> {
    match &a.location {
        AssetLocation::Managed { sha256, extension } => {
            Some((sha256.clone(), Some(extension.clone())))
        }
        AssetLocation::Library { sha256, .. } => Some((
            sha256.clone(),
            file_name(a)
                .map(|n| blobs::split_name(&n).1)
                .filter(|e| !e.is_empty()),
        )),
        _ => None,
    }
}

/// The asset's file name, as far as it is known.
fn file_name(a: &MediaAsset) -> Option<String> {
    known_paths(a)
        .first()
        .map(|p| base_name(p).to_string())
        .filter(|n| !n.is_empty())
}

/// An asset's availability on this box now.
pub fn availability(
    c: &CollectionContent,
    id: MediaAssetId,
) -> (MediaAvailability, Option<String>) {
    match c.asset_path(id) {
        Ok(_) => (MediaAvailability::Online, None),
        Err(e) => {
            // A show box: in the site's library, not on this box yet.
            let copying = c.assets.get(&id).is_some_and(|a| match &a.location {
                AssetLocation::Library { id, .. } => c.library.copying(id),
                _ => false,
            });
            if copying {
                (MediaAvailability::Copying, Some(e))
            } else {
                (MediaAvailability::Missing, Some(e))
            }
        }
    }
}

/// The status to store for an asset whose file is (or isn't) there.
fn status_for(stored: MediaStatus, exists: bool) -> MediaStatus {
    match (exists, stored) {
        (true, MediaStatus::Unsupported) => MediaStatus::Unsupported,
        (true, _) => MediaStatus::Online,
        (false, _) => MediaStatus::Missing,
    }
}

/// `GET /codex/collections/{id}/media?status=`: `status` is a comma-separated set of `online`,
/// `missing`, `unsupported` (stored), or `all` (default). Sorted by name.
pub fn list(c: &CollectionContent, status: Option<&str>) -> Result<Vec<CollectionMedia>, ApiError> {
    let wanted: Vec<String> = status
        .unwrap_or("all")
        .split([',', '+'])
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    for w in &wanted {
        if !matches!(w.as_str(), "all" | "online" | "missing" | "unsupported") {
            return Err(ApiError::invalid(
                "status",
                format!("status is online, missing, unsupported or all, not {w:?}"),
            ));
        }
    }
    let all = wanted.is_empty() || wanted.iter().any(|w| w == "all");
    let mut out: Vec<CollectionMedia> = c
        .assets
        .values()
        .filter_map(|a| {
            let (availability, problem) = availability(c, a.id);
            let keep = all
                || (wanted.iter().any(|w| w == "online")
                    && availability == MediaAvailability::Online)
                || (wanted.iter().any(|w| w == "missing")
                    && availability == MediaAvailability::Missing)
                || (wanted.iter().any(|w| w == "unsupported")
                    && a.status == MediaStatus::Unsupported);
            keep.then(|| CollectionMedia {
                revision: vesper_core::change::revision(a),
                asset: a.clone(),
                availability,
                problem,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        a.asset
            .name
            .to_lowercase()
            .cmp(&b.asset.name.to_lowercase())
            .then(a.asset.id.0.cmp(&b.asset.id.0))
    });
    Ok(out)
}

/// A relink found for one asset.
#[derive(Clone, Debug)]
pub struct Change {
    /// The asset.
    pub asset: MediaAssetId,
    /// Its location when the search looked (applied only if it is still that).
    pub previous: AssetLocation,
    /// Its new location.
    pub location: AssetLocation,
    /// How it was found.
    pub method: RelinkMethod,
    /// Its file exists on this box.
    pub exists: bool,
}

/// A stored status to change.
#[derive(Clone, Debug)]
pub struct StatusChange {
    /// The asset.
    pub asset: MediaAssetId,
    /// Its stored status when the search looked.
    pub from: MediaStatus,
    /// Its new status.
    pub to: MediaStatus,
}

/// What a search or a re-root found, to apply through the writer.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    /// The report (`relinked` lists `changes`).
    pub report: RelinkReport,
    /// Relinks.
    pub changes: Vec<Change>,
    /// Status changes without a relink.
    pub statuses: Vec<StatusChange>,
}

impl Outcome {
    fn relink(
        &mut self,
        a: &MediaAsset,
        location: AssetLocation,
        method: RelinkMethod,
        exists: bool,
    ) {
        self.report.relinked.push(RelinkFound {
            asset: a.id,
            name: a.name.clone(),
            method,
            location: location.clone(),
        });
        self.changes.push(Change {
            asset: a.id,
            previous: a.location.clone(),
            location,
            method,
            exists,
        });
    }
}

/// The kind of media at a location of the Codex, by its file's ending (managed files, library
/// items); `None` for other locations (left as the asset says).
pub fn location_kind(c: &CollectionContent, location: &AssetLocation) -> Option<model::MediaKind> {
    match location {
        AssetLocation::Managed { extension, .. } => blobs::kind_for(extension),
        AssetLocation::Library { id, .. } => {
            let p = c.library.resolve(id).ok()?;
            let name = p.file_name()?.to_string_lossy().to_string();
            blobs::kind_for(&blobs::split_name(&name).1)
        }
        _ => None,
    }
}

/// The asset after a relink: the record appended, the status set (`kind`: the new file's,
/// when known).
pub fn relinked(
    cur: &MediaAsset,
    location: AssetLocation,
    method: RelinkMethod,
    exists: bool,
    kind: Option<model::MediaKind>,
) -> MediaAsset {
    let mut a = cur.clone();
    let now = crate::auth::rfc3339(std::time::SystemTime::now(), true);
    a.relinks.push(RelinkRecord {
        method,
        previous: cur.location.clone(),
        at: Some(now.clone()),
    });
    if let Some(k) = kind {
        a.kind = k;
    }
    a.location = location;
    // Another file: its preparation starts again (the trim stays).
    if let Some(p) = &mut a.prep {
        p.forget_file();
        if p.is_empty() {
            a.prep = None;
        }
    }
    let status = status_for(
        if a.status == MediaStatus::Unsupported {
            MediaStatus::Unknown
        } else {
            a.status
        },
        exists,
    );
    if status != a.status {
        a.status = status;
        a.checked_at = Some(now);
    }
    a
}

/// The writes an outcome makes against the collection's current documents: a relink only
/// where the asset's location is still what the search saw, a status only where it is still
/// what the search saw.
pub fn apply(c: &CollectionContent, o: &Outcome) -> Vec<MediaAsset> {
    let mut out: HashMap<MediaAssetId, MediaAsset> = HashMap::new();
    for ch in &o.changes {
        if let Some(cur) = c.assets.get(&ch.asset)
            && cur.location == ch.previous
        {
            out.insert(
                ch.asset,
                relinked(
                    cur,
                    ch.location.clone(),
                    ch.method,
                    ch.exists,
                    location_kind(c, &ch.location),
                ),
            );
        }
    }
    let now = crate::auth::rfc3339(std::time::SystemTime::now(), true);
    for s in &o.statuses {
        if out.contains_key(&s.asset) {
            continue;
        }
        if let Some(cur) = c.assets.get(&s.asset)
            && cur.status == s.from
        {
            let mut a = cur.clone();
            a.status = s.to;
            a.checked_at = Some(now.clone());
            out.insert(s.asset, a);
        }
    }
    let mut v: Vec<MediaAsset> = out.into_values().collect();
    v.sort_by_key(|a| a.id.0);
    v
}

/// Media roots bound on this box whose folder exists, in a stable order.
fn bound_roots(c: &CollectionContent) -> Vec<(MediaRootId, PathBuf)> {
    let mut v: Vec<(MediaRootId, PathBuf)> = c
        .media_roots
        .iter()
        .filter(|(_, p)| p.is_dir())
        .map(|(r, p)| (*r, p.clone()))
        .collect();
    v.sort_by_key(|(r, _)| r.0);
    v
}

/// Every file under the bound roots, by lowercase name: (root, relative path).
fn index(roots: &[(MediaRootId, PathBuf)]) -> HashMap<String, Vec<(MediaRootId, String)>> {
    let mut out: HashMap<String, Vec<(MediaRootId, String)>> = HashMap::new();
    let mut n = 0usize;
    for (root, dir) in roots {
        let mut stack = vec![(dir.clone(), String::new())];
        while let Some((d, rel)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let Ok(t) = e.file_type() else { continue };
                let r = join(&rel, &name);
                if t.is_dir() {
                    stack.push((e.path(), r));
                } else if t.is_file() {
                    n += 1;
                    if n > MAX_INDEXED {
                        tracing::warn!(
                            "relink search: more than {MAX_INDEXED} files under the media roots, the rest not looked at"
                        );
                        return out;
                    }
                    out.entry(name.to_lowercase()).or_default().push((*root, r));
                }
            }
        }
    }
    for v in out.values_mut() {
        v.sort_by(|a, b| a.0.0.cmp(&b.0.0).then(a.1.cmp(&b.1)));
    }
    out
}

fn sha_of(path: &Path) -> std::io::Result<(String, u64)> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0u64;
    loop {
        let k = f.read(&mut buf)?;
        if k == 0 {
            break;
        }
        n += k as u64;
        h.update(&buf[..k]);
    }
    Ok((h.finalize().iter().map(|b| format!("{b:02x}")).collect(), n))
}

/// The assets a search or match looks at: those asked for, else every missing one.
fn targets(
    c: &CollectionContent,
    asked: Option<&[MediaAssetId]>,
) -> Result<Vec<MediaAsset>, ApiError> {
    match asked {
        Some(ids) => ids
            .iter()
            .map(|id| {
                c.assets.get(id).cloned().ok_or_else(|| {
                    ApiError::invalid(
                        "assets",
                        format!("media asset {} is not in the collection", id.0),
                    )
                })
            })
            .collect(),
        None => {
            let mut v: Vec<MediaAsset> = c
                .assets
                .values()
                .filter(|a| c.asset_path(a.id).is_err())
                .cloned()
                .collect();
            v.sort_by_key(|a| a.id.0);
            Ok(v)
        }
    }
}

/// `POST …/media/search` (blocking: walks the bound roots, hashes candidates).
pub fn search(c: &CollectionContent, asked: Option<&[MediaAssetId]>) -> Result<Outcome, ApiError> {
    let assets = targets(c, asked)?;
    let roots = bound_roots(c);
    let mut o = Outcome::default();
    let mut idx: Option<HashMap<String, Vec<(MediaRootId, String)>>> = None;
    let mut hashes: HashMap<PathBuf, Option<String>> = HashMap::new();
    let mut hashed = 0u64;
    for a in &assets {
        o.report.checked += 1;
        let exists = c.asset_path(a.id).is_ok();
        if exists {
            let to = status_for(a.status, true);
            if to != a.status {
                o.statuses.push(StatusChange {
                    asset: a.id,
                    from: a.status,
                    to,
                });
            }
            continue;
        }
        // 0. Library and managed media: the library's item with the same bytes.
        if let Some((sha, _)) = content_hash(a)
            && let Some((id, _)) = c.library.by_sha(&sha)
        {
            let loc = AssetLocation::Library { id, sha256: sha };
            if loc != a.location {
                o.relink(a, loc, RelinkMethod::ContentHash, true);
                continue;
            }
        }
        // 1. The same relative path under a bound root.
        let mut found: Vec<AssetLocation> = Vec::new();
        for rel in known_paths(a).iter().filter_map(|p| safe_relative(p)) {
            for (root, dir) in &roots {
                let loc = AssetLocation::Linked {
                    root: *root,
                    path: rel.clone(),
                };
                if dir.join(&rel).is_file() && !found.contains(&loc) {
                    found.push(loc);
                }
            }
            if !found.is_empty() {
                break;
            }
        }
        // The asset's own root first.
        if let AssetLocation::Linked { root, .. } = &a.location
            && let Some(own) = found
                .iter()
                .find(|l| matches!(l, AssetLocation::Linked { root: r, .. } if r == root))
        {
            found = vec![own.clone()];
        }
        if found.len() == 1 {
            o.relink(a, found.remove(0), RelinkMethod::RootBinding, true);
            continue;
        }
        if found.len() > 1 {
            found.truncate(MAX_CANDIDATES);
            o.report.ambiguous.push(RelinkAmbiguous {
                asset: a.id,
                name: a.name.clone(),
                candidates: found,
            });
            continue;
        }
        let idx = idx.get_or_insert_with(|| index(&roots));
        let root_dir = |r: &MediaRootId| roots.iter().find(|(x, _)| x == r).map(|(_, d)| d.clone());
        // 2. Managed media: a file with the same SHA-256 (files of the same name first, then
        //    every file with the same extension).
        if let Some((sha256, extension)) = content_hash(a) {
            let (sha256, extension) = (&sha256, extension.unwrap_or_default());
            let name = file_name(a).map(|n| n.to_lowercase());
            let mut cands: Vec<(MediaRootId, String)> = name
                .as_ref()
                .and_then(|n| idx.get(n))
                .cloned()
                .unwrap_or_default();
            let mut rest: Vec<(MediaRootId, String)> = idx
                .iter()
                .filter(|(n, _)| {
                    Some(*n) != name.as_ref()
                        && !extension.is_empty()
                        && blobs::split_name(n).1 == extension.to_ascii_lowercase()
                })
                .flat_map(|(_, v)| v.iter().cloned())
                .collect();
            rest.sort_by(|a, b| a.0.0.cmp(&b.0.0).then(a.1.cmp(&b.1)));
            cands.extend(rest);
            let mut hit = None;
            for (root, rel) in cands {
                let Some(dir) = root_dir(&root) else { continue };
                let p = dir.join(&rel);
                let h = match hashes.get(&p) {
                    Some(h) => h.clone(),
                    None => {
                        if hashed > MAX_HASHED {
                            break;
                        }
                        let h = sha_of(&p).ok().map(|(h, n)| {
                            hashed += n;
                            h
                        });
                        hashes.insert(p.clone(), h.clone());
                        h
                    }
                };
                if h.as_deref() == Some(sha256.as_str()) {
                    hit = Some(AssetLocation::Linked { root, path: rel });
                    break;
                }
            }
            match hit {
                Some(loc) => o.relink(a, loc, RelinkMethod::ContentHash, true),
                None => o.report.not_found.push(a.id),
            }
            continue;
        }
        // 3. A file of the same name, found once.
        let cands: Vec<AssetLocation> = file_name(a)
            .and_then(|n| idx.get(&n.to_lowercase()))
            .map(|v| {
                v.iter()
                    .map(|(root, rel)| AssetLocation::Linked {
                        root: *root,
                        path: rel.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        match cands.len() {
            0 => o.report.not_found.push(a.id),
            1 => o.relink(a, cands[0].clone(), RelinkMethod::FileName, true),
            _ => o.report.ambiguous.push(RelinkAmbiguous {
                asset: a.id,
                name: a.name.clone(),
                candidates: cands.into_iter().take(MAX_CANDIDATES).collect(),
            }),
        }
    }
    // Missing assets whose stored status still says otherwise.
    for a in &assets {
        let missing = c.asset_path(a.id).is_err();
        let relinked = o.changes.iter().any(|ch| ch.asset == a.id);
        if missing && !relinked && a.status != MediaStatus::Missing {
            o.statuses.push(StatusChange {
                asset: a.id,
                from: a.status,
                to: MediaStatus::Missing,
            });
        }
    }
    o.report.status_changed = o.statuses.len() as u32;
    Ok(o)
}

/// `POST …/media/match`: which client files are missing assets.
pub fn match_files(
    c: &CollectionContent,
    files: &[ClientFile],
    asked: Option<&[MediaAssetId]>,
) -> Result<MediaMatchReport, ApiError> {
    let assets = targets(c, asked)?;
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    let paths: Vec<String> = files
        .iter()
        .map(|f| normalize(f.relative_path.as_deref().unwrap_or(&f.name)).to_lowercase())
        .collect();
    for (i, f) in files.iter().enumerate() {
        by_name.entry(f.name.to_lowercase()).or_default().push(i);
    }
    let mut r = MediaMatchReport::default();
    for a in &assets {
        if asked.is_none() && c.asset_path(a.id).is_ok() {
            continue;
        }
        // By the end of the relative path: the most trailing parts in common (at least two:
        // a folder and the name).
        let mut best: (usize, Vec<usize>) = (0, Vec::new());
        for known in known_paths(a) {
            let kp: Vec<String> = known
                .to_lowercase()
                .split('/')
                .map(str::to_string)
                .collect();
            for (i, p) in paths.iter().enumerate() {
                let fp: Vec<&str> = p.split('/').collect();
                let common = kp
                    .iter()
                    .rev()
                    .zip(fp.iter().rev())
                    .take_while(|(x, y)| x.as_str() == **y)
                    .count();
                if common >= 2 {
                    if common > best.0 {
                        best = (common, vec![i]);
                    } else if common == best.0 && !best.1.contains(&i) {
                        best.1.push(i);
                    }
                }
            }
        }
        let pick = |i: usize, by| MediaMatch {
            asset: a.id,
            name: a.name.clone(),
            file: files[i].clone(),
            by,
        };
        match best.1.len() {
            1 => r.matches.push(pick(best.1[0], MatchBy::RelativePath)),
            0 => match file_name(a).and_then(|n| by_name.get(&n.to_lowercase())) {
                Some(v) if v.len() == 1 => r.matches.push(pick(v[0], MatchBy::FileName)),
                Some(v) if v.len() > 1 => r.ambiguous.push(a.id),
                _ => r.unmatched.push(a.id),
            },
            _ => r.ambiguous.push(a.id),
        }
    }
    Ok(r)
}

/// A root this collection knows: bound on this box, or one of its media root documents.
pub fn root_known(c: &CollectionContent, root: MediaRootId) -> bool {
    c.media_roots.contains_key(&root)
        || crate::store::doc_path(&c.root, "portable", model::DocumentKind::MediaRoot, root.0)
            .is_file()
}

/// `POST …/media/reroot`.
pub fn reroot(c: &CollectionContent, req: &RerootRequest) -> Result<Outcome, ApiError> {
    if !root_known(c, req.to.root) {
        return Err(ApiError::invalid(
            "to.root",
            format!(
                "media root {} is neither bound on this box nor in the collection",
                req.to.root.0
            ),
        ));
    }
    let from_prefix = normalize(&req.from.prefix);
    let to_prefix = match normalize(&req.to.prefix) {
        p if p.is_empty() => p,
        p => safe_relative(&p)
            .ok_or_else(|| ApiError::invalid("to.prefix", "a folder below the root"))?,
    };
    let to_dir = c.media_roots.get(&req.to.root).filter(|d| d.is_dir());
    let mut assets: Vec<&MediaAsset> = c.assets.values().collect();
    assets.sort_by_key(|a| a.id.0);
    let mut o = Outcome::default();
    for a in assets {
        let rest = match (&req.from.root, &a.location) {
            (Some(r), AssetLocation::Linked { root, path }) if r == root => {
                strip_dir(&normalize(path), &from_prefix).map(str::to_string)
            }
            (Some(_), _) => None,
            (None, _) => a
                .origin
                .as_ref()
                .and_then(|o| o.absolute_hint.as_deref())
                .and_then(|h| strip_dir(&normalize(h), &from_prefix).map(str::to_string)),
        };
        let Some(rest) = rest else { continue };
        o.report.checked += 1;
        let Some(path) = safe_relative(&join(&to_prefix, &rest)) else {
            o.report.not_found.push(a.id);
            continue;
        };
        let loc = AssetLocation::Linked {
            root: req.to.root,
            path: path.clone(),
        };
        if loc == a.location {
            continue;
        }
        let exists = to_dir.is_some_and(|d| d.join(&path).is_file());
        if req.only_existing && !exists {
            o.report.not_found.push(a.id);
            continue;
        }
        o.relink(a, loc, RelinkMethod::RootBinding, exists);
    }
    Ok(o)
}

/// A manual relink of one asset (`target` checked by the caller: a blob that exists, a root
/// this collection knows, a safe path). `None` when the asset already is there.
pub fn manual(
    c: &CollectionContent,
    asset: &MediaAsset,
    location: AssetLocation,
) -> Option<MediaAsset> {
    if asset.location == location {
        return None;
    }
    let exists = match &location {
        AssetLocation::Library { id, .. } => c.library.resolve(id).is_ok_and(|p| p.is_file()),
        AssetLocation::Managed { sha256, extension } => c
            .media_dir
            .join(blobs::blob_name(sha256, extension))
            .is_file(),
        AssetLocation::Linked { root, path } => c
            .media_roots
            .get(root)
            .is_some_and(|d| d.join(path).is_file()),
        _ => false,
    };
    let kind = location_kind(c, &location);
    Some(relinked(
        asset,
        location,
        RelinkMethod::Manual,
        exists,
        kind,
    ))
}

/// Runs blocking work on a thread of its own at low priority (relink searches).
pub async fn low_priority<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("codex-relink".into())
        .spawn(move || {
            super::disk::lower_thread_priority();
            let _ = tx.send(f());
        });
    if spawned.is_err() {
        return None;
    }
    rx.await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(normalize("\\\\srv\\Media\\a.jpg"), "srv/Media/a.jpg");
        assert_eq!(normalize("./a//b/"), "a/b");
        assert_eq!(safe_relative("a/b.jpg"), Some("a/b.jpg".into()));
        for bad in [
            "../a",
            "a/../../b",
            "/etc/x",
            "C:/x",
            "\\x",
            "",
            // Drive-relative: Windows joins these as a path of their own (A06-1).
            "D:Finance/payroll.xlsx",
            "C:drivers",
            "d:x",
        ] {
            assert_eq!(safe_relative(bad), None, "{bad}");
        }
        // A `:` further in is a plain name on Linux, a stream name on Windows.
        assert_eq!(
            safe_relative("Services/10:30.mp4").is_some(),
            !cfg!(windows)
        );
        assert_eq!(strip_dir("X:/Media/Bg/a.jpg", "x:/media"), Some("Bg/a.jpg"));
        assert_eq!(strip_dir("X:/Media2/a.jpg", "x:/media"), None);
        assert_eq!(strip_dir("a.jpg", ""), Some("a.jpg"));
        assert_eq!(join("", "a"), "a");
        assert_eq!(join("p", "a"), "p/a");
    }

    #[test]
    fn statuses() {
        assert_eq!(status_for(MediaStatus::Missing, true), MediaStatus::Online);
        assert_eq!(
            status_for(MediaStatus::Unsupported, true),
            MediaStatus::Unsupported
        );
        assert_eq!(status_for(MediaStatus::Online, false), MediaStatus::Missing);
    }
}
