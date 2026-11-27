//! Paired clients' site API keys.

use super::*;

impl Site {
    /// Checks a key or token a client shows, locally: on the Site Manager's box with its
    /// handle, on the site's other boxes with their membership (whose site pushes every
    /// withdrawal and key revocation to them, so they hold offline). Taken: a site-signed
    /// key or token for `app:vesper` with the scope `control`. The client it is.
    pub fn verify(&self, token: &str) -> Option<Client> {
        let mut why = Vec::new();
        let mut claims = None;
        if let Some(h) = self.handle() {
            // A named key's use is noted here by IDN itself (its "last used"); the other boxes
            // report theirs through `TokenCheck::check` (`Enrollment::report_key_use`).
            match h.verify_token(token, APP) {
                Ok(c) => claims = Some(c),
                Err(e) => why.push(e),
            }
        }
        if claims.is_none()
            && let Some(m) = self.member()
        {
            match m.check.check(token) {
                Ok(c) => claims = Some(c),
                Err(e) => why.push(e.to_string()),
            }
        }
        let Some(c) = claims else {
            tracing::debug!(why = ?why, "a key was refused");
            return None;
        };
        if c.scope != Scope::Control {
            tracing::debug!(sub = %c.sub, scope = %c.scope, "a key without the control scope was refused");
            return None;
        }
        Some(Client {
            id: c.sub.strip_prefix("key:").unwrap_or(&c.sub).to_string(),
            name: if c.name.is_empty() {
                c.sub.clone()
            } else {
                c.name.clone()
            },
            credential: Credential::Key,
        })
    }

    /// A paired client's key, made here at the site's server: a named site API key for
    /// `app:vesper` on every machine of the site, scope `control`, living as long as IDN allows
    /// (10 years; a browser's a year), made by the Site Manager as the site's administrator for
    /// `caller` (in its audit). `(key, id)`. (At another server of the site a pairing is
    /// relayed and approved instead: [`pairing`].)
    pub fn create_client_key(
        &self,
        name: &str,
        caller: &host::Caller,
        browser: bool,
    ) -> Result<(String, String), ApiError> {
        if let Some(h) = self.handle() {
            let v = h
                .run_from(
                    idn_users::ADMIN,
                    caller,
                    &SiteCommand::ApiKey {
                        name: name.to_string(),
                        aud: TokenAud {
                            engine: app_audience(),
                            machine: None,
                        },
                        scope: "control".into(),
                        days: pairing::key_days(browser),
                    },
                )
                .map_err(from_idn)?;
            return match (v["key"].as_str(), v["id"].as_str()) {
                (Some(k), Some(id)) => {
                    self.record_pairing_key(id);
                    Ok((k.to_string(), id.to_string()))
                }
                _ => Err(api_err(
                    ErrorCode::Internal,
                    format!("unexpected answer from the Site Manager: {}", v["ok"]),
                )),
            };
        }
        if let Some(m) = self.member().filter(|m| m.enrollment.creds().is_some()) {
            // A key without anyone's approval is made at the site's server only; relayed
            // pairings are answered there.
            return Err(api_err(
                ErrorCode::NotImplemented,
                format!(
                    "keys are made at the site's server ({}); at this machine of the site a pairing is relayed there",
                    m.server
                ),
            ));
        }
        Err(api_err(
            ErrorCode::Unavailable,
            "this server's Site Manager isn't running, and it isn't in another site: pairing needs a site",
        ))
    }

    /// `DELETE /api/v1/site/api-keys/{id}`: revokes a named key; it stops at once on every
    /// machine of the site (admins).
    pub fn revoke_key(&self, actor: &Actor, id: &str) -> Result<(), ApiError> {
        self.run(actor, &SiteCommand::ApiKeyRevoke { id: id.into() })
            .map(|_| ())
    }

    /// Revokes a named key as the server machine (a key handed over that was never used).
    pub fn revoke_key_as_server(&self, id: &str) -> Result<(), ApiError> {
        let h = self.running()?;
        h.run_from(
            idn_users::ADMIN,
            &host::Caller {
                address: String::new(),
                client: SERVER_MACHINE.into(),
            },
            &SiteCommand::ApiKeyRevoke { id: id.into() },
        )
        .map(|_| ())
        .map_err(from_idn)
    }

    /// A client swapped its legacy credential `old` for the key `new`: its sign-in and its
    /// PIN unlock go with it.
    pub fn rebind_client(&self, old: &str, new: &str) {
        let mut st = lock(&self.state);
        if let Some(t) = st.unlocks.remove(old) {
            st.unlocks.insert(new.to_string(), t);
        }
        if let Some(u) = st.stored.sessions.remove(old) {
            st.stored.sessions.insert(new.to_string(), u);
            self.save(&st);
        }
    }

    /// A service of this box itself joined its site (the Codex): its device isn't one of the
    /// site's other machines (this box may still join another site).
    pub fn note_own_service(&self, fingerprint: &str) {
        lock(&self.own_services).insert(fingerprint.to_string());
    }
}
