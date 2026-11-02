use crate::message_content::Message;

const INTERNAL_FIELDS: [&str; 3] = [
    "thinking_blocks",
    "reasoning_content",
    "provider_specific_fields",
];

// LiteLLM attaches these to assistant messages; strict OpenAI-compatible schemas reject them.
#[must_use]
pub fn strip_internal_message_fields(message: &Message) -> Message {
    let mut stripped = message.clone();
    stripped
        .provider_specific_fields
        .retain(|key, _| !INTERNAL_FIELDS.contains(&key.as_str()));
    stripped
}

mod tests;
