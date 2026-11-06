use betterlite_prompts::arguments::json::Json;
use serde::Serialize;

use crate::typesafe::typesafe_message_parsing::{as_object_list, as_str_object_dict};

// LiteLLM copies "name" and "arguments" as they are, so either value can be any JSON the assistant sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolCallEntry {
    pub name: Option<Json>,
    pub arguments: Option<Json>,
}

#[must_use]
pub fn tool_call_entry(tool_call: &Json) -> Option<ToolCallEntry> {
    let parsed_call = as_str_object_dict(tool_call)?;
    let function = parsed_call.get("function").and_then(as_str_object_dict);
    let source = function.unwrap_or(parsed_call);
    Some(ToolCallEntry {
        name: source.get("name").cloned(),
        arguments: source.get("arguments").cloned(),
    })
}

#[must_use]
pub fn tool_call_entries(assistant_message: &Json) -> Vec<ToolCallEntry> {
    let Some(tool_calls) = assistant_message
        .as_object()
        .and_then(|fields| fields.get("tool_calls"))
        .and_then(as_object_list)
    else {
        return Vec::new();
    };
    tool_calls.iter().filter_map(tool_call_entry).collect()
}

#[path = "typesafe_tool_extraction/tests.rs"]
mod tests;
