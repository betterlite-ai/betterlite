use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::error::HistoryError;
use crate::api::responses_tool_results::outbound_call::CallIdentity;
use crate::api::responses_tool_results::records::{FunctionChunk, ToolCallChunk};

pub fn output_tool_fields(
    item: &Map<String, Value>,
    index: usize,
) -> Result<ToolCallChunk, HistoryError> {
    let is_custom = item.get("type").and_then(Value::as_str) == Some("custom_tool_call");
    let payload = if is_custom {
        item.get("input")
    } else {
        item.get("arguments")
    };
    let arguments = match payload {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(_) => {
            return Err(HistoryError::ToolField { field: "arguments" });
        }
    };
    let default_name = if is_custom { "custom_tool" } else { "" };
    let name = match item.get("name") {
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        Some(_) | None => default_name.to_owned(),
    };
    let provider_specific_fields = match item.get("provider_specific_fields") {
        Some(Value::Object(fields)) if !fields.is_empty() => Some(provider_fields(fields)?),
        _ => None,
    };
    let identity = CallIdentity::new().map_err(|source| HistoryError::CallId { source })?;
    let id = identity.tool_call_id(
        item.get("id").and_then(Value::as_str),
        item.get("call_id").and_then(Value::as_str),
    );
    Ok(ToolCallChunk {
        id,
        kind: "function".to_owned(),
        function: FunctionChunk {
            name,
            arguments,
            provider_specific_fields: provider_specific_fields.clone(),
        },
        index,
        provider_specific_fields,
    })
}

fn provider_fields(fields: &Map<String, Value>) -> Result<IndexMap<String, Json>, HistoryError> {
    serde_json::from_value(Value::Object(fields.clone()))
        .map_err(|source| HistoryError::ProviderFields { source })
}
