use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::arguments::{ToolArguments, to_dd_arguments};
use super::records::ToolCall;
use super::values::{
    Object, as_object, as_text, content_blocks, is_filled, mapping_field, text_field,
};

#[must_use]
pub fn to_dd_tool_calls(message: &Object) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    if let Some(Json::List(raw_calls)) = message.get("tool_calls") {
        for raw_call in raw_calls {
            let Some(tool_call) = as_object(raw_call) else {
                continue;
            };
            let function = mapping_field(tool_call, "function");
            let arguments = function
                .and_then(|declared| declared.get("arguments"))
                .filter(|value| !matches!(value, Json::Null))
                .map_or_else(|| ToolArguments::Text(String::new()), to_dd_arguments);
            calls.push(ToolCall {
                name: function.map_or_else(String::new, |fields| text_field(fields, "name", "")),
                arguments,
                tool_id: text_field(tool_call, "id", ""),
                kind: text_field(tool_call, "type", "function"),
            });
        }
    }
    for block in content_blocks(message) {
        if block.get("type").and_then(as_text) != Some("tool_use") {
            continue;
        }
        let arguments = match block.get("input") {
            Some(value) if is_filled(value) => to_dd_arguments(value),
            Some(_) | None => ToolArguments::Object(IndexMap::new()),
        };
        calls.push(ToolCall {
            name: text_field(block, "name", ""),
            arguments,
            tool_id: text_field(block, "id", ""),
            kind: "tool_use".to_owned(),
        });
    }
    calls
}
