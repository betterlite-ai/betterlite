use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireDecodeLimits {
    pub max_json_bytes: usize,
    pub max_event_bytes: usize,
    pub max_items: usize,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ShapeDecodeError {
    #[error("JSON exceeds byte limit")]
    JsonTooLarge,
    #[error("JSON item limit exceeded")]
    TooManyItems,
    #[error("invalid JSON: {0}")]
    Json(String),
}

pub fn decode_responses_shapes(
    bytes: &[u8],
    limits: WireDecodeLimits,
) -> Result<Value, ShapeDecodeError> {
    if bytes.len() > limits.max_json_bytes || bytes.len() > limits.max_event_bytes {
        return Err(ShapeDecodeError::JsonTooLarge);
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| ShapeDecodeError::Json(error.to_string()))?;
    if count_items(&value, limits.max_items) > limits.max_items {
        return Err(ShapeDecodeError::TooManyItems);
    }
    Ok(value)
}

fn count_items(value: &Value, cap: usize) -> usize {
    match value {
        Value::Array(items) => items.iter().fold(items.len(), |count, item| {
            count
                .saturating_add(count_items(item, cap))
                .min(cap.saturating_add(1))
        }),
        Value::Object(items) => items.values().fold(items.len(), |count, item| {
            count
                .saturating_add(count_items(item, cap))
                .min(cap.saturating_add(1))
        }),
        _ => 0,
    }
}
