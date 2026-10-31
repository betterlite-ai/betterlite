use std::collections::BTreeMap;

use super::decode::{McpStoredDecodeError, decode_mcp_stored_contract};
use super::{JsonValue, MCPUpstreamProtocol, PinnedMCPTool, ToolDefinition};

#[derive(Debug, thiserror::Error)]
pub enum PinnedError {
    #[error("invalid stored pinned tools")]
    Decode(#[from] McpStoredDecodeError),
    #[error("invalid MCP record")]
    Validation(#[from] serde_json::Error),
    #[error("invalid pinned tool record")]
    InvalidToolRecord,
}

pub fn parse_pinned_tools(
    value: JsonValue,
) -> Result<Option<BTreeMap<String, PinnedMCPTool>>, PinnedError> {
    let decoded = match value {
        JsonValue::String(raw) if !raw.is_empty() => decode_mcp_stored_contract(raw.as_bytes())?,
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)) => other,
    };
    if is_falsey(&decoded) {
        return Ok(None);
    }
    let JsonValue::Object(tools) = decoded else {
        return Err(PinnedError::InvalidToolRecord);
    };
    let mut parsed = BTreeMap::new();
    for (name, record) in tools {
        parsed.insert(name, PinnedMCPTool(tool_definition_from_json(record)?));
    }
    Ok(Some(parsed))
}

fn tool_definition_from_json(value: JsonValue) -> Result<ToolDefinition, PinnedError> {
    let JsonValue::Object(mut fields) = value else {
        return Err(PinnedError::InvalidToolRecord);
    };
    let description = match fields.remove("description") {
        None => String::new(),
        Some(JsonValue::String(text)) => text,
        Some(_) => return Err(PinnedError::InvalidToolRecord),
    };
    let input_schema = match fields.remove("input_schema") {
        None => BTreeMap::new(),
        Some(JsonValue::Object(schema)) => schema,
        Some(_) => return Err(PinnedError::InvalidToolRecord),
    };
    if !fields.is_empty() {
        return Err(PinnedError::InvalidToolRecord);
    }
    Ok(ToolDefinition {
        description,
        input_schema,
    })
}

pub fn validate_mcp_protocol_metadata(
    metadata: BTreeMap<String, JsonValue>,
) -> Result<BTreeMap<String, JsonValue>, PinnedError> {
    if let Some(protocol) = metadata.get("protocol_version") {
        let encoded = serde_json::to_string(protocol)?;
        serde_json::from_str::<MCPUpstreamProtocol>(&encoded)?;
    }
    Ok(metadata)
}

fn is_falsey(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => true,
        JsonValue::Bool(flag) => !flag,
        JsonValue::Number(number) => number.as_f64() == Some(0.0),
        JsonValue::String(text) => text.is_empty(),
        JsonValue::Array(items) => items.is_empty(),
        JsonValue::Object(items) => items.is_empty(),
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(try_from = "BTreeMap<String, JsonValue>")]
pub struct MCPInfo(BTreeMap<String, JsonValue>);

impl TryFrom<BTreeMap<String, JsonValue>> for MCPInfo {
    type Error = PinnedError;

    fn try_from(metadata: BTreeMap<String, JsonValue>) -> Result<Self, Self::Error> {
        validate_mcp_protocol_metadata(metadata).map(Self)
    }
}
