use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::apply_error::ApplyError;
use super::extracted_content::ToolCallMapping;
use super::json_access::{choice_mut, field_mut};

mod tests;

// Only function.arguments and function.name come back from the guardrail; the id, type and other fields of the response stay.
fn write_function_fields(existing: &mut GuardrailValue, guardrailed: &Object) {
    let Some(GuardrailValue::Object(function)) = field_mut(existing, "function") else {
        return;
    };
    let Some(GuardrailValue::Object(updates)) = guardrailed.get("function") else {
        return;
    };
    for key in ["arguments", "name"] {
        if let Some(value) = updates.get(key) {
            function.insert(key.to_owned(), value.clone());
        }
    }
}

pub fn apply_output_tool_calls(
    response: &mut Object,
    tool_calls: &[Object],
    mappings: &[ToolCallMapping],
) -> Result<(), ApplyError> {
    for (task_index, &(choice_index, tool_call_index)) in mappings.iter().enumerate() {
        let Some(guardrailed) = tool_calls.get(task_index) else {
            continue;
        };
        let choice = choice_mut(response, choice_index)?;
        let Some(message) = field_mut(choice, "message") else {
            continue;
        };
        let Some(GuardrailValue::Array(existing_calls)) = field_mut(message, "tool_calls") else {
            continue;
        };
        if let Some(existing) = existing_calls.get_mut(tool_call_index) {
            write_function_fields(existing, guardrailed);
        }
    }
    Ok(())
}
