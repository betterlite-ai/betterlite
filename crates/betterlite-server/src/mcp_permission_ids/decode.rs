use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpToolGrantsJsonError {
    #[error("the stored MCP tool grants are not valid JSON")]
    Bounded(#[from] PrivateJsonError),
    #[error("the stored MCP tool grants could not be rebuilt as JSON")]
    Document(#[source] serde_json::Error),
    #[error("the stored MCP tool grants are not a JSON object")]
    NotAnObject,
}

// LiteLLM json.loads the stored column, so the text is bounded and must decode to a grant map
pub fn decode_mcp_tool_grants_json(
    bytes: &[u8],
) -> Result<BTreeMap<String, JsonData>, McpToolGrantsJsonError> {
    let loaded = safe_json_loads(bytes)?;
    let grants =
        serde_json::from_value::<JsonData>(loaded).map_err(McpToolGrantsJsonError::Document)?;
    let JsonData::Object(fields) = grants else {
        return Err(McpToolGrantsJsonError::NotAnObject);
    };
    Ok(fields)
}
