use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod tests;

#[derive(Debug, Error)]
pub enum CursorSource {
    #[error(transparent)]
    Base64(base64::DecodeError),
    #[error(transparent)]
    Json(serde_json::Error),
}

#[derive(Debug, Error)]
pub enum CursorError {
    #[error("Invalid trace cursor")]
    InvalidTrace(#[source] Option<CursorSource>),
    #[error("Invalid diagnostic cursor")]
    InvalidDiagnostic(#[source] Option<CursorSource>),
}

#[derive(Deserialize, Serialize)]
struct TraceCursorPayload(i64, String);

fn decode_payload<T>(
    cursor: Option<&str>,
    make_error: fn(Option<CursorSource>) -> CursorError,
) -> Result<Option<T>, CursorError>
where
    T: DeserializeOwned,
{
    let Some(raw) = cursor.filter(|candidate| !candidate.is_empty()) else {
        return Ok(None);
    };
    let bytes = URL_SAFE
        .decode(raw)
        .map_err(|source| make_error(Some(CursorSource::Base64(source))))?;
    let payload: T = serde_json::from_slice(&bytes)
        .map_err(|source| make_error(Some(CursorSource::Json(source))))?;
    Ok(Some(payload))
}

#[must_use]
pub fn encode_trace_cursor(start_ms: i64, trace_id: &str) -> String {
    let payload = TraceCursorPayload(start_ms, trace_id.to_owned());
    URL_SAFE.encode(serde_json::to_string(&payload).unwrap_or_default())
}

pub fn decode_trace_cursor(cursor: Option<&str>) -> Result<(i64, String), CursorError> {
    let Some(payload) = decode_payload::<TraceCursorPayload>(cursor, CursorError::InvalidTrace)?
    else {
        return Ok((0, String::new()));
    };
    if payload.0 <= 0 || payload.1.is_empty() {
        return Err(CursorError::InvalidTrace(None));
    }
    Ok((payload.0, payload.1))
}

// The database's hash of a stored error message, 64 uppercase hex characters (SHA-256)
const VERSION_LENGTH: usize = 64;
const MAX_OFFSET: u64 = (1_u64 << 63) - 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ErrorCursor {
    pub offset: u64,
    pub version: String,
}

pub fn decode_error_cursor(cursor: Option<&str>) -> Result<Option<ErrorCursor>, CursorError> {
    let Some(parsed) = decode_payload::<ErrorCursor>(cursor, CursorError::InvalidDiagnostic)?
    else {
        return Ok(None);
    };
    if parsed.offset > MAX_OFFSET || !is_valid_version(&parsed.version) {
        return Err(CursorError::InvalidDiagnostic(None));
    }
    Ok(Some(parsed))
}

fn is_valid_version(version: &str) -> bool {
    let valid_chars = version
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (byte.is_ascii_uppercase() && byte <= b'F'));
    version.len() == VERSION_LENGTH && valid_chars
}
