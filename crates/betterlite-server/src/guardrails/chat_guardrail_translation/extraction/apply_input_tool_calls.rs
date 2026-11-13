use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::apply_error::ApplyError;
use super::extracted_content::ToolCallMapping;
use super::json_access::field_mut;

mod tests;

// A guardrail that returns fewer tool calls than were sent leaves the rest of the messages as they were.
pub fn apply_input_tool_calls(
    messages: &mut [GuardrailValue],
    tool_calls: &[Object],
    mappings: &[ToolCallMapping],
) -> Result<(), ApplyError> {
    for (task_index, &(message_index, tool_call_index)) in mappings.iter().enumerate() {
        let Some(guardrailed) = tool_calls.get(task_index) else {
            continue;
        };
        let message = messages
            .get_mut(message_index)
            .ok_or(ApplyError::MissingEntry {
                index: message_index,
            })?;
        let Some(GuardrailValue::Array(message_tool_calls)) = field_mut(message, "tool_calls")
        else {
            continue;
        };
        if let Some(slot) = message_tool_calls.get_mut(tool_call_index) {
            *slot = GuardrailValue::Object(guardrailed.clone());
        }
    }
    Ok(())
}
