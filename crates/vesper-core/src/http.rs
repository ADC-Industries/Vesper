//! API errors as HTTP responses, and the request/response helpers every route module uses.
//! Errors are `{"error": {"code", "message"}}` with the code's HTTP status.

use crate::change::ConfigError;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use protocol::{ApiError, ErrorCode, ErrorResponse};
use serde::Serialize;

/// An API error as an HTTP response.
pub struct Error(pub ApiError);

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            axum::Json(ErrorResponse {
                error: self.0,
                current: None,
                codex: None,
            }),
        )
            .into_response()
    }
}

impl IntoResponse for ConfigError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.error.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            axum::Json(ErrorResponse {
                error: self.error,
                current: self.current,
                codex: None,
            }),
        )
            .into_response()
    }
}

impl From<ApiError> for Error {
    fn from(e: ApiError) -> Self {
        Error(e)
    }
}

/// A JSON answer or an API error.
pub type ApiResult<T> = Result<axum::Json<T>, Error>;

/// An API error with `code` and `msg`.
pub fn err(code: ErrorCode, msg: impl Into<String>) -> Error {
    Error(ApiError::new(code, msg))
}

/// `v` as a JSON answer.
pub fn ok<T: Serialize>(v: T) -> ApiResult<T> {
    Ok(axum::Json(v))
}

/// A JSON request body (`400` when it doesn't parse).
pub fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(body)
        .map_err(|e| err(ErrorCode::BadRequest, format!("invalid JSON: {e}")))
}

/// A query parameter (no percent-decoding needed for the values used here: numbers, tokens).
pub fn query_param<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    query?
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

/// The `If-Match` header, normalized ([`crate::change::parse_if_match`]).
pub fn if_match(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(crate::change::parse_if_match)
}

/// A JSON answer with `status` and the ETag of `revision`.
pub fn with_etag<T: Serialize>(status: StatusCode, revision: &str, body: T) -> Response {
    let etag = HeaderValue::from_str(&format!("\"{revision}\""))
        .unwrap_or(HeaderValue::from_static("\"\""));
    (status, [(header::ETAG, etag)], axum::Json(body)).into_response()
}
