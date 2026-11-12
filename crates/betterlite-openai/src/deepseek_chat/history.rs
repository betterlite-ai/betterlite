use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;

const ASSISTANT: &str = "assistant";
const REASONING_CONTENT: &str = "reasoning_content";
const PROVIDER_FIELDS: &str = "provider_specific_fields";
const PLACEHOLDER: &str = " ";

#[must_use]
pub fn thinking_active(supports_reasoning: bool, optional_params: &Object) -> bool {
    let thinking = optional_params.get("thinking").and_then(Json::as_object);
    let enabled = thinking
        .and_then(|fields| fields.get("type"))
        .and_then(Json::as_str)
        == Some("enabled");
    supports_reasoning && enabled
}

#[must_use]
pub fn fill_reasoning_content(messages: &[Message]) -> Vec<Message> {
    let mut repaired = Vec::with_capacity(messages.len());
    for message in messages {
        repaired.push(repaired_message(message));
    }
    repaired
}

fn repaired_message(message: &Message) -> Message {
    let current = message.provider_specific_fields.get(REASONING_CONTENT);
    if message.role != ASSISTANT || current.is_some_and(Json::is_truthy) {
        return message.clone();
    }
    let mut patched = message.clone();
    // Only the nested copy goes away; the rest of provider_specific_fields stays, even when empty
    let stored = match patched.provider_specific_fields.get_mut(PROVIDER_FIELDS) {
        Some(Json::Object(nested))
            if nested.get(REASONING_CONTENT).is_some_and(Json::is_truthy) =>
        {
            nested.shift_remove(REASONING_CONTENT)
        }
        _ => None,
    };
    let value = stored.unwrap_or_else(|| {
        tracing::warn!(
            provider = "deepseek",
            "DeepSeek thinking mode: assistant message is missing reasoning_content; a single-space placeholder is injected"
        );
        Json::Text(PLACEHOLDER.to_owned())
    });
    patched
        .provider_specific_fields
        .insert(REASONING_CONTENT.to_owned(), value);
    patched
}
