//! The site's typed events: the Site Manager's own
//! ([`idn_site_manager::host::Handle::events_wait`]), as the
//! protocol's [`SiteEvent`]s, and which parts of the state each one changes ([`SitePart`],
//! for `siteChanged`).
//!
//! Names are the Site Manager's push channel's (`join_requested`, `device_revoked`,
//! `join_code_wrong`, `key_requested`, `device_left`, …; the machines' own, such as
//! `two_sites`, pass through). Some come from this server, for what it does itself:
//! `join_code_issued` and `join_code_cancelled` (a code issued or taken back through this API),
//! and `key_requested` / `key_request_cancelled` for pairings waiting at this server.
//! A relayed `key_requested` says in `detail.codeOk` whether the client typed the pairing
//! code the site gave its relay (checked by the Site Manager): such a pairing is approved at
//! once ([`super::pairing`]).

use idn_site_manager::host::{SiteEvent as Idn, SiteEventEntry};
use protocol::{SiteEvent, SiteEventLevel, SiteEventName, SitePart};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Unix seconds now.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// RFC 3339 of Unix seconds.
pub fn rfc3339(secs: u64) -> String {
    crate::auth::rfc3339(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs),
        true,
    )
}

/// An event of this server's own, now.
pub fn ours(name: SiteEventName, text: String) -> SiteEvent {
    SiteEvent {
        name: name_str(name).to_string(),
        level: SiteEventLevel::Info,
        text,
        at: rfc3339(unix_now()),
        fingerprint: None,
        machine: None,
        address: None,
        request_id: None,
        code_id: None,
        tries_left: None,
        detail: json!({}),
    }
}

/// An event name as sent.
pub fn name_str(n: SiteEventName) -> &'static str {
    match n {
        SiteEventName::JoinRequested => "join_requested",
        SiteEventName::JoinCancelled => "join_cancelled",
        SiteEventName::DeviceJoined => "device_joined",
        SiteEventName::DeviceOnline => "device_online",
        SiteEventName::DeviceOffline => "device_offline",
        SiteEventName::JoinDenied => "join_denied",
        SiteEventName::JoinRefused => "join_refused",
        SiteEventName::JoinWithdrawn => "join_withdrawn",
        SiteEventName::DeviceRevoked => "device_revoked",
        SiteEventName::JoinCodeIssued => "join_code_issued",
        SiteEventName::JoinCodeUsed => "join_code_used",
        SiteEventName::JoinCodeWrong => "join_code_wrong",
        SiteEventName::JoinCodeBurned => "join_code_burned",
        SiteEventName::JoinCodeExpired => "join_code_expired",
        SiteEventName::JoinCodeCancelled => "join_code_cancelled",
        SiteEventName::AlreadyInSite => "already_in_site",
        SiteEventName::SiteManagerUnreachable => "site_manager_unreachable",
        SiteEventName::Revoked => "revoked",
        SiteEventName::TwoSites => "two_sites",
        SiteEventName::KeyRequested => "key_requested",
        SiteEventName::KeyRequestCancelled => "key_request_cancelled",
        SiteEventName::DeviceLeft => "device_left",
    }
}

pub fn level(s: &str) -> SiteEventLevel {
    match s {
        "error" => SiteEventLevel::Error,
        "warning" => SiteEventLevel::Warning,
        "ok" => SiteEventLevel::Ok,
        _ => SiteEventLevel::Info,
    }
}

/// A relayed pairing typed with the right code (its IDN id), to approve at once.
pub fn coded_key_request(entry: &SiteEventEntry) -> Option<u64> {
    match &entry.event {
        Idn::KeyRequested {
            id, code_ok: true, ..
        } => Some(*id),
        _ => None,
    }
}

/// A Site Manager event as the protocol's, and the parts of the state it changes.
pub fn convert(entry: &SiteEventEntry) -> (SiteEvent, BTreeSet<SitePart>) {
    use SiteEventName as N;
    use SitePart as P;
    // Every field the event carries, as the detail (the tag left out).
    let mut detail = serde_json::to_value(&entry.event).unwrap_or_else(|_| json!({}));
    let tag = detail
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(o) = detail.as_object_mut() {
        o.remove("event");
    }
    let mut e = SiteEvent {
        name: String::new(),
        level: level(&entry.level),
        text: entry.text.clone(),
        at: rfc3339(entry.at),
        fingerprint: None,
        machine: None,
        address: None,
        request_id: None,
        code_id: None,
        tries_left: None,
        detail: json!({}),
    };
    let machine = |e: &mut SiteEvent, fp: &str, name: &str| {
        e.fingerprint = (!fp.is_empty()).then(|| fp.to_string());
        e.machine = (!name.is_empty()).then(|| name.to_string());
    };
    let (name, parts): (String, &[SitePart]) = match &entry.event {
        Idn::JoinRequested {
            id,
            name,
            kind,
            fingerprint,
            address,
            by_code,
            ..
        } => {
            machine(&mut e, fingerprint, name);
            e.request_id = Some(*id);
            e.address = address.clone();
            detail = json!({"kind": kind, "byCode": by_code});
            (name_str(N::JoinRequested).into(), &[P::JoinRequests])
        }
        Idn::JoinCancelled { id } => {
            e.request_id = Some(*id);
            (name_str(N::JoinCancelled).into(), &[P::JoinRequests])
        }
        Idn::DeviceJoined {
            fingerprint,
            name,
            kind,
        } => {
            machine(&mut e, fingerprint, name);
            detail = json!({"kind": kind});
            (
                name_str(N::DeviceJoined).into(),
                &[P::Devices, P::JoinRequests],
            )
        }
        Idn::JoinDenied { fingerprint, name } => {
            machine(&mut e, fingerprint, name);
            (name_str(N::JoinDenied).into(), &[P::JoinRequests])
        }
        Idn::JoinRefused { fingerprint, name } => {
            machine(&mut e, fingerprint, name);
            (name_str(N::JoinRefused).into(), &[P::JoinRequests])
        }
        Idn::JoinWithdrawn { fingerprint, name } => {
            machine(&mut e, fingerprint, name);
            (
                name_str(N::JoinWithdrawn).into(),
                &[P::JoinRequests, P::Devices],
            )
        }
        Idn::DeviceRemoved { fingerprint, name } => {
            machine(&mut e, fingerprint, name);
            (name_str(N::DeviceRevoked).into(), &[P::Devices])
        }
        Idn::DeviceOnline { fingerprint, name } => {
            machine(&mut e, fingerprint, name);
            (name_str(N::DeviceOnline).into(), &[P::Devices])
        }
        Idn::DeviceOffline { fingerprint, name } => {
            machine(&mut e, fingerprint, name);
            (name_str(N::DeviceOffline).into(), &[P::Devices])
        }
        Idn::CodeUsed {
            id,
            fingerprint,
            name,
        } => {
            machine(&mut e, fingerprint, name);
            e.code_id = Some(*id);
            (name_str(N::JoinCodeUsed).into(), &[P::JoinCodes])
        }
        Idn::CodeExpired { id } => {
            e.code_id = Some(*id);
            (name_str(N::JoinCodeExpired).into(), &[P::JoinCodes])
        }
        Idn::CodeBurned { id, replaced } => {
            e.code_id = Some(*id);
            detail = json!({"replaced": replaced});
            (name_str(N::JoinCodeBurned).into(), &[P::JoinCodes])
        }
        Idn::CodeWrong {
            name,
            address,
            tries_left,
        } => {
            machine(&mut e, "", name);
            e.address = address.clone();
            e.tries_left = Some(*tries_left);
            (name_str(N::JoinCodeWrong).into(), &[P::JoinCodes])
        }
        Idn::KeyRequested {
            id,
            code_ok,
            client,
            via,
            via_fingerprint,
            ..
        } => {
            machine(&mut e, via_fingerprint, via);
            detail = json!({"pairing": format!("k{id}"), "client": client, "relayed": true,
                "codeOk": code_ok});
            (name_str(N::KeyRequested).into(), &[P::Pairings])
        }
        Idn::KeyRequestCancelled { id } => {
            detail = json!({"pairing": format!("k{id}")});
            (name_str(N::KeyRequestCancelled).into(), &[P::Pairings])
        }
        Idn::DeviceLeft { fingerprint, name } => {
            machine(&mut e, fingerprint, name);
            (name_str(N::DeviceLeft).into(), &[P::Devices])
        }
        Idn::Other { name, fields } => {
            let s = |k: &str| fields.get(k).and_then(Value::as_str).unwrap_or("");
            machine(&mut e, s("fingerprint"), s("machine"));
            e.address = fields
                .get("address")
                .and_then(Value::as_str)
                .map(str::to_owned);
            detail = fields.clone();
            let parts: &[SitePart] = if e.fingerprint.is_some() {
                &[P::Devices]
            } else {
                &[]
            };
            (name.clone(), parts)
        }
        // A kind of event newer than this server: its name and fields as they come.
        _ => (tag, &[]),
    };
    e.name = name;
    e.detail = detail;
    (e, parts.iter().copied().collect())
}

/// The join code an event ends (used, expired, burned).
pub fn code_ended(entry: &SiteEventEntry) -> Option<u64> {
    match entry.event {
        Idn::CodeUsed { id, .. } | Idn::CodeExpired { id } | Idn::CodeBurned { id, .. } => Some(id),
        _ => None,
    }
}

/// A machine joined the site (approved, by code or by the passphrase).
pub fn joined(entry: &SiteEventEntry) -> bool {
    matches!(entry.event, Idn::DeviceJoined { .. })
}

/// The device an event says came online (`Some(true)`) or went offline (`Some(false)`).
pub fn online(entry: &SiteEventEntry) -> Option<(&str, bool)> {
    match &entry.event {
        Idn::DeviceOnline { fingerprint, .. } => Some((fingerprint, true)),
        Idn::DeviceOffline { fingerprint, .. } => Some((fingerprint, false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(event: Idn) -> SiteEventEntry {
        SiteEventEntry {
            seq: 1,
            at: 1_790_000_000,
            level: "warning".into(),
            text: "something".into(),
            event,
        }
    }

    #[test]
    fn names_match_the_wire() {
        for n in [
            SiteEventName::JoinRequested,
            SiteEventName::JoinWithdrawn,
            SiteEventName::JoinCodeWrong,
            SiteEventName::SiteManagerUnreachable,
            SiteEventName::TwoSites,
        ] {
            assert_eq!(serde_json::to_value(n).unwrap(), json!(name_str(n)));
        }
    }

    #[test]
    fn site_manager_events_become_typed_events() {
        let (e, parts) = convert(&entry(Idn::JoinRequested {
            id: 7,
            name: "BOX".into(),
            kind: "app".into(),
            fingerprint: "ab".into(),
            code: "482913".into(),
            address: Some("192.0.2.9".into()),
            by_code: false,
        }));
        assert_eq!(e.name, "join_requested");
        assert_eq!(e.request_id, Some(7));
        assert_eq!(e.fingerprint.as_deref(), Some("ab"));
        assert_eq!(e.machine.as_deref(), Some("BOX"));
        assert_eq!(e.address.as_deref(), Some("192.0.2.9"));
        assert_eq!(e.level, SiteEventLevel::Warning);
        assert_eq!(e.detail, json!({"kind": "app", "byCode": false}));
        assert_eq!(parts, [SitePart::JoinRequests].into());

        let wrong = entry(Idn::CodeWrong {
            name: "BOX".into(),
            address: None,
            tries_left: 2,
        });
        let (e, parts) = convert(&wrong);
        assert_eq!(e.name, "join_code_wrong");
        assert_eq!(e.tries_left, Some(2));
        assert_eq!(parts, [SitePart::JoinCodes].into());
        assert_eq!(code_ended(&wrong), None);

        let removed = entry(Idn::DeviceRemoved {
            fingerprint: "cd".into(),
            name: "Stage".into(),
        });
        assert_eq!(convert(&removed).0.name, "device_revoked");
        let burned = entry(Idn::CodeBurned {
            id: 4,
            replaced: true,
        });
        let (e, _) = convert(&burned);
        assert_eq!((e.name.as_str(), e.code_id), ("join_code_burned", Some(4)));
        assert_eq!(e.detail, json!({"replaced": true}));
        assert_eq!(code_ended(&burned), Some(4));

        let other = entry(Idn::Other {
            name: "two_sites".into(),
            fields: json!({"fingerprint": "ef", "machine": "Booth", "other": 1}),
        });
        let (e, parts) = convert(&other);
        assert_eq!(e.name, "two_sites");
        assert_eq!(e.fingerprint.as_deref(), Some("ef"));
        assert_eq!(e.detail["other"], 1);
        assert_eq!(parts, [SitePart::Devices].into());

        let (e, parts) = convert(&entry(Idn::KeyRequested {
            id: 3,
            code_ok: false,
            client: "Oriel – Booth".into(),
            via: "BOX-B (Vesper)".into(),
            via_fingerprint: "ab".into(),
            pin: false,
        }));
        assert_eq!(e.name, "key_requested");
        assert_eq!(e.machine.as_deref(), Some("BOX-B (Vesper)"));
        assert_eq!(e.detail["pairing"], "k3");
        assert_eq!(e.detail["codeOk"], false);
        assert_eq!(parts, [SitePart::Pairings].into());
        let (e, parts) = convert(&entry(Idn::DeviceLeft {
            fingerprint: "ab".into(),
            name: "BOX-B (Vesper)".into(),
        }));
        assert_eq!(e.name, "device_left");
        assert_eq!(parts, [SitePart::Devices].into());

        let off = entry(Idn::DeviceOffline {
            fingerprint: "cd".into(),
            name: "Stage".into(),
        });
        assert_eq!(online(&off), Some(("cd", false)));
    }
}
