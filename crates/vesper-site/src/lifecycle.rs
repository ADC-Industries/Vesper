//! Starting and stopping the Site Manager, and taking its events.

use super::*;

impl Site {
    /// At the server's start: the membership of another site (when joining or in one), and
    /// this server's own Site Manager unless it is in another site. A failure (a port in
    /// use) is logged and shown in the status; the server runs on without it.
    pub fn start_up(&self) {
        let (member, enabled) = {
            let st = lock(&self.state);
            (st.stored.member.clone(), st.stored.enabled)
        };
        if let Some(server) = member {
            match self.start_member(&server) {
                Ok(m) => {
                    let enrolled = m.enrollment.creds().is_some();
                    *self.member.write().unwrap_or_else(|e| e.into_inner()) = Some(m);
                    if enrolled {
                        tracing::info!(%server, "this server is a machine of the site at {server}");
                    }
                }
                Err(e) => tracing::warn!("rejoining the site at {server}: {e}"),
            }
        }
        if enabled && let Err(e) = self.start() {
            tracing::warn!("the site's Site Manager did not start: {e}");
        }
    }

    pub(super) fn start(&self) -> Result<(), String> {
        let _l = lock(&self.lifecycle);
        if self.handle().is_some() {
            return Ok(());
        }
        if !self.cfg.available {
            let e = "turned off on the command line (--no-site-manager)".to_string();
            lock(&self.state).error = Some(e.clone());
            return Err(e);
        }
        let cfg = host::Config {
            dir: self.idn_dir.clone(),
            passphrase: None,
            discovery_port: self.cfg.discovery_port,
            api_port: Some(self.cfg.api_port),
            announce: self.cfg.announce,
            open_new_site: self.cfg.open_new_site,
            password: None,
            restored: false,
            host: None,
            // As before: whether Vesper runs IDN Voice is the owner's decision still.
            voice: true,
        };
        let handle = match host::start(
            cfg,
            |line| tracing::info!(target: "vesper_server::site", "{line}"),
        ) {
            Ok(h) => Arc::new(h),
            Err(e) => {
                let e = format!(
                    "{e} (discovery port {}, API port {})",
                    self.cfg.discovery_port, self.cfg.api_port
                );
                let mut st = lock(&self.state);
                st.error = Some(e.clone());
                st.start_failed = Some(Instant::now());
                return Err(e);
            }
        };
        let first = !lock(&self.state).stored.initialized;
        // Done once the join mode is set: that needs the API, so a first start without it
        // sets it at the next start.
        let mut initialized = !first;
        if first {
            // A new site joins by code or approval (the owner's default).
            let mode = JoinMode::default();
            let set = SiteCommand::Option {
                scope: "site".into(),
                target: None,
                key: idn_trust::JOIN_MODE_KEY.into(),
                value: mode.as_str().into(),
            };
            let me = host::Caller {
                address: String::new(),
                client: SERVER_MACHINE.into(),
            };
            match handle.run_from(idn_users::ADMIN, &me, &set) {
                Ok(_) => initialized = true,
                Err(e) => tracing::warn!("setting the new site's join mode: {e}"),
            }
        }
        let a = handle.authority();
        // It runs without its API (the admin panel) when that port can't be bound; no other
        // port is tried. Shown in the status until a start with the API.
        let api_error = handle.api().is_none().then(|| match handle.api_error() {
            Some(why) => format!("{why} (API port {})", self.cfg.api_port),
            None => format!("the admin panel port {} is in use", self.cfg.api_port),
        });
        {
            let mut st = lock(&self.state);
            st.stored.initialized = initialized;
            st.stored.join_mode = our_mode(a.join_mode());
            st.pairing_open = a.pairing_open();
            st.error = None;
            st.api_error = api_error.clone();
            st.start_failed = None;
            self.save(&st);
        }
        if let Some((port, sha)) = handle.api() {
            tracing::info!(
                discovery_port = handle.discovery_port(),
                api_port = port,
                certificate = %sha,
                site = %a.site_id(),
                "the site's Site Manager is running"
            );
        } else if let Some(why) = &api_error {
            tracing::warn!(
                discovery_port = handle.discovery_port(),
                site = %a.site_id(),
                "the site's Site Manager is running without its API: {why}"
            );
        }
        // The events from now on (what happened before is in the state clients fetch).
        let since = handle.events(0).next;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (me, h, stop, wait) = (
                self.me.clone(),
                handle.clone(),
                stop.clone(),
                self.cfg.event_wait,
            );
            std::thread::Builder::new()
                .name("site-events".into())
                .spawn(move || event_thread(me, h, stop, since, wait))
                .map_err(|e| e.to_string())?
        };
        *lock(&self.pump) = Some((stop, thread));
        *self.handle.write().unwrap_or_else(|e| e.into_inner()) = Some(handle);
        self.changed([
            SitePart::Status,
            SitePart::Devices,
            SitePart::JoinRequests,
            SitePart::JoinCodes,
        ]);
        // An existing site (since IDN Beta11): its Vesper boxes not marked yet relay pairings.
        self.mark_vesper_relays("at the site server's start");
        Ok(())
    }

    /// The server stops: the Site Manager, and the membership of another site (kept for the
    /// next start).
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let _l = lock(&self.membership_lock);
        self.stop();
        let m = self
            .member
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        drop(m);
    }

    /// Stops the Site Manager (when running) and writes the last-seen times.
    pub fn stop(&self) {
        let _l = lock(&self.lifecycle);
        let h = self
            .handle
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some((stop, thread)) = lock(&self.pump).take() {
            // It notices within `event_wait`.
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
        }
        if let Some(h) = &h {
            let now = unix_now();
            let mut st = lock(&self.state);
            for d in h.device_health().iter().filter(|d| d.online) {
                st.stored.last_seen.insert(d.fingerprint.clone(), now);
            }
            self.save(&st);
        }
        if let Some(h) = h {
            // A request may still hold it for a moment.
            let mut h = h;
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match Arc::try_unwrap(h) {
                    Ok(h) => {
                        h.stop();
                        break;
                    }
                    Err(still) if Instant::now() < deadline => {
                        h = still;
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(still) => {
                        drop(still); // stops when the last request holding it ends
                        break;
                    }
                }
            }
            tracing::info!("the site's Site Manager stopped");
            self.changed([SitePart::Status]);
        }
    }

    // ---- events ---------------------------------------------------------------------------

    /// Sends events of the site (kept for `GET /api/v1/site/events`, logged) and
    /// `siteChanged` for `parts`.
    pub(super) fn emit(&self, events: Vec<SiteEvent>, parts: impl IntoIterator<Item = SitePart>) {
        {
            let mut st = lock(&self.state);
            for e in &events {
                if st.recent.len() == RECENT_EVENTS {
                    st.recent.pop_front();
                }
                st.recent.push_back(e.clone());
            }
        }
        for e in events {
            tracing::info!(target: "vesper_server::site", event = %e.name, "{}", e.text);
            self.send(ServerMessage::SiteEvent { event: e });
        }
        self.changed(parts);
    }

    /// Takes in a batch of the Site Manager's events (from the event thread).
    pub(super) fn take_events(&self, batch: host::SiteEvents) {
        let mut parts = BTreeSet::new();
        if batch.missed {
            // Some went by unseen: clients read the whole state again.
            parts.extend([
                SitePart::Status,
                SitePart::JoinRequests,
                SitePart::JoinCodes,
                SitePart::Devices,
            ]);
        }
        let now = unix_now();
        let mut out = Vec::new();
        let mut seen = false;
        let coded = batch
            .events
            .iter()
            .any(|e| events::coded_key_request(e).is_some());
        let joined = batch.missed || batch.events.iter().any(events::joined);
        {
            let mut st = lock(&self.state);
            for entry in &batch.events {
                let (e, p) = events::convert(entry);
                parts.extend(p);
                if let Some(id) = events::code_ended(entry) {
                    st.held_codes.remove(&id);
                }
                if let Some((fp, on)) = events::online(entry) {
                    st.stored.last_seen.insert(fp.to_string(), now);
                    seen = true;
                    if let Some((_, seen_off)) = st.restarting.get_mut(fp) {
                        if !on {
                            *seen_off = true;
                        } else if *seen_off {
                            st.restarting.remove(fp);
                        }
                    }
                }
                out.push(e);
            }
            if seen {
                self.save(&st);
            }
        }
        self.emit(out, parts);
        if coded {
            self.approve_coded_pairings();
        }
        if joined {
            self.mark_vesper_relays("it joined the site");
        }
    }

    /// What isn't an event: the approval window opening or closing by itself, the join mode
    /// changed elsewhere, PIN unlocks idle for too long, restarts not back in time. Called
    /// every [`SiteConfig::tick`].
    pub fn tick(&self) {
        if self.closing.load(Ordering::SeqCst) {
            return;
        }
        self.expire_unlocks();
        self.expire_pairings();
        self.follow_membership();
        self.start_again();
        self.refresh_relay_code(false);
        self.approve_coded_pairings();
        self.revoke_unused_keys();
        let Some(h) = self.handle() else { return };
        let a = h.authority();
        let (open, mode) = (a.pairing_open(), our_mode(a.join_mode()));
        let changed = {
            let mut st = lock(&self.state);
            st.restarting
                .retain(|_, (since, _)| since.elapsed() < RESTART_GRACE);
            let changed = st.pairing_open != open || st.stored.join_mode != mode;
            if st.stored.join_mode != mode {
                st.stored.join_mode = mode;
                self.save(&st);
            }
            st.pairing_open = open;
            changed
        };
        if changed {
            self.changed([SitePart::Status]);
        }
    }

    /// A Site Manager that should run but didn't start (a port still in use at boot) is tried
    /// again every [`SiteConfig::start_retry`]: until it runs, no paired client's key can be
    /// checked and no client can pair.
    fn start_again(&self) {
        let due = {
            let st = lock(&self.state);
            st.stored.enabled
                && self.cfg.available
                && st
                    .start_failed
                    .is_some_and(|t| t.elapsed() >= self.cfg.start_retry)
        };
        if !due || self.handle().is_some() || self.enrolled_elsewhere() {
            return;
        }
        match self.start() {
            Ok(()) => tracing::info!("the site's Site Manager started on a later try"),
            Err(e) => {
                tracing::debug!("the site's Site Manager did not start (tried again later): {e}")
            }
        }
    }

    /// The newest `n` site events, newest first.
    pub fn recent_events(&self, n: usize) -> Vec<SiteEvent> {
        lock(&self.state)
            .recent
            .iter()
            .rev()
            .take(n)
            .cloned()
            .collect()
    }
}
