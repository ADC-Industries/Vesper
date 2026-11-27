//! Schema versioning: migrations on raw JSON, and the "never overwrite a newer document" rule.
//!
//! Every stored document carries `schemaVersion` (always written, even when it equals the
//! current version; a missing field means version 1). Storage runs [`migrate`] on the raw
//! `serde_json::Value` **before** deserializing, so old shapes never need Rust types, then
//! rewrites the row.
//!
//! Our enums decode values they do not know to an `Unknown` fallback (`#[serde(other)]`),
//! which is lossy. So a server must never load-and-resave a document written by a newer
//! schema: [`migrate`] fails with [`MigrationError::TooNew`], and storage calls
//! [`ensure_writable`] on the stored row before overwriting it.

use crate::{Document, DocumentKind, NEWEST_SCHEMA_VERSION, SCHEMA_VERSION};
use serde_json::Value;
use std::fmt;

/// Serde default for `schemaVersion` fields: documents written before versioning are v1.
pub(crate) fn schema_v1() -> u32 {
    1
}

/// One upgrade step: transforms a document of version `n` into version `n + 1` in place.
type Step = fn(DocumentKind, &mut Value) -> Result<(), String>;

/// `STEPS[i]` upgrades version `i + 1` to `i + 2`. Add one entry per schema bump.
const STEPS: &[Step] = &[v1_ndi_to_idn, v2_library_locations];

/// 1 → 2: the official NDI® runtime was removed; IDN Open Protocol (compatible with NDI®) is
/// the only network video transport. An output binding's `transport` and a video / audio
/// input binding's `source` of type `ndi` become `idn` with the same fields (`sourceName`,
/// `groups`), so a church's configured outputs keep their source names and groups. Other
/// kinds are unchanged.
fn v1_ndi_to_idn(kind: DocumentKind, doc: &mut Value) -> Result<(), String> {
    let field = match kind {
        DocumentKind::OutputBinding => "transport",
        DocumentKind::InputBinding | DocumentKind::AudioInputBinding => "source",
        _ => return Ok(()),
    };
    if let Some(t) = doc.get_mut(field).and_then(Value::as_object_mut)
        && t.get("type").and_then(Value::as_str) == Some("ndi")
    {
        t.insert("type".into(), Value::from("idn"));
    }
    Ok(())
}

/// 2 → 3: media assets may have a `library` location (the Codex's media library). Nothing in
/// a version 2 document changes.
fn v2_library_locations(_kind: DocumentKind, _doc: &mut Value) -> Result<(), String> {
    Ok(())
}

/// The schema version a document is written with: [`NEWEST_SCHEMA_VERSION`] (3) when it holds
/// a media library location (a `location` of type `library`, its own or, in a trash entry,
/// its documents'; a relink's record of an earlier one doesn't count), else
/// [`SCHEMA_VERSION`].
pub fn version_for(doc: &Value) -> u32 {
    fn has_library(v: &Value) -> bool {
        match v {
            Value::Object(m) => m.iter().any(|(k, x)| {
                if k == "location" {
                    x.get("type").and_then(Value::as_str) == Some("library") || has_library(x)
                } else {
                    k != "relinks" && has_library(x)
                }
            }),
            Value::Array(a) => a.iter().any(has_library),
            _ => false,
        }
    }
    if has_library(doc) {
        NEWEST_SCHEMA_VERSION
    } else {
        SCHEMA_VERSION
    }
}

const _: () = assert!(
    STEPS.len() as u32 + 1 == NEWEST_SCHEMA_VERSION,
    "one migration step per bump"
);

/// Why a document could not be migrated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationError {
    /// The document is not a JSON object.
    NotAnObject,
    /// `schemaVersion` is not a positive integer.
    InvalidVersion,
    /// Written by a newer schema than this build supports: refuse to load or overwrite it.
    TooNew {
        /// Version found in the document.
        found: u32,
        /// Newest version this build supports ([`NEWEST_SCHEMA_VERSION`]).
        supported: u32,
    },
    /// The document kind is unknown to this build.
    UnknownKind,
    /// A migration step failed.
    Step {
        /// Version being upgraded from.
        from: u32,
        /// Reason.
        reason: String,
    },
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrationError::NotAnObject => write!(f, "document is not a JSON object"),
            MigrationError::InvalidVersion => write!(f, "schemaVersion is not a positive integer"),
            MigrationError::TooNew { found, supported } => write!(
                f,
                "document schema {found} is newer than supported {supported}; refusing to load or overwrite it"
            ),
            MigrationError::UnknownKind => write!(f, "unknown document kind"),
            MigrationError::Step { from, reason } => {
                write!(f, "migration from schema {from} failed: {reason}")
            }
        }
    }
}

impl std::error::Error for MigrationError {}

/// Reads `schemaVersion` (missing = 1).
fn version_of(doc: &Value) -> Result<u32, MigrationError> {
    let obj = doc.as_object().ok_or(MigrationError::NotAnObject)?;
    match obj.get("schemaVersion") {
        None => Ok(1),
        Some(v) => v
            .as_u64()
            .filter(|&n| n >= 1 && n <= u32::MAX as u64)
            .map(|n| n as u32)
            .ok_or(MigrationError::InvalidVersion),
    }
}

/// Upgrades `doc` (a stored document of `kind`) to [`NEWEST_SCHEMA_VERSION`] in place and sets
/// its `schemaVersion` to the version it is written with ([`version_for`]). Fails with
/// [`MigrationError::TooNew`] for documents from a newer schema.
pub fn migrate(kind: DocumentKind, doc: &mut Value) -> Result<(), MigrationError> {
    if kind == DocumentKind::Unknown {
        return Err(MigrationError::UnknownKind);
    }
    let found = version_of(doc)?;
    if found > NEWEST_SCHEMA_VERSION {
        return Err(MigrationError::TooNew {
            found,
            supported: NEWEST_SCHEMA_VERSION,
        });
    }
    for from in found..NEWEST_SCHEMA_VERSION {
        STEPS[(from - 1) as usize](kind, doc)
            .map_err(|reason| MigrationError::Step { from, reason })?;
    }
    doc["schemaVersion"] = Value::from(version_for(doc));
    Ok(())
}

/// Checks that the currently stored version of a document may be overwritten by this build:
/// fails with [`MigrationError::TooNew`] when it was written by a newer schema.
pub fn ensure_writable(stored: &Value) -> Result<(), MigrationError> {
    let found = version_of(stored)?;
    if found > NEWEST_SCHEMA_VERSION {
        return Err(MigrationError::TooNew {
            found,
            supported: NEWEST_SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Why a stored document could not be decoded.
#[derive(Debug)]
pub enum DecodeError {
    /// Migration failed (including [`MigrationError::TooNew`]).
    Migration(MigrationError),
    /// The migrated JSON does not match the Rust type.
    Json(serde_json::Error),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Migration(e) => e.fmt(f),
            DecodeError::Json(e) => write!(f, "invalid document: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Migrates and deserializes a stored document.
pub fn decode<T: Document>(mut doc: Value) -> Result<T, DecodeError> {
    migrate(T::KIND, &mut doc).map_err(DecodeError::Migration)?;
    serde_json::from_value(doc).map_err(DecodeError::Json)
}
