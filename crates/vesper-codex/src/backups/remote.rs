//! Show boxes' settings backups and restores over the box-to-Codex link ([`crate::link`]).
//!
//! **The show box asks**: restores wait in the Codex's queue ([`super::restores`]) until the
//! box asks for them. The Codex tells the box at once when it queues one
//! ([`crate::link::RESTORE_QUEUED`]); the box also asks at each poll (the fallback).
//!
//! | Kind | Request | Answer |
//! |---|---|---|
//! | [`BACKUP_OFFER`] | [`OfferWire`]: the box's settings ([`super::this_box_offer`]) | `SnapshotTaken` |
//! | [`PENDING_RESTORE`] | [`PendingRequest`] `{v}` | [`PendingAnswer`]: the oldest restore queued for the asking device, if any |
//! | [`RESTORE_BEGIN`] | [`BeginRequest`] `{v, id, offer}`: the box's `beforeRestore` offer | [`BeginAnswer`]: the files to write or remove, the server ID to take over, the undo snapshot |
//! | [`RESTORE_RESULT`] | [`ResultRequest`] `{v, id, ok, error?, restart}` | `{}` |
//!
//! The Codex checks every offer against the device it came from ([`super::Backups`]'s
//! `accept_from_box`): a Vesper offer only; never the Codex box's own server ID; a machine whose
//! set was offered by another device of the site is refused unless the site lists that device
//! as removed. Restores go only to the device whose set names the target box. Errors come back
//! as `Reply::Error` holding the `ApiError` as JSON ([`error_reply`], [`error_of`]).

use super::{OfferedItem, RestorePackage, RestoreWrite, SnapshotOffer};
use model::{BackupSetId, ChangeBy, SnapshotId, SnapshotItem, SnapshotReason};
use protocol::{ApiError, ErrorCode, SnapshotSummary};
use serde::{Deserialize, Serialize};

/// `vesper.codex.backup_offer` (show box → Codex).
pub const BACKUP_OFFER: &str = "vesper.codex.backup_offer";
/// `vesper.codex.pending_restore` (show box → Codex).
pub const PENDING_RESTORE: &str = "vesper.codex.pending_restore";
/// `vesper.codex.restore_begin` (show box → Codex).
pub const RESTORE_BEGIN: &str = "vesper.codex.restore_begin";
/// `vesper.codex.restore_result` (show box → Codex).
pub const RESTORE_RESULT: &str = "vesper.codex.restore_result";

fn v1() -> u32 {
    crate::link::VERSION
}

/// An `ApiError` as a channel error answer (JSON, so the box gets the code and field back).
pub fn error_reply(e: &ApiError) -> idn_open_protocol::channel::Reply {
    idn_open_protocol::channel::Reply::Error(
        serde_json::to_string(e).unwrap_or_else(|_| e.message.clone()),
    )
}

/// The `ApiError` of an error answer (`kind`'s), or one saying the Codex answered `text`.
pub fn error_of(kind: &str, text: &str) -> ApiError {
    serde_json::from_str::<ApiError>(text)
        .unwrap_or_else(|_| ApiError::new(ErrorCode::Unavailable, format!("{kind}: {text}")))
}

/// One item of an offer, its bytes as text (settings are JSON documents).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemWire {
    /// The item.
    pub item: SnapshotItem,
    /// Its file.
    pub body: String,
}

/// [`BACKUP_OFFER`]'s request: a [`SnapshotOffer`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferWire {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// `vesper`.
    pub product: String,
    /// The box's server ID.
    pub machine: String,
    /// Its name.
    pub name: String,
    /// Its fingerprint in the site (checked against the device the request came from).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// Why.
    pub reason: SnapshotReason,
    /// Who asked.
    pub by: ChangeBy,
    /// The box's version.
    #[serde(default)]
    pub version: String,
    /// The items.
    pub items: Vec<ItemWire>,
}

impl OfferWire {
    /// An offer for the link (an item that isn't UTF-8 text can't travel: an error).
    pub fn from_offer(o: &SnapshotOffer) -> Result<OfferWire, String> {
        let items = o
            .items
            .iter()
            .map(|i| {
                Ok(ItemWire {
                    item: i.item.clone(),
                    body: String::from_utf8(i.bytes.clone())
                        .map_err(|_| format!("{} is not UTF-8 text", i.item.key))?,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(OfferWire {
            v: crate::link::VERSION,
            product: o.product.clone(),
            machine: o.machine.clone(),
            name: o.name.clone(),
            fingerprint: o.fingerprint.clone(),
            reason: o.reason,
            by: o.by.clone(),
            version: o.version.clone(),
            items,
        })
    }

    /// The offer it carries (its items are checked by `Backups::accept`).
    pub fn into_offer(self) -> SnapshotOffer {
        SnapshotOffer {
            product: self.product,
            machine: self.machine,
            name: self.name,
            fingerprint: self.fingerprint,
            reason: self.reason,
            by: self.by,
            version: self.version,
            items: self
                .items
                .into_iter()
                .map(|i| OfferedItem {
                    item: i.item,
                    bytes: i.body.into_bytes(),
                })
                .collect(),
        }
    }
}

/// [`PENDING_RESTORE`]'s request.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
}

/// [`PENDING_RESTORE`]'s answer.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAnswer {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The oldest restore queued for the asking box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore: Option<PendingRestore>,
}

/// A restore waiting for the box.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRestore {
    /// The queued restore's ID.
    pub id: String,
    /// The set restored.
    pub set: BackupSetId,
    /// The snapshot restored.
    pub snapshot: SnapshotId,
    /// Who asked for it (the box's `beforeRestore` snapshot names them).
    pub by: ChangeBy,
}

/// [`RESTORE_BEGIN`]'s request: the box takes the restore, with its settings now.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The queued restore.
    pub id: String,
    /// The box's settings now: its `beforeRestore` snapshot (the undo).
    pub offer: OfferWire,
}

/// One file a restore writes (`body`) or removes (`null`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteWire {
    /// The item's key (its path in the data folder; checked again by the box).
    pub key: String,
    /// Its file from the snapshot; `None` removes it.
    #[serde(default)]
    pub body: Option<String>,
}

/// [`RESTORE_BEGIN`]'s answer: what the box applies ([`super::apply_here`]).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginAnswer {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The files.
    pub writes: Vec<WriteWire>,
    /// The server ID a replacement box takes over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopt_server_id: Option<uuid::Uuid>,
    /// The box's `beforeRestore` snapshot.
    pub before_restore: SnapshotSummary,
}

impl BeginAnswer {
    /// The answer for a package (a file that isn't UTF-8 text can't travel: an error).
    pub fn new(p: &RestorePackage, before: SnapshotSummary) -> Result<BeginAnswer, ApiError> {
        let writes = p
            .writes
            .iter()
            .map(|w| {
                Ok(WriteWire {
                    key: w.key.clone(),
                    body: match &w.bytes {
                        None => None,
                        Some(b) => Some(String::from_utf8(b.clone()).map_err(|_| {
                            ApiError::new(
                                ErrorCode::Internal,
                                format!("the snapshot's copy of {} is not UTF-8 text", w.key),
                            )
                        })?),
                    },
                })
            })
            .collect::<Result<_, ApiError>>()?;
        Ok(BeginAnswer {
            v: crate::link::VERSION,
            writes,
            adopt_server_id: p.adopt_server_id,
            before_restore: before,
        })
    }

    /// The package the box applies.
    pub fn package(&self) -> RestorePackage {
        RestorePackage {
            writes: self
                .writes
                .iter()
                .map(|w| RestoreWrite {
                    key: w.key.clone(),
                    bytes: w.body.as_ref().map(|b| b.clone().into_bytes()),
                })
                .collect(),
            adopt_server_id: self.adopt_server_id,
        }
    }
}

/// [`RESTORE_RESULT`]'s request: how applying went on the box.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The queued restore.
    pub id: String,
    /// Applied.
    pub ok: bool,
    /// Why not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What takes effect after the box restarts.
    #[serde(default)]
    pub restart: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_and_packages_travel_as_text() {
        let o = SnapshotOffer {
            product: "vesper".into(),
            machine: "m".into(),
            name: "Box".into(),
            fingerprint: Some("fp".into()),
            reason: SnapshotReason::Change,
            by: ChangeBy::default(),
            version: "1".into(),
            items: vec![OfferedItem::new(
                "portable/look/a.json".into(),
                "look".into(),
                Some("a".into()),
                Some("Plain".into()),
                2,
                br#"{"name":"Plain"}"#.to_vec(),
            )],
        };
        let w: OfferWire = serde_json::from_slice(
            &serde_json::to_vec(&OfferWire::from_offer(&o).unwrap()).unwrap(),
        )
        .unwrap();
        let back = w.into_offer();
        assert_eq!(back.items[0].bytes, o.items[0].bytes);
        assert_eq!(back.items[0].item, o.items[0].item);
        let mut bad = o.clone();
        bad.items[0].bytes = vec![0xff, 0xfe];
        assert!(OfferWire::from_offer(&bad).is_err());

        let e = ApiError::invalid("items", "nope");
        let idn_open_protocol::channel::Reply::Error(text) = error_reply(&e) else {
            panic!()
        };
        assert_eq!(error_of("k", &text), e);
        assert_eq!(error_of("k", "plain").code, ErrorCode::Unavailable);
    }
}
