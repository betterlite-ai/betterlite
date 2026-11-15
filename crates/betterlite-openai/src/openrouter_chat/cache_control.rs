use indexmap::IndexMap;
use serde_json::Value;

use crate::chat::{ChatMessage, ContentPart, MessageContent};

// LiteLLM's CacheControlSupportedModels, matched as substrings of the lowercased model name
const SUPPORTED_MODELS: [&str; 5] = ["claude", "gemini", "minimax", "glm", "z-ai"];
const CACHE_CONTROL: &str = "cache_control";

#[must_use]
pub fn supports_cache_control(model: &str) -> bool {
    let lowered = model.to_lowercase();
    SUPPORTED_MODELS
        .iter()
        .any(|supported| lowered.contains(supported))
}

// OpenRouter reads cache_control only inside content, and Anthropic allows four breakpoints per request
#[must_use]
pub fn move_cache_control_to_content(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut moved = Vec::with_capacity(messages.len());
    for message in messages {
        moved.push(moved_message(message));
    }
    moved
}

fn moved_message(original: &ChatMessage) -> ChatMessage {
    let mut message = original.clone();
    let Some(cache_control) = message.passthrough.shift_remove(CACHE_CONTROL) else {
        return message;
    };
    if cache_control.is_null() {
        return message;
    }
    match message.content.as_ref() {
        // an empty block list has no last block, so it stays empty
        Some(MessageContent::Parts(parts)) => {
            let mut blocks = parts.clone();
            if let Some(last) = blocks.last_mut() {
                last.fields.insert(CACHE_CONTROL.to_owned(), cache_control);
            }
            message.content = Some(MessageContent::Parts(blocks));
        }
        Some(MessageContent::Text(text)) => {
            message.content = Some(MessageContent::Parts(vec![text_block(
                Value::String(text.clone()),
                cache_control,
            )]));
        }
        None => {
            message.content = Some(MessageContent::Parts(vec![text_block(
                Value::Null,
                cache_control,
            )]));
        }
    }
    message
}

fn text_block(text: Value, cache_control: Value) -> ContentPart {
    let mut fields = IndexMap::new();
    fields.insert(String::from("text"), text);
    fields.insert(CACHE_CONTROL.to_owned(), cache_control);
    ContentPart {
        kind: String::from("text"),
        fields,
    }
}
