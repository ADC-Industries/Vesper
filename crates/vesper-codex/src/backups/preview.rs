//! What restoring a snapshot would change: per item ([`diff`]) and in words ([`summary`]).

use super::vesper::{SERVER_KEY, SERVER_KIND};
use model::{DocChange, SnapshotItem};
use protocol::RestoreChange;
use std::collections::BTreeMap;

/// Whether an item of this kind takes effect only after a restart.
pub fn needs_restart(kind: &str) -> bool {
    kind == SERVER_KIND || kind == "fontSubstitutionTable"
}

/// What restoring `snapshot` onto a box holding `current` changes, sorted by key, and how many
/// items stay the same. The server's identity counts as changed only when its name differs
/// (its ID is never restored, except by a replacement box taking it over).
pub fn diff(current: &[SnapshotItem], snapshot: &[SnapshotItem]) -> (Vec<RestoreChange>, u32) {
    let cur: BTreeMap<&str, &SnapshotItem> = current.iter().map(|i| (i.key.as_str(), i)).collect();
    let snap: BTreeMap<&str, &SnapshotItem> =
        snapshot.iter().map(|i| (i.key.as_str(), i)).collect();
    let mut keys: Vec<&str> = cur.keys().chain(snap.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    let mut out = Vec::new();
    let mut same = 0;
    for key in keys {
        let (c, s) = (cur.get(key), snap.get(key));
        let change = match (c, s) {
            (None, Some(_)) => DocChange::Added,
            (Some(_), None) if key == SERVER_KEY => {
                same += 1;
                continue;
            }
            (Some(_), None) => DocChange::Removed,
            (Some(c), Some(s)) if key == SERVER_KEY => {
                if c.name == s.name {
                    same += 1;
                    continue;
                }
                DocChange::Changed
            }
            (Some(c), Some(s)) if c.sha256 == s.sha256 => {
                same += 1;
                continue;
            }
            (Some(_), Some(_)) => DocChange::Changed,
            (None, None) => continue,
        };
        let item = s.or(c).expect("one side has it");
        out.push(RestoreChange {
            key: key.to_string(),
            kind: item.kind.clone(),
            id: item.id.clone(),
            name: s
                .and_then(|i| i.name.clone())
                .or(c.and_then(|i| i.name.clone())),
            change,
            restart: needs_restart(&item.kind),
        });
    }
    (out, same)
}

/// A kind's words in a summary: singular and plural.
fn noun(kind: &str) -> (String, String) {
    let fixed = |s: &str, p: &str| (s.to_string(), p.to_string());
    match kind {
        "showSettings" => fixed("show settings", "show settings"),
        "audioDeviceConfig" => fixed("audio device setting", "audio device settings"),
        "fontSubstitutionTable" => fixed("font substitution table", "font substitution tables"),
        "server" => fixed("server name", "server names"),
        "orielPreferences" => fixed("preferences", "preferences"),
        other => {
            // camelCase → words: `outputBinding` → `output binding`.
            let mut words = String::new();
            for c in other.chars() {
                if c.is_ascii_uppercase() {
                    words.push(' ');
                    words.push(c.to_ascii_lowercase());
                } else {
                    words.push(c);
                }
            }
            (words.clone(), format!("{words}s"))
        }
    }
}

/// The changes in words: `3 screens, 2 looks changed; 1 output binding added`; `nothing would
/// change` when there are none.
pub fn summary(changes: &[RestoreChange]) -> String {
    let mut parts = Vec::new();
    for (change, word) in [
        (DocChange::Changed, "changed"),
        (DocChange::Added, "added"),
        (DocChange::Removed, "removed"),
    ] {
        // Kinds in the order they first appear.
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for c in changes.iter().filter(|c| c.change == change) {
            match counts.iter_mut().find(|(k, _)| *k == c.kind) {
                Some((_, n)) => *n += 1,
                None => counts.push((&c.kind, 1)),
            }
        }
        if counts.is_empty() {
            continue;
        }
        let list: Vec<String> = counts
            .iter()
            .map(|(k, n)| {
                let (one, many) = noun(k);
                format!("{n} {}", if *n == 1 { one } else { many })
            })
            .collect();
        parts.push(format!("{} {word}", list.join(", ")));
    }
    if parts.is_empty() {
        "nothing would change".into()
    } else {
        parts.join("; ")
    }
}

/// What a set of changes needs a restart for (`server name`, `font substitutions`).
pub fn restart_list(changes: &[RestoreChange]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in changes.iter().filter(|c| c.restart) {
        let what = if c.kind == SERVER_KIND {
            "server name"
        } else {
            "font substitutions"
        };
        if !out.iter().any(|x| x == what) {
            out.push(what.into());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, kind: &str, sha: &str, name: Option<&str>) -> SnapshotItem {
        SnapshotItem {
            key: key.into(),
            kind: kind.into(),
            id: None,
            name: name.map(str::to_string),
            schema_version: 2,
            sha256: sha.into(),
            size: 1,
        }
    }

    #[test]
    fn diff_and_summary() {
        let current = vec![
            item("portable/screen/a.json", "screen", "1", Some("A")),
            item("portable/screen/b.json", "screen", "2", None),
            item("portable/screen/c.json", "screen", "3", None),
            item("portable/look/x.json", "look", "4", None),
            item("machine/outputBinding/o.json", "outputBinding", "5", None),
            item(SERVER_KEY, "server", "6", Some("Booth")),
        ];
        let snapshot = vec![
            item("portable/screen/a.json", "screen", "1x", Some("A old")),
            item("portable/screen/b.json", "screen", "2x", None),
            item("portable/screen/c.json", "screen", "3x", None),
            item("portable/look/x.json", "look", "4", None),
            item("portable/look/y.json", "look", "7", None),
            item(SERVER_KEY, "server", "other", Some("Booth")),
        ];
        let (changes, same) = diff(&current, &snapshot);
        assert_eq!(same, 2, "the look x and the server (same name)");
        assert_eq!(changes.len(), 5);
        assert_eq!(changes[0].key, "machine/outputBinding/o.json");
        assert_eq!(changes[0].change, DocChange::Removed);
        let a = changes.iter().find(|c| c.key.ends_with("a.json")).unwrap();
        assert_eq!(a.name.as_deref(), Some("A old"));
        assert_eq!(
            summary(&changes),
            "3 screens changed; 1 look added; 1 output binding removed"
        );
        assert!(restart_list(&changes).is_empty());

        let renamed = vec![item(SERVER_KEY, "server", "x", Some("Stage"))];
        let (changes, _) = diff(&current[5..], &renamed);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].restart);
        assert_eq!(summary(&changes), "1 server name changed");
        assert_eq!(restart_list(&changes), vec!["server name".to_string()]);
        assert_eq!(summary(&[]), "nothing would change");
    }
}
