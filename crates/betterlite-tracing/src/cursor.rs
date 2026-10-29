use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod tests;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CursorError {
    #[error("Invalid trace cursor")]
    InvalidTrace,
    #[error("Invalid diagnostic cursor")]
    InvalidDiagnostic,
}

#[derive(Deserialize, Serialize)]
struct TraceCursorPayload(i64, String);

fn decode_payload<T>(cursor: Option<&str>, error: CursorError) -> Result<Option<T>, CursorError>
where
    T: DeserializeOwned,
{
    let Some(raw) = cursor.filter(|candidate| !candidate.is_empty()) else {
        return Ok(None);
    };
    let bytes = URL_SAFE.decode(raw).map_err(|_source| error.clone())?;
    let payload: T = serde_json::from_slice(&bytes).map_err(|_source| error)?;
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
        return Err(CursorError::InvalidTrace);
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

#[must_use]
pub fn encode_error_cursor(cursor: &ErrorCursor) -> String {
    let encoded = serde_json::to_string(cursor).unwrap_or_default();
    URL_SAFE.encode(encoded)
}

pub fn decode_error_cursor(cursor: Option<&str>) -> Result<Option<ErrorCursor>, CursorError> {
    let Some(parsed) = decode_payload::<ErrorCursor>(cursor, CursorError::InvalidDiagnostic)?
    else {
        return Ok(None);
    };
    if parsed.offset > MAX_OFFSET || !is_valid_version(&parsed.version) {
        return Err(CursorError::InvalidDiagnostic);
    }
    Ok(Some(parsed))
}

fn is_valid_version(version: &str) -> bool {
    let valid_chars = version
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (byte.is_ascii_uppercase() && byte <= b'F'));
    version.len() == VERSION_LENGTH && valid_chars
}
