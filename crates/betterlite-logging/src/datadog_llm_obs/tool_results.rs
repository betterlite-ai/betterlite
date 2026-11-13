use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, MAX_DEPTH_MARKER};
use crate::python_json;

use super::records::ToolResult;
use super::values::{
    Object, as_object, as_text, content_blocks, mapping_field, safe_identifier, text_field,
};

// LiteLLM reads every OpenAI pair before every Anthropic pair, so an Anthropic tool_use name wins a shared id.
#[must_use]
pub fn tool_call_names_by_id(messages: &[Json]) -> IndexMap<String, String> {
    let objects: Vec<&Object> = messages.iter().filter_map(as_object).collect();
    let mut names = IndexMap::new();
    for message in &objects {
        openai_names(message, &mut names);
    }
    for message in &objects {
        anthropic_names(message, &mut names);
    }
    names
}

fn openai_names(message: &Object, names: &mut IndexMap<String, String>) {
    let Some(Json::List(raw_calls)) = message.get("tool_calls") else {
        return;
    };
    for raw_call in raw_calls {
        let Some(tool_call) = as_object(raw_call) else {
            continue;
        };
        let Some(tool_id) = linked_id(tool_call, "id") else {
            continue;
        };
        let name = mapping_field(tool_call, "function")
            .map_or_else(String::new, |function| text_field(function, "name", ""));
        names.insert(tool_id, name);
    }
}

fn anthropic_names(message: &Object, names: &mut IndexMap<String, String>) {
    for block in content_blocks(message) {
        if block.get("type").and_then(as_text) != Some("tool_use") {
            continue;
        }
        if let Some(tool_id) = linked_id(block, "id") {
            names.insert(tool_id, text_field(block, "name", ""));
        }
    }
}

#[must_use]
pub fn to_dd_tool_results(
    message: &Object,
    tool_call_names: &IndexMap<String, String>,
) -> Vec<ToolResult> {
    if text_field(message, "role", "") == "tool" {
        return vec![linked_result(
            message.get("tool_call_id"),
            message.get("content"),
            tool_call_names,
        )];
    }
    content_blocks(message)
        .into_iter()
        .filter(|block| block.get("type").and_then(as_text) == Some("tool_result"))
        .map(|block| {
            linked_result(
                block.get("tool_use_id"),
                block.get("content"),
                tool_call_names,
            )
        })
        .collect()
}

fn linked_id(fields: &Object, key: &str) -> Option<String> {
    fields
        .get(key)
        .and_then(as_text)
        .filter(|identifier| !identifier.is_empty())
        .map(str::to_owned)
}

fn linked_result(
    tool_identifier: Option<&Json>,
    content: Option<&Json>,
    tool_call_names: &IndexMap<String, String>,
) -> ToolResult {
    let tool_id = safe_identifier(tool_identifier);
    let name = tool_call_names.get(&tool_id).cloned().unwrap_or_default();
    ToolResult {
        name,
        result: result_text(content),
        tool_id,
        kind: "function".to_owned(),
    }
}

// A result string is kept as sent; anything else goes through LiteLLM's safe_dumps.
fn result_text(content: Option<&Json>) -> String {
    match content {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null) | None => String::new(),
        Some(value) => python_json::dumps(&bounded_structure(value, 0)),
    }
}

fn bounded_structure(value: &Json, depth: usize) -> Json {
    if depth > DEFAULT_MAX_RECURSE_DEPTH {
        return Json::Text(MAX_DEPTH_MARKER.to_owned());
    }
    match value {
        Json::Null | Json::Bool(_) | Json::Number(_) => value.clone(),
        Json::Text(text) => Json::Text(text.replace('\0', "")),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| bounded_structure(item, depth.saturating_add(1)))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| {
                    (
                        key.replace('\0', ""),
                        bounded_structure(item, depth.saturating_add(1)),
                    )
                })
                .collect(),
        ),
    }
}
