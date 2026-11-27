//! Pairing a client: with the code shown on the server, or by a site admin's approval.
//!
//! - **At the site's server** (this server runs the Site Manager): the right code (this
//!   server's own, [`crate::auth`]) makes the client's key at once. Without a code the pairing
//!   waits here for an admin to approve it (`GET /api/v1/site/pairings`, `…/{id}/approve`,
//!   `…/deny`; ids `p<n>`).
//! - **At another server of the site** (a relay: a machine of it, which the site's server marks
//!   a pairing relay as a Vesper box: [`crate::relays`]; IDN gives an unmarked one no code,
//!   `PermissionDenied`, and pairings through it wait for an admin): the code it shows is made by
//!   the site's Site Manager ([`RelayCode`], `Enrollment::pairing_code`: six digits, good for 5
//!   minutes, until used, or until 5 wrong tries burn it). The pairing is relayed to the site's
//!   server (`Enrollment::request_key`) with the code the client typed as its PIN; the relay
//!   never judges it. The Site Manager checks it and tells its host (`code_ok`): the site's
//!   server approves such a pairing at once, without anyone's click
//!   ([`Site::approve_coded_pairings`]), as a right code pairs at the site's server itself;
//!   without the right code it waits for an admin's approval there (ids `k<n>`, IDN's). The
//!   relay passes the answer on (`Enrollment::key_answer`; with a code typed it waits
//!   [`SiteConfig::relayed_code_wait`] for it, so a right code pairs in one request) and keeps
//!   no key. A relayed key lives 10 years ([`RELAYED_KEY_DAYS`]): the relayed request can't
//!   say whether its client is a browser.
//!
//! The client asks again with the pairing's secret id (`GET /api/v1/pair/{pairingId}`) until
//! it is answered, or takes it back (`DELETE /api/v1/pair/{pairingId}`: it cancels, or types
//! its code again after `codeWrong`; [`Site::withdraw_pairing`]). A pairing ends unanswered
//! after [`SiteConfig::pairing_life`] ([`PAIRING_LIFE`]), or once its client hasn't asked for
//! its answer for [`PAIRING_IDLE`]; at most [`MAX_WAITING_PER_ADDRESS`] wait from one address
//! (an IPv6 /64). A key approved here whose client hasn't fetched it within the pairing's life
//! is revoked (it reached no one). A key Vesper recorded as made for a pairing that is still unused a day
//! after it was made is revoked too ([`Site::revoke_unused_keys`]); keys are never picked by
//! name.

use super::*;
use idn_open_protocol::trust::KeyStatus;
use protocol::{PairPending, PairResponse, PairingCode, PairingRequest, SiteApiKey};

/// How long a pairing waits for its answer (as IDN's relayed ones).
pub const PAIRING_LIFE: Duration = Duration::from_secs(600);
/// At most this many pairings wait at this server at once…
const MAX_WAITING: usize = 64;
/// …and this many from one address (an IPv6 /64): a device waits for one, so no one device
/// fills the room (owner, 2026-11-24).
const MAX_WAITING_PER_ADDRESS: usize = 3;
/// A pairing waiting for approval whose client hasn't asked for its answer this long ends
/// ([`SiteConfig::pairing_idle`]): a waiting client asks every second or so, one that went
/// away no longer holds its place for the whole [`PAIRING_LIFE`].
pub const PAIRING_IDLE: Duration = Duration::from_secs(60);
/// A browser client's key lives this many days; the desktop app's as long as IDN allows.
pub const BROWSER_KEY_DAYS: u32 = 365;
/// A relayed pairing's key life: as long as IDN allows (10 years), whatever the client, since
/// the relayed request doesn't say whether it is a browser.
pub const RELAYED_KEY_DAYS: Option<u32> = None;
/// A key Vesper recorded as made for an Oriel pairing that is still unused this long after it
/// was made is revoked (owner, 2026-11-22): it most likely never reached anyone who uses it.
/// Only recorded keys, never by name (owner, 2026-11-23).
pub const UNUSED_KEY_LIFE: Duration = Duration::from_secs(24 * 3600);
/// How long a relay waits for the site's pairing code.
const RELAY_CODE_WAIT: Duration = Duration::from_secs(5);
/// Asked again no sooner than this after the site's server gave no code.
const RELAY_CODE_RETRY: Duration = Duration::from_secs(2);
/// The site's codes last 5 minutes; a code is asked for again after at most this long…
const RELAY_CODE_MAX: Duration = Duration::from_secs(300);
/// …and at least this long.
const RELAY_CODE_MIN: Duration = Duration::from_secs(5);

/// The pairing code a relay shows (see the module docs): asked of the site's Site Manager when
/// there is none, when it ends, and after each pairing with a code typed (used up, or a try of
/// it burnt).
#[derive(Default)]
pub(super) struct RelayCode {
    /// The code, when it ends (the site's Unix seconds), and when it is asked for again here.
    code: Option<(String, u64, Instant)>,
    /// No asking before this (after a failure).
    retry_at: Option<Instant>,
    /// The site's server said this machine isn't a pairing relay (logged once).
    not_marked: bool,
}

impl RelayCode {
    /// Time to ask for a code.
    pub(super) fn due(&self, now: Instant) -> bool {
        if self.retry_at.is_some_and(|t| now < t) {
            return false;
        }
        self.code.as_ref().is_none_or(|(_, _, ask)| *ask <= now)
    }

    /// The site's answer (the code, when it ends, and how long it has left as the site counts
    /// it, whatever this machine's clock says), asked at `now`. The code when it is a new one
    /// (to show in the log).
    pub(super) fn took(
        &mut self,
        answer: std::io::Result<(String, u64, Duration)>,
        now: Instant,
    ) -> std::io::Result<Option<String>> {
        match answer {
            Ok((code, until, left)) => {
                self.retry_at = None;
                self.not_marked = false;
                let left = left.clamp(RELAY_CODE_MIN, RELAY_CODE_MAX);
                let new = self.code.as_ref().is_none_or(|(c, _, _)| *c != code);
                self.code = Some((code.clone(), until, now + left));
                Ok(new.then_some(code))
            }
            Err(e) => {
                self.retry_at = Some(now + RELAY_CODE_RETRY);
                Err(e)
            }
        }
    }

    /// The code to show and its seconds left, while it lasts.
    pub(super) fn shown(&self, now: Instant) -> Option<PairingCode> {
        let (code, _, ask) = self.code.as_ref()?;
        let left = ask.checked_duration_since(now).filter(|l| !l.is_zero())?;
        Some(PairingCode {
            code: code.clone(),
            expires_in_seconds: left.as_secs_f64().ceil() as u64,
        })
    }

    /// Forgets the code (this server isn't a relay any more).
    pub(super) fn clear(&mut self) {
        *self = RelayCode::default();
    }
}

/// How a pairing waits.
enum Wait {
    /// At this server, the site's server: answered here.
    Local {
        n: u64,
        client: String,
        address: String,
        asked_at: u64,
        browser: bool,
        /// A Vesper Launcher's pairing: its machine ID (it gets that machine's Launcher key,
        /// [`crate::launchers`]).
        launcher: Option<String>,
        /// The answer, until the client fetches it: its key, or why not.
        answer: Option<Result<PairResponse, String>>,
    },
    /// Relayed to the site's server: IDN's ticket, and whether the site said the code typed
    /// with it was wrong (it still waits for an admin there; the client may type it again).
    Relayed { ticket: u64, code_wrong: bool },
}

struct Pending {
    wait: Wait,
    since: Instant,
    /// Where it came from ([`address_group`]).
    from: IpAddr,
    /// When its client last asked for the answer.
    asked: Instant,
}

impl Pending {
    /// Still waiting for an admin's answer.
    fn unanswered(&self) -> bool {
        matches!(
            self.wait,
            Wait::Local { answer: None, .. } | Wait::Relayed { .. }
        )
    }
}

/// The address pairings are counted by: an IPv4 address, or an IPv6 address's /64 (a device
/// makes up addresses within its network's).
fn address_group(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
        v4 => v4,
    }
}

/// The pairings waiting at this server, by their secret id.
#[derive(Default)]
pub(super) struct Pairings {
    next: u64,
    by_secret: HashMap<String, Pending>,
}

/// What `POST /api/v1/pair` (and asking again) comes to.
pub enum PairOutcome {
    /// Paired: the key.
    Paired(PairResponse),
    /// Waiting for a site admin's approval.
    Waiting(PairPending),
}

/// The key's life in days for the client (`None`: as long as IDN allows).
pub fn key_days(browser: bool) -> Option<u32> {
    browser.then_some(BROWSER_KEY_DAYS)
}

fn secret() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the operating system's random source failed");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn pending(secret: &str, since: Instant, life: Duration, code_wrong: bool) -> PairPending {
    PairPending {
        pairing_id: secret.to_string(),
        expires_in_seconds: life.saturating_sub(since.elapsed()).as_secs_f64().ceil() as u64,
        code_wrong,
    }
}

fn ended() -> ApiError {
    api_err(
        ErrorCode::NotFound,
        "no such pairing: it ended unanswered (after 10 minutes, or a minute after its client last asked), or was answered already; pair again",
    )
}

fn no_site() -> ApiError {
    api_err(
        ErrorCode::Unavailable,
        "this server's Site Manager isn't running, and it isn't in another site: pairing needs a site",
    )
}

fn refused(why: &str) -> ApiError {
    api_err(
        ErrorCode::Forbidden,
        format!("the site's administrator refused this pairing: {why}"),
    )
}

fn granted(token: String, id: String) -> PairResponse {
    PairResponse {
        token,
        client_id: id,
    }
}

impl Site {
    /// `POST /api/v1/pair` once the code (if any) was checked: `code_ok` when the right code
    /// was typed here, at the site's server. At a relay the code isn't checked here: `pin` is
    /// the code the client typed, passed on to the site's server, which checks it. `name` is
    /// the key's name (`Oriel – Booth laptop`). `launcher`: a Vesper Launcher's pairing for
    /// its machine (that machine's Launcher key, [`crate::launchers`]; at the site's server
    /// only).
    pub fn pair(
        &self,
        name: &str,
        ip: IpAddr,
        code_ok: bool,
        pin: Option<&str>,
        browser: bool,
        launcher: Option<&str>,
    ) -> Result<PairOutcome, ApiError> {
        if self.handle().is_some() {
            if code_ok {
                let caller = host::Caller {
                    address: ip.to_string(),
                    client: name.to_string(),
                };
                let (token, id) = match launcher {
                    Some(machine) => {
                        self.make_launcher_key(idn_users::ADMIN, &caller, name, machine)?
                    }
                    None => self.create_client_key(name, &caller, browser)?,
                };
                return Ok(PairOutcome::Paired(granted(token, id)));
            }
            return self.wait_here(name, ip, browser, launcher);
        }
        if let Some(m) = self.member().filter(|m| m.enrollment.creds().is_some()) {
            if launcher.is_some() {
                // The update channel is the site's server's: a Launcher joins there.
                return Err(api_err(
                    ErrorCode::Conflict,
                    format!(
                        "a Vesper Launcher joins the site at the site's server ({}), not at this machine of the site",
                        m.server
                    ),
                ));
            }
            // The client's kind isn't carried: its key gets IDN's longest life
            // (`RELAYED_KEY_DAYS`).
            let _ = browser;
            self.room_for(ip)?;
            let ticket = m.enrollment.request_key(name, pin).map_err(|e| {
                api_err(
                    ErrorCode::Unavailable,
                    format!("the site's server ({}) can't be asked now: {e}", m.server),
                )
            })?;
            if pin.is_some() {
                // The code was used up there, or one of its tries burnt: show the next.
                self.refresh_relay_code(true);
                // A right code is approved at once at the site's server: the key comes back
                // in a moment. A wrong one is said at once too (IDN's `code_wrong`): the
                // pairing still waits for an admin's approval there, and the client is told
                // (`codeWrong`), so it may type the code again.
                let end = Instant::now() + self.cfg.relayed_code_wait;
                while Instant::now() < end {
                    match m.enrollment.key_status(ticket) {
                        KeyStatus::Answered(g) if g.approved => {
                            tracing::info!(client = name, %ip, key = %g.id, "pairing with the site's code: approved at the site's server");
                            return Ok(PairOutcome::Paired(granted(g.key, g.id)));
                        }
                        KeyStatus::Answered(g) => return Err(refused(&g.error)),
                        KeyStatus::Waiting { code_wrong: true } => {
                            tracing::warn!(client = name, %ip, server = %m.server, "relayed pairing with a wrong code; waiting for approval there");
                            return self.waiting(
                                Wait::Relayed {
                                    ticket,
                                    code_wrong: true,
                                },
                                ip,
                            );
                        }
                        KeyStatus::Waiting { code_wrong: false } => {}
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            tracing::info!(client = name, %ip, server = %m.server, code = pin.is_some(), "pairing relayed to the site's server; waiting for approval there");
            return self.waiting(
                Wait::Relayed {
                    ticket,
                    code_wrong: false,
                },
                ip,
            );
        }
        Err(no_site())
    }

    /// This server relays pairings: it is a machine of another server's site (and doesn't run
    /// the Site Manager). Its pairing code is then the site's ([`Site::relay_code`]).
    pub fn relays_pairings(&self) -> bool {
        self.handle().is_none() && self.enrolled_elsewhere()
    }

    /// The pairing code a relay shows, made by the site's Site Manager (while it lasts; `None`
    /// before the site's server gave one, and at a server that isn't a relay).
    pub fn relay_code(&self) -> Option<PairingCode> {
        if !self.relays_pairings() {
            return None;
        }
        lock(&self.relay_code).shown(Instant::now())
    }

    /// Asks the site's Site Manager for the relay's pairing code when it is time (`force`: now,
    /// after a pairing with a code). A new code is logged. Called every tick; blocks up to
    /// [`RELAY_CODE_WAIT`]. Forgets the code at a server that isn't a relay.
    pub(super) fn refresh_relay_code(&self, force: bool) {
        let _l = lock(&self.relay_fetch);
        let m = self
            .member()
            .filter(|m| self.handle().is_none() && m.enrollment.creds().is_some());
        let Some(m) = m else {
            lock(&self.relay_code).clear();
            return;
        };
        if !force && !lock(&self.relay_code).due(Instant::now()) {
            return;
        }
        let answer = m.enrollment.pairing_code_left(RELAY_CODE_WAIT);
        let took = lock(&self.relay_code).took(answer, Instant::now());
        match took {
            Ok(Some(code)) => tracing::info!(
                "pairing code: {code} (from the site's Site Manager; valid 5 minutes or until used)"
            ),
            Ok(None) => {}
            // Not marked a pairing relay (yet: the site's server marks a Vesper box as it
            // joins): asked again as after any failure, said once.
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                if !std::mem::replace(&mut lock(&self.relay_code).not_marked, true) {
                    tracing::info!(server = %m.server, "the site's server hasn't marked this server a pairing relay: no pairing code here, and pairings here wait for an admin's approval at the site's server ({e})");
                }
            }
            Err(e) => {
                tracing::debug!(server = %m.server, "no pairing code from the site's server yet: {e}")
            }
        }
    }

    /// At the site's server: the relayed pairings typed with the right code (`code_ok`, checked
    /// by the Site Manager) are approved at once, as a right code pairs here: a key for
    /// `app:vesper`, scope `control`, [`RELAYED_KEY_DAYS`], made as the site's administrator for
    /// the client (the Site Manager audits it, with the machine it came through). Called on
    /// their `key_requested` and every tick (for one whose event went by unseen).
    pub(super) fn approve_coded_pairings(&self) {
        let Some(h) = self.handle() else { return };
        let _l = lock(&self.coded_approvals);
        let mut answered = false;
        for r in h.key_requests().into_iter().filter(|r| r.code_ok) {
            let caller = host::Caller {
                address: String::new(),
                client: r.client.clone(),
            };
            let made = h.approve_key_request(
                r.id,
                idn_users::ADMIN,
                &caller,
                &TokenAud {
                    engine: app_audience(),
                    machine: None,
                },
                "control",
                RELAYED_KEY_DAYS,
            );
            answered = true;
            match made {
                Ok(v) => {
                    if let Some(kid) = v["id"].as_str() {
                        self.record_pairing_key(kid);
                    }
                    tracing::info!(client = %r.client, via = %r.via, key = %v["id"].as_str().unwrap_or(""), "a pairing with the site's code at {}: approved at once", r.via);
                }
                Err(e) => {
                    tracing::warn!(client = %r.client, via = %r.via, "approving a pairing with the site's code: {e}");
                }
            }
        }
        if answered {
            self.changed([SitePart::Pairings, SitePart::ApiKeys]);
        }
    }

    /// Pairing needs a site: this server's Site Manager, or a membership of another site.
    pub fn can_pair(&self) -> Result<(), ApiError> {
        if self.handle().is_some() || self.enrolled_elsewhere() {
            Ok(())
        } else {
            Err(no_site())
        }
    }

    fn wait_here(
        &self,
        name: &str,
        ip: IpAddr,
        browser: bool,
        launcher: Option<&str>,
    ) -> Result<PairOutcome, ApiError> {
        self.room_for(ip)?;
        let n = {
            let mut p = lock(&self.pairings);
            p.next += 1;
            p.next
        };
        let out = self.waiting(
            Wait::Local {
                n,
                client: name.to_string(),
                address: ip.to_string(),
                asked_at: unix_now(),
                browser,
                launcher: launcher.map(str::to_string),
                answer: None,
            },
            ip,
        )?;
        let mut e = events::ours(
            protocol::SiteEventName::KeyRequested,
            format!("{name} asks to pair, at {}", self.name),
        );
        e.machine = Some(self.name.clone());
        e.address = Some(ip.to_string());
        e.detail = json!({"pairing": format!("p{n}"), "client": name, "relayed": false, "launcher": launcher});
        self.emit(vec![e], [SitePart::Pairings]);
        Ok(out)
    }

    /// A pairing from `ip` may wait: fewer than [`MAX_WAITING`] wait, fewer than
    /// [`MAX_WAITING_PER_ADDRESS`] from its address (`429` otherwise).
    fn room_for(&self, ip: IpAddr) -> Result<(), ApiError> {
        self.expire_pairings();
        let from = address_group(ip);
        let p = lock(&self.pairings);
        if p.by_secret.len() >= MAX_WAITING {
            return Err(api_err(
                ErrorCode::RateLimited,
                "too many pairings waiting; try again later",
            ));
        }
        if p.by_secret.values().filter(|e| e.from == from).count() >= MAX_WAITING_PER_ADDRESS {
            tracing::warn!(%ip, "pairing refused: too many waiting from this address");
            return Err(api_err(
                ErrorCode::RateLimited,
                "too many pairings are waiting from this address: wait for their answer, or try again later",
            ));
        }
        Ok(())
    }

    fn waiting(&self, wait: Wait, ip: IpAddr) -> Result<PairOutcome, ApiError> {
        self.expire_pairings();
        let mut p = lock(&self.pairings);
        if p.by_secret.len() >= MAX_WAITING {
            return Err(api_err(
                ErrorCode::RateLimited,
                "too many pairings waiting; try again later",
            ));
        }
        let s = secret();
        let since = Instant::now();
        let code_wrong = matches!(
            wait,
            Wait::Relayed {
                code_wrong: true,
                ..
            }
        );
        p.by_secret.insert(
            s.clone(),
            Pending {
                wait,
                since,
                from: address_group(ip),
                asked: since,
            },
        );
        Ok(PairOutcome::Waiting(pending(
            &s,
            since,
            self.cfg.pairing_life,
            code_wrong,
        )))
    }

    /// `GET /api/v1/pair/{pairingId}`: the pairing's answer once there is one (taken once).
    pub fn pairing_status(&self, secret: &str) -> Result<PairOutcome, ApiError> {
        self.expire_pairings();
        let ticket = {
            let mut p = lock(&self.pairings);
            let Some(e) = p.by_secret.get_mut(secret) else {
                return Err(ended());
            };
            e.asked = Instant::now();
            match &e.wait {
                Wait::Local { answer: None, .. } => {
                    return Ok(PairOutcome::Waiting(pending(
                        secret,
                        e.since,
                        self.cfg.pairing_life,
                        false,
                    )));
                }
                Wait::Local { .. } => {
                    let Some(Pending {
                        wait:
                            Wait::Local {
                                answer: Some(a), ..
                            },
                        ..
                    }) = p.by_secret.remove(secret)
                    else {
                        return Err(ended());
                    };
                    return a.map(PairOutcome::Paired).map_err(|why| refused(&why));
                }
                Wait::Relayed { ticket, code_wrong } => (*ticket, e.since, *code_wrong),
            }
        };
        let (ticket, since, was_wrong) = ticket;
        let Some(m) = self.member() else {
            lock(&self.pairings).by_secret.remove(secret);
            return Err(api_err(
                ErrorCode::NotFound,
                "this server left the site meanwhile; pair again",
            ));
        };
        let g = match m.enrollment.key_status(ticket) {
            KeyStatus::Answered(g) => g,
            KeyStatus::Waiting { code_wrong } => {
                let code_wrong = code_wrong || was_wrong;
                if code_wrong
                    && !was_wrong
                    && let Some(Pending {
                        wait: Wait::Relayed { code_wrong: w, .. },
                        ..
                    }) = lock(&self.pairings).by_secret.get_mut(secret)
                {
                    *w = true;
                }
                return Ok(PairOutcome::Waiting(pending(
                    secret,
                    since,
                    self.cfg.pairing_life,
                    code_wrong,
                )));
            }
        };
        lock(&self.pairings).by_secret.remove(secret);
        if g.approved {
            tracing::info!(key = %g.id, "relayed pairing approved; the key went to its client");
            Ok(PairOutcome::Paired(granted(g.key, g.id)))
        } else {
            Err(refused(&g.error))
        }
    }

    /// `DELETE /api/v1/pair/{pairingId}`: the client takes its waiting pairing back (it
    /// cancels, or types its code again after `codeWrong`). Waiting here: it ends, with
    /// `key_request_cancelled` as when it expires. Relayed: taken back at the site's server
    /// (`Enrollment::withdraw_key_request`: gone from its waiting list). `404` for a pairing
    /// unknown or already answered (an answer stays to be fetched).
    pub fn withdraw_pairing(&self, secret: &str) -> Result<(), ApiError> {
        let answered = || {
            api_err(
                ErrorCode::NotFound,
                "this pairing was answered already: ask for its answer",
            )
        };
        let ticket = {
            let mut p = lock(&self.pairings);
            let Some(e) = p.by_secret.get(secret) else {
                return Err(ended());
            };
            match &e.wait {
                Wait::Local {
                    answer: Some(_), ..
                } => return Err(answered()),
                Wait::Local { n, .. } => {
                    let n = *n;
                    p.by_secret.remove(secret);
                    drop(p);
                    let mut e = events::ours(
                        protocol::SiteEventName::KeyRequestCancelled,
                        format!("pairing p{n} taken back by its client"),
                    );
                    e.detail = json!({"pairing": format!("p{n}"), "withdrawn": true});
                    self.emit(vec![e], [SitePart::Pairings]);
                    return Ok(());
                }
                Wait::Relayed { ticket, .. } => *ticket,
            }
        };
        let Some(m) = self.member() else {
            lock(&self.pairings).by_secret.remove(secret);
            return Ok(());
        };
        if let KeyStatus::Answered(_) = m.enrollment.key_status(ticket) {
            return Err(answered());
        }
        // Kept until the site's server says: answered between the look above and the
        // withdrawal (`AlreadyExists`, IDN Beta8), the answer stays to be fetched.
        let r = m.enrollment.withdraw_key_request(ticket);
        if matches!(&r, Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists) {
            tracing::info!(
                ticket,
                "a relayed pairing was answered as its client took it back: the answer stays"
            );
            return Err(answered());
        }
        lock(&self.pairings).by_secret.remove(secret);
        if let Err(e) = r {
            tracing::warn!(ticket, "taking a relayed pairing back: {e}");
            return Err(api_err(
                ErrorCode::NotFound,
                format!("the site's server didn't take this pairing back: {e}"),
            ));
        }
        tracing::info!(ticket, "a relayed pairing was taken back by its client");
        Ok(())
    }

    /// Pairings past [`SiteConfig::pairing_life`] end (`key_request_cancelled` for the ones
    /// waiting here). A key approved here and not fetched by then is revoked: it reached no
    /// one.
    pub(super) fn expire_pairings(&self) {
        let (gone, unfetched, idle_relayed) = {
            let mut p = lock(&self.pairings);
            let (mut gone, mut unfetched, mut idle_relayed) = (Vec::new(), Vec::new(), Vec::new());
            p.by_secret.retain(|_, e| {
                // Its time is up, or (still unanswered) its client stopped asking.
                let alive = e.since.elapsed() < self.cfg.pairing_life;
                let keep = alive && !(e.unanswered() && e.asked.elapsed() >= self.cfg.pairing_idle);
                if !keep {
                    match &e.wait {
                        Wait::Local {
                            n, client, answer, ..
                        } => match answer {
                            None => gone.push(*n),
                            Some(Ok(r)) => unfetched.push((client.clone(), r.client_id.clone())),
                            Some(Err(_)) => {}
                        },
                        // Its client went before IDN's own life for it ended: the site's
                        // server is told, so it doesn't stay listed there (and approved, mint
                        // a key no one fetches).
                        Wait::Relayed { ticket, .. } if alive => idle_relayed.push(*ticket),
                        Wait::Relayed { .. } => {}
                    }
                }
                keep
            });
            (gone, unfetched, idle_relayed)
        };
        if !idle_relayed.is_empty()
            && let Some(m) = self.member()
        {
            for ticket in idle_relayed {
                match m.enrollment.withdraw_key_request(ticket) {
                    Ok(()) => {
                        tracing::info!(ticket, "a relayed pairing whose client went was withdrawn")
                    }
                    Err(e) => tracing::warn!(
                        ticket,
                        "withdrawing a relayed pairing whose client went: {e}"
                    ),
                }
            }
        }
        for (client, key) in unfetched {
            tracing::warn!(%client, %key, "an approved pairing's key was never fetched: it is revoked");
            self.revoke_unreached_key(&key);
        }
        if gone.is_empty() {
            return;
        }
        let events = gone
            .into_iter()
            .map(|n| {
                let mut e = events::ours(
                    protocol::SiteEventName::KeyRequestCancelled,
                    format!("pairing p{n} ended unanswered"),
                );
                e.detail = json!({"pairing": format!("p{n}")});
                e
            })
            .collect();
        self.emit(events, [SitePart::Pairings]);
    }

    /// `GET /api/v1/site/pairings`: the pairings waiting for approval, oldest first: here, and
    /// relayed by the site's other servers (on the site's server).
    pub fn pairing_requests(&self) -> Vec<PairingRequest> {
        self.expire_pairings();
        let mut out: Vec<(u64, PairingRequest)> = lock(&self.pairings)
            .by_secret
            .values()
            .filter_map(|e| match &e.wait {
                Wait::Local {
                    n,
                    client,
                    address,
                    asked_at,
                    launcher,
                    answer: None,
                    ..
                } => Some((
                    *asked_at,
                    PairingRequest {
                        id: format!("p{n}"),
                        client: client.clone(),
                        via: self.name.clone(),
                        via_fingerprint: None,
                        address: Some(address.clone()),
                        relayed: false,
                        asked_at: rfc3339(*asked_at),
                        launcher: launcher.clone(),
                    },
                )),
                _ => None,
            })
            .collect();
        if let Some(h) = self.handle() {
            out.extend(h.key_requests().into_iter().map(|r| {
                (
                    r.asked_at,
                    PairingRequest {
                        id: format!("k{}", r.id),
                        client: r.client,
                        via: r.via,
                        via_fingerprint: Some(r.via_fingerprint),
                        address: None,
                        relayed: true,
                        asked_at: rfc3339(r.asked_at),
                        launcher: None,
                    },
                )
            }));
        }
        out.sort_by(|a, b| (a.0, &a.1.id).cmp(&(b.0, &b.1.id)));
        out.into_iter().map(|(_, r)| r).collect()
    }

    /// `POST /api/v1/site/pairings/{id}/approve` (admins): the client's key is made, and goes
    /// to it (through its server, for a relayed one).
    pub fn approve_pairing(&self, actor: &Actor, id: &str) -> Result<(), ApiError> {
        let r = self.try_approve(actor, id);
        if let Err(e) = &r {
            self.audit(
                actor,
                "pairing/approve",
                &json!({"id": id}),
                Some(&e.message),
            );
        }
        self.changed([SitePart::Pairings, SitePart::ApiKeys]);
        r
    }

    fn try_approve(&self, actor: &Actor, id: &str) -> Result<(), ApiError> {
        let user = self.require(actor, Role::Admin)?;
        let h = self.running()?;
        match pairing_id(id)? {
            ('k', n) => h
                .approve_key_request(
                    n,
                    &user,
                    &actor.caller(),
                    &TokenAud {
                        engine: app_audience(),
                        machine: None,
                    },
                    "control",
                    RELAYED_KEY_DAYS,
                )
                .map(|v| {
                    if let Some(kid) = v["id"].as_str() {
                        self.record_pairing_key(kid);
                    }
                })
                .map_err(from_idn),
            (_, n) => {
                let (client, browser, launcher) = self.local_waiting(n)?;
                let (key, kid) = match &launcher {
                    Some(machine) => {
                        self.make_launcher_key(&user, &actor.caller(), &client, machine)?
                    }
                    None => {
                        let v = self.run(
                            actor,
                            &SiteCommand::ApiKey {
                                name: client.clone(),
                                aud: TokenAud {
                                    engine: app_audience(),
                                    machine: None,
                                },
                                scope: "control".into(),
                                days: key_days(browser),
                            },
                        )?;
                        let (Some(key), Some(kid)) = (v["key"].as_str(), v["id"].as_str()) else {
                            return Err(api_err(
                                ErrorCode::Internal,
                                "unexpected answer from the Site Manager",
                            ));
                        };
                        self.record_pairing_key(kid);
                        (key.to_string(), kid.to_string())
                    }
                };
                let (key, kid) = (key.as_str(), kid.as_str());
                if !self.answer_local(n, Ok(granted(key.into(), kid.into()))) {
                    // It ended meanwhile: the key reached no one.
                    self.revoke_unreached_key(kid);
                    return Err(ended());
                }
                self.audit(
                    actor,
                    "pairing/approve",
                    &json!({"id": id, "client": client, "key": kid, "launcher": launcher}),
                    None,
                );
                Ok(())
            }
        }
    }

    /// `POST /api/v1/site/pairings/{id}/deny` (admins).
    pub fn deny_pairing(
        &self,
        actor: &Actor,
        id: &str,
        reason: Option<&str>,
    ) -> Result<(), ApiError> {
        let why = reason
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .unwrap_or("not approved")
            .chars()
            .take(200)
            .collect::<String>();
        let r = (|| {
            let user = self.require(actor, Role::Admin)?;
            let h = self.running()?;
            match pairing_id(id)? {
                ('k', n) => h
                    .deny_key_request(n, &user, &actor.caller(), &why)
                    .map_err(from_idn),
                (_, n) => {
                    self.local_waiting(n)?;
                    if self.answer_local(n, Err(why.clone())) {
                        Ok(())
                    } else {
                        Err(ended())
                    }
                }
            }
        })();
        // (IDN audits its own: the relayed ones.)
        if !id.starts_with('k') || r.is_err() {
            self.audit(
                actor,
                "pairing/deny",
                &json!({"id": id, "why": why}),
                r.as_ref().err().map(|e: &ApiError| e.message.as_str()),
            );
        }
        self.changed([SitePart::Pairings]);
        r
    }

    /// The pairing `p<n>` waiting here: its client, kind and Launcher machine.
    fn local_waiting(&self, n: u64) -> Result<(String, bool, Option<String>), ApiError> {
        self.expire_pairings();
        lock(&self.pairings)
            .by_secret
            .values()
            .find_map(|e| match &e.wait {
                Wait::Local {
                    n: m,
                    client,
                    browser,
                    launcher,
                    answer: None,
                    ..
                } if *m == n => Some((client.clone(), *browser, launcher.clone())),
                _ => None,
            })
            .ok_or_else(|| api_err(ErrorCode::NotFound, format!("no pairing p{n} is waiting")))
    }

    fn answer_local(&self, n: u64, a: Result<PairResponse, String>) -> bool {
        let mut p = lock(&self.pairings);
        for e in p.by_secret.values_mut() {
            if let Wait::Local {
                n: m,
                answer: answer @ None,
                ..
            } = &mut e.wait
                && *m == n
            {
                *answer = Some(a);
                return true;
            }
        }
        false
    }

    /// Revokes a pairing's key that reached no one, once (the pairing is already gone, so it is
    /// never tried again). One revoked meanwhile (by an admin, or by the Site Manager itself: since
    /// IDN Beta8 it revokes a key approved as its client withdrew) is fine: a refusal for a key
    /// listed as revoked (or gone) is noted quietly, anything else logged once.
    pub(super) fn revoke_unreached_key(&self, id: &str) {
        let revoked_already = |h: &host::Handle| {
            h.api_keys()
                .iter()
                .find(|k| k.id == id)
                .is_none_or(|k| k.revoked)
        };
        if self.handle().is_some_and(|h| revoked_already(&h)) {
            tracing::debug!(key = %id, "a pairing's key that reached no one was revoked already");
            return;
        }
        if let Err(e) = self.revoke_key_as_server(id) {
            if self.handle().is_some_and(|h| revoked_already(&h)) {
                tracing::debug!(key = %id, "a pairing's key that reached no one was revoked meanwhile: {}", e.message);
            } else {
                tracing::error!(key = %id, "revoking a pairing's key that reached no one: {}", e.message);
            }
        }
    }

    /// A key this server made for an Oriel pairing (looked after by
    /// [`Site::revoke_unused_keys`]).
    pub(super) fn record_pairing_key(&self, id: &str) {
        let mut st = lock(&self.state);
        if st.stored.pairing_keys.insert(id.to_string()) {
            self.save(&st);
        }
    }

    /// At the site's server, every [`SiteConfig::unused_key_sweep`]: the keys Vesper recorded as
    /// made for Oriel pairings ([`Site::record_pairing_key`]) that are still unused
    /// [`SiteConfig::unused_key_life`] after they were made are revoked as the server machine
    /// (each audited here, `api_key/revoke_unused`, and by the Site Manager). Only recorded keys
    /// (owner, 2026-11-23): a key an admin made in the Site Manager is never touched, whatever
    /// its name, even one named `Oriel – …` by hand.
    pub(super) fn revoke_unused_keys(&self) {
        let Some(h) = self.handle() else { return };
        let made = {
            let mut st = lock(&self.state);
            if st
                .key_sweep
                .is_some_and(|t| t.elapsed() < self.cfg.unused_key_sweep)
            {
                return;
            }
            st.key_sweep = Some(Instant::now());
            st.stored.pairing_keys.clone()
        };
        // (Before the keys are listed: a Launcher key bound after that keeps its binding.)
        let bound = self.bound_launcher_keys();
        let now = unix_now();
        let life = self.cfg.unused_key_life.as_secs();
        let keys = h.api_keys();
        let mut revoked = false;
        for k in &keys {
            if !made.contains(&k.id) || k.revoked || k.last_used.is_some() || k.created + life > now
            {
                continue;
            }
            let r = self.revoke_key_as_server(&k.id);
            let detail = json!({
                "id": k.id, "name": k.name, "createdAt": rfc3339(k.created), "by": k.by,
                "why": format!("unused {} hours after it was made", life / 3600),
            });
            self.audit(
                &self.server_actor(),
                "api_key/revoke_unused",
                &detail,
                r.as_ref().err().map(|e| e.message.as_str()),
            );
            match r {
                Ok(()) => {
                    tracing::info!(key = %k.id, name = %k.name, "a pairing key never used since it was made is revoked");
                    revoked = true;
                }
                Err(e) => {
                    tracing::warn!(key = %k.id, "revoking an unused pairing key: {}", e.message)
                }
            }
        }
        // The recorded ids of keys gone or revoked are forgotten (and Launcher keys' bindings).
        self.forget_launcher_keys(&bound, &keys);
        let mut st = lock(&self.state);
        let before = st.stored.pairing_keys.len();
        st.stored
            .pairing_keys
            .retain(|id| !made.contains(id) || keys.iter().any(|k| &k.id == id && !k.revoked));
        if st.stored.pairing_keys.len() != before || revoked {
            self.save(&st);
        }
        drop(st);
        if revoked {
            self.changed([SitePart::ApiKeys]);
        }
    }

    /// `GET /api/v1/site/api-keys`: the site's named keys as the Site Manager lists them
    /// (empty at a server that doesn't run it).
    pub fn api_keys(&self) -> Vec<SiteApiKey> {
        let Some(h) = self.handle() else {
            return Vec::new();
        };
        let mut out: Vec<SiteApiKey> = h
            .api_keys()
            .into_iter()
            .map(|k| SiteApiKey {
                id: k.id,
                name: k.name,
                engine: k.engine,
                machine: k.machine,
                scope: k.scope,
                created_at: rfc3339(k.created),
                by: k.by,
                expires_at: rfc3339(k.expires),
                revoked: k.revoked,
                revoked_at: k.revoked_at.map(rfc3339),
                last_used: k.last_used.map(rfc3339),
                last_machine: k.last_machine,
            })
            .collect();
        out.sort_by(|a, b| {
            (a.revoked, a.name.to_lowercase(), &a.id).cmp(&(
                b.revoked,
                b.name.to_lowercase(),
                &b.id,
            ))
        });
        out
    }
}

/// `p<n>` or `k<n>`.
fn pairing_id(id: &str) -> Result<(char, u64), ApiError> {
    let mut c = id.chars();
    match (c.next(), c.as_str().parse::<u64>()) {
        (Some(k @ ('p' | 'k')), Ok(n)) => Ok((k, n)),
        _ => Err(api_err(
            ErrorCode::BadRequest,
            format!("{id:?} is not a pairing id (p<n> or k<n>)"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairings_are_counted_per_ipv4_address_and_ipv6_network() {
        let g = |s: &str| address_group(s.parse().unwrap());
        assert_eq!(g("192.0.2.7"), g("192.0.2.7"));
        assert_ne!(g("192.0.2.7"), g("192.0.2.8"));
        assert_eq!(g("::ffff:192.0.2.7"), g("192.0.2.7"));
        assert_eq!(g("2001:db8:1:2::1"), g("2001:db8:1:2:aaaa:bbbb:cccc:dddd"));
        assert_ne!(g("2001:db8:1:2::1"), g("2001:db8:1:3::1"));
    }

    #[test]
    fn pairing_ids_and_lives() {
        assert_eq!(pairing_id("p7").unwrap(), ('p', 7));
        assert_eq!(pairing_id("k12").unwrap(), ('k', 12));
        for bad in ["", "7", "x7", "p", "k-1", "pp1"] {
            assert!(pairing_id(bad).is_err(), "{bad}");
        }
        assert_eq!(key_days(true), Some(365));
        assert_eq!(key_days(false), None);
        assert_eq!(secret().len(), 32);
        assert_eq!(RELAYED_KEY_DAYS, None, "relayed keys: 10 years");
    }

    /// A relay's code: asked for when there is none, again once it ends (as the site says,
    /// within 5 s and 5 minutes here), and after a failure only after a pause.
    #[test]
    fn a_relay_asks_for_its_code_again_when_it_ends() {
        let mut c = RelayCode::default();
        let t0 = Instant::now();
        let unix: u64 = 1_800_000_000;
        // What the site says is left (its clock), for a code ending at `until`.
        let secs = |until: u64, site_now: u64| Duration::from_secs(until.saturating_sub(site_now));
        assert!(c.due(t0) && c.shown(t0).is_none());
        let r = c
            .took(
                Ok(("123456".into(), unix + 300, secs(unix + 300, unix))),
                t0,
            )
            .unwrap();
        assert_eq!(r.as_deref(), Some("123456"), "a new code is logged");
        assert!(!c.due(t0));
        let shown = c.shown(t0).unwrap();
        assert_eq!(
            (shown.code.as_str(), shown.expires_in_seconds),
            ("123456", 300)
        );
        // Still the same code while it lasts (not logged again).
        let later = t0 + Duration::from_secs(100);
        assert!(!c.due(later));
        assert_eq!(c.shown(later).unwrap().expires_in_seconds, 200);
        let r = c
            .took(
                Ok(("123456".into(), unix + 300, secs(unix + 300, unix + 100))),
                later,
            )
            .unwrap();
        assert_eq!(r, None);
        // Ended: not shown any more, asked for again; the site's next code is shown.
        let ended = t0 + Duration::from_secs(300);
        assert!(c.shown(ended).is_none());
        assert!(c.due(ended));
        let r = c
            .took(
                Ok(("654321".into(), unix + 600, secs(unix + 600, unix + 300))),
                ended,
            )
            .unwrap();
        assert_eq!(r.as_deref(), Some("654321"));
        assert_eq!(c.shown(ended).unwrap().code, "654321");
        // Whatever the site says is left, asked for again within the bounds.
        let mut skew = RelayCode::default();
        skew.took(Ok(("111111".into(), unix - 50, secs(unix - 50, unix))), t0)
            .unwrap();
        assert!(!skew.due(t0) && skew.due(t0 + RELAY_CODE_MIN));
        skew.took(
            Ok(("111111".into(), unix + 9_999, secs(unix + 9_999, unix))),
            t0,
        )
        .unwrap();
        assert!(!skew.due(t0 + Duration::from_secs(299)) && skew.due(t0 + RELAY_CODE_MAX));
        // No answer: asked again after a pause.
        let mut none = RelayCode::default();
        let e = none.took(
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "no")),
            t0,
        );
        assert!(e.is_err() && none.shown(t0).is_none());
        assert!(!none.due(t0 + Duration::from_secs(1)));
        assert!(none.due(t0 + RELAY_CODE_RETRY));
        c.clear();
        assert!(c.shown(ended).is_none() && c.due(ended));
    }
}
