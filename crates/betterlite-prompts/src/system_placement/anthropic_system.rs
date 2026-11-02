use serde::Serialize;

use crate::message_content::{ContentPart, Message};
use crate::system_placement::text_parts::{text_block, text_parts};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AnthropicSystemMessage {
    pub role: String,
    pub content: Vec<ContentPart>,
}

// The Anthropic wire message for a system message, or nothing when it carries no text.
#[must_use]
pub fn anthropic_system_messages(message: &Message) -> Vec<AnthropicSystemMessage> {
    let content: Vec<ContentPart> = text_parts(message).iter().map(text_block).collect();
    if content.is_empty() {
        return Vec::new();
    }
    vec![AnthropicSystemMessage {
        role: "system".to_owned(),
        content,
    }]
}

mod tests;
