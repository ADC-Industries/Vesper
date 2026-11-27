//! A Vesper box's own settings as snapshot items: what is collected ([`collect`], the box's
//! side of "offer my snapshot"), and how a restore is written back ([`apply`], the box's side
//! of "apply this restore").
//!
//! **What a snapshot holds** (only these; every key is checked against this list again when a
//! restore is applied):
//! - `portable/<kind>/<id>.json` of the box's portable kinds ([`BOX_KINDS`]): screens, looks,
//!   show settings, masks, video and audio inputs;
//! - `machine/<kind>/<id>.json` of its machine-local documents ([`MACHINE_KINDS`]): output and
//!   input bindings, media root bindings, the audio device, font substitutions;
//! - `machine/server.json` ([`SERVER_KEY`]): the server's name and ID. A restore writes the
//!   name; the ID only when a replacement box takes over the old box's (see
//!   [`RestorePackage::adopt_server_id`]).
//!
//! **Never**: the collections (they are the Codex itself), `machine/codex.json` (this box's
//! Codex role and assignment), `machine/site-member/`, `machine/codex-member/`,
//! `machine/site-manager/` (device identities, the site CA key), `machine/site.json` (the
//! PIN hash, sign-ins), `machine/clients.json` (credentials), `machine/live-look.json` (the
//! live look, see the server's `LIVE_LOOK_FILE`), `cache/`, fonts and media.

use super::store::sha256;
use crate::data::kind_dir;
use model::{DocumentKind, SnapshotItem};
use serde_json::Value;
use std::path::Path;

/// The box's portable kinds in a snapshot.
pub const BOX_KINDS: &[DocumentKind] = &[
    DocumentKind::Screen,
    DocumentKind::Look,
    DocumentKind::ShowSettings,
    DocumentKind::Mask,
    DocumentKind::VideoInput,
    DocumentKind::AudioInput,
];

/// The box's machine-local kinds in a snapshot.
pub const MACHINE_KINDS: &[DocumentKind] = &[
    DocumentKind::OutputBinding,
    DocumentKind::InputBinding,
    DocumentKind::AudioInputBinding,
    DocumentKind::MediaRootBinding,
    DocumentKind::AudioDeviceConfig,
    DocumentKind::FontSubstitutionTable,
];

/// The server's identity file.
pub const SERVER_KEY: &str = "machine/server.json";
/// The kind of [`SERVER_KEY`]'s item.
pub const SERVER_KIND: &str = "server";

/// A collected item with its bytes.
#[derive(Clone, Debug)]
pub struct OfferedItem {
    /// The item (its `sha256` and `size` are of `bytes`).
    pub item: SnapshotItem,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

impl OfferedItem {
    /// An item of `bytes`.
    pub fn new(
        key: String,
        kind: String,
        id: Option<String>,
        name: Option<String>,
        schema_version: u32,
        bytes: Vec<u8>,
    ) -> OfferedItem {
        OfferedItem {
            item: SnapshotItem {
                key,
                kind,
                id,
                name,
                schema_version,
                sha256: sha256(&bytes),
                size: bytes.len() as u64,
            },
            bytes,
        }
    }
}

/// The server's identity as `machine/server.json` holds it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ServerIdentity {
    /// Its ID.
    pub id: uuid::Uuid,
    /// Its name.
    pub name: String,
}

/// The kind of a snapshot key that is one of the box's settings (`None`: it isn't).
pub fn key_kind(key: &str) -> Option<String> {
    if key == SERVER_KEY {
        return Some(SERVER_KIND.into());
    }
    let mut parts = key.split('/');
    let (section, kind, file) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let kinds = match section {
        "portable" => BOX_KINDS,
        "machine" => MACHINE_KINDS,
        _ => return None,
    };
    if !kinds.iter().any(|k| kind_dir(*k) == kind) {
        return None;
    }
    let stem = file.strip_suffix(".json")?;
    let ok = !stem.is_empty()
        && !stem.starts_with('.')
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| kind.to_string())
}

/// Collects this box's settings from the data folder `data` (sorted by key). A file that
/// isn't a JSON object is left out (with a warning).
pub fn collect(data: &Path) -> anyhow::Result<Vec<OfferedItem>> {
    let mut out = Vec::new();
    for (section, kinds) in [("portable", BOX_KINDS), ("machine", MACHINE_KINDS)] {
        for kind in kinds {
            let kind = kind_dir(*kind);
            let dir = data.join(section).join(&kind);
            let rd = match std::fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => anyhow::bail!("reading {}: {e}", dir.display()),
            };
            for e in rd.flatten() {
                let file = e.file_name().to_string_lossy().to_string();
                let key = format!("{section}/{kind}/{file}");
                if key_kind(&key).is_none() || !e.path().is_file() {
                    continue;
                }
                let bytes = std::fs::read(e.path())?;
                let Ok(Value::Object(doc)) = serde_json::from_slice::<Value>(&bytes) else {
                    tracing::warn!(file = %e.path().display(), "settings backup: not a JSON document, left out");
                    continue;
                };
                let name = doc
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|n| !n.is_empty())
                    .map(str::to_string);
                let schema = doc
                    .get("schemaVersion")
                    .and_then(Value::as_u64)
                    .unwrap_or(1) as u32;
                let id = file.strip_suffix(".json").map(str::to_string);
                out.push(OfferedItem::new(key, kind.clone(), id, name, schema, bytes));
            }
        }
    }
    let path = data.join(SERVER_KEY);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let identity: ServerIdentity = serde_json::from_slice(&bytes)
                .map_err(|e| anyhow::anyhow!("{} is not valid: {e}", path.display()))?;
            out.push(OfferedItem::new(
                SERVER_KEY.into(),
                SERVER_KIND.into(),
                None,
                Some(identity.name),
                1,
                bytes,
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => anyhow::bail!("reading {}: {e}", path.display()),
    }
    out.sort_by(|a, b| a.item.key.cmp(&b.item.key));
    Ok(out)
}

/// One file a restore writes (`bytes`) or removes (`None`).
#[derive(Clone, Debug)]
pub struct RestoreWrite {
    /// The item's key (its path in the data folder).
    pub key: String,
    /// Its bytes from the snapshot; `None` removes it.
    pub bytes: Option<Vec<u8>>,
}

/// What a box applies to restore a snapshot (made by the Codex:
/// [`super::Backups::prepare_restore`]; applied by the box: [`apply`]).
#[derive(Clone, Debug, Default)]
pub struct RestorePackage {
    /// The files to write or remove.
    pub writes: Vec<RestoreWrite>,
    /// A replacement box takes over the old box's server ID (checked by the Codex: the old box
    /// was removed from the site).
    pub adopt_server_id: Option<uuid::Uuid>,
}

/// What [`apply`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Files written or removed.
    pub files: usize,
    /// Documents the running server reloads (`reloadData`).
    pub reload: bool,
    /// What takes effect only after a restart: `server name`, `server ID`, `font
    /// substitutions`.
    pub restart: Vec<String>,
}

/// Writes a restore into the data folder `data`: every key is checked first (only the box's
/// settings, never over a document from a newer schema), then each file is written atomically.
/// The server's ID stays unless the package adopts one; device identities are never touched.
pub fn apply(data: &Path, package: &RestorePackage) -> anyhow::Result<Applied> {
    let server_path = data.join(SERVER_KEY);
    let current: Option<ServerIdentity> = std::fs::read(&server_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    // Check everything before writing anything.
    let mut planned: Vec<(std::path::PathBuf, Option<Vec<u8>>)> = Vec::new();
    let mut applied = Applied::default();
    let mut server: Option<ServerIdentity> = None;
    for w in &package.writes {
        let Some(kind) = key_kind(&w.key) else {
            anyhow::bail!("{:?} is not one of this box's settings", w.key);
        };
        if w.key == SERVER_KEY {
            let Some(bytes) = &w.bytes else {
                continue; // the server's identity is never removed
            };
            let snap: ServerIdentity = serde_json::from_slice(bytes)
                .map_err(|e| anyhow::anyhow!("the snapshot's server.json is not valid: {e}"))?;
            server = Some(snap);
            continue;
        }
        let path = data.join(&w.key);
        if let Err(e) = crate::store::check_writable(&path) {
            anyhow::bail!("{e}");
        }
        if let Some(b) = &w.bytes {
            serde_json::from_slice::<serde_json::Map<String, Value>>(b)
                .map_err(|e| anyhow::anyhow!("{} in the snapshot is not valid: {e}", w.key))?;
        }
        if kind == kind_dir(DocumentKind::FontSubstitutionTable) {
            push_once(&mut applied.restart, "font substitutions");
        } else {
            applied.reload = true;
        }
        planned.push((path, w.bytes.clone()));
    }
    // The server's identity: the snapshot's name, this box's own ID unless one is adopted.
    if server.is_some() || package.adopt_server_id.is_some() {
        let own = current.clone();
        let id = package
            .adopt_server_id
            .or(own.as_ref().map(|c| c.id))
            .ok_or_else(|| anyhow::anyhow!("{} is missing", server_path.display()))?;
        let name = server
            .map(|s| s.name)
            .or(own.as_ref().map(|c| c.name.clone()))
            .unwrap_or_default();
        let changed_name = own.as_ref().is_none_or(|c| c.name != name);
        let changed_id = own.as_ref().is_none_or(|c| c.id != id);
        if changed_name {
            push_once(&mut applied.restart, "server name");
        }
        if changed_id {
            push_once(&mut applied.restart, "server ID");
        }
        if changed_name || changed_id {
            let bytes = serde_json::to_vec_pretty(&ServerIdentity { id, name })?;
            planned.push((server_path, Some(bytes)));
        }
    }
    // What each file holds now, to put back should a write fail part-way: all of it or
    // nothing is restored.
    let mut was: Vec<Option<Vec<u8>>> = Vec::with_capacity(planned.len());
    for (path, _) in &planned {
        was.push(match std::fs::read(path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => anyhow::bail!("reading {}: {e}", path.display()),
        });
    }
    for (i, (path, bytes)) in planned.iter().enumerate() {
        if let Err(e) = put(path, bytes.as_deref()) {
            let mut left = Vec::new();
            for (j, (p, _)) in planned.iter().enumerate().take(i + 1).rev() {
                if std::fs::read(p).ok() == was[j] {
                    continue; // (as it was: the failed one, mostly)
                }
                if let Err(e) = put(p, was[j].as_deref()) {
                    tracing::error!(file = %p.display(), "putting a file back after a failed restore: {e:#}");
                    left.push(p.display().to_string());
                }
            }
            if left.is_empty() {
                return Err(e.context("nothing was restored"));
            }
            return Err(e.context(PartlyRestored(left)));
        }
        applied.files += 1;
    }
    Ok(applied)
}

/// A restore that failed part-way and whose files couldn't all be put back (their paths): they
/// hold the snapshot's settings, the rest the box's own.
#[derive(Debug)]
pub struct PartlyRestored(pub Vec<String>);

impl std::fmt::Display for PartlyRestored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "these files couldn't be put back and hold the snapshot's settings: {}",
            self.0.join(", ")
        )
    }
}

/// Writes `bytes` to `path`, or removes it (`None`; already gone is fine).
fn put(path: &Path, bytes: Option<&[u8]>) -> anyhow::Result<()> {
    match bytes {
        Some(b) => crate::store::write_atomic(path, b),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                anyhow::bail!("removing {}: {e}", path.display())
            }
            _ => Ok(()),
        },
    }
}

fn push_once(list: &mut Vec<String>, what: &str) {
    if !list.iter().any(|x| x == what) {
        list.push(what.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn put(dir: &Path, key: &str, v: Value) {
        let p = dir.join(key);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    }

    fn data() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("backup-vesper-{}", uuid::Uuid::new_v4()));
        let s = uuid::Uuid::new_v4();
        put(
            &d,
            &format!("portable/screen/{s}.json"),
            json!({"schemaVersion": 2, "id": s, "name": "Audience"}),
        );
        put(
            &d,
            "portable/showSettings/00000000-0000-0000-0000-000000000000.json",
            json!({"schemaVersion": 2}),
        );
        put(
            &d,
            "portable/presentation/x.json",
            json!({"name": "a song"}),
        );
        put(
            &d,
            "machine/outputBinding/o1.json",
            json!({"schemaVersion": 2, "name": "Main out"}),
        );
        put(
            &d,
            "machine/server.json",
            json!({"id": uuid::Uuid::new_v4(), "name": "Booth"}),
        );
        put(&d, "machine/codex.json", json!({"role": "host"}));
        put(&d, "machine/clients.json", json!({"clients": []}));
        put(&d, "machine/site.json", json!({"pin": "hash"}));
        put(&d, "machine/site-member/creds.json", json!({"secret": 1}));
        put(&d, "machine/codex-member/creds.json", json!({"secret": 1}));
        put(&d, "cache/thumbnails/x.json", json!({}));
        std::fs::write(d.join("portable/screen/broken.json"), b"{not json").unwrap();
        d
    }

    #[test]
    fn collects_only_the_box_settings() {
        let d = data();
        let items = collect(&d).unwrap();
        let keys: Vec<&str> = items.iter().map(|i| i.item.key.as_str()).collect();
        assert_eq!(keys.len(), 4, "{keys:?}");
        assert!(keys.iter().any(|k| k.starts_with("portable/screen/")));
        assert!(keys.contains(&"portable/showSettings/00000000-0000-0000-0000-000000000000.json"));
        assert!(keys.contains(&"machine/outputBinding/o1.json"));
        assert!(keys.contains(&SERVER_KEY));
        let screen = items.iter().find(|i| i.item.kind == "screen").unwrap();
        assert_eq!(screen.item.name.as_deref(), Some("Audience"));
        assert_eq!(screen.item.schema_version, 2);
        assert_eq!(screen.item.sha256, sha256(&screen.bytes));
        let server = items.iter().find(|i| i.item.key == SERVER_KEY).unwrap();
        assert_eq!(server.item.name.as_deref(), Some("Booth"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn keys_are_checked() {
        assert_eq!(key_kind("portable/look/abc.json").as_deref(), Some("look"));
        assert_eq!(key_kind(SERVER_KEY).as_deref(), Some("server"));
        for bad in [
            "portable/presentation/abc.json",
            "machine/codex.json",
            "machine/clients.json",
            "machine/site.json",
            "machine/site-member/x.json",
            "machine/outputBinding/../server.json",
            "portable/look/.hidden.json",
            "portable/look/a/b.json",
            "portable/look/a.txt",
            "/etc/passwd",
        ] {
            assert!(key_kind(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn applies_writes_removals_and_the_server_name() {
        let d = data();
        let before: ServerIdentity =
            serde_json::from_slice(&std::fs::read(d.join(SERVER_KEY)).unwrap()).unwrap();
        let look = "portable/look/l1.json";
        let package = RestorePackage {
            writes: vec![
                RestoreWrite {
                    key: look.into(),
                    bytes: Some(br#"{"schemaVersion": 2, "name": "Plain"}"#.to_vec()),
                },
                RestoreWrite {
                    key: "machine/outputBinding/o1.json".into(),
                    bytes: None,
                },
                RestoreWrite {
                    key: SERVER_KEY.into(),
                    bytes: Some(
                        serde_json::to_vec(&json!({"id": uuid::Uuid::new_v4(), "name": "Stage"}))
                            .unwrap(),
                    ),
                },
            ],
            adopt_server_id: None,
        };
        let a = apply(&d, &package).unwrap();
        assert_eq!(a.files, 3);
        assert!(a.reload);
        assert_eq!(a.restart, vec!["server name".to_string()]);
        assert!(d.join(look).is_file());
        assert!(!d.join("machine/outputBinding/o1.json").exists());
        let after: ServerIdentity =
            serde_json::from_slice(&std::fs::read(d.join(SERVER_KEY)).unwrap()).unwrap();
        assert_eq!(after.id, before.id, "the box keeps its own ID");
        assert_eq!(after.name, "Stage");
        assert!(d.join("machine/site-member/creds.json").is_file());

        // A replacement takes over the old ID.
        let old = uuid::Uuid::new_v4();
        let a = apply(
            &d,
            &RestorePackage {
                writes: vec![],
                adopt_server_id: Some(old),
            },
        )
        .unwrap();
        assert_eq!(a.restart, vec!["server ID".to_string()]);
        let after: ServerIdentity =
            serde_json::from_slice(&std::fs::read(d.join(SERVER_KEY)).unwrap()).unwrap();
        assert_eq!((after.id, after.name.as_str()), (old, "Stage"));

        // Nothing outside the box's settings, and nothing at all when one key is wrong.
        let bad = RestorePackage {
            writes: vec![
                RestoreWrite {
                    key: "portable/look/l2.json".into(),
                    bytes: Some(b"{}".to_vec()),
                },
                RestoreWrite {
                    key: "machine/site-member/creds.json".into(),
                    bytes: None,
                },
            ],
            adopt_server_id: None,
        };
        assert!(apply(&d, &bad).is_err());
        assert!(!d.join("portable/look/l2.json").exists());
        assert!(d.join("machine/site-member/creds.json").is_file());

        // Never over a document from a newer schema.
        put(&d, "portable/look/new.json", json!({"schemaVersion": 999}));
        let r = apply(
            &d,
            &RestorePackage {
                writes: vec![RestoreWrite {
                    key: "portable/look/new.json".into(),
                    bytes: Some(b"{}".to_vec()),
                }],
                adopt_server_id: None,
            },
        );
        assert!(r.is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_write_failing_part_way_puts_the_earlier_ones_back() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = data();
        let screen = std::fs::read_dir(d.join("portable/screen"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let before = std::fs::read(&screen).unwrap();
        let key = |p: &Path| p.strip_prefix(&d).unwrap().to_string_lossy().to_string();
        put(&d, "portable/look/l1.json", json!({"schemaVersion": 2}));
        // The looks' folder can be read but not written: the third write fails.
        let looks = d.join("portable/look");
        std::fs::set_permissions(&looks, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(looks.join("probe"), b"").is_ok() {
            // (Run as root: permissions don't stop it; nothing to show here.)
            std::fs::set_permissions(&looks, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::remove_dir_all(&d).unwrap();
            return;
        }
        let new_screen = "portable/screen/00000000-0000-4000-8000-00000000000a.json";
        let r = apply(
            &d,
            &RestorePackage {
                writes: vec![
                    RestoreWrite {
                        key: key(&screen),
                        bytes: Some(br#"{"schemaVersion": 2, "name": "Restored"}"#.to_vec()),
                    },
                    RestoreWrite {
                        key: new_screen.into(),
                        bytes: Some(br#"{"schemaVersion": 2}"#.to_vec()),
                    },
                    RestoreWrite {
                        key: "portable/look/l1.json".into(),
                        bytes: Some(br#"{"schemaVersion": 2, "name": "Restored"}"#.to_vec()),
                    },
                ],
                adopt_server_id: None,
            },
        );
        std::fs::set_permissions(&looks, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = r.unwrap_err();
        assert!(e.downcast_ref::<PartlyRestored>().is_none(), "{e:#}");
        assert!(format!("{e:#}").contains("nothing was restored"), "{e:#}");
        assert_eq!(std::fs::read(&screen).unwrap(), before, "written back");
        assert!(!d.join(new_screen).exists(), "the new one removed again");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
