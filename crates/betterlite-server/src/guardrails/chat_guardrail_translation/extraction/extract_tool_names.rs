use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::json_access::{field, object_items};

mod tests;

fn non_empty_name(value: &GuardrailValue) -> Option<&String> {
    match field(value, "name") {
        Some(GuardrailValue::Text(name)) if !name.is_empty() => Some(name),
        _ => None,
    }
}

#[must_use]
pub fn extract_request_tool_names(data: &Object) -> Vec<String> {
    let function_tools = object_items(data, "tools")
        .iter()
        .filter(|tool| {
            matches!(field(tool, "type"), Some(GuardrailValue::Text(kind)) if kind == "function")
        })
        .filter_map(|tool| field(tool, "function").and_then(non_empty_name));
    let legacy_functions = object_items(data, "functions")
        .iter()
        .filter_map(non_empty_name);
    function_tools.chain(legacy_functions).cloned().collect()
}
