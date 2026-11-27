//! The Codex's own site membership, on the Codex box (the site's server, which runs the Site
//! Manager): the Codex service joins its own site as an App member named `Codex`
//! (`host::Handle::join_own_site`, approved at once by the Site Manager it runs beside), so it
//! has an identity and certificate of its own for the box-to-Codex link over `channel/1` (M8).
//!
//! - **Identity:** `<data>/machine/codex-member/` ([`MEMBER_DIR`]), kept across restarts (an
//!   existing membership is used as it is). It is a device identity: **never in a settings
//!   backup** (like `machine/site-member/`).
//! - **When:** at the Site Manager's start, and again whenever it starts anew (a restart of
//!   the module, this box back in its own site after leaving another); followed every site
//!   tick ([`OwnMembership::follow`]). While this box is a machine of another site (its own
//!   Site Manager off) the membership is put away, and taken up again when it runs its own.
//! - Only on a box whose Codex role is `host`.

use crate::site::Site;
use idn_open_protocol::trust::{DeviceRole, Enrollment};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// `<data>/machine/<MEMBER_DIR>`: the Codex's own identity in its site.
pub const MEMBER_DIR: &str = "codex-member";
/// The Codex's device name in its site.
pub const MEMBER_NAME: &str = "Codex";
/// After a failed join, asked again no sooner than this.
const RETRY: Duration = Duration::from_secs(60);

/// The running Site Manager a membership was joined through: its site and discovery port.
type Joined = (String, u16);

#[derive(Default)]
struct Current {
    /// The membership, and the Site Manager it was joined through.
    member: Option<(Joined, Arc<Enrollment>)>,
    /// No new try before this (after a failure).
    retry_at: Option<Instant>,
}

/// The Codex's own site membership (see the module docs).
pub struct OwnMembership {
    dir: PathBuf,
    current: Mutex<Current>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl OwnMembership {
    /// The membership kept in `<data>/machine/codex-member/` (nothing is started).
    pub fn new(data: &Path) -> OwnMembership {
        OwnMembership {
            dir: data.join("machine").join(MEMBER_DIR),
            current: Mutex::new(Current::default()),
        }
    }

    /// Its folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The Codex's enrollment in its site, while it is a member (for the box-to-Codex link).
    pub fn enrollment(&self) -> Option<Arc<Enrollment>> {
        lock(&self.current).member.as_ref().map(|(_, e)| e.clone())
    }

    /// Its device fingerprint in the site, while it is a member.
    pub fn fingerprint(&self) -> Option<String> {
        self.enrollment().map(|e| e.fingerprint())
    }

    /// The server stops: the membership is put away (taken up again at the next start).
    pub fn stop(&self) {
        let mut cur = lock(&self.current);
        drop(cur.member.take());
        cur.retry_at = None;
    }

    /// Follows the site: joins it through the running Site Manager (once per start of it;
    /// an existing membership as it is), or puts the membership away while this box doesn't
    /// run the Site Manager or isn't the Codex's host (`host: false`). Blocks while joining
    /// (up to 15 s, IDN's limit).
    pub fn follow(&self, site: &Site, host: bool) {
        let handle = site.handle().filter(|_| host);
        let mut cur = lock(&self.current);
        let Some(h) = handle else {
            if let Some(((site_id, _), e)) = cur.member.take() {
                tracing::info!(site = %site_id, fingerprint = %e.fingerprint(), "the Codex's site membership is put away (this box doesn't run its site now)");
            }
            cur.retry_at = None;
            return;
        };
        let joined: Joined = (h.authority().site_id(), h.discovery_port());
        if cur.member.as_ref().is_some_and(|(j, _)| *j == joined) {
            return;
        }
        if cur.retry_at.is_some_and(|t| Instant::now() < t) {
            return;
        }
        // One enrollment per identity folder: the one of an earlier start goes first.
        drop(cur.member.take());
        match h.join_own_site(&self.dir, MEMBER_NAME, DeviceRole::App) {
            Ok(e) => {
                tracing::info!(site = %joined.0, fingerprint = %e.fingerprint(), "the Codex is a member of its site");
                site.note_own_service(&e.fingerprint());
                cur.member = Some((joined, e));
                cur.retry_at = None;
            }
            Err(e) => {
                tracing::warn!(
                    "the Codex couldn't join its own site (tried again in a minute): {e}"
                );
                cur.retry_at = Some(Instant::now() + RETRY);
            }
        }
    }
}
