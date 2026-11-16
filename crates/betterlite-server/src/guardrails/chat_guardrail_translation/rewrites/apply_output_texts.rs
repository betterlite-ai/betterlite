use super::error::RewriteError;
use super::response::{ChatResponse, MessageContent};
use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;
use crate::guardrails::chat_guardrail_translation::extraction::extracted_content::TextMapping;

mod tests;

// A string slot takes a string rewrite and a list slot takes an item rewrite; any other pairing is left alone.
pub fn apply_output_texts(
    response: &mut ChatResponse,
    responses: &[String],
    mappings: &[TextMapping],
) -> Result<(), RewriteError> {
    for (response_index, text) in responses.iter().enumerate() {
        let Some(&(choice_index, content_index)) = mappings.get(response_index) else {
            return Err(RewriteError::from(ApplyError::UnmappedResponse {
                response_index,
            }));
        };
        let choice = response
            .choices
            .get_mut(choice_index)
            .ok_or(ApplyError::MissingEntry {
                index: choice_index,
            })?;
        match (&mut choice.message.content, content_index) {
            (Some(MessageContent::Text(current)), None) => text.clone_into(current),
            (Some(MessageContent::Parts(parts)), Some(slot)) => {
                let part = parts.get_mut(slot).ok_or(ApplyError::MissingContentItem {
                    index: choice_index,
                    content_index: slot,
                })?;
                part.text = Some(text.clone());
            }
            (Some(MessageContent::Text(_)), Some(_))
            | (Some(MessageContent::Parts(_)), None)
            | (None, _) => {}
        }
    }
    Ok(())
}
