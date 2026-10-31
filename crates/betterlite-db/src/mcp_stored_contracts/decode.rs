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
    check_depth(bytes)?;
    serde_json::from_slice(bytes).map_err(McpStoredDecodeError::Json)
}

fn check_depth(bytes: &[u8]) -> Result<(), McpStoredDecodeError> {
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0_u8;
    for &byte in bytes {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            b'[' | b'{' if !quoted => {
                depth = depth.saturating_add(1);
                if usize::from(depth) > MAX_STORED_DEPTH {
                    return Err(McpStoredDecodeError::TooDeep);
                }
            }
            b']' | b'}' if !quoted => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}
