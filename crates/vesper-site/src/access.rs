//! Who acts, the audit, running changes, the site's status and settings.

use super::*;

impl Site {
    pub(super) fn users(&self) -> Users {
        Users::new(&self.idn_dir)
    }

    /// Who makes a request (see [`Actor`]).
    pub fn actor(&self, access: &Access, peer: IpAddr) -> Actor {
        let client = match access {
            Access::Local => SERVER_MACHINE.to_string(),
            Access::Paired(c) => c.name.clone(),
            Access::Unpaired => String::new(),
        };
        let admin = || (Some(idn_users::ADMIN.to_string()), Some(Role::Admin));
        let (level, bound, unlocked, damaged) = {
            let st = lock(&self.state);
            let id = match access {
                Access::Paired(c) => Some(c.id.clone()),
                _ => None,
            };
            let bound = id
                .as_ref()
                .and_then(|id| st.stored.sessions.get(id).cloned());
            let unlocked = id
                .as_ref()
                .and_then(|id| st.unlocks.get(id))
                .and_then(|at| self.cfg.unlock_idle.checked_sub(at.elapsed()))
                .filter(|left| !left.is_zero());
            (st.stored.access, bound, unlocked, st.stored.damaged)
        };
        let mut actor = Actor {
            access: level,
            user: None,
            role: None,
            signed_in: false,
            unlocked: None,
            client_id: None,
            client,
            peer: peer.to_string(),
        };
        let id = match access {
            Access::Local => {
                (actor.user, actor.role) = admin();
                return actor;
            }
            Access::Unpaired => return actor,
            Access::Paired(c) => c.id.clone(),
        };
        actor.client_id = Some(id);
        match level {
            SiteAccess::Open => (actor.user, actor.role) = admin(),
            SiteAccess::Pin => match unlocked {
                Some(left) => {
                    (actor.user, actor.role) = admin();
                    actor.unlocked = Some(left);
                }
                None => actor.role = Some(Role::Viewer),
            },
            SiteAccess::Users => {
                let users = self.users();
                match bound.and_then(|u| users.role(&u).map(|r| (u, r))) {
                    Some((u, r)) => {
                        (actor.user, actor.role) = (Some(u), Some(r));
                        actor.signed_in = true;
                    }
                    // After a damaged site.json, fail closed (the access may have been `pin`).
                    None if damaged => actor.role = Some(Role::Viewer),
                    None if !has_admin_user(&users) => (actor.user, actor.role) = admin(),
                    None => {}
                }
            }
        }
        actor
    }

    /// The server machine itself, acting on its own (the site's administrator).
    pub(crate) fn server_actor(&self) -> Actor {
        Actor {
            access: lock(&self.state).stored.access,
            user: Some(idn_users::ADMIN.to_string()),
            role: Some(Role::Admin),
            signed_in: false,
            unlocked: None,
            client_id: None,
            client: SERVER_MACHINE.to_string(),
            peer: String::new(),
        }
    }

    /// Signs a paired client in as a site user (`access: users`).
    pub fn sign_in(
        &self,
        actor: &Actor,
        ip: IpAddr,
        user: &str,
        password: &str,
    ) -> Result<SiteSession, ApiError> {
        let Some(client_id) = actor.client_id.clone() else {
            return Err(api_err(
                ErrorCode::BadRequest,
                "the server machine itself acts as admin; sign in from a paired client",
            ));
        };
        if actor.access != SiteAccess::Users {
            return Err(api_err(
                ErrorCode::Conflict,
                format!(
                    "the site's access is \"{}\": signing in is for access \"users\"",
                    actor.access.as_str()
                ),
            ));
        }
        if !lock(&self.sign_ins).attempt(ip, SIGN_INS_PER_MINUTE) {
            return Err(api_err(
                ErrorCode::RateLimited,
                "too many sign-in attempts; wait a minute",
            ));
        }
        let verified = (user != idn_users::ADMIN)
            .then(|| self.users().verify(user, password, None))
            .flatten();
        let detail = json!({"user": user});
        let Some((name, role)) = verified else {
            self.audit(actor, "sign_in", &detail, Some("wrong user or password"));
            return Err(api_err(ErrorCode::Unauthorized, "wrong user or password"));
        };
        {
            let mut st = lock(&self.state);
            st.stored.sessions.insert(client_id, name.clone());
            self.save(&st);
        }
        let mut signed = actor.clone();
        signed.user = Some(name.clone());
        signed.role = Some(role);
        signed.signed_in = true;
        self.audit(&signed, "sign_in", &detail, None);
        Ok(signed.session())
    }

    /// Ends a paired client's sign-in and its PIN unlock.
    pub fn sign_out(&self, actor: &Actor) {
        if let Some(id) = &actor.client_id {
            let mut st = lock(&self.state);
            if st.stored.sessions.remove(id).is_some() {
                self.save(&st);
                drop(st);
                self.audit(actor, "sign_out", &json!({}), None);
            } else {
                drop(st);
                self.lock_client(actor);
            }
        }
    }

    /// `POST /api/v1/site/unlock`: unlocks a paired client with the admin PIN (`access: pin`).
    pub fn unlock(&self, actor: &Actor, pin: &str) -> Result<SiteSession, ApiError> {
        let r = self.try_unlock(actor, pin);
        self.audit(
            actor,
            "unlock",
            &json!({}),
            r.as_ref().err().map(|e: &ApiError| e.message.as_str()),
        );
        let mut unlocked = actor.clone();
        r.map(|left| {
            (unlocked.user, unlocked.role) = (Some(idn_users::ADMIN.into()), Some(Role::Admin));
            unlocked.unlocked = Some(left);
            unlocked.session()
        })
    }

    fn try_unlock(&self, actor: &Actor, pin: &str) -> Result<Duration, ApiError> {
        let Some(id) = actor.client_id.clone() else {
            return Err(api_err(
                ErrorCode::BadRequest,
                "the server machine itself acts as admin; unlock from a paired client",
            ));
        };
        // One PIN at a time: a burst sent at once is counted (and locked out) as it goes, not
        // all checked against the count from before it.
        let _one = lock(&self.pin_check);
        let hash = {
            let st = lock(&self.state);
            if st.stored.access != SiteAccess::Pin {
                return Err(api_err(
                    ErrorCode::Conflict,
                    format!(
                        "the site's access is \"{}\": the PIN is for access \"pin\"",
                        st.stored.access.as_str()
                    ),
                ));
            }
            if let Some(until) = st.pin_tries.get(&id).and_then(|t| t.until)
                && let Some(left) = until.checked_duration_since(Instant::now())
            {
                return Err(locked_out(left));
            }
            st.stored.pin_hash.clone()
        };
        // Verified outside the lock (Argon2 takes a moment).
        let right = hash.is_some_and(|h| verify_secret(&h, pin));
        let mut st = lock(&self.state);
        if right {
            st.pin_tries.remove(&id);
            st.unlocks.insert(id, Instant::now());
            return Ok(self.cfg.unlock_idle);
        }
        let base = self.cfg.pin_lockout;
        let t = st.pin_tries.entry(id).or_default();
        t.wrong += 1;
        if t.wrong < PIN_TRIES {
            let left = PIN_TRIES - t.wrong;
            return Err(api_err(
                ErrorCode::Unauthorized,
                format!("wrong PIN ({left} more tries before a lockout)"),
            ));
        }
        let lockout = base
            .saturating_mul(1u32 << t.lockouts.min(16))
            .min(PIN_LOCKOUT_MAX);
        t.wrong = 0;
        t.lockouts += 1;
        t.until = Some(Instant::now() + lockout);
        Err(api_err(
            ErrorCode::Unauthorized,
            format!(
                "wrong PIN; too many wrong PINs, try again in {} s",
                lockout.as_secs_f64().ceil() as u64
            ),
        ))
    }

    /// `DELETE /api/v1/site/unlock`: locks a client again.
    pub fn lock_client(&self, actor: &Actor) {
        let Some(id) = &actor.client_id else { return };
        if lock(&self.state).unlocks.remove(id).is_some() {
            self.audit(actor, "lock", &json!({}), None);
        }
    }

    /// Ends the unlocks idle for too long (`siteChanged` `session`).
    pub(super) fn expire_unlocks(&self) {
        let idle = self.cfg.unlock_idle;
        let ended = {
            let mut st = lock(&self.state);
            let before = st.unlocks.len();
            st.unlocks.retain(|_, at| at.elapsed() < idle);
            st.unlocks.len() != before
        };
        if ended {
            self.changed([SitePart::Session]);
        }
    }

    /// Any site role (the audit, logs): a locked client with `access: pin` may read them.
    pub fn require_viewer(&self, actor: &Actor) -> Result<(), ApiError> {
        if actor.role.is_some() {
            Ok(())
        } else {
            Err(api_err(
                ErrorCode::Forbidden,
                "sign in as a site user first",
            ))
        }
    }

    /// The user to act as, with at least the role `least`. An admin action by an unlocked
    /// client starts its idle time again.
    pub fn require(&self, actor: &Actor, least: Role) -> Result<String, ApiError> {
        match (&actor.user, actor.role) {
            (Some(u), Some(r)) if r >= least => {
                if actor.unlocked.is_some()
                    && let Some(id) = &actor.client_id
                    && let Some(at) = lock(&self.state).unlocks.get_mut(id)
                {
                    *at = Instant::now();
                }
                Ok(u.clone())
            }
            (Some(_), Some(r)) => Err(api_err(
                ErrorCode::Forbidden,
                format!("a site {} may not do this", r.as_str()),
            )),
            _ if actor.access == SiteAccess::Pin => Err(api_err(
                ErrorCode::Forbidden,
                "the site is locked: unlock it with the admin PIN first",
            )),
            _ => Err(api_err(
                ErrorCode::Forbidden,
                "sign in as a site user first",
            )),
        }
    }

    // ---- the audit ------------------------------------------------------------------------

    /// Writes an entry in `site-audit.jsonl`.
    pub fn audit(&self, actor: &Actor, action: &str, detail: &Value, error: Option<&str>) {
        let line = json!({
            "at": unix_now(),
            "user": actor.user.clone().unwrap_or_default(),
            "peer": actor.peer,
            "client": actor.client,
            "action": action,
            "ok": error.is_none(),
            "error": error,
            "detail": idn_users::redact(detail),
        });
        let _l = lock(&self.audit_lock);
        let res = std::fs::create_dir_all(self.audit_file.parent().unwrap_or(Path::new(".")))
            .and_then(|_| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.audit_file)
            })
            .and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = res {
            tracing::error!("writing {}: {e}", self.audit_file.display());
        }
    }

    /// The newest `n` audit entries, newest first: this server's own, and the Site Manager's
    /// (the changes run through it, with the client and its address; machines; its HTTPS API).
    pub fn audit_log(&self, n: usize) -> Vec<SiteAuditEntry> {
        let mut out: Vec<(u64, SiteAuditEntry)> = Vec::new();
        let ours = {
            let _l = lock(&self.audit_lock);
            std::fs::read_to_string(&self.audit_file).unwrap_or_default()
        };
        let lines: Vec<&str> = ours.lines().collect();
        for l in lines.iter().rev().take(n) {
            if let Ok(v) = serde_json::from_str::<Value>(l) {
                out.push(audit_entry(&v, SiteAuditSource::Vesper));
            }
        }
        for v in self.users().audit_log(n) {
            out.push(audit_entry(&v, SiteAuditSource::SiteManager));
        }
        out.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        out.into_iter().take(n).map(|(_, e)| e).collect()
    }

    // ---- running changes ------------------------------------------------------------------

    /// Runs `cmd` as the actor through the Site Manager, for the actor's client and address
    /// (role checked there and here). The Site Manager audits what reaches it; this server
    /// audits what it refuses before.
    pub(super) fn run(&self, actor: &Actor, cmd: &SiteCommand) -> Result<Value, ApiError> {
        let action = cmd.path();
        let body = cmd.body();
        let checked = self
            .running()
            .and_then(|h| Ok((h, self.require(actor, idn_users::needs(&action, &body))?)));
        match checked {
            Ok((h, user)) => h.run_from(&user, &actor.caller(), cmd).map_err(from_idn),
            Err(e) => {
                self.audit(actor, &action, &body, Some(&e.message));
                Err(e)
            }
        }
    }

    // ---- status and settings --------------------------------------------------------------

    /// `GET /api/v1/site`.
    pub fn status(&self, actor: &Actor) -> SiteStatus {
        let h = self.handle();
        let st = lock(&self.state);
        let settings = self.settings_of(&st);
        let api = h.as_ref().and_then(|h| h.api());
        SiteStatus {
            enabled: st.stored.enabled,
            running: h.is_some(),
            error: if st.stored.enabled && h.is_none() {
                st.error.clone()
            } else {
                None
            },
            api_error: if h.is_some() && api.is_none() {
                st.api_error.clone()
            } else {
                None
            },
            settings_damaged: st.stored.damaged,
            join_mode: st.stored.join_mode,
            pairing_open: h.as_ref().is_some_and(|h| h.authority().pairing_open()),
            site_id: h.as_ref().map(|h| h.authority().site_id()),
            ca_fingerprint: h.as_ref().map(|h| h.authority().ca_fingerprint()),
            discovery_port: h.as_ref().map(|h| h.discovery_port()),
            api_port: api.as_ref().map(|a| a.0),
            api_certificate_sha256: api.map(|a| a.1),
            you: actor.session(),
            revision: settings.revision.unwrap_or_default(),
            membership: self.membership(),
        }
    }

    fn settings_of(&self, st: &State) -> SiteSettings {
        let mut s = SiteSettings {
            enabled: st.stored.enabled,
            join_mode: st.stored.join_mode,
            access: st.stored.access,
            pin: None,
            pin_set: st.stored.pin_hash.is_some(),
            revision: None,
        };
        // A new PIN is a new revision too.
        s.revision = Some(vesper_core::change::revision(&(&s, &st.stored.pin_hash)));
        s
    }

    /// `GET /api/v1/site/settings`.
    pub fn settings(&self) -> SiteSettings {
        self.settings_of(&lock(&self.state))
    }

    /// `PUT /api/v1/site/settings`: designate this server (start / stop the module), set the
    /// join mode and the access level (with the PIN).
    pub fn put_settings(
        &self,
        actor: &Actor,
        want: SiteSettings,
        if_match: Option<String>,
    ) -> Result<SiteSettings, (ApiError, Option<SiteSettings>)> {
        let current = self.settings();
        let detail = json!({
            "enabled": want.enabled, "joinMode": want.join_mode.as_str(),
            "access": want.access.as_str(), "newPin": want.pin.is_some(),
        });
        let fail = |e: ApiError, cur: Option<SiteSettings>| {
            self.audit(actor, "settings", &detail, Some(&e.message));
            Err((e, cur))
        };
        if let Err(e) = self.require(actor, Role::Admin) {
            return fail(e, None);
        }
        if let Some(m) = if_match.as_deref().filter(|m| *m != "*")
            && Some(m) != current.revision.as_deref()
        {
            return fail(
                api_err(
                    ErrorCode::Conflict,
                    "the site settings were changed by another operator; reload them or overwrite",
                ),
                Some(current),
            );
        }
        // The access level, checked before anything changes.
        if let Some(pin) = &want.pin {
            if want.access != SiteAccess::Pin {
                return fail(
                    api_err(ErrorCode::BadRequest, "a PIN goes with access \"pin\""),
                    None,
                );
            }
            if !valid_pin(pin) {
                return fail(
                    api_err(ErrorCode::BadRequest, "the PIN is 4 to 8 digits"),
                    None,
                );
            }
        }
        if want.access == SiteAccess::Pin && want.pin.is_none() && !current.pin_set {
            return fail(
                api_err(
                    ErrorCode::BadRequest,
                    "access \"pin\" needs a PIN: 4 to 8 digits (\"pin\")",
                ),
                None,
            );
        }
        if want.access == SiteAccess::Users
            && current.access != SiteAccess::Users
            && !has_admin_user(&self.users())
        {
            return fail(
                api_err(
                    ErrorCode::Conflict,
                    "access \"users\" needs an admin user: add a user with the role admin first",
                ),
                None,
            );
        }
        let new_hash = match want.pin.as_deref().map(hash_secret).transpose() {
            Ok(h) => h,
            Err(e) => return fail(e, None),
        };
        if want.enabled != current.enabled {
            let enrolled = self.enrolled_elsewhere();
            if want.enabled && enrolled {
                return fail(
                    api_err(
                        ErrorCode::Conflict,
                        "this server is a machine of another server's site, which has the site's one Site Manager: leave that site first",
                    ),
                    None,
                );
            }
            if !want.enabled && !enrolled {
                return fail(
                    api_err(
                        ErrorCode::Conflict,
                        "every server is in a site: this one runs its own until it joins another server's site, which turns this one off",
                    ),
                    None,
                );
            }
            {
                let mut st = lock(&self.state);
                st.stored.enabled = want.enabled;
                self.save(&st);
            }
            if want.enabled {
                if let Err(e) = self.start() {
                    return fail(
                        api_err(
                            ErrorCode::Unavailable,
                            format!("the Site Manager did not start: {e}"),
                        ),
                        None,
                    );
                }
            } else {
                self.stop();
            }
        } else if want.enabled && self.handle().is_none() && !self.enrolled_elsewhere() {
            // On, but it didn't start (a port in use): saving the settings tries again. The
            // rest of the change doesn't wait on it; the status says why when it fails.
            if let Err(e) = self.start() {
                tracing::warn!("the site's Site Manager did not start: {e}");
            }
        }
        if want.join_mode != self.settings().join_mode {
            if self.handle().is_none() {
                return fail(not_running(), None);
            }
            self.run(
                actor,
                &SiteCommand::Option {
                    scope: "site".into(),
                    target: None,
                    key: idn_trust::JOIN_MODE_KEY.into(),
                    value: want.join_mode.as_str().into(),
                },
            )
            .map_err(|e| (e, None))?;
            let mut st = lock(&self.state);
            st.stored.join_mode = want.join_mode;
            self.save(&st);
        }
        let access_changed = want.access != current.access || new_hash.is_some();
        // Set again after a damaged site.json (even to the same `users`): no longer read-only.
        let was_damaged = lock(&self.state).stored.damaged;
        if access_changed || was_damaged {
            let mut st = lock(&self.state);
            st.stored.damaged = false;
            st.stored.access = want.access;
            if want.access == SiteAccess::Pin {
                if let Some(h) = new_hash {
                    st.stored.pin_hash = Some(h);
                    // Clients unlocked with the old PIN lock (not the one setting it).
                    st.unlocks
                        .retain(|id, _| Some(id) == actor.client_id.as_ref());
                }
            } else {
                st.stored.pin_hash = None;
                st.unlocks.clear();
                st.pin_tries.clear();
            }
            if want.access != SiteAccess::Users {
                st.stored.sessions.clear();
            }
            self.save(&st);
        }
        // Lifting the damaged-file lockdown is a change of access too (E05-2), even when the
        // level and PIN stay the same.
        if want.enabled != current.enabled || access_changed || was_damaged {
            let mut detail = detail.clone();
            if was_damaged {
                detail["wasDamaged"] = json!(true);
            }
            self.audit(actor, "settings", &detail, None);
        }
        self.changed([SitePart::Status, SitePart::Session]);
        Ok(self.settings())
    }
}
