//! Document revisions and refused changes, shared by every part that edits documents (screens
//! and outputs, the site's settings and users, the Codex).

use protocol::ApiError;
use serde::Serialize;
use serde_json::Value;
use sha2::Digest;

/// A refused change: the error, and on a conflict the current document (`Value::Null` when
/// it was deleted).
#[derive(Clone, Debug)]
pub struct ConfigError {
    /// The error.
    pub error: ApiError,
    /// The current document (conflicts only).
    pub current: Option<Value>,
}

impl From<ApiError> for ConfigError {
    fn from(error: ApiError) -> Self {
        ConfigError {
            error,
            current: None,
        }
    }
}

/// Content revision of a document (16 hex digits of the SHA-256 of its JSON).
pub fn revision<T: Serialize>(doc: &T) -> String {
    let bytes = serde_json::to_vec(doc).unwrap_or_default();
    let d = sha2::Sha256::digest(&bytes);
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Checks `If-Match` (`if_match`; `*` or none: anything) against the current document: a
/// conflict with the current entry (`entry`), or `null` when it was deleted.
pub fn check_revision<T: Serialize>(
    current: Option<&T>,
    entry: impl Fn(&T) -> Value,
    if_match: &Option<String>,
    what: &str,
) -> Result<(), ConfigError> {
    let Some(want) = if_match.as_deref().filter(|w| *w != "*") else {
        return Ok(());
    };
    match current {
        Some(c) if revision(c) == want => Ok(()),
        Some(c) => Err(ConfigError {
            error: ApiError::new(
                protocol::ErrorCode::Conflict,
                format!("{what} was changed by another operator; reload it or overwrite"),
            ),
            current: Some(entry(c)),
        }),
        None => Err(ConfigError {
            error: ApiError::new(
                protocol::ErrorCode::Conflict,
                format!("{what} was deleted by another operator"),
            ),
            current: Some(Value::Null),
        }),
    }
}

/// Normalizes an `If-Match` header value (`"abc"`, `W/"abc"`, `abc`, `*`).
pub fn parse_if_match(h: &str) -> String {
    h.trim()
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_string()
}
