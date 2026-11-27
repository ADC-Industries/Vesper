//! Pairing relays: the Vesper boxes of the site that relay Oriel pairings ([`pairing`]).
//!
//! Since IDN v0.3.4-Beta11 only a machine the site marked a pairing relay
//! (`SiteCommand::Relay`) gets the site's pairing codes to show; a pairing through any other
//! machine waits for an admin's approval. Vesper marks its own boxes itself (owner,
//! 2026-11-25: no manual step), at the site's server (only the box running the Site Manager
//! can mark):
//!
//! - **A Vesper box** is a machine enrolled as an app (`kind` `app`) whose name is the one
//!   every Vesper server joins with, `<its name> (Vesper)` ([`member_name`],
//!   [`is_vesper_box`]). Never any other machine: the mark lets a machine have pairings
//!   approved by its code with nobody's click, so an IDN display box or camera PC (`output`,
//!   `input`), or another program's app, is never marked by Vesper. The Codex's own membership
//!   (`Codex`, built in) isn't one either: pairing at the site's server is answered there.
//! - **Joining:** a Vesper box that joins (approved here, in IDN's admin panel, or with a join
//!   code: the site's `device_joined`) is marked. One approved here is known by the name it
//!   asked with (it may be renamed as it is approved), kept in memory until it is enrolled.
//! - **At the start** of the Site Manager (an existing site after the upgrade), every enrolled
//!   Vesper box not marked yet is marked.
//! - **Once per machine:** the machines Vesper marked, and those an admin marked or unmarked
//!   (`POST /site/devices/{fp}/relay`), are kept in `site.json` (`relayDecided`): an admin's
//!   "off" stays off.
//! - **An admin** can take the mark off any machine, but put it on only a Vesper box (owner,
//!   2026-11-25, M-6): any other machine is refused (`409`), as Oriel offers the toggle only
//!   for Vesper boxes.
//! - Leaving the site, a withdrawn join or a removal ends the mark (IDN); a box joining again
//!   is a new machine, marked as it joins.
//!
//! Each mark runs through the Site Manager as the site's administrator from the server
//! machine: IDN audits it (`relay`, client `server machine`), and it is logged. A box renamed
//! as it was approved before this version isn't recognized at the start, nor can an admin mark
//! it (its name isn't a Vesper box's): it leaves the site and joins again.

use super::*;

/// The name a Vesper server joins another site with: `<name> (Vesper)`.
pub(super) fn member_name(name: &str) -> String {
    format!("{name} ({})", protocol::PRODUCT_NAME)
}

/// A machine of the site is a Vesper box (see the module docs): enrolled as an app, with
/// the name a Vesper server joins with ([`member_name`]).
pub fn is_vesper_box(kind: &str, name: &str) -> bool {
    kind == "app"
        && name
            .strip_suffix(&format!(" ({})", protocol::PRODUCT_NAME))
            .is_some_and(|n| !n.trim().is_empty())
}

impl Site {
    /// At the site's server: marks every enrolled Vesper box that isn't marked and wasn't
    /// decided on before (`why`: for the log). Called at the Site Manager's start, when a
    /// machine joins, and when events went by unseen.
    pub(super) fn mark_vesper_relays(&self, why: &str) {
        let Some(h) = self.handle() else { return };
        let mut approved = lock(&self.relay_marking);
        let own = lock(&self.own_services).clone();
        let devices = h.authority().devices();
        let todo: Vec<(String, String)> = {
            let st = lock(&self.state);
            devices
                .iter()
                .filter(|d| {
                    !d.revoked
                        && !d.relay
                        && (is_vesper_box(&d.kind, &d.name) || approved.contains(&d.fingerprint))
                        && !own.contains(&d.fingerprint)
                        && !st.stored.relay_decided.contains(&d.fingerprint)
                })
                .map(|d| (d.fingerprint.clone(), d.name.clone()))
                .collect()
        };
        for (fingerprint, name) in &todo {
            self.mark_relay(fingerprint, name, why);
        }
        // Approved ones enrolled (marked, or decided on) or gone: no longer waited for.
        approved.retain(|fp| !devices.iter().any(|d| d.fingerprint == *fp));
        if !todo.is_empty() {
            self.changed([SitePart::Devices]);
        }
    }

    /// At the site's server: a Vesper box being approved here (known by the name it asked
    /// with: it may be renamed). Marked once it is enrolled (its `device_joined`).
    pub(super) fn approving_vesper_box(&self, fingerprint: &str) {
        lock(&self.relay_marking).insert(fingerprint.to_string());
    }

    /// Marks one machine (holding `relay_marking`); remembered as decided when it took.
    fn mark_relay(&self, fingerprint: &str, name: &str, why: &str) {
        let cmd = SiteCommand::Relay {
            fingerprint: fingerprint.to_string(),
            on: true,
        };
        match self.run(&self.server_actor(), &cmd) {
            Ok(_) => {
                let mut st = lock(&self.state);
                st.stored.relay_decided.insert(fingerprint.to_string());
                self.save(&st);
                drop(st);
                tracing::info!(%fingerprint, "{name} relays Oriel pairings now: marked a pairing relay ({why}; it is a Vesper box)");
            }
            Err(e) => {
                tracing::warn!(%fingerprint, "marking {name} a pairing relay ({why}): {}", e.message)
            }
        }
    }

    /// `POST /api/v1/site/devices/{fp}/relay` (admins): marks a machine a pairing relay, or
    /// not. Kept as set: Vesper doesn't mark it again at its next start. Only a Vesper box is
    /// marked (`409` for any other machine, M-6); the mark comes off any machine.
    pub fn set_relay(&self, actor: &Actor, fingerprint: &str, on: bool) -> Result<(), ApiError> {
        let d = self.device(fingerprint)?;
        if d.built_in {
            return Err(api_err(
                ErrorCode::Conflict,
                format!(
                    "{} is built into Vesper: pairing at the site's server is answered there, it relays nothing",
                    d.name
                ),
            ));
        }
        if on && d.health == DeviceHealth::Removed {
            return Err(api_err(
                ErrorCode::Conflict,
                format!("{} was removed from the site", d.name),
            ));
        }
        if on && !is_vesper_box(&d.kind, &d.name) {
            return Err(api_err(
                ErrorCode::Conflict,
                format!(
                    "{} isn't a Vesper box: only a Vesper box relays Oriel pairings (an app joined as \"<name> ({})\")",
                    d.name,
                    protocol::PRODUCT_NAME
                ),
            ));
        }
        let _l = lock(&self.relay_marking);
        self.run(
            actor,
            &SiteCommand::Relay {
                fingerprint: fingerprint.into(),
                on,
            },
        )?;
        {
            let mut st = lock(&self.state);
            st.stored.relay_decided.insert(fingerprint.to_string());
            self.save(&st);
        }
        tracing::info!(%fingerprint, on, "{} {} by an admin", d.name, if on { "relays Oriel pairings now" } else { "no longer relays Oriel pairings" });
        self.changed([SitePart::Devices]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_machine_joined_as_vesper_is_a_vesper_box() {
        assert!(is_vesper_box("app", &member_name("BOX-B")));
        assert!(is_vesper_box("app", "Sanctuary (Vesper)"));
        // Another program's app, a display box, a camera PC, the Codex's own membership.
        assert!(!is_vesper_box("app", "Lighting desk"));
        assert!(!is_vesper_box("output", "Lobby (Vesper)"));
        assert!(!is_vesper_box("input", "Camera PC (Vesper)"));
        assert!(!is_vesper_box("app", "Codex"));
        // Only the suffix, no name; or the product name inside, not at the end.
        assert!(!is_vesper_box("app", " (Vesper)"));
        assert!(!is_vesper_box("app", "Vesper"));
        assert!(!is_vesper_box("app", "BOX (Vesper) spare"));
    }
}
