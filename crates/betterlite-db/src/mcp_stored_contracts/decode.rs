use super::{JsonValue, MAX_STORED_BYTES, MAX_STORED_DEPTH};

#[derive(Debug, thiserror::Error)]
pub enum McpStoredDecodeError {
    #[error("stored MCP JSON exceeds {MAX_STORED_BYTES} bytes")]
    TooLarge,
    #[error("stored MCP JSON exceeds depth {MAX_STORED_DEPTH}")]
    TooDeep,
    #[error("stored MCP JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn decode_mcp_stored_contract(bytes: &[u8]) -> Result<JsonValue, McpStoredDecodeError> {
    if bytes.len() > MAX_STORED_BYTES {
        return Err(McpStoredDecodeError::TooLarge);
    }
    let value = serde_json::from_slice(bytes)?;
    ensure_depth(&value, 0)?;
    Ok(value)
}

fn ensure_depth(value: &JsonValue, depth: usize) -> Result<(), McpStoredDecodeError> {
    if depth > MAX_STORED_DEPTH {
        return Err(McpStoredDecodeError::TooDeep);
    }
    match value {
        JsonValue::Array(values) => values
            .iter()
            .try_for_each(|child| ensure_depth(child, depth.saturating_add(1))),
        JsonValue::Object(values) => values
            .values()
            .try_for_each(|child| ensure_depth(child, depth.saturating_add(1))),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => {
            Ok(())
        }
    }
}
