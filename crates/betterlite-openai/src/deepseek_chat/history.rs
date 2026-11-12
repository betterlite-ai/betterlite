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
    let reasoning = message.provider_specific_fields.get(REASONING_CONTENT);
    if message.role != ASSISTANT || reasoning.is_some_and(Json::is_truthy) {
        return message.clone();
    }
    let stored = stored_reasoning(message);
    let mut patched = message.clone();
    if let Some(value) = stored {
        patched
            .provider_specific_fields
            .insert(REASONING_CONTENT.to_owned(), value);
        remove_stored_reasoning(message, &mut patched);
    } else {
        tracing::warn!(
            provider = "deepseek",
            "DeepSeek thinking mode: assistant message is missing reasoning_content; a single-space placeholder is injected"
        );
        patched.provider_specific_fields.insert(
            REASONING_CONTENT.to_owned(),
            Json::Text(PLACEHOLDER.to_owned()),
        );
    }
    patched
}

fn stored_reasoning(message: &Message) -> Option<Json> {
    let fields = message
        .provider_specific_fields
        .get(PROVIDER_FIELDS)?
        .as_object()?;
    fields
        .get(REASONING_CONTENT)
        .filter(|value| value.is_truthy())
        .cloned()
}

// Only the nested copy goes away; the rest of provider_specific_fields stays on the message, even when empty
fn remove_stored_reasoning(message: &Message, patched: &mut Message) {
    let Some(Json::Object(mut fields)) = message
        .provider_specific_fields
        .get(PROVIDER_FIELDS)
        .cloned()
    else {
        return;
    };
    fields.shift_remove(REASONING_CONTENT);
    patched
        .provider_specific_fields
        .insert(PROVIDER_FIELDS.to_owned(), Json::Object(fields));
}
