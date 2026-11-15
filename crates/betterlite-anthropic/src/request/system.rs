use serde::{Deserialize, Serialize};

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::{Content, ContentPart, Message};

const BILLING_HEADER_PREFIX: &str = "x-anthropic-billing-header:";
const CACHE_CONTROL: &str = "cache_control";
const TEXT: &str = "text";

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct AnthropicSystemMessageContent {
    #[serde(rename = "type")]
    pub kind: Json,
    pub text: Json,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<Json>,
}

// Every system message leaves the list, not only the leading run this assembly turns into the top-level prompt
pub fn translate_system_message(
    messages: &mut Vec<Message>,
    strips_billing_metadata: bool,
) -> Vec<AnthropicSystemMessageContent> {
    let mut contents = Vec::new();
    let mut indices = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if message.role != "system" {
            continue;
        }
        indices.push(index);
        match &message.content {
            Some(Content::Text(text)) => {
                push_text(&mut contents, message, text, strips_billing_metadata);
            }
            Some(Content::Parts(parts)) => {
                push_parts(&mut contents, parts, strips_billing_metadata);
            }
            None => {}
        }
    }
    for index in indices.into_iter().rev() {
        messages.remove(index);
    }
    contents
}

fn push_text(
    contents: &mut Vec<AnthropicSystemMessageContent>,
    message: &Message,
    text: &str,
    strips_billing_metadata: bool,
) {
    if text.is_empty() {
        return;
    }
    if strips_billing_metadata && text.starts_with(BILLING_HEADER_PREFIX) {
        return;
    }
    contents.push(AnthropicSystemMessageContent {
        kind: Json::Text(TEXT.to_owned()),
        text: Json::Text(text.to_owned()),
        cache_control: message.provider_specific_fields.get(CACHE_CONTROL).cloned(),
    });
}

fn push_parts(
    contents: &mut Vec<AnthropicSystemMessageContent>,
    parts: &[ContentPart],
    strips_billing_metadata: bool,
) {
    for part in parts {
        let text = part.fields.get(TEXT);
        let is_text = part.kind == TEXT;
        if is_text && !text.is_some_and(Json::is_truthy) {
            continue;
        }
        if strips_billing_metadata && is_text && text.is_some_and(is_billing_header) {
            continue;
        }
        contents.push(AnthropicSystemMessageContent {
            kind: Json::Text(part.kind.clone()),
            text: text.cloned().unwrap_or(Json::Null),
            cache_control: part.fields.get(CACHE_CONTROL).cloned(),
        });
    }
}

fn is_billing_header(value: &Json) -> bool {
    match value.as_str() {
        Some(text) => text.starts_with(BILLING_HEADER_PREFIX),
        None => false,
    }
}
