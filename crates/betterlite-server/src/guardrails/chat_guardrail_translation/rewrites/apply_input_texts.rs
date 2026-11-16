use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;

use super::error::InputError;
use crate::guardrails::chat_guardrail_translation::extraction::apply_input_texts as write_texts;
use crate::guardrails::chat_guardrail_translation::extraction::extracted_content::TextMapping;

mod tests;

// Nothing is written when the guardrail returned no text or none was sent; a different count cannot be mapped back, so the request is refused.
pub fn apply_guardrailed_texts(
    messages: &mut [GuardrailValue],
    returned: &[String],
    sent: &[String],
    mappings: &[TextMapping],
    guardrail_name: Option<&str>,
) -> Result<(), InputError> {
    if returned.is_empty() || sent.is_empty() {
        return Ok(());
    }
    if returned.len() != mappings.len() {
        return Err(InputError::from(UnappliableRequestRewrite::new(
            guardrail_name,
        )));
    }
    write_texts::apply_input_texts(messages, returned, mappings)?;
    Ok(())
}
