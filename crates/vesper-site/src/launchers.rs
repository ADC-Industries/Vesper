//! Per-machine Vesper Launcher keys (`docs/LAUNCHER-PLAN.md`, the join round).
//!
//! A Launcher joins the site by pairing with the site's server (`POST /api/v1/pair` with
//! `launcher: <machineId>`: the server's code, or an admin's approval, as for Oriel). What it
//! gets is not a client's key but **that machine's Launcher key**:
//!
//! - a named site API key from the Site Manager with its own audience, `app:vesper-launcher`
//!   ([`launcher_audience`]), scope `control`, living as long as IDN allows. Every other route
//!   of the server refuses it: it isn't for `app:vesper` ([`Site::verify`] takes only those);
//! - **bound to the machine** it was asked for: the site's server keeps key ID → machine ID in
//!   `site.json` (`launcherKeys`), so the key only reports as that machine and only picks up
//!   that machine's queued actions ([`Site::verify_launcher`]). A new key for the same machine
//!   (the wizard run again) revokes the one before.
//!
//! IDN offers an audience per program (`app:<name>`), not a binding of a key to the machine
//! holding it (a token's `aud.machine` is where it may be *used*), hence the binding here.
//! Revoking the key in Site ▸ API keys ends it at once, as any key; a key still unused a day
//! after it was made is revoked as an Oriel pairing's is ([`Site::revoke_unused_keys`]).

use super::*;

/// The Launcher's program name in key audiences.
pub const LAUNCHER_APP: &str = "vesper-launcher";

/// The audience of a Launcher key as IDN writes it (`app:vesper-launcher`).
pub fn launcher_audience() -> String {
    format!(
        "{}{LAUNCHER_APP}",
        idn_open_protocol::trust::token::APP_PREFIX
    )
}

/// A Launcher key's name: `Vesper Launcher – <machine name>`.
pub const LAUNCHER_KEY_PREFIX: &str = "Vesper Launcher – ";

/// A Launcher key's name for its machine `name` (as [`crate::auth::key_name`]).
pub fn launcher_key_name(name: &str, ip: IpAddr) -> String {
    let name = match name.trim() {
        "" => format!("machine at {ip}"),
        n => n.chars().filter(|c| !c.is_control()).collect(),
    };
    format!("{LAUNCHER_KEY_PREFIX}{name}")
        .chars()
        .take(crate::auth::KEY_NAME_MAX)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// A machine ID a Launcher may pair for: 1 to 64 of `A-Z a-z 0-9 - _ .` (a Launcher's is a
/// UUID).
pub fn valid_machine_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A Launcher key shown with a request, checked ([`Site::verify_launcher`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherKey {
    /// The key's ID in the site.
    pub key_id: String,
    /// The machine it is bound to.
    pub machine_id: String,
}

impl Site {
    /// Checks a Launcher key, at the site's server: signed by the site for
    /// `app:vesper-launcher`, scope `control`, a named key (not revoked: IDN checks), and
    /// bound here to a machine. `None` for anything else (an Oriel key too).
    pub fn verify_launcher(&self, token: &str) -> Option<LauncherKey> {
        let h = self.handle()?;
        let c = match h.verify_token(token, LAUNCHER_APP) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("a Launcher key was refused: {e}");
                return None;
            }
        };
        if c.scope != Scope::Control || !c.key {
            return None;
        }
        let key_id = c.sub.strip_prefix("key:")?.to_string();
        let machine_id = lock(&self.state)
            .stored
            .launcher_keys
            .get(&key_id)
            .cloned()?;
        Some(LauncherKey { key_id, machine_id })
    }

    /// Makes machine `machine`'s Launcher key at the site's server, as `user` for `caller`
    /// (in the Site Manager's audit), and binds it to the machine; the machine's key from
    /// before is revoked. `(key, id)`.
    pub(super) fn make_launcher_key(
        &self,
        user: &str,
        caller: &host::Caller,
        name: &str,
        machine: &str,
    ) -> Result<(String, String), ApiError> {
        let h = self.running()?;
        let v = h
            .run_from(
                user,
                caller,
                &SiteCommand::ApiKey {
                    name: name.to_string(),
                    aud: TokenAud {
                        engine: launcher_audience(),
                        machine: None,
                    },
                    scope: "control".into(),
                    days: None,
                },
            )
            .map_err(from_idn)?;
        let (Some(key), Some(id)) = (v["key"].as_str(), v["id"].as_str()) else {
            return Err(api_err(
                ErrorCode::Internal,
                format!("unexpected answer from the Site Manager: {}", v["ok"]),
            ));
        };
        self.record_pairing_key(id);
        self.bind_launcher_key(id, machine);
        tracing::info!(key = %id, %machine, "{name}: Launcher key made for its machine");
        Ok((key.to_string(), id.to_string()))
    }

    /// An IDN join code for the machine of Launcher key `key`: its IDN Node (a display box)
    /// joins the site with it (`idn-node join CODE`), so nobody types a second code. The
    /// server makes it as the site's administrator (IDN lets only admins mint codes, and a
    /// Launcher key isn't one), named `client` from `peer` in the Site Manager's audit. Codes
    /// are single use and expire in 10 minutes; the machine's code from before, if still
    /// waiting, is taken back (one waiting code per machine).
    pub fn launcher_join_code(
        &self,
        key: &LauncherKey,
        client: &str,
        peer: IpAddr,
    ) -> Result<protocol::LauncherJoinCode, ApiError> {
        let mut actor = self.server_actor();
        actor.client = client.to_string();
        actor.peer = peer.to_string();
        let v = self.run(&actor, &SiteCommand::JoinCode { cancel: None })?;
        let (Some(id), Some(code)) = (v["id"].as_u64(), v["code"].as_str()) else {
            return Err(api_err(
                ErrorCode::Internal,
                format!("unexpected answer from the Site Manager: {v}"),
            ));
        };
        let expires = v["expires"].as_u64().unwrap_or(0);
        let before = lock(&self.state)
            .launcher_codes
            .insert(key.machine_id.clone(), id);
        if let Some(old) = before
            && let Ok(h) = self.running()
            && h.authority().join_codes().iter().any(|c| c.id == old)
        {
            // Best effort: it may be used or expire meanwhile.
            let _ = self.run(&actor, &SiteCommand::JoinCode { cancel: Some(old) });
        }
        let mut issued = events::ours(
            protocol::SiteEventName::JoinCodeIssued,
            format!("join code {id} issued for {client}'s IDN Node"),
        );
        issued.code_id = Some(id);
        issued.detail =
            json!({"issuer": client, "machine": key.machine_id, "expiresAt": rfc3339(expires)});
        self.emit(vec![issued], [SitePart::JoinCodes]);
        tracing::info!(code = id, machine = %key.machine_id, "join code made for a Launcher's IDN Node");
        Ok(protocol::LauncherJoinCode {
            code: code.chars().filter(char::is_ascii_digit).collect(),
            expires_in_seconds: expires.saturating_sub(unix_now()),
        })
    }

    /// Binds key `id` to `machine`; the machine's keys from before are revoked (one Launcher
    /// key per machine).
    fn bind_launcher_key(&self, id: &str, machine: &str) {
        let before: Vec<String> = {
            let mut st = lock(&self.state);
            let before = st
                .stored
                .launcher_keys
                .iter()
                .filter(|(k, m)| *m == machine && *k != id)
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>();
            for k in &before {
                st.stored.launcher_keys.remove(k);
            }
            st.stored
                .launcher_keys
                .insert(id.to_string(), machine.to_string());
            self.save(&st);
            before
        };
        for k in before {
            match self.revoke_key_as_server(&k) {
                Ok(()) => {
                    tracing::info!(key = %k, %machine, "the machine's Launcher key from before is revoked")
                }
                Err(e) => {
                    tracing::warn!(key = %k, %machine, "revoking the machine's Launcher key from before: {}", e.message)
                }
            }
        }
        self.changed([SitePart::ApiKeys]);
    }

    /// The IDs of the Launcher keys bound now (taken before the site's keys are listed, for
    /// [`Site::forget_launcher_keys`]).
    pub(super) fn bound_launcher_keys(&self) -> BTreeSet<String> {
        lock(&self.state)
            .stored
            .launcher_keys
            .keys()
            .cloned()
            .collect()
    }

    /// Forgets the bindings of Launcher keys that are gone or revoked (`keys`: the site's, as
    /// listed after `bound` was taken). A key bound since (a Launcher that joined meanwhile)
    /// isn't in that list yet, and keeps its binding.
    pub(super) fn forget_launcher_keys(&self, bound: &BTreeSet<String>, keys: &[host::ApiKeyInfo]) {
        let live: BTreeSet<&str> = keys
            .iter()
            .filter(|k| !k.revoked)
            .map(|k| k.id.as_str())
            .collect();
        let mut st = lock(&self.state);
        if forget_bindings(&mut st.stored.launcher_keys, bound, &live) {
            self.save(&st);
        }
    }
}

/// Removes from `bindings` the keys of `bound` that aren't `live`; whether any was removed.
fn forget_bindings(
    bindings: &mut BTreeMap<String, String>,
    bound: &BTreeSet<String>,
    live: &BTreeSet<&str>,
) -> bool {
    let before = bindings.len();
    bindings.retain(|id, _| !bound.contains(id) || live.contains(id.as_str()));
    bindings.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The unused-key sweep lists the site's keys, then revokes (Site Manager runs), then
    /// forgets the bindings of keys gone or revoked: a Launcher key made and bound meanwhile
    /// isn't in its list, and keeps its binding.
    #[test]
    fn a_launcher_key_bound_during_the_sweep_keeps_its_binding() {
        let mut bindings: BTreeMap<String, String> = [
            ("k1", "machine-a"),
            ("k2", "machine-b"),
            ("k3", "machine-c"),
        ]
        .into_iter()
        .map(|(k, m)| (k.to_string(), m.to_string()))
        .collect();
        // Taken before the keys were listed: k1, k2, k3 (k3 is gone from the site since).
        let bound: BTreeSet<String> = bindings.keys().cloned().collect();
        let live: BTreeSet<&str> = ["k1", "k2"].into_iter().collect();
        // Meanwhile machine-d joined: k4 made and bound after the list was taken.
        bindings.insert("k4".into(), "machine-d".into());
        assert!(forget_bindings(&mut bindings, &bound, &live));
        assert_eq!(
            bindings.keys().map(String::as_str).collect::<Vec<_>>(),
            ["k1", "k2", "k4"]
        );
        assert!(!forget_bindings(&mut bindings, &bound, &live));
    }

    #[test]
    fn launcher_keys_have_their_own_audience_and_name() {
        assert_eq!(launcher_audience(), "app:vesper-launcher");
        assert_ne!(launcher_audience(), app_audience());
        let ip: IpAddr = "192.0.2.8".parse().unwrap();
        assert_eq!(
            launcher_key_name(" Chapel Box ", ip),
            "Vesper Launcher – Chapel Box"
        );
        assert_eq!(
            launcher_key_name("", ip),
            "Vesper Launcher – machine at 192.0.2.8"
        );
        assert!(valid_machine_id("6d6f636b-0000-4000-8000-000000000001"));
        assert!(!valid_machine_id(""));
        assert!(!valid_machine_id("a/b"));
        assert!(!valid_machine_id(&"a".repeat(65)));
    }
}
