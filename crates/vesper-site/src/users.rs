//! Site users, the site password and tokens for the IDN engines.

use super::*;

impl Site {
    /// `GET /api/v1/site/users`.
    pub fn user_list(&self) -> Vec<SiteUser> {
        self.users()
            .list()
            .into_iter()
            .map(|(name, role)| user_entry(&name, role))
            .collect()
    }

    fn check_user_revision(
        &self,
        name: &str,
        if_match: &Option<String>,
    ) -> Result<Option<SiteUser>, (ApiError, Option<SiteUser>)> {
        let current = self.users().role(name).map(|r| user_entry(name, r));
        if let Some(m) = if_match.as_deref().filter(|m| *m != "*")
            && current.as_ref().map(|c| c.revision.as_str()) != Some(m)
        {
            return Err((
                api_err(
                    ErrorCode::Conflict,
                    format!("user {name} was changed by another operator; reload or overwrite"),
                ),
                current,
            ));
        }
        Ok(current)
    }

    /// `PUT /api/v1/site/users/{name}`: adds or changes a user. A changed role or password
    /// ends their engine tokens.
    pub fn put_user(
        &self,
        actor: &Actor,
        name: &str,
        role: Option<SiteRole>,
        password: Option<String>,
        if_match: Option<String>,
    ) -> Result<SiteUser, (ApiError, Option<SiteUser>)> {
        // The name as the Site Manager takes it (trimmed): the checks here are about that user.
        let name = name.trim();
        // The rest of the name and password rules are the Site Manager's.
        if name == idn_users::ADMIN {
            return Err((
                api_err(
                    ErrorCode::BadRequest,
                    "\"admin\" is the site's built-in administrator: its password is the site password",
                ),
                None,
            ));
        }
        let before = self.check_user_revision(name, &if_match)?;
        if before.as_ref().is_some_and(|b| b.role == SiteRole::Admin)
            && role.is_some_and(|r| r != SiteRole::Admin)
        {
            self.keep_an_admin(name).map_err(|e| (e, None))?;
        }
        if before.is_none() && password.is_none() {
            return Err((
                api_err(ErrorCode::BadRequest, "a new user needs a password"),
                None,
            ));
        }
        let h = self.running().map_err(|e| (e, None))?;
        let new_password = password.is_some();
        self.run(
            actor,
            &SiteCommand::User {
                action: "set".into(),
                name: name.into(),
                role: role.map(|r| idn_role(r).as_str().to_string()),
                password,
            },
        )
        .map_err(|e| (e, None))?;
        let after = self
            .users()
            .role(name)
            .map(|r| user_entry(name, r))
            .ok_or_else(|| (api_err(ErrorCode::Internal, "the user wasn't kept"), None))?;
        if before.is_some() {
            // A role or password change: their tokens end (the Site Manager does it for its
            // own; this covers the ones issued through the host handle).
            if let Err(e) = h.withdraw_tokens(name) {
                tracing::warn!("withdrawing {name}'s tokens: {e}");
            }
        }
        if before.is_some() && new_password {
            // A new password (perhaps because the old one leaked) ends the user's sign-ins on
            // other clients (owner, 2026-11-24); the client that set it, signed in as them,
            // stays signed in.
            let own = actor
                .client_id
                .as_deref()
                .filter(|_| actor.user.as_deref() == Some(name));
            let ended = {
                let mut st = lock(&self.state);
                let n = st.stored.sessions.len();
                st.stored
                    .sessions
                    .retain(|c, u| u != name || Some(c.as_str()) == own);
                let ended = n - st.stored.sessions.len();
                if ended > 0 {
                    self.save(&st);
                }
                ended
            };
            if ended > 0 {
                tracing::info!(user = %name, ended, "a new password: the user's other sign-ins ended");
                self.changed([SitePart::Users, SitePart::Session]);
                return Ok(after);
            }
        }
        self.changed([SitePart::Users]);
        Ok(after)
    }

    /// With `access: users`, the last admin user stays an admin.
    fn keep_an_admin(&self, name: &str) -> Result<(), ApiError> {
        let others = self
            .users()
            .list()
            .iter()
            .any(|(n, r)| *r == Role::Admin && n != name);
        if lock(&self.state).stored.access == SiteAccess::Users && !others {
            return Err(api_err(
                ErrorCode::Conflict,
                format!(
                    "{name} is the site's last admin user, and the site's access is \"users\": add another admin, or change the access first"
                ),
            ));
        }
        Ok(())
    }

    /// `DELETE /api/v1/site/users/{name}`: removes a user, their sign-ins and their tokens.
    pub fn delete_user(
        &self,
        actor: &Actor,
        name: &str,
        if_match: Option<String>,
    ) -> Result<(), (ApiError, Option<SiteUser>)> {
        let name = name.trim();
        let Some(before) = self.check_user_revision(name, &if_match)? else {
            return Err((
                api_err(ErrorCode::NotFound, format!("no user {name}")),
                None,
            ));
        };
        if before.role == SiteRole::Admin {
            self.keep_an_admin(name).map_err(|e| (e, None))?;
        }
        let h = self.running().map_err(|e| (e, None))?;
        self.run(
            actor,
            &SiteCommand::User {
                action: "remove".into(),
                name: name.into(),
                role: None,
                password: None,
            },
        )
        .map_err(|e| (e, None))?;
        if let Err(e) = h.withdraw_tokens(name) {
            tracing::warn!("withdrawing {name}'s tokens: {e}");
        }
        {
            let mut st = lock(&self.state);
            st.stored.sessions.retain(|_, u| u != name);
            self.save(&st);
        }
        self.changed([SitePart::Users]);
        Ok(())
    }

    /// `PUT /api/v1/site/password`: the site password (the built-in administrator's; it
    /// also opens the Site Manager's own HTTPS API).
    pub fn set_password(&self, actor: &Actor, password: &str) -> Result<(), ApiError> {
        let r = (|| {
            self.require(actor, Role::Admin)?;
            let h = self.running()?;
            h.set_password(password)
                .map_err(|e| api_err(ErrorCode::BadRequest, e.to_string()))
        })();
        self.audit(
            actor,
            "password",
            &json!({"password": password}),
            r.as_ref().err().map(|e: &ApiError| e.message.as_str()),
        );
        r
    }

    /// `POST /api/v1/site/tokens`: a token for an IDN engine, for the user the actor's tokens
    /// are issued to ([`Actor::token_user`]), with a scope its role may have ([`scope_role`]).
    /// The Site Manager audits it; this server audits a request it refuses before.
    pub fn issue_token(
        &self,
        actor: &Actor,
        req: &protocol::IssueToken,
    ) -> Result<protocol::IssuedToken, ApiError> {
        let checked = (|| -> Result<_, ApiError> {
            // Such a token would be taken as a paired client's key ([`Site::verify`]), with
            // no key to revoke: a client could keep itself in past its key's revocation.
            if req.engine == app_audience() {
                return Err(api_err(
                    ErrorCode::BadRequest,
                    format!(
                        "engine: {} is this server's own API: a client pairs with the server for its key",
                        app_audience()
                    ),
                ));
            }
            let least = scope_role(&req.scope).ok_or_else(|| {
                api_err(
                    ErrorCode::BadRequest,
                    "scope: read, control, bus:<bus> or iem:<mix>",
                )
            })?;
            self.require(actor, least)?;
            let user = actor.token_user().ok_or_else(|| {
                api_err(ErrorCode::Forbidden, "no site user to issue a token for")
            })?;
            Ok((self.running()?, user))
        })();
        let (h, user) = match checked {
            Ok(x) => x,
            Err(e) => {
                let detail = json!({
                    "engine": req.engine, "machine": req.machine, "scope": req.scope,
                    "ttlSeconds": req.ttl_seconds,
                });
                self.audit(actor, "token", &detail, Some(&e.message));
                return Err(e);
            }
        };
        let aud = TokenAud {
            engine: req.engine.clone(),
            machine: req.machine.clone(),
        };
        let v = h
            .issue_token_from(
                &user,
                &actor.caller(),
                &aud,
                &req.scope,
                req.ttl_seconds.map(Duration::from_secs),
            )
            .map_err(from_idn)?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        Ok(protocol::IssuedToken {
            token: s("token"),
            id: s("jti"),
            user,
            engine: v["aud"]["engine"]
                .as_str()
                .unwrap_or(&req.engine)
                .to_string(),
            machine: v["aud"]["machine"].as_str().unwrap_or("*").to_string(),
            scope: s("scope"),
            issued_at: rfc3339(n("iat")),
            expires_at: rfc3339(n("exp")),
        })
    }

    /// `DELETE /api/v1/site/tokens/{user}`: ends every engine token issued to `user` so far
    /// (a site user, or a client's own `vesper:<client id>`): admins, or the user or client
    /// themselves.
    pub fn withdraw_tokens(&self, actor: &Actor, user: &str) -> Result<(), ApiError> {
        let r = (|| {
            let own = actor.token_user().as_deref() == Some(user)
                && (actor.signed_in || user.starts_with(CLIENT_TOKEN_PREFIX));
            if !own {
                self.require(actor, Role::Admin)?;
            }
            let h = self.running()?;
            h.withdraw_tokens(user).map_err(from_idn)
        })();
        self.audit(
            actor,
            "tokens/withdraw",
            &json!({"user": user}),
            r.as_ref().err().map(|e: &ApiError| e.message.as_str()),
        );
        r
    }
}
