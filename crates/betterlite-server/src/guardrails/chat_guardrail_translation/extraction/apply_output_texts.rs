use betterlite_guardrails::guardrail_message_scope::Object;

use super::apply_error::ApplyError;
use super::extracted_content::TextMapping;
use super::json_access::{choice_mut, field_mut};
use super::text_slot_write::write_text_slot;

mod tests;

pub fn apply_output_texts(
    response: &mut Object,
    responses: &[String],
    mappings: &[TextMapping],
) -> Result<(), ApplyError> {
    for (response_index, text) in responses.iter().enumerate() {
        let Some(&(choice_index, content_index)) = mappings.get(response_index) else {
            return Err(ApplyError::UnmappedResponse { response_index });
        };
        let choice = choice_mut(response, choice_index)?;
        if let Some(message) = field_mut(choice, "message") {
            write_text_slot(message, choice_index, content_index, text)?;
        }
    }
    Ok(())
}
