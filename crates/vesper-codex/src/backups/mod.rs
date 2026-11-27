//! Settings backups in the Codex (M11) and restoring them (M12); see `docs/CODEX-PLAN.md`.
//!
//! ```text
//! <data>/codex/backups/sets/<set>.json                  BackupSet: one per product and machine
//! <data>/codex/backups/snapshots/<set>/<snapshot>.json  SettingsSnapshot: its items
//! <data>/codex/backups/blobs/<sha256>                   an item's bytes (content-addressed)
//! ```
//!
//! - **Sets.** One per product and machine: Vesper's own per box (by its server ID; this box's
//!   on the Codex box, show boxes' offered over the box-to-Codex link), Oriel's per paired
//!   client (by its key's id: [`Backups::put_preferences`]). Products to come offer their own
//!   items ([`SnapshotOffer`]); each item carries its own `schemaVersion`, so they need no
//!   Codex code.
//! - **Snapshots.** Every snapshot goes through [`Backups::accept`]: its items are checked
//!   (SHA-256, size, keys), the blobs stored, and a snapshot with the same items as the set's
//!   newest makes none (that one is answered). Then the set is pruned ([`retention`]) and
//!   blobs no snapshot names are removed.
//! - **When** ([`Backups::tick`], on the Codex box): this box's settings daily
//!   ([`BackupOptions::interval`]), and a while after they last changed
//!   ([`BackupOptions::change_delay`], 10 minutes; the settings are looked at every
//!   [`BackupOptions::poll`]); on demand (`POST /codex/backups/snapshots`).
//! - **Restore.** [`Backups::preview`] compares a snapshot with the box's settings
//!   ([`preview::diff`]); [`Backups::restore_here`] restores onto this box: a `beforeRestore`
//!   snapshot first (the undo), then the files ([`vesper::apply`]) and a `reloadData`, which
//!   applies screens, looks, outputs, show settings and the audio device at once (the server's
//!   name and font substitutions need a restart). Device identities are never in a snapshot
//!   and never written.
//!
//! **Show boxes** (over the box-to-Codex link, [`remote`]; the show box always asks):
//! - a show box runs the same schedule ([`Backups::tick`]) and offers its settings
//!   ([`this_box_offer`]) to the Codex, which checks the offer against the device it came from
//!   and takes it ([`Backups::accept_from_box`]);
//! - an admin's restore of a show box's set (or onto another box: `RestoreRequest.target`) is
//!   queued on the Codex ([`restores`]; `202` with a `QueuedRestore`); the box asks for it at
//!   its next poll, offers its `beforeRestore` snapshot, gets the package
//!   ([`Backups::prepare_restore`]: the files, and a replacement box's takeover of the old
//!   server ID, checked with the site), applies it ([`apply_here`]) and reports how it went.

pub mod preview;
pub mod remote;
pub mod restores;
pub mod retention;
pub mod store;
pub mod vesper;

use crate::Shared;
use crate::Show;
use model::{
    BackupSet, BackupSetId, ChangeBy, PRODUCT_ORIEL, PRODUCT_VESPER, SettingsSnapshot, SnapshotId,
    SnapshotItem, SnapshotReason,
};
use protocol::{
    ApiError, BackupSetSummary, CodexPart, DeviceHealth, ErrorCode, OrielPreferences,
    OrielPreferencesBackup, QueuedRestore, RestoreChange, RestorePreview, RestoreRequest,
    RestoreResult, RestoreState, SnapshotContents, SnapshotItemContent, SnapshotSummary,
    SnapshotTaken,
};
use retention::Retention;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use store::Store;
pub use vesper::{OfferedItem, RestorePackage, RestoreWrite};

/// The key and kind of Oriel's one item.
pub const PREFERENCES_KEY: &str = "preferences";
/// The kind of Oriel's item.
pub const PREFERENCES_KIND: &str = "orielPreferences";
/// The Oriel set of the server machine's own clients (trusted local requests have no key).
pub const LOCAL_CLIENT: &str = "local";

/// Most items in one snapshot.
const MAX_ITEMS: usize = 4096;
/// Largest item.
const MAX_ITEM_BYTES: usize = 16 << 20;

/// When settings snapshots are taken and how many are kept ([`crate::CodexOptions`]).
#[derive(Clone, Debug)]
pub struct BackupOptions {
    /// This box's settings at least this often, when they changed (default daily).
    pub interval: Duration,
    /// A snapshot this long after the last change of this box's settings (default 10 min).
    pub change_delay: Duration,
    /// How often the settings are looked at for changes (default 1 min).
    pub poll: Duration,
    /// Snapshots kept per set (last 10, 14 daily, 8 weekly).
    pub retention: Retention,
    /// Largest Oriel preferences upload (default 1 MiB).
    pub max_preferences_bytes: usize,
    /// A restore queued for a show box that the box hasn't taken by then expires (default
    /// 24 h).
    pub restore_expiry: Duration,
}

impl Default for BackupOptions {
    fn default() -> Self {
        BackupOptions {
            interval: Duration::from_secs(86_400),
            change_delay: Duration::from_secs(600),
            poll: Duration::from_secs(60),
            retention: Retention::default(),
            max_preferences_bytes: 1 << 20,
            restore_expiry: Duration::from_secs(86_400),
        }
    }
}

/// A snapshot offered to the Codex: by this box, a show box (over the link, M8), or a product
/// itself (Oriel's preferences).
#[derive(Clone, Debug)]
pub struct SnapshotOffer {
    /// `vesper`, `oriel`, or a product to come (1–32 of `a-z 0-9 -`).
    pub product: String,
    /// The machine: a box's server ID, a client's key id.
    pub machine: String,
    /// Its name now.
    pub name: String,
    /// A box's fingerprint in its site (a machine of the site).
    pub fingerprint: Option<String>,
    /// Why.
    pub reason: SnapshotReason,
    /// Who asked.
    pub by: ChangeBy,
    /// The product's version.
    pub version: String,
    /// The items.
    pub items: Vec<OfferedItem>,
}

/// The box a restore is for (the Codex's view, [`Backups::prepare_restore`]).
#[derive(Clone, Debug)]
pub struct RestoreTarget {
    /// Its server ID.
    pub server_id: String,
    /// Its settings now (its `beforeRestore` snapshot's items).
    pub current: Vec<SnapshotItem>,
}

/// What `POST …/restore` did: restored this box, or queued the restore for another box.
#[derive(Clone, Debug)]
pub enum RestoreOutcome {
    /// Restored here.
    Done(Box<RestoreResult>),
    /// Queued for a show box (`202`).
    Queued(Box<QueuedRestore>),
}

/// A prepared restore: what the box applies and what it changes.
#[derive(Clone, Debug)]
pub struct PreparedRestore {
    /// For the box.
    pub package: RestorePackage,
    /// The items it changes.
    pub changes: Vec<RestoreChange>,
}

/// Change detection of this box's settings.
#[derive(Clone, Default)]
struct Watch {
    /// Settings that differ from the newest snapshot: their digest and since when.
    pending: Option<(String, Instant)>,
    /// The last daily check (identical settings make no snapshot; not again before the next).
    last_daily: Option<SystemTime>,
    /// On a show box: the newest snapshot of its set on the Codex, as the last offer's answer
    /// said (digest, taken at).
    remote_latest: Option<(String, String)>,
    /// The snapshots went to the site's Codex (a show box), not this box's own: when this
    /// changes the schedule starts over (the new destination gets its first snapshot).
    remote: bool,
}

/// The Codex's settings backups (one per server, [`crate::Server::backups`]).
pub struct Backups {
    /// Serializes every change of the backups on disk.
    lock: tokio::sync::Mutex<()>,
    /// One restore at a time.
    restoring: tokio::sync::Mutex<()>,
    /// Serializes the changes of queued restores.
    queue: tokio::sync::Mutex<()>,
    watch: Mutex<Watch>,
    opts: BackupOptions,
}

fn api(code: ErrorCode, msg: impl Into<String>) -> ApiError {
    ApiError::new(code, msg)
}

fn join_err(e: tokio::task::JoinError) -> ApiError {
    api(ErrorCode::Internal, e.to_string())
}

fn io_err(e: anyhow::Error) -> ApiError {
    api(ErrorCode::Internal, format!("settings backups: {e:#}"))
}

fn now() -> String {
    crate::auth::rfc3339(SystemTime::now(), true)
}

/// The summary of a snapshot.
pub fn summarize(s: &SettingsSnapshot) -> SnapshotSummary {
    SnapshotSummary {
        id: s.id,
        set: s.set,
        seq: s.seq,
        taken_at: s.taken_at.clone(),
        reason: s.reason,
        by: s.by.clone(),
        digest: s.digest.clone(),
        items: s.items.len() as u32,
        bytes: s.bytes(),
    }
}

fn valid_product(p: &str) -> bool {
    (1..=32).contains(&p.len())
        && p.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}

/// This box's server ID.
fn own_machine(sh: &Shared) -> String {
    sh.auth().identity().id.to_string()
}

/// Collects this box's settings as an offer ("offer my snapshot": what a show box sends the
/// Codex over the link, and what the Codex box takes of itself).
pub fn this_box_offer(
    sh: &Shared,
    reason: SnapshotReason,
    by: ChangeBy,
) -> anyhow::Result<SnapshotOffer> {
    let identity = sh.auth().identity();
    let items = vesper::collect(&sh.data().root)?;
    let fingerprint = sh
        .site()
        .enrolled_elsewhere()
        .then(|| sh.site().membership().fingerprint)
        .flatten();
    Ok(SnapshotOffer {
        product: PRODUCT_VESPER.into(),
        machine: identity.id.to_string(),
        name: identity.name,
        fingerprint,
        reason,
        by,
        version: sh.version().into(),
        items,
    })
}

/// Applies a restore package on this box ("apply this restore": what a show box does with
/// the Codex's package over the link, and the last step of a restore on the Codex box): the
/// files, then a `reloadData` when documents the running server reloads changed.
pub async fn apply_here(
    sh: &Arc<Shared>,
    show: &dyn Show,
    package: RestorePackage,
) -> Result<vesper::Applied, ApiError> {
    let root = sh.data().root.clone();
    let applied = match tokio::task::spawn_blocking(move || vesper::apply(&root, &package))
        .await
        .map_err(join_err)?
    {
        Ok(a) => a,
        Err(e) => {
            // The files were put back. Should some not have been, the running box takes what
            // is on disk, so it and the next start agree.
            if e.downcast_ref::<vesper::PartlyRestored>().is_some()
                && let Err(r) = sh.codex_writer().reload(sh, show).await
            {
                tracing::warn!("reloading after a failed restore: {}", r.message);
            }
            return Err(api(ErrorCode::Conflict, format!("restoring failed: {e:#}")));
        }
    };
    if applied.reload {
        sh.codex_writer().reload(sh, show).await?;
    }
    Ok(applied)
}

impl Backups {
    /// The backups with these options.
    pub fn new(opts: BackupOptions) -> Backups {
        Backups {
            lock: tokio::sync::Mutex::new(()),
            restoring: tokio::sync::Mutex::new(()),
            queue: tokio::sync::Mutex::new(()),
            watch: Mutex::new(Watch::default()),
            opts,
        }
    }

    /// The options.
    pub fn options(&self) -> &BackupOptions {
        &self.opts
    }

    /// Holds the backups on disk (no snapshot is taken or pruned meanwhile): an export copies
    /// them.
    pub async fn hold(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.lock.lock().await
    }

    fn store(sh: &Shared) -> Store {
        Store::new(&sh.codex().layout.codex())
    }

    /// Backups live on the box that holds the Codex.
    fn host(sh: &Shared) -> Result<(), ApiError> {
        if sh.codex_is_host() {
            Ok(())
        } else {
            Err(api(
                ErrorCode::Conflict,
                "settings backups are kept by the box that holds the site's Codex",
            ))
        }
    }

    /// Takes an offered snapshot into its set (made when new): checks the items, stores the
    /// blobs, makes no snapshot when the set's newest holds the same items (answers that one,
    /// `created: false`), prunes the set and removes unused blobs; `codexChanged {backups}`.
    pub async fn accept(
        &self,
        sh: &Arc<Shared>,
        offer: SnapshotOffer,
    ) -> Result<SnapshotTaken, ApiError> {
        self.accept_keeping(sh, offer, None).await
    }

    /// [`Backups::accept`], never pruning the snapshot `keep` (the one a restore is about to
    /// restore).
    async fn accept_keeping(
        &self,
        sh: &Arc<Shared>,
        offer: SnapshotOffer,
        keep: Option<SnapshotId>,
    ) -> Result<SnapshotTaken, ApiError> {
        Self::host(sh)?;
        check_offer(&offer)?;
        let _w = self.lock.lock().await;
        let store = Self::store(sh);
        let queue = Self::queue_of(sh);
        let retention = self.opts.retention;
        let taken = tokio::task::spawn_blocking(move || {
            // Also the snapshots queued restores are waiting to restore.
            let mut kept = queue.pinned();
            kept.extend(keep);
            accept_blocking(&store, offer, retention, &kept)
        })
        .await
        .map_err(join_err)?
        .map_err(io_err)?;
        if taken.created {
            crate::writer::codex_changed(sh, &[CodexPart::Backups], None);
        }
        Ok(taken)
    }

    /// A snapshot of this box's settings now (`POST /codex/backups/snapshots`, and the
    /// scheduled ones). On a show box it is offered to the site's Codex over the link.
    pub async fn snapshot_now(
        &self,
        sh: &Arc<Shared>,
        reason: SnapshotReason,
        by: ChangeBy,
    ) -> Result<SnapshotTaken, ApiError> {
        if !sh.codex_is_host() && sh.mirror().linked() {
            let s = sh.clone();
            let offer = tokio::task::spawn_blocking(move || this_box_offer(&s, reason, by))
                .await
                .map_err(join_err)?
                .map_err(io_err)?;
            return self.offer_to_codex(sh, offer).await;
        }
        self.snapshot_keeping(sh, reason, by, None).await
    }

    /// A show box offers its settings to the site's Codex (blocking on the link, off the
    /// async threads); the answer says what the set's newest snapshot is now.
    async fn offer_to_codex(
        &self,
        sh: &Arc<Shared>,
        offer: SnapshotOffer,
    ) -> Result<SnapshotTaken, ApiError> {
        let s = sh.clone();
        let taken = tokio::task::spawn_blocking(move || s.mirror().offer_backup(offer))
            .await
            .map_err(join_err)??;
        crate::lock(&self.watch).remote_latest = Some((
            taken.snapshot.digest.clone(),
            taken.snapshot.taken_at.clone(),
        ));
        Ok(taken)
    }

    /// A snapshot offered by a show box over the link; `from` is the device fingerprint the
    /// channel authenticated. The offer must be that device's: a Vesper box's, not the Codex
    /// box's own server ID, and a machine whose set another device offered is refused unless
    /// the site lists that device as removed (a box that joined again). `keep`: never pruned
    /// by it (the snapshot a restore is about to restore).
    pub async fn accept_from_box(
        &self,
        sh: &Arc<Shared>,
        from: &str,
        mut offer: SnapshotOffer,
        keep: Option<SnapshotId>,
    ) -> Result<SnapshotTaken, ApiError> {
        Self::host(sh)?;
        self.check_peer(sh, from, &offer).await?;
        offer.fingerprint = Some(from.to_string());
        self.accept_keeping(sh, offer, keep).await
    }

    async fn check_peer(
        &self,
        sh: &Arc<Shared>,
        from: &str,
        offer: &SnapshotOffer,
    ) -> Result<(), ApiError> {
        let refuse = |m: String| api(ErrorCode::Forbidden, m);
        if offer.product != PRODUCT_VESPER {
            return Err(ApiError::invalid(
                "product",
                "only Vesper boxes offer their settings over the box-to-Codex link",
            ));
        }
        if offer.machine == own_machine(sh) {
            return Err(refuse(format!(
                "{} is the server ID of the box that holds the Codex: an offer claiming it is refused",
                offer.machine
            )));
        }
        if offer.fingerprint.as_deref().is_some_and(|f| f != from) {
            return Err(refuse(
                "the offer names another device than the one it came from: refused".into(),
            ));
        }
        let (store, machine) = (Self::store(sh), offer.machine.clone());
        let set = tokio::task::spawn_blocking(move || find_set(&store, PRODUCT_VESPER, &machine))
            .await
            .map_err(join_err)?;
        let Some(set) = set else {
            return Ok(());
        };
        match &set.fingerprint {
            Some(f) if f == from => Ok(()),
            Some(old) => {
                let (site, old) = (sh.site().clone(), old.clone());
                let removed = tokio::task::spawn_blocking(move || {
                    site.devices()
                        .into_iter()
                        .any(|d| d.fingerprint == old && d.health == DeviceHealth::Removed)
                })
                .await
                .map_err(join_err)?;
                if removed {
                    Ok(())
                } else {
                    Err(refuse(format!(
                        "server ID {} belongs to {}, another machine of the site: the offer is refused",
                        set.machine, set.name
                    )))
                }
            }
            None => Err(refuse(format!(
                "server ID {} belongs to a box that isn't a machine of the site: the offer is refused",
                set.machine
            ))),
        }
    }

    async fn snapshot_keeping(
        &self,
        sh: &Arc<Shared>,
        reason: SnapshotReason,
        by: ChangeBy,
        keep: Option<SnapshotId>,
    ) -> Result<SnapshotTaken, ApiError> {
        Self::host(sh)?;
        let s = sh.clone();
        let offer = tokio::task::spawn_blocking(move || this_box_offer(&s, reason, by))
            .await
            .map_err(join_err)?
            .map_err(io_err)?;
        self.accept_keeping(sh, offer, keep).await
    }

    /// The periodic look at this box's settings (every [`BackupOptions::poll`]): the daily
    /// snapshot when due, and one a while after the settings last changed. On the Codex box
    /// into its own set; on a show box offered to the site's Codex over the link.
    pub async fn tick(&self, sh: &Arc<Shared>, now: SystemTime) {
        let host = sh.codex_is_host();
        if !host && !sh.mirror().linked() {
            return;
        }
        let by = ChangeBy {
            client: crate::site::SERVER_MACHINE.into(),
            client_id: None,
            machine: own_machine(sh),
        };
        let s = sh.clone();
        let offer = match tokio::task::spawn_blocking(move || {
            this_box_offer(&s, SnapshotReason::Scheduled, by)
        })
        .await
        {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                tracing::warn!("settings backup: collecting this box's settings: {e:#}");
                return;
            }
            Err(e) => {
                tracing::error!("settings backup: {e}");
                return;
            }
        };
        let mut items: Vec<SnapshotItem> = offer.items.iter().map(|i| i.item.clone()).collect();
        items.sort_by(|a, b| a.key.cmp(&b.key));
        let digest = model::snapshot_digest(&items, store::sha256);
        // The set's newest snapshot: here, or as the Codex last answered a show box.
        let latest: Option<(String, String)> = if host {
            let (store, machine) = (Self::store(sh), offer.machine.clone());
            tokio::task::spawn_blocking(move || {
                find_set(&store, PRODUCT_VESPER, &machine)
                    .and_then(|set| store.snapshots(set.id).into_iter().next())
            })
            .await
            .ok()
            .flatten()
            .map(|l| (l.digest, l.taken_at))
        } else {
            crate::lock(&self.watch).remote_latest.clone()
        };
        let since = |t: SystemTime| crate::auth::rfc3339(t, true);
        let interval_ago = since(now.checked_sub(self.opts.interval).unwrap_or(now));
        // What to put back when the snapshot fails, so the next look tries again: the last
        // daily check before this one, or the change waiting since then.
        let mut undo: Option<Watch> = None;
        let reason = {
            let mut w = crate::lock(&self.watch);
            if w.remote == host {
                *w = Watch {
                    remote: !host,
                    ..Watch::default()
                };
            }
            let daily_due = latest.as_ref().is_none_or(|l| l.1 <= interval_ago)
                && w.last_daily.is_none_or(|t| since(t) <= interval_ago);
            if daily_due {
                undo = Some(w.clone());
                w.last_daily = Some(now);
                w.pending = None;
                Some(SnapshotReason::Scheduled)
            } else if latest.as_ref().is_some_and(|l| l.0 != digest) {
                // Settings differ from the newest snapshot: one once they stayed the same
                // for `change_delay`.
                let since = match w.pending.take() {
                    Some((d, at)) if d == digest => at,
                    _ => Instant::now(),
                };
                if since.elapsed() >= self.opts.change_delay {
                    undo = Some(Watch {
                        pending: Some((digest, since)),
                        ..w.clone()
                    });
                    Some(SnapshotReason::Change)
                } else {
                    w.pending = Some((digest, since));
                    None
                }
            } else {
                w.pending = None;
                None
            }
        };
        let Some(reason) = reason else { return };
        let offer = SnapshotOffer { reason, ..offer };
        let taken = if host {
            self.accept(sh, offer).await
        } else {
            self.offer_to_codex(sh, offer).await
        };
        match taken {
            Ok(t) if t.created => {
                tracing::info!(reason = ?reason, items = t.snapshot.items, "settings snapshot taken")
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("settings backup: {}", e.message);
                // Not done: tried again at the next look (a show box whose first offer failed
                // knows no newest snapshot, so nothing else would take one).
                let mut w = crate::lock(&self.watch);
                if let Some(before) = undo
                    && w.remote == before.remote
                {
                    w.last_daily = before.last_daily;
                    w.pending = before.pending;
                }
            }
        }
    }

    /// `GET /codex/backups`: every set with its newest snapshot. `client` is the asking
    /// client's Oriel set machine (its own: `yours`).
    pub async fn list(
        &self,
        sh: &Arc<Shared>,
        client: Option<String>,
    ) -> Result<Vec<BackupSetSummary>, ApiError> {
        Self::host(sh)?;
        let store = Self::store(sh);
        let own = own_machine(sh);
        tokio::task::spawn_blocking(move || {
            store
                .sets()
                .into_iter()
                .map(|set| {
                    let snaps = store.snapshots(set.id);
                    BackupSetSummary {
                        snapshots: snaps.len() as u32,
                        latest: snaps.first().map(summarize),
                        this_box: set.product == PRODUCT_VESPER && set.machine == own,
                        yours: set.product == PRODUCT_ORIEL
                            && client.as_deref() == Some(set.machine.as_str()),
                        set,
                    }
                })
                .collect()
        })
        .await
        .map_err(join_err)
    }

    fn set_of(store: &Store, id: BackupSetId) -> Result<BackupSet, ApiError> {
        store
            .set(id)
            .ok_or_else(|| api(ErrorCode::NotFound, format!("backup set {id} not found")))
    }

    fn snapshot_of(
        store: &Store,
        set: BackupSetId,
        id: SnapshotId,
    ) -> Result<SettingsSnapshot, ApiError> {
        store
            .snapshot(set, id)
            .ok_or_else(|| api(ErrorCode::NotFound, format!("snapshot {id} not found")))
    }

    /// `GET /codex/backups/{set}/snapshots`: newest first.
    pub async fn snapshots(
        &self,
        sh: &Arc<Shared>,
        set: BackupSetId,
    ) -> Result<Vec<SnapshotSummary>, ApiError> {
        Self::host(sh)?;
        let store = Self::store(sh);
        tokio::task::spawn_blocking(move || {
            Self::set_of(&store, set)?;
            Ok(store.snapshots(set).iter().map(summarize).collect())
        })
        .await
        .map_err(join_err)?
    }

    /// `GET /codex/backups/{set}/snapshots/{snapshot}` (admins): with the items' contents.
    pub async fn contents(
        &self,
        sh: &Arc<Shared>,
        set: BackupSetId,
        id: SnapshotId,
    ) -> Result<SnapshotContents, ApiError> {
        Self::host(sh)?;
        let store = Self::store(sh);
        tokio::task::spawn_blocking(move || {
            let snap = Self::snapshot_of(&store, set, id)?;
            let items = snap
                .items
                .iter()
                .map(|item| {
                    let content = store
                        .blob(&item.sha256)
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .unwrap_or(serde_json::Value::Null);
                    SnapshotItemContent {
                        item: item.clone(),
                        content,
                    }
                })
                .collect();
            Ok(SnapshotContents {
                snapshot: summarize(&snap),
                items,
            })
        })
        .await
        .map_err(join_err)?
    }

    /// `GET /codex/backups/{set}/snapshots/{snapshot}/preview` (admins): what restoring it
    /// onto `target` (a box's server ID; absent: the set's own box, as a restore) would do,
    /// compared with this box's settings now when the target is this box, else with the
    /// target box's newest snapshot here.
    pub async fn preview(
        &self,
        sh: &Arc<Shared>,
        set: BackupSetId,
        id: SnapshotId,
        target: Option<String>,
    ) -> Result<RestorePreview, ApiError> {
        Self::host(sh)?;
        let store = Self::store(sh);
        let s = sh.clone();
        tokio::task::spawn_blocking(move || {
            let the_set = Self::set_of(&store, set)?;
            if the_set.product != PRODUCT_VESPER {
                return Err(api(
                    ErrorCode::Conflict,
                    format!(
                        "a {} set is restored by {} itself",
                        the_set.product, the_set.product
                    ),
                ));
            }
            let snap = Self::snapshot_of(&store, set, id)?;
            let target = target.unwrap_or_else(|| the_set.machine.clone());
            let (current, current_as_of) = if target == own_machine(&s) {
                let items = vesper::collect(&s.data().root).map_err(io_err)?;
                (items.into_iter().map(|i| i.item).collect(), None)
            } else {
                // A show box: as its own set here holds its settings (a restore onto it is
                // prepared against what it offers then).
                let target_set = find_set(&store, PRODUCT_VESPER, &target).ok_or_else(|| {
                    api(
                        ErrorCode::Conflict,
                        format!(
                            "no box with server ID {target} has offered its settings to the Codex: a show box does once it is linked to the site's Codex"
                        ),
                    )
                })?;
                match store.snapshots(target_set.id).into_iter().next() {
                    Some(l) => (l.items, Some(l.taken_at)),
                    None => (Vec::new(), None),
                }
            };
            let (changes, unchanged) = preview::diff(&current, &snap.items);
            Ok(RestorePreview {
                snapshot: summarize(&snap),
                current_as_of,
                summary: preview::summary(&changes),
                restart: preview::restart_list(&changes),
                changes,
                unchanged,
            })
        })
        .await
        .map_err(join_err)?
    }

    /// Makes the package that restores a snapshot onto `target` (the Codex's side of a
    /// restore; for a show box it travels over the link, M8). All items, or the chosen ones;
    /// with every item, what the snapshot doesn't hold is removed. A replacement box (the set is
    /// another box's) may take over the old box's server ID only once the site lists the old
    /// box as removed.
    pub async fn prepare_restore(
        &self,
        sh: &Arc<Shared>,
        set: BackupSetId,
        id: SnapshotId,
        req: &RestoreRequest,
        target: RestoreTarget,
    ) -> Result<PreparedRestore, ApiError> {
        Self::host(sh)?;
        let store = Self::store(sh);
        let req = req.clone();
        let (the_set, snap, changes, writes) = tokio::task::spawn_blocking(move || {
            let the_set = Self::set_of(&store, set)?;
            if the_set.product != PRODUCT_VESPER {
                return Err(api(
                    ErrorCode::Conflict,
                    format!(
                        "a {} set is restored by {} itself",
                        the_set.product, the_set.product
                    ),
                ));
            }
            let snap = Self::snapshot_of(&store, set, id)?;
            let (all, _) = preview::diff(&target.current, &snap.items);
            let changes: Vec<RestoreChange> = match &req.items {
                None => all,
                Some(keys) => {
                    for k in keys {
                        let known = snap.items.iter().any(|i| &i.key == k)
                            || target.current.iter().any(|i| &i.key == k);
                        if !known {
                            return Err(ApiError::invalid(
                                "items",
                                format!("{k:?} is neither in the snapshot nor on the box"),
                            ));
                        }
                    }
                    all.into_iter().filter(|c| keys.contains(&c.key)).collect()
                }
            };
            let mut writes = Vec::new();
            for c in &changes {
                let bytes = match c.change {
                    model::DocChange::Removed => None,
                    _ => {
                        let item = snap
                            .items
                            .iter()
                            .find(|i| i.key == c.key)
                            .expect("added and changed items are in the snapshot");
                        Some(store.blob(&item.sha256).ok_or_else(|| {
                            api(
                                ErrorCode::Internal,
                                format!("the snapshot's copy of {} is missing or damaged", c.key),
                            )
                        })?)
                    }
                };
                writes.push(RestoreWrite {
                    key: c.key.clone(),
                    bytes,
                });
            }
            Ok((the_set, snap, changes, writes))
        })
        .await
        .map_err(join_err)??;
        let replacement = the_set.machine != target.server_id;
        let adopt_server_id = if req.adopt_server_id && replacement {
            Some(self.check_takeover(sh, &the_set).await?)
        } else {
            None
        };
        tracing::info!(
            set = %the_set.id,
            snapshot = %snap.id,
            items = changes.len(),
            replacement,
            adopt = adopt_server_id.is_some(),
            "restore prepared"
        );
        Ok(PreparedRestore {
            package: RestorePackage {
                writes,
                adopt_server_id,
            },
            changes,
        })
    }

    /// A replacement box may take over the old box's server ID: only once the site lists the
    /// old box (its fingerprint when it last offered a snapshot) as removed.
    async fn check_takeover(
        &self,
        sh: &Arc<Shared>,
        set: &BackupSet,
    ) -> Result<uuid::Uuid, ApiError> {
        let old: uuid::Uuid = set.machine.parse().map_err(|_| {
            api(
                ErrorCode::Conflict,
                format!("{:?} is not a server ID", set.machine),
            )
        })?;
        let Some(fp) = set.fingerprint.clone() else {
            return Err(api(
                ErrorCode::Conflict,
                format!(
                    "{} was not a machine of the site when its settings were saved: its server ID can't be taken over",
                    set.name
                ),
            ));
        };
        if sh.site().handle().is_none() {
            return Err(api(
                ErrorCode::Unavailable,
                "the site's machines can only be checked at the site's server",
            ));
        }
        let site = sh.site().clone();
        let device = tokio::task::spawn_blocking(move || {
            site.devices().into_iter().find(|d| d.fingerprint == fp)
        })
        .await
        .map_err(join_err)?;
        match device {
            Some(d) if d.health == DeviceHealth::Removed => Ok(old),
            Some(d) => Err(api(
                ErrorCode::Conflict,
                format!(
                    "{} is still a machine of the site: remove it there first, then its server ID can be taken over",
                    d.name
                ),
            )),
            None => Err(api(
                ErrorCode::Conflict,
                format!(
                    "the site doesn't list {} as removed: its server ID can't be taken over",
                    set.name
                ),
            )),
        }
    }

    /// `POST /codex/backups/{set}/snapshots/{snapshot}/restore` (admins): onto the box
    /// `req.target` names (absent: the set's own box). This box: restored at once
    /// ([`Backups::restore_here`]); another box: queued until it asks for it.
    pub async fn restore(
        &self,
        sh: &Arc<Shared>,
        show: &dyn Show,
        set: BackupSetId,
        id: SnapshotId,
        req: RestoreRequest,
        by: ChangeBy,
    ) -> Result<RestoreOutcome, ApiError> {
        Self::host(sh)?;
        let store = Self::store(sh);
        let (the_set, _) = tokio::task::spawn_blocking(move || {
            let the_set = Self::set_of(&store, set)?;
            let snap = Self::snapshot_of(&store, set, id)?;
            Ok::<_, ApiError>((the_set, snap))
        })
        .await
        .map_err(join_err)??;
        if the_set.product != PRODUCT_VESPER {
            return Err(api(
                ErrorCode::Conflict,
                format!(
                    "a {} set is restored by {} itself",
                    the_set.product, the_set.product
                ),
            ));
        }
        let target = req
            .target
            .clone()
            .unwrap_or_else(|| the_set.machine.clone());
        if target == own_machine(sh) {
            return self
                .restore_here(sh, show, set, id, req, by)
                .await
                .map(|r| RestoreOutcome::Done(Box::new(r)));
        }
        self.queue_restore(sh, &the_set, id, req, target, by)
            .await
            .map(|q| RestoreOutcome::Queued(Box::new(q)))
    }

    fn queue_of(sh: &Shared) -> restores::Queue {
        restores::Queue::new(&sh.codex().layout.codex())
    }

    /// Queues a restore for the show box `target` (its server ID): the device that offers that
    /// box's settings takes it when it next asks.
    async fn queue_restore(
        &self,
        sh: &Arc<Shared>,
        the_set: &BackupSet,
        snapshot: SnapshotId,
        req: RestoreRequest,
        target: String,
        by: ChangeBy,
    ) -> Result<QueuedRestore, ApiError> {
        let _q = self.queue.lock().await;
        let (store, t) = (Self::store(sh), target.clone());
        let target_set = tokio::task::spawn_blocking(move || find_set(&store, PRODUCT_VESPER, &t))
            .await
            .map_err(join_err)?
            .ok_or_else(|| {
                api(
                    ErrorCode::Conflict,
                    format!(
                        "no box with server ID {target} has offered its settings to the Codex: a show box does once it is linked to the site's Codex"
                    ),
                )
            })?;
        let Some(fingerprint) = target_set.fingerprint.clone() else {
            return Err(api(
                ErrorCode::Conflict,
                format!(
                    "{} isn't a machine of the site: its settings can't be restored from here",
                    target_set.name
                ),
            ));
        };
        let replacement = the_set.machine != target;
        if req.adopt_server_id && replacement {
            self.check_takeover(sh, the_set).await?;
        }
        let queue = Self::queue_of(sh);
        let now = SystemTime::now();
        let open = {
            let t = target.clone();
            let q = Self::queue_of(sh);
            tokio::task::spawn_blocking(move || {
                q.all(now)
                    .into_iter()
                    .find(|j| j.open() && j.restore.target == t)
            })
            .await
            .map_err(join_err)?
        };
        if let Some(j) = open {
            return Err(api(
                ErrorCode::Conflict,
                format!(
                    "a restore of {} is already waiting (restore {}, {:?}): cancel it or wait for it",
                    j.restore.target_name, j.restore.id, j.restore.state
                ),
            ));
        }
        let at = crate::auth::rfc3339(now, true);
        let job = restores::Job {
            restore: QueuedRestore {
                id: uuid::Uuid::new_v4().to_string(),
                set: the_set.id,
                snapshot,
                target,
                target_name: target_set.name.clone(),
                items: req.items.clone(),
                adopt_server_id: req.adopt_server_id && replacement,
                state: RestoreState::Queued,
                queued_at: at.clone(),
                updated_at: at,
                expires_at: crate::auth::rfc3339(now + self.opts.restore_expiry, true),
                by,
                error: None,
                result: None,
            },
            fingerprint,
            changes: Vec::new(),
            before: None,
            adopt: None,
            applying_since: None,
        };
        let j = job.clone();
        tokio::task::spawn_blocking(move || queue.put(&j))
            .await
            .map_err(join_err)?
            .map_err(io_err)?;
        tracing::info!(restore = %job.restore.id, target = %job.restore.target_name, set = %the_set.id, %snapshot, "settings restore queued for a show box");
        crate::writer::codex_changed(sh, &[CodexPart::Restores], None);
        // The box asks for it at once (its poll is the fallback).
        sh.codex_link().restore_queued(&job.fingerprint);
        Ok(job.restore)
    }

    /// `GET /codex/backups/restores` (admins): the queued restores, newest first.
    pub async fn restores(&self, sh: &Arc<Shared>) -> Result<Vec<QueuedRestore>, ApiError> {
        Self::host(sh)?;
        let _q = self.queue.lock().await;
        let q = Self::queue_of(sh);
        tokio::task::spawn_blocking(move || {
            q.all(SystemTime::now())
                .into_iter()
                .map(|j| j.restore)
                .collect()
        })
        .await
        .map_err(join_err)
    }

    /// One queued restore, aged to now.
    async fn job(&self, sh: &Shared, id: &str) -> Result<restores::Job, ApiError> {
        let (q, id) = (Self::queue_of(sh), id.to_string());
        tokio::task::spawn_blocking(move || {
            let mut j = q
                .get(&id)
                .ok_or_else(|| api(ErrorCode::NotFound, format!("restore {id} not found")))?;
            if restores::age(&mut j, SystemTime::now()) {
                q.put(&j).map_err(io_err)?;
            }
            Ok(j)
        })
        .await
        .map_err(join_err)?
    }

    async fn put_job(&self, sh: &Shared, job: &restores::Job) -> Result<(), ApiError> {
        let (q, j) = (Self::queue_of(sh), job.clone());
        tokio::task::spawn_blocking(move || q.put(&j))
            .await
            .map_err(join_err)?
            .map_err(io_err)?;
        crate::writer::codex_changed(sh, &[CodexPart::Restores], None);
        Ok(())
    }

    /// `GET /codex/backups/restores/{id}` (admins).
    pub async fn queued_restore(
        &self,
        sh: &Arc<Shared>,
        id: &str,
    ) -> Result<QueuedRestore, ApiError> {
        Self::host(sh)?;
        let _q = self.queue.lock().await;
        Ok(self.job(sh, id).await?.restore)
    }

    /// `DELETE /codex/backups/restores/{id}` (admins): a restore the box hasn't taken yet is
    /// cancelled.
    pub async fn cancel_restore(
        &self,
        sh: &Arc<Shared>,
        id: &str,
    ) -> Result<QueuedRestore, ApiError> {
        Self::host(sh)?;
        let _q = self.queue.lock().await;
        let mut job = self.job(sh, id).await?;
        if job.restore.state != RestoreState::Queued {
            return Err(api(
                ErrorCode::Conflict,
                format!(
                    "restore {id} is {:?}: only a waiting one can be cancelled",
                    job.restore.state
                ),
            ));
        }
        job.restore.state = RestoreState::Cancelled;
        job.restore.updated_at = now();
        self.put_job(sh, &job).await?;
        Ok(job.restore)
    }

    /// The show box `from` (its device fingerprint) asks for a restore queued for it: the
    /// oldest one waiting.
    pub async fn pending_for(
        &self,
        sh: &Arc<Shared>,
        from: &str,
    ) -> Result<Option<remote::PendingRestore>, ApiError> {
        Self::host(sh)?;
        let _q = self.queue.lock().await;
        let (q, from) = (Self::queue_of(sh), from.to_string());
        let job = tokio::task::spawn_blocking(move || {
            q.all(SystemTime::now())
                .into_iter()
                .rev()
                .find(|j| j.restore.state == RestoreState::Queued && j.fingerprint == from)
        })
        .await
        .map_err(join_err)?;
        Ok(job.map(|j| remote::PendingRestore {
            id: j.restore.id,
            set: j.restore.set,
            snapshot: j.restore.snapshot,
            by: j.restore.by,
        }))
    }

    /// The show box `from` takes restore `id`, with its settings now (`offer`, its
    /// `beforeRestore` snapshot: the undo): the package it applies, and that snapshot.
    pub async fn begin_restore(
        &self,
        sh: &Arc<Shared>,
        from: &str,
        id: &str,
        mut offer: SnapshotOffer,
    ) -> Result<(RestorePackage, SnapshotSummary), ApiError> {
        Self::host(sh)?;
        let _r = self.restoring.lock().await;
        let _q = self.queue.lock().await;
        let mut job = self.job(sh, id).await?;
        if job.fingerprint != from {
            return Err(api(
                ErrorCode::Forbidden,
                format!("restore {id} is for another box"),
            ));
        }
        if job.restore.state != RestoreState::Queued {
            return Err(api(
                ErrorCode::Conflict,
                format!("restore {id} is {:?}", job.restore.state),
            ));
        }
        if offer.machine != job.restore.target {
            return Err(api(
                ErrorCode::Forbidden,
                format!(
                    "restore {id} is for server ID {}, not {}",
                    job.restore.target, offer.machine
                ),
            ));
        }
        offer.reason = SnapshotReason::BeforeRestore;
        offer.by = job.restore.by.clone();
        let r = self.prepare_for_box(sh, from, &job, offer).await;
        match r {
            Ok((prepared, before)) => {
                job.restore.state = RestoreState::Applying;
                job.restore.updated_at = now();
                job.applying_since = Some(job.restore.updated_at.clone());
                job.changes = prepared.changes;
                job.before = Some(before.clone());
                job.adopt = prepared.package.adopt_server_id;
                self.put_job(sh, &job).await?;
                tracing::info!(restore = %job.restore.id, target = %job.restore.target_name, "settings restore taken by its show box");
                Ok((prepared.package, before))
            }
            Err(e) => {
                job.restore.state = RestoreState::Failed;
                job.restore.updated_at = now();
                job.restore.error = Some(e.message.clone());
                self.put_job(sh, &job).await?;
                tracing::warn!(restore = %job.restore.id, "settings restore failed: {}", e.message);
                Err(e)
            }
        }
    }

    async fn prepare_for_box(
        &self,
        sh: &Arc<Shared>,
        from: &str,
        job: &restores::Job,
        offer: SnapshotOffer,
    ) -> Result<(PreparedRestore, SnapshotSummary), ApiError> {
        let r = &job.restore;
        let before = self
            .accept_from_box(sh, from, offer, Some(r.snapshot))
            .await?;
        let store = Self::store(sh);
        let b = before.snapshot.clone();
        let current = tokio::task::spawn_blocking(move || store.snapshot(b.set, b.id))
            .await
            .map_err(join_err)?
            .map(|s| s.items)
            .ok_or_else(|| api(ErrorCode::Internal, "the beforeRestore snapshot is gone"))?;
        let req = RestoreRequest {
            items: r.items.clone(),
            adopt_server_id: r.adopt_server_id,
            target: None,
        };
        let target = RestoreTarget {
            server_id: r.target.clone(),
            current,
        };
        let prepared = self
            .prepare_restore(sh, r.set, r.snapshot, &req, target)
            .await?;
        Ok((prepared, before.snapshot))
    }

    /// The show box `from` reports how applying restore `id` went.
    pub async fn finish_restore(
        &self,
        sh: &Arc<Shared>,
        from: &str,
        report: remote::ResultRequest,
    ) -> Result<(), ApiError> {
        Self::host(sh)?;
        let _q = self.queue.lock().await;
        let mut job = self.job(sh, &report.id).await?;
        if job.fingerprint != from {
            return Err(api(
                ErrorCode::Forbidden,
                format!("restore {} is for another box", report.id),
            ));
        }
        if job.restore.state != RestoreState::Applying {
            return Err(api(
                ErrorCode::Conflict,
                format!("restore {} is {:?}", report.id, job.restore.state),
            ));
        }
        job.restore.updated_at = now();
        if report.ok {
            let before = job
                .before
                .clone()
                .ok_or_else(|| api(ErrorCode::Internal, "the restore has no undo snapshot"))?;
            job.restore.state = RestoreState::Applied;
            job.restore.result = Some(RestoreResult {
                before_restore: before,
                summary: preview::summary(&job.changes),
                applied: job.changes.clone(),
                restart: report.restart,
                server_id_adopted: job.adopt.is_some(),
            });
            tracing::info!(restore = %job.restore.id, target = %job.restore.target_name, "settings restored on a show box");
        } else {
            job.restore.state = RestoreState::Failed;
            job.restore.error = Some(
                report
                    .error
                    .unwrap_or_else(|| "the box couldn't apply it".into()),
            );
            tracing::warn!(restore = %job.restore.id, target = %job.restore.target_name, "a show box couldn't apply a settings restore: {:?}", job.restore.error);
        }
        self.put_job(sh, &job).await
    }

    /// Restores a set's snapshot onto this box. A `beforeRestore` snapshot of its settings
    /// first (the undo); then the files and a `reloadData`. Another box's set is a replacement
    /// of that box by this one (its server ID taken over with `adoptServerId`).
    pub async fn restore_here(
        &self,
        sh: &Arc<Shared>,
        show: &dyn Show,
        set: BackupSetId,
        id: SnapshotId,
        req: RestoreRequest,
        by: ChangeBy,
    ) -> Result<RestoreResult, ApiError> {
        Self::host(sh)?;
        let _r = self.restoring.lock().await;
        let store = Self::store(sh);
        let the_set = tokio::task::spawn_blocking(move || Self::set_of(&store, set))
            .await
            .map_err(join_err)??;
        let own = own_machine(sh);
        if the_set.product != PRODUCT_VESPER {
            return Err(api(
                ErrorCode::Conflict,
                format!(
                    "a {} set is restored by {} itself",
                    the_set.product, the_set.product
                ),
            ));
        }
        // Make sure the snapshot is there before taking the undo.
        let store = Self::store(sh);
        tokio::task::spawn_blocking(move || Self::snapshot_of(&store, set, id))
            .await
            .map_err(join_err)??;
        let before = self
            .snapshot_keeping(sh, SnapshotReason::BeforeRestore, by, Some(id))
            .await?;
        // The undo is filed in this box's own set, which isn't `set` for another box's backup
        // (a replacement restore).
        let store = Self::store(sh);
        let (before_set, before_id) = (before.snapshot.set, before.snapshot.id);
        let current = tokio::task::spawn_blocking(move || {
            store.snapshot(before_set, before_id).map(|s| s.items)
        })
        .await
        .map_err(join_err)?
        .ok_or_else(|| api(ErrorCode::Internal, "the beforeRestore snapshot is gone"))?;
        let prepared = self
            .prepare_restore(
                sh,
                set,
                id,
                &req,
                RestoreTarget {
                    server_id: own,
                    current,
                },
            )
            .await?;
        let adopted = prepared.package.adopt_server_id.is_some();
        let applied = apply_here(sh, show, prepared.package).await?;
        let summary = preview::summary(&prepared.changes);
        tracing::info!(%set, snapshot = %id, %summary, "settings restored");
        Ok(RestoreResult {
            before_restore: before.snapshot,
            summary,
            restart: applied.restart,
            applied: prepared.changes,
            server_id_adopted: adopted,
        })
    }

    /// `PUT /codex/backups/oriel`: a client's preferences, into its own set.
    pub async fn put_preferences(
        &self,
        sh: &Arc<Shared>,
        client: (String, String),
        prefs: OrielPreferences,
        by: ChangeBy,
    ) -> Result<SnapshotTaken, ApiError> {
        Self::host(sh)?;
        if !prefs.preferences.is_object() {
            return Err(ApiError::invalid(
                "preferences",
                "the preferences are a JSON object",
            ));
        }
        let bytes = serde_json::to_vec(&prefs.preferences)
            .map_err(|e| api(ErrorCode::Internal, e.to_string()))?;
        if bytes.len() > self.opts.max_preferences_bytes {
            return Err(ApiError::invalid(
                "preferences",
                format!(
                    "the preferences are at most {} bytes",
                    self.opts.max_preferences_bytes
                ),
            ));
        }
        let (machine, name) = client;
        let offer = SnapshotOffer {
            product: PRODUCT_ORIEL.into(),
            machine,
            name,
            fingerprint: None,
            reason: SnapshotReason::Upload,
            by,
            version: String::new(),
            items: vec![OfferedItem::new(
                PREFERENCES_KEY.into(),
                PREFERENCES_KIND.into(),
                None,
                None,
                prefs.schema_version,
                bytes,
            )],
        };
        self.accept(sh, offer).await
    }

    /// `GET /codex/backups/oriel[?snapshot=]`: a client's own preferences, the newest or the
    /// given snapshot's.
    pub async fn preferences(
        &self,
        sh: &Arc<Shared>,
        client: String,
        set: Option<BackupSetId>,
        snapshot: Option<SnapshotId>,
    ) -> Result<OrielPreferencesBackup, ApiError> {
        Self::host(sh)?;
        let store = Self::store(sh);
        tokio::task::spawn_blocking(move || {
            let none = || {
                api(
                    ErrorCode::NotFound,
                    match set {
                        None => "no preferences backed up for this client",
                        Some(_) => "no such set of Oriel preferences",
                    },
                )
            };
            // Another computer's Oriel preferences (any paired client may take them over),
            // never a box's settings.
            let set = match set {
                Some(id) => store
                    .set(id)
                    .filter(|s| s.product == PRODUCT_ORIEL)
                    .ok_or_else(none)?,
                None => find_set(&store, PRODUCT_ORIEL, &client).ok_or_else(none)?,
            };
            let snap = match snapshot {
                Some(id) => Self::snapshot_of(&store, set.id, id)?,
                None => store
                    .snapshots(set.id)
                    .into_iter()
                    .next()
                    .ok_or_else(none)?,
            };
            let item = snap
                .items
                .iter()
                .find(|i| i.key == PREFERENCES_KEY)
                .ok_or_else(none)?;
            let preferences = store
                .blob(&item.sha256)
                .and_then(|b| serde_json::from_slice(&b).ok())
                .ok_or_else(|| {
                    api(
                        ErrorCode::Internal,
                        "the backed-up preferences are missing or damaged",
                    )
                })?;
            Ok(OrielPreferencesBackup {
                schema_version: item.schema_version,
                preferences,
                snapshot: summarize(&snap),
            })
        })
        .await
        .map_err(join_err)?
    }
}

/// The set of a product and machine.
fn find_set(store: &Store, product: &str, machine: &str) -> Option<BackupSet> {
    store
        .sets()
        .into_iter()
        .find(|s| s.product == product && s.machine == machine)
}

/// Checks an offer: the product and machine, every item's key, size and SHA-256.
fn check_offer(o: &SnapshotOffer) -> Result<(), ApiError> {
    if !valid_product(&o.product) {
        return Err(ApiError::invalid(
            "product",
            "a product is 1 to 32 of a-z, 0-9 and -",
        ));
    }
    if !valid_text(&o.machine, 128) {
        return Err(ApiError::invalid("machine", "a machine is named by its ID"));
    }
    if o.items.len() > MAX_ITEMS {
        return Err(ApiError::invalid(
            "items",
            format!("a snapshot holds at most {MAX_ITEMS} items"),
        ));
    }
    let mut keys = std::collections::HashSet::new();
    for i in &o.items {
        let it = &i.item;
        if !valid_text(&it.key, 512) || !keys.insert(it.key.as_str()) {
            return Err(ApiError::invalid(
                "items",
                format!("item key {:?} is empty, too long or repeated", it.key),
            ));
        }
        if i.bytes.len() > MAX_ITEM_BYTES
            || it.size != i.bytes.len() as u64
            || it.sha256 != store::sha256(&i.bytes)
        {
            return Err(ApiError::invalid(
                "items",
                format!("item {:?}: its size or SHA-256 doesn't match", it.key),
            ));
        }
    }
    Ok(())
}

fn accept_blocking(
    store: &Store,
    offer: SnapshotOffer,
    retention: Retention,
    keep: &[SnapshotId],
) -> anyhow::Result<SnapshotTaken> {
    let mut items: Vec<SnapshotItem> = offer.items.iter().map(|i| i.item.clone()).collect();
    items.sort_by(|a, b| a.key.cmp(&b.key));
    let digest = model::snapshot_digest(&items, store::sha256);
    let found = find_set(store, &offer.product, &offer.machine);
    let is_new = found.is_none();
    let mut set = found.unwrap_or_else(|| BackupSet {
        schema_version: 1,
        id: BackupSetId::new(),
        product: offer.product.clone(),
        machine: offer.machine.clone(),
        name: String::new(),
        fingerprint: None,
        created_at: now(),
    });
    // The set follows the machine's name and site fingerprint.
    let fingerprint = offer.fingerprint.clone().or(set.fingerprint.clone());
    // A new set is written after its first snapshot, so it is never listed without one.
    if is_new || set.name != offer.name || set.fingerprint != fingerprint {
        set.name = offer.name.clone();
        set.fingerprint = fingerprint;
        if !is_new {
            store.put_set(&set)?;
        }
    }
    let snaps = store.snapshots(set.id);
    if let Some(latest) = snaps.first()
        && latest.digest == digest
    {
        return Ok(SnapshotTaken {
            snapshot: summarize(latest),
            created: false,
        });
    }
    for i in &offer.items {
        store.put_blob(&i.item.sha256, &i.bytes)?;
    }
    let snap = SettingsSnapshot {
        schema_version: 1,
        id: SnapshotId::new(),
        set: set.id,
        seq: snaps.first().map_or(1, |l| l.seq + 1),
        taken_at: now(),
        reason: offer.reason,
        by: offer.by,
        digest,
        version: offer.version,
        items,
    };
    store.put_snapshot(&snap)?;
    if is_new {
        store.put_set(&set)?;
    }
    let mut all = snaps;
    all.insert(0, snap.clone());
    let gone: Vec<SnapshotId> = retention::prune(&all, retention)
        .into_iter()
        .filter(|id| !keep.contains(id))
        .collect();
    for id in &gone {
        store.remove_snapshot(set.id, *id)?;
    }
    let blobs = store.gc();
    if !gone.is_empty() || blobs > 0 {
        tracing::debug!(set = %set.id, snapshots = gone.len(), blobs, "settings backups pruned");
    }
    Ok(SnapshotTaken {
        snapshot: summarize(&snap),
        created: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(machine: &str, items: &[(&str, &str)]) -> SnapshotOffer {
        SnapshotOffer {
            product: "narthex".into(),
            machine: machine.into(),
            name: "Lobby".into(),
            fingerprint: None,
            reason: SnapshotReason::Change,
            by: ChangeBy::default(),
            version: "1".into(),
            items: items
                .iter()
                .map(|(k, v)| {
                    OfferedItem::new(
                        k.to_string(),
                        "signage".into(),
                        None,
                        None,
                        7,
                        v.as_bytes().to_vec(),
                    )
                })
                .collect(),
        }
    }

    fn accept_blocking_t(
        store: &Store,
        o: SnapshotOffer,
        r: Retention,
    ) -> anyhow::Result<SnapshotTaken> {
        accept_blocking(store, o, r, &[])
    }

    #[test]
    fn accept_dedups_prunes_and_collects_blobs() {
        let dir = std::env::temp_dir().join(format!("codex-backups-{}", uuid::Uuid::new_v4()));
        let store = Store::new(&dir);
        let r = Retention {
            last: 2,
            daily: 0,
            weekly: 0,
        };
        // A product the Codex has no code for: its items are kept as they are.
        let a = accept_blocking_t(&store, offer("m1", &[("x", "1"), ("y", "2")]), r).unwrap();
        assert!(a.created);
        assert_eq!(a.snapshot.seq, 1);
        let same = accept_blocking_t(&store, offer("m1", &[("y", "2"), ("x", "1")]), r).unwrap();
        assert!(!same.created, "the same items make no snapshot");
        assert_eq!(same.snapshot.id, a.snapshot.id);
        let b = accept_blocking_t(&store, offer("m1", &[("x", "1"), ("y", "3")]), r).unwrap();
        let c = accept_blocking_t(&store, offer("m1", &[("x", "4")]), r).unwrap();
        assert_eq!((b.snapshot.seq, c.snapshot.seq), (2, 3));
        let set = find_set(&store, "narthex", "m1").unwrap();
        let left: Vec<u64> = store.snapshots(set.id).iter().map(|s| s.seq).collect();
        assert_eq!(left, vec![3, 2], "the last 2 stay");
        // Blobs of the pruned snapshot that nothing else names went.
        assert!(store.blob(&store::sha256(b"2")).is_none());
        assert!(store.blob(&store::sha256(b"1")).is_some());
        assert!(store.blob(&store::sha256(b"3")).is_some());
        let snap = store.snapshot(set.id, b.snapshot.id).unwrap();
        assert_eq!(snap.items[0].schema_version, 7);
        // Another machine: its own set.
        accept_blocking_t(&store, offer("m2", &[("x", "1")]), r).unwrap();
        assert_eq!(store.sets().len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_snapshot_a_queued_restore_waits_for_is_not_pruned() {
        let dir = std::env::temp_dir().join(format!("codex-backups-{}", uuid::Uuid::new_v4()));
        let (store, queue) = (Store::new(&dir), restores::Queue::new(&dir));
        let r = Retention {
            last: 1,
            daily: 0,
            weekly: 0,
        };
        let old = accept_blocking_t(&store, offer("m1", &[("x", "1")]), r).unwrap();
        let set = find_set(&store, "narthex", "m1").unwrap();
        // A restore of it waits for its box.
        let at = crate::auth::rfc3339(SystemTime::now(), true);
        let job = |state: RestoreState| restores::Job {
            restore: QueuedRestore {
                id: uuid::Uuid::new_v4().to_string(),
                set: set.id,
                snapshot: old.snapshot.id,
                target: "m1".into(),
                target_name: "Lobby".into(),
                items: None,
                adopt_server_id: false,
                state,
                queued_at: at.clone(),
                updated_at: at.clone(),
                expires_at: crate::auth::rfc3339(
                    SystemTime::now() + Duration::from_secs(3600),
                    true,
                ),
                by: ChangeBy::default(),
                error: None,
                result: None,
            },
            fingerprint: "fp".into(),
            changes: Vec::new(),
            before: None,
            adopt: None,
            applying_since: None,
        };
        let waiting = job(RestoreState::Queued);
        queue.put(&waiting).unwrap();
        queue.put(&job(RestoreState::Cancelled)).unwrap();
        assert_eq!(queue.pinned(), [old.snapshot.id]);
        // The box's ordinary offers (newest 1 kept) leave it.
        for v in ["2", "3"] {
            accept_blocking(&store, offer("m1", &[("x", v)]), r, &queue.pinned()).unwrap();
        }
        assert!(store.snapshot(set.id, old.snapshot.id).is_some(), "pinned");
        assert_eq!(store.snapshots(set.id).len(), 2);
        // Once the restore is done, the next offer prunes it.
        queue
            .put(&restores::Job {
                restore: QueuedRestore {
                    state: RestoreState::Applied,
                    ..waiting.restore.clone()
                },
                ..waiting
            })
            .unwrap();
        assert!(queue.pinned().is_empty());
        accept_blocking(&store, offer("m1", &[("x", "4")]), r, &queue.pinned()).unwrap();
        assert!(store.snapshot(set.id, old.snapshot.id).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn offers_are_checked() {
        let mut o = offer("m1", &[("x", "1")]);
        assert!(check_offer(&o).is_ok());
        o.items[0].bytes = b"2".to_vec();
        assert_eq!(check_offer(&o).unwrap_err().field.as_deref(), Some("items"));
        let mut o = offer("m1", &[("x", "1"), ("x", "2")]);
        assert!(check_offer(&o).is_err(), "repeated key");
        o.items.truncate(1);
        o.product = "Bad Product".into();
        assert_eq!(
            check_offer(&o).unwrap_err().field.as_deref(),
            Some("product")
        );
    }
}
