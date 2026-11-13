use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::json_access::{array_field, field};

mod tests;

// The legacy function_call goes after the tool_calls, so a delta that carries both lists the tool calls first.
#[must_use]
pub fn streamed_delta_tool_calls(delta: &GuardrailValue) -> Vec<&GuardrailValue> {
    array_field(delta, "tool_calls")
        .iter()
        .chain(field(delta, "function_call"))
        .collect()
}
