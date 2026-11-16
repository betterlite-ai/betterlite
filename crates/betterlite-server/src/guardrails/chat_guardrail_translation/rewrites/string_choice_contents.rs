use super::response::{ChatResponse, MessageContent};

mod tests;

// One entry per choice, in order; a choice whose content is not a string has none.
#[must_use]
pub fn string_choice_contents(response: &ChatResponse) -> Vec<Option<String>> {
    response
        .choices
        .iter()
        .map(|choice| match &choice.message.content {
            Some(MessageContent::Text(text)) => Some(text.clone()),
            Some(MessageContent::Parts(_)) | None => None,
        })
        .collect()
}
