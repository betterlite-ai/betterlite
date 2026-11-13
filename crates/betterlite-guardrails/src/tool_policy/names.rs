use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::RequestToolNames;
use super::metadata::request_route_from_data;
use crate::generic_guardrail_contracts::GuardrailToolParam;

fn function_name(value: Option<&GuardrailValue>) -> Option<&String> {
    let Some(GuardrailValue::Object(function)) = value else {
        return None;
    };
    let Some(GuardrailValue::Text(name)) = function.get("name") else {
        return None;
    };
    (!name.is_empty()).then_some(name)
}

// LiteLLM falls back to the route extractor only when the tools give no name at all
#[must_use]
pub(super) fn request_tool_names(
    tools: Option<&[GuardrailToolParam]>,
    names: &dyn RequestToolNames,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Vec<String> {
    let collected: Vec<String> = tools
        .unwrap_or_default()
        .iter()
        .filter_map(|tool| function_name(tool.extra.get("function")))
        .cloned()
        .collect();
    if !collected.is_empty() {
        return collected;
    }
    let Some(route) = request_route_from_data(request_data).filter(|route| !route.is_empty())
    else {
        return Vec::new();
    };
    names.names(&route, request_data)
}

#[must_use]
pub(super) fn response_tool_names(tool_calls: Option<&[GuardrailValue]>) -> Vec<String> {
    let mut collected = Vec::new();
    for tool_call in tool_calls.unwrap_or_default() {
        let GuardrailValue::Object(fields) = tool_call else {
            continue;
        };
        if let Some(name) = function_name(fields.get("function")) {
            collected.push(name.clone());
        }
    }
    collected
}

#[must_use]
pub(super) fn tool_call_names_from_messages(
    messages: &[GuardrailValue],
) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for message in messages {
        let GuardrailValue::Object(fields) = message else {
            continue;
        };
        if !matches!(fields.get("role"), Some(GuardrailValue::Text(role)) if role == "assistant") {
            continue;
        }
        let Some(GuardrailValue::Array(tool_calls)) = fields.get("tool_calls") else {
            continue;
        };
        for tool_call in tool_calls {
            let GuardrailValue::Object(call) = tool_call else {
                continue;
            };
            let Some(GuardrailValue::Text(id)) = call.get("id") else {
                continue;
            };
            let Some(name) = function_name(call.get("function")) else {
                continue;
            };
            if !id.is_empty() {
                names.insert(id.clone(), name.clone());
            }
        }
    }
    names
}
