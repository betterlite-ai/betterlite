use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, Message};

const PREFIX_CONTINUATION_PROMPT: &str = "You are a helpful assistant. You are given a message and you need to respond to it. You are also given a generated content. You need to respond to the message in continuation of the generated content. Do not repeat the same content. Your response should be in continuation of this text: ";

// `prefix` is not a declared message field, so only a literal boolean true counts.
#[must_use]
pub fn convert_prefix_messages(messages: &[Message]) -> Vec<Message> {
    let mut converted = Vec::with_capacity(messages.len());
    for message in messages {
        if is_prefix_message(message) {
            converted.push(Message {
                role: "system".to_owned(),
                content: Some(Content::Text(PREFIX_CONTINUATION_PROMPT.to_owned())),
                name: None,
                provider_specific_fields: Object::new(),
            });
            converted.push(without_prefix(message));
        } else {
            converted.push(message.clone());
        }
    }
    converted
}

fn is_prefix_message(message: &Message) -> bool {
    match message.provider_specific_fields.get("prefix") {
        Some(Json::Bool(prefix)) => *prefix,
        _ => false,
    }
}

fn without_prefix(message: &Message) -> Message {
    let mut stripped = message.clone();
    stripped.provider_specific_fields.shift_remove("prefix");
    stripped
}

mod tests;
