use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use serde::Deserialize;

mod tests;

#[derive(Deserialize)]
struct StoredAllowlistSwitch {
    #[serde(default, rename = "tool_allowlist_enforced")]
    enforced: bool,
}

#[must_use]
pub fn allowlist_enforced(mcp_info: Option<&MCPInfo>) -> bool {
    let Some(info) = mcp_info else {
        return false;
    };
    serde_json::to_value(info)
        .ok()
        .and_then(|value| serde_json::from_value::<StoredAllowlistSwitch>(value).ok())
        .is_some_and(|switch| switch.enforced)
}

#[must_use]
pub fn server_applies_tool_allowlist(
    mcp_info: Option<&MCPInfo>,
    allowed_tools: Option<&[String]>,
) -> bool {
    if allowlist_enforced(mcp_info) {
        return true;
    }
    allowed_tools.is_some_and(|tools| !tools.is_empty())
}

#[must_use]
pub fn tool_result_error_message(result: &JsonValue) -> Option<String> {
    let JsonValue::Object(fields) = result else {
        return None;
    };
    let flag = fields
        .get("isError")
        .and_then(|value| (!matches!(value, JsonValue::Null)).then_some(value))
        .or_else(|| fields.get("is_error"));
    if !matches!(flag, Some(JsonValue::Bool(true))) {
        return None;
    }
    if let Some(JsonValue::Array(items)) = fields.get("content") {
        for item in items {
            if let JsonValue::Object(entry) = item
                && let Some(JsonValue::String(text)) = entry.get("text")
                && !text.is_empty()
            {
                return Some(text.clone());
            }
        }
    }
    Some("MCP tool call returned isError=true".to_owned())
}
