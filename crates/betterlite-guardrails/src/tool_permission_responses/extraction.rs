use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_message_scope::Object;
use crate::tool_permission::types::{mapping_value, object_list};

// A tool call in the unified shape LiteLLM hands to the permission rules, kept as the JSON pieces it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallInfo {
    pub id: String,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<GuardrailValue>,
}

// The id LiteLLM reads off a parsed tool call; a missing or non-text id acts as the empty string here
pub(super) fn id_text(value: &GuardrailValue) -> &str {
    match mapping_value(value, "id") {
        Some(GuardrailValue::Text(id)) => id,
        None | Some(_) => "",
    }
}

// LiteLLM's _extract_tool_calls_from_response: every modern tool call and every legacy function call, in choice order
#[must_use]
pub fn extract_response_tools(response: &Object) -> Vec<ToolCallInfo> {
    let Some(choices) = response.get("choices").and_then(object_list) else {
        return Vec::new();
    };
    let mut calls = Vec::new();
    for (choice_index, choice) in choices.iter().enumerate() {
        let Some(message) = mapping_value(choice, "message") else {
            continue;
        };
        if let Some(tools) = mapping_value(message, "tool_calls").and_then(object_list) {
            calls.extend(tools.iter().filter_map(modern_tool_call));
        }
        if let Some(function_call) = mapping_value(message, "function_call")
            && let Some(legacy) = legacy_to_tool_call(function_call, choice_index)
        {
            calls.push(legacy);
        }
    }
    calls
}

// A tool_calls entry is only a call when it is an object; LiteLLM's typed response guarantees that much
fn modern_tool_call(value: &GuardrailValue) -> Option<ToolCallInfo> {
    if !matches!(value, GuardrailValue::Object(_)) {
        return None;
    }
    let function = mapping_value(value, "function");
    let name = function.and_then(|callee| match mapping_value(callee, "name") {
        Some(GuardrailValue::Text(name)) => Some(name.clone()),
        None | Some(_) => None,
    });
    let arguments = function.and_then(|callee| mapping_value(callee, "arguments").cloned());
    let kind = match mapping_value(value, "type") {
        Some(GuardrailValue::Text(kind)) => Some(kind.clone()),
        None | Some(_) => None,
    };
    Some(ToolCallInfo {
        id: id_text(value).to_owned(),
        kind,
        name,
        arguments,
    })
}

// LiteLLM's _legacy_function_call_to_tool_call: a function call needs a name, and its arguments default to an empty string
#[must_use]
pub fn legacy_to_tool_call(
    function_call: &GuardrailValue,
    choice_index: usize,
) -> Option<ToolCallInfo> {
    let name = match mapping_value(function_call, "name") {
        Some(GuardrailValue::Text(name)) if !name.is_empty() => name.clone(),
        None | Some(_) => return None,
    };
    let arguments = match mapping_value(function_call, "arguments") {
        Some(GuardrailValue::Null) | None => GuardrailValue::Text(String::new()),
        Some(GuardrailValue::Text(arguments)) => GuardrailValue::Text(arguments.clone()),
        Some(other) => other.clone(),
    };
    Some(ToolCallInfo {
        // LiteLLM's _legacy_function_call_id: the choice position, not the choice's own index field
        id: format!("legacy_function_call_{choice_index}"),
        kind: Some("function".to_owned()),
        name: Some(name),
        arguments: Some(arguments),
    })
}

mod tests;
