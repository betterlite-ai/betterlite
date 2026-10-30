use serde_json::Value;
use std::str::{Utf8Error, from_utf8};

const MAX_ROW_BYTES: usize = 1_048_576;
const MAX_JSON_DEPTH: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum TeamRowError {
    #[error("stored row is {len} bytes, over the 1048576 byte limit")]
    TooLarge { len: usize },
    #[error("stored row is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("stored row exceeds JSON depth 64")]
    TooDeep,
    #[error("stored row is invalid JSON: {0}")]
    Json(#[source] serde_json::Error),
    #[error("{0}")]
    Validation(String),
    #[error("Field {0} should be a valid dictionary")]
    Field(String),
    #[error("missing required field {0}")]
    Missing(&'static str),
}

pub fn decode_team_rows(bytes: &[u8]) -> Result<Value, TeamRowError> {
    if bytes.len() > MAX_ROW_BYTES {
        return Err(TeamRowError::TooLarge { len: bytes.len() });
    }
    let text = from_utf8(bytes).map_err(TeamRowError::InvalidUtf8)?;
    let value: Value = serde_json::from_str(text).map_err(TeamRowError::Json)?;
    check_depth(&value, 0)?;
    Ok(value)
}

fn check_depth(value: &Value, depth: usize) -> Result<(), TeamRowError> {
    if depth > MAX_JSON_DEPTH {
        return Err(TeamRowError::TooDeep);
    }
    match value {
        Value::Array(items) => items
            .iter()
            .try_for_each(|item| check_depth(item, depth + 1)),
        Value::Object(items) => items
            .values()
            .try_for_each(|item| check_depth(item, depth + 1)),
        _ => Ok(()),
    }
}
