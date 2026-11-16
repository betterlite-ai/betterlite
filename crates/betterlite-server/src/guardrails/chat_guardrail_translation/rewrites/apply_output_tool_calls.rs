use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::error::RewriteError;
use super::response::ChatResponse;
use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;
use crate::guardrails::chat_guardrail_translation::extraction::extracted_content::ToolCallMapping;

mod tests;

fn text_update(
    value: &GuardrailValue,
    field: &'static str,
) -> Result<Option<String>, RewriteError> {
    match value {
        GuardrailValue::Text(text) => Ok(Some(text.clone())),
        GuardrailValue::Null => Ok(None),
        GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => Err(RewriteError::FieldNotText { field }),
    }
}

// Only function.arguments and function.name come back from the guardrail; a guardrailed call the response has no slot for is skipped.
pub fn apply_output_tool_calls(
    response: &mut ChatResponse,
    tool_calls: &[Object],
    mappings: &[ToolCallMapping],
) -> Result<(), RewriteError> {
    for (task_index, &(choice_index, tool_call_index)) in mappings.iter().enumerate() {
        let Some(guardrailed) = tool_calls.get(task_index) else {
            continue;
        };
        let choice = response
            .choices
            .get_mut(choice_index)
            .ok_or(ApplyError::MissingEntry {
                index: choice_index,
            })?;
        let Some(function) = choice
            .message
            .tool_calls
            .as_mut()
            .and_then(|calls| calls.get_mut(tool_call_index))
            .and_then(|call| call.function.as_mut())
        else {
            continue;
        };
        let Some(GuardrailValue::Object(updates)) = guardrailed.get("function") else {
            continue;
        };
        if let Some(value) = updates.get("arguments") {
            function.arguments = text_update(value, "arguments")?;
        }
        if let Some(value) = updates.get("name") {
            function.name = text_update(value, "name")?;
        }
    }
    Ok(())
}
