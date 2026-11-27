//! Machines joining this site: join requests, approval, join codes.

use super::*;

impl Site {
    /// `GET /api/v1/site/join-requests`.
    pub fn join_requests(&self) -> Vec<JoinRequestInfo> {
        let Some(h) = self.handle() else {
            return Vec::new();
        };
        h.authority()
            .pending()
            .into_iter()
            .map(|r| JoinRequestInfo {
                id: r.id,
                name: r.name,
                kind: r.kind,
                fingerprint: r.fingerprint,
                code: (!r.code.is_empty()).then(|| idn_trust::show_code(&r.code)),
                address: r.address.map(|a| a.to_string()),
                by_code: r.by_code,
                asked_at: rfc3339(r.asked_at),
            })
            .collect()
    }

    fn pending_request(&self, id: u64) -> Result<idn_trust::JoinRequest, ApiError> {
        let h = self.running()?;
        h.authority()
            .pending()
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| {
                api_err(
                    ErrorCode::NotFound,
                    format!("no join request {id} is waiting"),
                )
            })
    }

    fn pending_exists(&self, id: u64) -> Result<(), ApiError> {
        self.pending_request(id).map(|_| ())
    }

    /// Approves a join request.
    /// A Vesper box approved is marked a pairing relay ([`relays`]), by the name it asked
    /// with (`name` may rename it).
    pub fn approve(&self, actor: &Actor, id: u64, name: Option<String>) -> Result<(), ApiError> {
        let asked = self.pending_request(id)?;
        let vesper = relays::is_vesper_box(&asked.kind, &asked.name);
        if vesper {
            self.approving_vesper_box(&asked.fingerprint);
        }
        self.run(actor, &SiteCommand::Approve { id, name })?;
        self.changed([SitePart::JoinRequests]);
        if vesper {
            // Enrolled already (else at its `device_joined`).
            self.mark_vesper_relays("it joined the site");
        }
        Ok(())
    }

    /// Denies a join request.
    pub fn deny(&self, actor: &Actor, id: u64) -> Result<(), ApiError> {
        self.pending_exists(id)?;
        self.run(actor, &SiteCommand::Deny { id })?;
        self.changed([SitePart::JoinRequests]);
        Ok(())
    }

    /// Opens (`Some(seconds)`) or closes (`None`) the approval window.
    pub fn pairing(&self, actor: &Actor, open: Option<Option<u64>>) -> Result<(), ApiError> {
        let cmd = match open {
            Some(seconds) => SiteCommand::Pairing { seconds },
            None => SiteCommand::Close {},
        };
        self.run(actor, &cmd)?;
        self.tick();
        Ok(())
    }

    fn code_info(&self, c: &idn_trust::JoinCodeInfo, held: &BTreeMap<u64, String>) -> JoinCodeInfo {
        JoinCodeInfo {
            id: c.id,
            code: held.get(&c.id).map(|s| idn_trust::show_code(s)),
            expires_at: rfc3339(c.expires),
            expires_in_seconds: c.expires.saturating_sub(unix_now()),
            tries_left: c.tries_left,
            issuer: c.issuer.clone(),
        }
    }

    /// `GET /api/v1/site/join-codes`: the codes waiting, oldest first.
    pub fn join_codes(&self) -> Vec<JoinCodeInfo> {
        let Some(h) = self.handle() else {
            return Vec::new();
        };
        let codes = h.authority().join_codes();
        let held = lock(&self.state).held_codes.clone();
        codes.iter().map(|c| self.code_info(c, &held)).collect()
    }

    /// `GET /api/v1/site/join-code`: the site code, the newest code issued here and still
    /// waiting.
    pub fn current_code(&self) -> Option<JoinCodeInfo> {
        self.join_codes()
            .into_iter()
            .filter(|c| c.code.is_some())
            .max_by_key(|c| c.id)
    }

    /// `POST /api/v1/site/join-code`: a new site code. Earlier codes issued here are taken
    /// back, so one code is shown at a time.
    pub fn new_code(&self, actor: &Actor) -> Result<JoinCodeInfo, ApiError> {
        let v = self.run(actor, &SiteCommand::JoinCode { cancel: None })?;
        let (Some(id), Some(code)) = (
            v.get("id").and_then(Value::as_u64),
            v.get("code").and_then(Value::as_str),
        ) else {
            return Err(api_err(
                ErrorCode::Internal,
                format!("unexpected answer from the Site Manager: {v}"),
            ));
        };
        let older: Vec<u64> = {
            let mut st = lock(&self.state);
            let older = st.held_codes.keys().copied().collect();
            st.held_codes.insert(id, code.to_string());
            older
        };
        let mut issued = events::ours(
            protocol::SiteEventName::JoinCodeIssued,
            format!("join code {id} issued"),
        );
        issued.code_id = Some(id);
        issued.detail = json!({"issuer": actor.user, "expiresAt": rfc3339(v.get("expires").and_then(Value::as_u64).unwrap_or(0))});
        self.emit(vec![issued], [SitePart::JoinCodes]);
        let h = self.running()?;
        let waiting: Vec<u64> = h.authority().join_codes().iter().map(|c| c.id).collect();
        for old in older.into_iter().filter(|o| waiting.contains(o)) {
            // Best effort: it may be used or burned meanwhile.
            let _ = self.cancel_code(actor, old);
        }
        let held = lock(&self.state).held_codes.clone();
        let info = h
            .authority()
            .join_codes()
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| self.code_info(&c, &held))
            .unwrap_or_else(|| JoinCodeInfo {
                id,
                code: Some(idn_trust::show_code(code)),
                expires_at: rfc3339(v.get("expires").and_then(Value::as_u64).unwrap_or(0)),
                expires_in_seconds: v
                    .get("expires")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .saturating_sub(unix_now()),
                tries_left: 0,
                issuer: actor.user.clone().unwrap_or_default(),
            });
        Ok(info)
    }

    /// Takes the waiting join code `id` back.
    pub fn cancel_code(&self, actor: &Actor, id: u64) -> Result<(), ApiError> {
        self.run(actor, &SiteCommand::JoinCode { cancel: Some(id) })?;
        lock(&self.state).held_codes.remove(&id);
        let mut e = events::ours(
            protocol::SiteEventName::JoinCodeCancelled,
            format!("join code {id} taken back"),
        );
        e.code_id = Some(id);
        self.emit(vec![e], [SitePart::JoinCodes]);
        Ok(())
    }
}
