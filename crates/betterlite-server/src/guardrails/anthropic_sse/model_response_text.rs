use betterlite_core::api::messages_chat_output::records::ResponseRecord;

mod tests;

// Only string content counts, so an all-image or tool-call response has no text.
#[must_use]
pub fn model_response_text(response: &ResponseRecord) -> String {
    response
        .choices
        .iter()
        .filter_map(|choice| choice.message.content.as_deref())
        .collect()
}
