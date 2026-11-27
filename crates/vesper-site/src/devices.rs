//! The site's devices: health, removal, identify, restart, logs.

use super::*;

/// A machine's details (G9) as its Node reported them at `reported` (seconds since 1970):
/// what its system doesn't say is left out.
fn machine(m: &host::DeviceMachine, reported: Option<u64>) -> protocol::SiteDeviceMachine {
    let text = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    protocol::SiteDeviceMachine {
        cpu: text(&m.cpu),
        cpus: m.cpus.filter(|n| *n > 0),
        gpus: m.gpus.iter().filter_map(|g| text(g)).collect(),
        os: text(&m.os),
        os_version: text(&m.os_version),
        kernel: text(&m.kernel),
        booted_at: m.boot_time.filter(|t| *t > 0).map(rfc3339),
        memory_total: m.memory_total.filter(|n| *n > 0),
        memory_free: m.memory_free,
        reported_at: reported.map(rfc3339),
    }
}

impl Site {
    /// `GET /api/v1/site/devices`: every machine of the site (removed ones last), with its
    /// health as the Site Manager sees it.
    pub fn devices(&self) -> Vec<SiteDevice> {
        let Some(h) = self.handle() else {
            return Vec::new();
        };
        let a = h.authority();
        let devices = a.devices();
        let health = h.device_health();
        let now = unix_now();
        let own = lock(&self.own_services).clone();
        let st = lock(&self.state);
        let mut out: Vec<SiteDevice> = devices
            .iter()
            .map(|d| {
                let hh = health.iter().find(|x| x.fingerprint == d.fingerprint);
                let online = !d.revoked && hh.map_or(d.online, |x| x.online);
                let problems: Vec<DeviceProblem> = hh
                    .map(|x| {
                        x.attention
                            .iter()
                            .map(|p| DeviceProblem {
                                problem: p.problem.clone(),
                                level: events::level(&p.level),
                                text: p.text.clone(),
                                fields: p.fields.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let restarting: Vec<String> = hh.map(|x| x.restarting.clone()).unwrap_or_default();
                let asked = st
                    .restarting
                    .get(&d.fingerprint)
                    .is_some_and(|(since, _)| since.elapsed() < RESTART_GRACE);
                let updating = hh.and_then(|x| x.updating.clone());
                let health = if d.revoked {
                    DeviceHealth::Removed
                } else if asked || !restarting.is_empty() {
                    DeviceHealth::Restarting
                } else if updating.is_some() {
                    DeviceHealth::Updating
                } else if !online {
                    DeviceHealth::Offline
                } else if !problems.is_empty() {
                    DeviceHealth::NeedsAttention
                } else {
                    DeviceHealth::Online
                };
                // Connected or reported, whichever is newer (ours is kept across restarts).
                let seen = st
                    .stored
                    .last_seen
                    .get(&d.fingerprint)
                    .copied()
                    .max(hh.and_then(|x| x.last_seen));
                let version = hh.and_then(|x| x.version.clone());
                SiteDevice {
                    fingerprint: d.fingerprint.clone(),
                    name: hh
                        .map(|x| x.name.clone())
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| d.name.clone()),
                    kind: d.kind.clone(),
                    admin: d.admin,
                    health,
                    reason: (health == DeviceHealth::NeedsAttention)
                        .then(|| problems.first().map(|p| p.text.clone()))
                        .flatten(),
                    problems,
                    last_seen: if online { Some(now) } else { seen }.map(rfc3339),
                    node: version.is_some() || hh.is_some_and(|x| x.last_seen.is_some()),
                    version,
                    restarting,
                    services: hh
                        .map(|x| {
                            x.services
                                .iter()
                                .map(|s| protocol::DeviceService {
                                    id: s.id.clone(),
                                    name: s.name.clone(),
                                    state: s.state.clone(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    updating,
                    update_managed_by: hh.and_then(|x| x.update_managed_by.clone()),
                    node_started_at: hh.and_then(|x| x.node_started).map(rfc3339),
                    machine: hh.and_then(|x| x.machine.as_ref().map(|m| machine(m, x.last_seen))),
                    built_in: own.contains(&d.fingerprint),
                    relay: d.relay && !d.revoked,
                }
            })
            .collect();
        out.sort_by_key(|d| (d.health == DeviceHealth::Removed, d.name.to_lowercase()));
        out
    }

    /// One machine.
    pub fn device(&self, fingerprint: &str) -> Result<SiteDevice, ApiError> {
        self.running()?;
        self.devices()
            .into_iter()
            .find(|d| d.fingerprint == fingerprint)
            .ok_or_else(|| {
                api_err(
                    ErrorCode::NotFound,
                    format!("no device {fingerprint} in the site"),
                )
            })
    }

    /// Removes a machine from the site (revoke). Never a built-in one (the Codex's own
    /// membership, owner 2026-11-23): it would only join again at the next start.
    pub fn remove(&self, actor: &Actor, fingerprint: &str) -> Result<(), ApiError> {
        let d = self.device(fingerprint)?;
        if d.built_in {
            return Err(api_err(
                ErrorCode::Conflict,
                format!(
                    "{} is built into Vesper (the Codex's own membership of its site): it can't be removed, it would join again at the next start",
                    d.name
                ),
            ));
        }
        if d.health == DeviceHealth::Removed {
            return Err(api_err(
                ErrorCode::Conflict,
                format!("{} was already removed", d.name),
            ));
        }
        self.run(
            actor,
            &SiteCommand::Revoke {
                fingerprint: fingerprint.into(),
            },
        )?;
        self.changed([SitePart::Devices]);
        Ok(())
    }

    /// Lets a removed or denied machine ask to join again.
    pub fn allow(&self, actor: &Actor, fingerprint: &str) -> Result<(), ApiError> {
        if fingerprint.len() != 64 || !fingerprint.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(api_err(
                ErrorCode::BadRequest,
                "a fingerprint is 64 hex digits",
            ));
        }
        self.run(
            actor,
            &SiteCommand::Clear {
                fingerprint: fingerprint.into(),
            },
        )?;
        self.changed([SitePart::Devices, SitePart::JoinRequests]);
        Ok(())
    }

    /// Shows a machine's name on its screens.
    pub fn identify(
        &self,
        actor: &Actor,
        fingerprint: &str,
        seconds: Option<u64>,
    ) -> Result<(), ApiError> {
        self.device(fingerprint)?;
        self.run(
            actor,
            &SiteCommand::Identify {
                fingerprint: fingerprint.into(),
                seconds,
            },
        )
        .map(|_| ())
    }

    /// Changes machine `fingerprint`'s IDN Node settings as the server machine
    /// (`SiteCommand::Apply`, audited by the Site Manager): Mosaic's routers on the site's
    /// server and a display box's display service. Fails when it isn't online or isn't a Node.
    pub fn apply_node_settings(
        &self,
        fingerprint: &str,
        settings: serde_json::Map<String, Value>,
    ) -> Result<Value, ApiError> {
        let h = self.running()?;
        h.run_from(
            idn_users::ADMIN,
            &host::Caller {
                address: String::new(),
                client: SERVER_MACHINE.into(),
            },
            &SiteCommand::Apply {
                to: vec![fingerprint.to_string()],
                settings,
            },
        )
        .map_err(from_idn)
    }

    /// The addresses the Site Manager sees the machines' IDN Nodes at (their trust
    /// connections; online Nodes only), by fingerprint.
    pub fn node_addresses(&self) -> HashMap<String, IpAddr> {
        self.handle()
            .map(|h| {
                h.authority()
                    .node_reports()
                    .into_iter()
                    .filter_map(|r| r.address.map(|a| (r.fingerprint, a)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Machine `fingerprint`'s IDN Node's latest report (`idn_node_report`: its `<routers
    /// list>`, `<apps display display_boot>` …), while it is online.
    pub fn node_report(&self, fingerprint: &str) -> Option<idn_open_protocol::xml::Element> {
        self.handle()?
            .authority()
            .node_reports()
            .into_iter()
            .find(|r| r.fingerprint == fingerprint)
            .map(|r| r.report)
    }

    /// Restarts a service on a machine.
    pub fn restart(&self, actor: &Actor, fingerprint: &str, service: &str) -> Result<(), ApiError> {
        self.device(fingerprint)?;
        self.run(
            actor,
            &SiteCommand::Restart {
                fingerprint: fingerprint.into(),
                service: service.into(),
            },
        )?;
        lock(&self.state)
            .restarting
            .insert(fingerprint.to_string(), (Instant::now(), false));
        self.changed([SitePart::Devices]);
        Ok(())
    }

    /// Fetches a machine's logs now; returns the snapshot's name.
    pub fn collect_logs(&self, actor: &Actor, fingerprint: &str) -> Result<String, ApiError> {
        self.device(fingerprint)?;
        let v = self.run(
            actor,
            &SiteCommand::Logs {
                fingerprint: fingerprint.into(),
            },
        )?;
        Ok(v.get("snapshot")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_default())
    }

    /// The log snapshots kept for a machine, newest first.
    pub fn log_snapshots(&self, fingerprint: &str) -> Result<Vec<protocol::LogSnapshot>, ApiError> {
        let h = self.running()?;
        Ok(h.authority()
            .log_snapshots()
            .into_iter()
            .filter(|(d, _, _)| d == fingerprint)
            .map(|(_, snapshot, files)| protocol::LogSnapshot {
                snapshot,
                files: files
                    .into_iter()
                    .map(|(name, size)| protocol::LogFile { name, size })
                    .collect(),
            })
            .collect())
    }

    /// One file of a log snapshot.
    pub fn log_file(
        &self,
        fingerprint: &str,
        snapshot: &str,
        file: &str,
    ) -> Result<String, ApiError> {
        let h = self.running()?;
        h.authority()
            .log_file(fingerprint, snapshot, file)
            .map_err(|e| api_err(ErrorCode::NotFound, e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_details_leave_out_what_the_system_doesnt_say() {
        let mut m = host::DeviceMachine::default();
        m.cpu = "Example CPU 8-Core ".into();
        m.cpus = Some(16);
        m.gpus = vec!["Example GPU".into(), " ".into()];
        m.os = "Ubuntu".into();
        m.os_version = "24.04".into();
        m.boot_time = Some(1_700_000_000);
        m.memory_total = Some(32 << 30);
        m.memory_free = Some(0);
        let d = machine(&m, Some(1_700_000_100));
        assert_eq!(d.cpu.as_deref(), Some("Example CPU 8-Core"));
        assert_eq!(d.gpus, ["Example GPU"]);
        assert_eq!(d.kernel, None, "not said: left out");
        assert_eq!(d.booted_at.as_deref(), Some("2023-11-14T22:13:20Z"));
        assert_eq!(d.memory_free, Some(0), "none free is a fact");
        assert_eq!(d.reported_at.as_deref(), Some("2023-11-14T22:15:00Z"));
        // An empty report: nothing but the time it came.
        let none = machine(&host::DeviceMachine::default(), None);
        assert_eq!(none, protocol::SiteDeviceMachine::default());
    }
}
