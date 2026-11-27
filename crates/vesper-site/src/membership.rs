//! Membership of another site: joining it, leaving it, following it.

use super::*;

impl Site {
    pub(super) fn member(&self) -> Option<Arc<Member>> {
        self.member
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// This server's membership of another server's site, once enrolled there (its channel
    /// to the site's Codex, the server's `codex::mirror`).
    pub fn member_enrollment(&self) -> Option<Arc<Enrollment>> {
        self.member()
            .filter(|m| m.enrollment.creds().is_some())
            .map(|m| m.enrollment.clone())
    }

    /// This server is in or joining another server's site (enrolled there or not yet).
    pub fn has_member(&self) -> bool {
        self.member().is_some()
    }

    /// This server is a machine of another server's site (enrolled there).
    pub fn enrolled_elsewhere(&self) -> bool {
        self.member()
            .is_some_and(|m| m.enrollment.creds().is_some())
    }

    pub(super) fn start_member(&self, server: &str) -> std::io::Result<Arc<Member>> {
        let enrollment = Enrollment::start(EnrollmentConfig {
            discovery_server: server.to_string(),
            state_dir: self.member_dir.clone(),
            name: relays::member_name(&self.name),
            role: DeviceRole::App,
            passphrase: None,
            admin_code: None,
        })?;
        let check = TokenCheck::from_enrollment(&app_audience(), enrollment.clone());
        Ok(Arc::new(Member {
            server: server.to_string(),
            enrollment,
            check,
        }))
    }

    /// Ends the membership here (the site still lists this machine until it is removed
    /// there) and forgets this machine's identity in it.
    fn drop_member(&self) {
        let m = self
            .member
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        drop(m);
        if let Err(e) = std::fs::remove_dir_all(&self.member_dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("removing {}: {e}", self.member_dir.display());
        }
        let mut st = lock(&self.state);
        st.stored.member = None;
        self.save(&st);
    }

    /// `GET /api/v1/site/membership`.
    pub fn membership(&self) -> SiteMembership {
        let Some(m) = self.member() else {
            return SiteMembership::default();
        };
        let e = &m.enrollment;
        let mut out = SiteMembership {
            server: Some(m.server.clone()),
            site_id: e.ca_id(),
            fingerprint: Some(e.fingerprint()),
            ..SiteMembership::default()
        };
        let state = e.state();
        out.state = match state {
            // In the site (also while its server isn't reachable yet).
            TrustState::Enrolled { connected } => {
                out.connected = connected;
                MembershipState::Enrolled
            }
            _ if e.creds().is_some() && state != TrustState::Revoked => MembershipState::Enrolled,
            TrustState::Starting => MembershipState::Starting,
            TrustState::NoAuthority => MembershipState::NoSite,
            TrustState::WaitingForPairing => MembershipState::WaitingForWindow,
            TrustState::Pending { code } | TrustState::ConfirmCode { code } => {
                out.code = Some(idn_trust::show_code(&code));
                MembershipState::WaitingForApproval
            }
            TrustState::CodeNeeded => MembershipState::CodeNeeded,
            TrustState::CodeAccepted => MembershipState::CodeAccepted,
            TrustState::WrongCode { left } => {
                out.tries_left = Some(left);
                MembershipState::WrongCode
            }
            TrustState::CodeExpired => MembershipState::CodeExpired,
            TrustState::Denied => MembershipState::Denied,
            TrustState::Revoked => MembershipState::Removed,
            TrustState::Error(why) => {
                out.error = Some(why);
                MembershipState::Error
            }
        };
        out
    }

    /// `POST /api/v1/site/join` (admins): joins another server's site as one of its
    /// machines, or types a code into a join under way. Refused while this server's own site
    /// has other machines (they'd be left without a site).
    pub fn join(
        &self,
        actor: &Actor,
        req: &protocol::JoinSite,
    ) -> Result<SiteMembership, ApiError> {
        let detail = json!({"server": req.server, "code": req.code.is_some()});
        let r = self.try_join(actor, req);
        self.audit(
            actor,
            "join",
            &detail,
            r.as_ref().err().map(|e: &ApiError| e.message.as_str()),
        );
        r
    }

    fn try_join(
        &self,
        actor: &Actor,
        req: &protocol::JoinSite,
    ) -> Result<SiteMembership, ApiError> {
        self.require(actor, Role::Admin)?;
        let _l = lock(&self.membership_lock);
        let server = req
            .server
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let code = req.code.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if code.is_some_and(|c| idn_trust::normalize_join_code(c).is_none()) {
            return Err(api_err(ErrorCode::BadRequest, "a join code is six digits"));
        }
        let typed = |e: &Enrollment, c: &str| {
            e.join_with_code(c)
                .map_err(|e| api_err(ErrorCode::BadRequest, e.to_string()))
        };
        if let Some(m) = self.member() {
            if m.enrollment.creds().is_some() {
                return Err(api_err(
                    ErrorCode::Conflict,
                    format!(
                        "this server is already a machine of the site at {}: leave it first",
                        m.server
                    ),
                ));
            }
            if server.is_none_or(|s| s == m.server) {
                match code {
                    Some(c) => typed(&m.enrollment, c)?,
                    None => m.enrollment.retry_now(),
                }
                self.changed([SitePart::Membership]);
                return Ok(self.membership());
            }
            // Another site: the join under way gives way.
            self.drop_member();
        }
        let server = server.ok_or_else(|| {
            api_err(
                ErrorCode::BadRequest,
                "server: the site server's address (host, or host:port; port 5959 by default)",
            )
        })?;
        if let Some(h) = self.handle() {
            let own = lock(&self.own_services).clone();
            let others: Vec<String> = h
                .authority()
                .devices()
                .into_iter()
                .filter(|d| !d.revoked && !own.contains(&d.fingerprint))
                .map(|d| d.name)
                .collect();
            if !others.is_empty() {
                return Err(api_err(
                    ErrorCode::Conflict,
                    format!(
                        "this server's own site still has {} other machine{} ({}): remove {} from the site or move {} to the other site first, or {} be left without a site",
                        others.len(),
                        if others.len() == 1 { "" } else { "s" },
                        others.join(", "),
                        if others.len() == 1 { "it" } else { "them" },
                        if others.len() == 1 { "it" } else { "them" },
                        if others.len() == 1 {
                            "it would"
                        } else {
                            "they would"
                        },
                    ),
                ));
            }
        }
        let m = self
            .start_member(server)
            .map_err(|e| api_err(ErrorCode::BadRequest, format!("{server}: {e}")))?;
        if let Some(c) = code {
            typed(&m.enrollment, c)?;
        }
        *self.member.write().unwrap_or_else(|e| e.into_inner()) = Some(m);
        {
            let mut st = lock(&self.state);
            st.stored.member = Some(server.to_string());
            self.save(&st);
        }
        tracing::info!(%server, "joining the site at {server}");
        self.changed([SitePart::Membership, SitePart::Status]);
        Ok(self.membership())
    }

    /// `POST /api/v1/site/leave` (admins): leaves the other server's site (or stops joining
    /// it); this server runs a site of its own again.
    pub fn leave(&self, actor: &Actor) -> Result<(), ApiError> {
        let r = (|| {
            self.require(actor, Role::Admin)?;
            let _l = lock(&self.membership_lock);
            let Some(m) = self.member() else {
                return Err(api_err(
                    ErrorCode::Conflict,
                    "this server isn't in or joining another server's site",
                ));
            };
            tracing::info!(server = %m.server, "leaving the site at {}", m.server);
            // The site revokes this machine and lists it no more (it may join again). Only
            // a connected membership can tell it (IDN says nothing when the message can't
            // go): otherwise the site lists this server until it is removed there.
            let connected = matches!(
                m.enrollment.state(),
                TrustState::Enrolled { connected: true }
            );
            if !connected {
                tracing::warn!(server = %m.server, "the site's server can't be reached: it lists this server until it is removed there");
            }
            if let Err(e) = m.enrollment.tell_leaving() {
                tracing::warn!("telling the site at {} this server leaves: {e}", m.server);
            }
            drop(m);
            self.drop_member();
            self.own_site_again();
            Ok(())
        })();
        self.audit(
            actor,
            "leave",
            &json!({}),
            r.as_ref().err().map(|e: &ApiError| e.message.as_str()),
        );
        self.changed([SitePart::Membership, SitePart::Status]);
        r
    }

    /// Runs this server's own site again (after leaving another, or being removed from it).
    fn own_site_again(&self) {
        {
            let mut st = lock(&self.state);
            st.stored.enabled = true;
            self.save(&st);
        }
        if let Err(e) = self.start() {
            tracing::warn!("the site's Site Manager did not start: {e}");
        }
    }

    /// Follows the membership (every tick): once this server is in another site, its own
    /// Site Manager stops (one per site); removed from that site, it forgets that site (its
    /// enrollment would keep asking to join, and could pull it back in by an admin's click
    /// there, whatever its own site gained meanwhile: joining again is a fresh join) and runs
    /// its own again. `siteChanged` `membership` when where it stands changed.
    pub(super) fn follow_membership(&self) {
        let _l = lock(&self.membership_lock);
        if self.closing.load(Ordering::SeqCst) {
            return;
        }
        let m = self.member();
        let enrolled = m.as_ref().is_some_and(|m| m.enrollment.creds().is_some());
        let enabled = lock(&self.state).stored.enabled;
        if let Some(m) = &m
            && enrolled
        {
            if let Some(h) = self.handle()
                && m.enrollment.ca_id().as_deref() == Some(h.authority().site_id().as_str())
            {
                // Its own site's address: nothing to join.
                tracing::warn!(server = %m.server, "that is this server's own site; the join is dropped");
                drop(h);
                self.drop_member();
                self.changed([SitePart::Membership, SitePart::Status]);
                return;
            }
            if enabled || self.handle().is_some() {
                tracing::info!(server = %m.server, "this server is now a machine of the site at {}: its own Site Manager stops (one per site)", m.server);
                {
                    let mut st = lock(&self.state);
                    st.stored.enabled = false;
                    self.save(&st);
                }
                self.stop();
                self.changed([SitePart::Status]);
            }
        } else if let Some(m) = &m
            && m.enrollment.state() == TrustState::Revoked
        {
            tracing::warn!(server = %m.server, "this server was removed from the site at {}: it forgets that site (joining it again is a new join) and runs its own site again", m.server);
            self.drop_member();
            if !enabled {
                self.own_site_again();
            }
            self.changed([SitePart::Membership, SitePart::Status]);
        } else if !enrolled && !enabled {
            tracing::warn!("this server is no longer in another site: it runs its own site again");
            self.own_site_again();
            self.changed([SitePart::Status]);
        }
        let now = self.membership().state;
        let was = std::mem::replace(&mut lock(&self.state).membership_seen, now);
        if was != now {
            self.changed([SitePart::Membership, SitePart::Status]);
        }
    }
}
