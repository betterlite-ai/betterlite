use betterlite_models::decisions::Json;
use std::str::{Utf8Error, from_utf8};
use thiserror::Error;
#[derive(Debug, Error)]
pub enum LogEventError {
    #[error("the passthrough log event is not valid UTF-8")]
    NotUtf8(#[source] Utf8Error),
    #[error("the passthrough log event is not valid JSON")]
    NotJson(#[source] serde_json::Error),
    #[error("the passthrough log event is not a JSON object")]
    NotObject,
}
pub fn decode_passthrough_log_event(bytes: &[u8]) -> Result<Json, LogEventError> {
    let text = from_utf8(bytes).map_err(LogEventError::NotUtf8)?;
    let value: Json = serde_json::from_str(text).map_err(LogEventError::NotJson)?;
    if matches!(value, Json::Object(_)) {
        return Ok(value);
    }
    Err(LogEventError::NotObject)
}
