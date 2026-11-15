use std::collections::BTreeMap;

use serde::Serialize;

use crate::mcp_stored_contracts::JsonValue;

use super::super::convert::tool::SearchTool;
use super::super::error::StoreError;

mod tests;

pub struct DumpedFields {
    pub litellm_params: String,
    pub search_tool_info: String,
}

// PostgreSQL text and jsonb columns reject NUL bytes, so safe_dumps strips them from keys and values
fn strip_nul(data: JsonValue) -> JsonValue {
    match data {
        JsonValue::String(text) => JsonValue::String(text.replace('\0', "")),
        JsonValue::Array(items) => JsonValue::Array(items.into_iter().map(strip_nul).collect()),
        JsonValue::Object(fields) => JsonValue::Object(
            fields
                .into_iter()
                .map(|(name, item)| (name.replace('\0', ""), strip_nul(item)))
                .collect(),
        ),
        other @ (JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_)) => other,
    }
}

// Credentials pass through unchanged: this text is what the table stores
pub fn safe_dumps<Data>(data: &Data) -> Result<String, serde_json::Error>
where
    Data: Serialize,
{
    let text = serde_json::to_string(data)?;
    let tree: JsonValue = serde_json::from_str(&text)?;
    serde_json::to_string(&strip_nul(tree))
}

pub fn dump_fields(search_tool: &SearchTool) -> Result<DumpedFields, StoreError> {
    let litellm_params =
        safe_dumps(&search_tool.litellm_params).map_err(|source| StoreError::Field {
            field: "litellm_params",
            source,
        })?;
    let search_tool_info = match &search_tool.search_tool_info {
        Some(info) => safe_dumps(info),
        None => safe_dumps(&BTreeMap::<String, JsonValue>::new()),
    }
    .map_err(|source| StoreError::Field {
        field: "search_tool_info",
        source,
    })?;
    Ok(DumpedFields {
        litellm_params,
        search_tool_info,
    })
}
