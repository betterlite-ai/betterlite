use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::InputError;
use super::structured::tool_call_objects;
use crate::guardrails::chat_guardrail_translation::extraction::apply_input_tool_calls as write_calls;
use crate::guardrails::chat_guardrail_translation::extraction::extracted_content::ToolCallMapping;

mod tests;

// A guardrail that returns fewer calls than were sent leaves the later messages as they were.
pub fn apply_guardrailed_tool_calls(
    messages: &mut [GuardrailValue],
    returned: Vec<GuardrailValue>,
    mappings: &[ToolCallMapping],
) -> Result<(), InputError> {
    if returned.is_empty() {
        return Ok(());
    }
    write_calls::apply_input_tool_calls(messages, &tool_call_objects(returned)?, mappings)?;
    Ok(())
}
