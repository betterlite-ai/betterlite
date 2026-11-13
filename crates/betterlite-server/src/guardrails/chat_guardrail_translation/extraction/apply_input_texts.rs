use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::apply_error::ApplyError;
use super::extracted_content::TextMapping;
use super::text_slot_write::write_text_slot;

mod tests;

pub fn apply_input_texts(
    messages: &mut [GuardrailValue],
    responses: &[String],
    mappings: &[TextMapping],
) -> Result<(), ApplyError> {
    for (response_index, response) in responses.iter().enumerate() {
        let Some(&(message_index, content_index)) = mappings.get(response_index) else {
            return Err(ApplyError::UnmappedResponse { response_index });
        };
        let message = messages
            .get_mut(message_index)
            .ok_or(ApplyError::MissingEntry {
                index: message_index,
            })?;
        write_text_slot(message, message_index, content_index, response)?;
    }
    Ok(())
}
