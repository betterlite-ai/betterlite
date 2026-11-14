use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::fields::{get_field, non_blank_text};

#[must_use]
pub fn tool_call_args(message: &GuardrailValue) -> Vec<&str> {
    let mut args = Vec::new();
    if let Some(GuardrailValue::Array(tool_calls)) = get_field(Some(message), "tool_calls") {
        for tool_call in tool_calls {
            let arguments = get_field(get_field(Some(tool_call), "function"), "arguments");
            if let Some(text) = non_blank_text(arguments) {
                args.push(text);
            }
        }
    }
    let legacy = get_field(get_field(Some(message), "function_call"), "arguments");
    if let Some(text) = non_blank_text(legacy) {
        args.push(text);
    }
    args
}
