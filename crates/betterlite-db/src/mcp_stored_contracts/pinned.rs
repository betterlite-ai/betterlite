use std::collections::BTreeMap;

use super::{JsonValue, MCPUpstreamProtocol, PinnedMCPTool};

pub fn parse_pinned_tools(
    value: JsonValue,
) -> Result<Option<BTreeMap<String, PinnedMCPTool>>, serde_json::Error> {
    let decoded = match value {
        JsonValue::String(raw) if !raw.is_empty() => serde_json::from_str(&raw)?,
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
    let encoded = serde_json::to_string(&decoded)?;
    serde_json::from_str(&encoded).map(Some)
}

pub fn validate_mcp_protocol_metadata(
    metadata: BTreeMap<String, JsonValue>,
) -> Result<BTreeMap<String, JsonValue>, serde_json::Error> {
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
