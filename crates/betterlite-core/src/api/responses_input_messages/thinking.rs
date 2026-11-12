use serde_json::Value;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_input_content::is_truthy;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThinkingError {
    #[error("the serialized thinking blocks are {observed} bytes; the maximum is {limit}")]
    TooLarge { observed: usize, limit: usize },
}

// LiteLLM writes this field itself as a JSON array of the provider's signed blocks, and falls back to none on anything else
pub fn decode_thinking_blocks(
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<Option<Vec<Value>>, ThinkingError> {
    if bytes.len() > limits.max_json_bytes {
        return Err(ThinkingError::TooLarge {
            observed: bytes.len(),
            limit: limits.max_json_bytes,
        });
    }
    let Ok(Value::Array(blocks)) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(None);
    };
    let replayable: Vec<Value> = blocks
        .into_iter()
        .filter(is_replayable_thinking_block)
        .collect();
    Ok((!replayable.is_empty()).then_some(replayable))
}

#[must_use]
pub fn is_replayable_thinking_block(block: &Value) -> bool {
    let Value::Object(fields) = block else {
        return false;
    };
    match fields.get("type").and_then(Value::as_str) {
        Some("thinking") => is_truthy(fields.get("signature")),
        Some("redacted_thinking") => is_truthy(fields.get("data")),
        _ => false,
    }
}
