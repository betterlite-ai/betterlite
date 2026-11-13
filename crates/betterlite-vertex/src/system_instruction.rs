use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use serde::Serialize;

mod tests;

const SYSTEM: &str = "system";
const USER: &str = "user";
const TEXT: &str = "text";
const SYSTEM_ONLY_USER_TEXT: &str = ".";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TextPart {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SystemInstruction {
    pub parts: Vec<TextPart>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemExtraction {
    pub instruction: Option<SystemInstruction>,
    pub messages: Vec<Message>,
}

#[must_use]
pub fn extract_system_instruction(
    supports_system_message: bool,
    messages: &[Message],
) -> SystemExtraction {
    if !supports_system_message {
        return SystemExtraction {
            instruction: None,
            messages: messages.to_vec(),
        };
    }
    let mut parts = Vec::new();
    let mut kept: Vec<Message> = Vec::with_capacity(messages.len());
    for message in messages {
        if message.role != SYSTEM {
            kept.push(message.clone());
            continue;
        }
        match &message.content {
            Some(Content::Text(text)) => parts.push(TextPart { text: text.clone() }),
            Some(Content::Parts(items)) => parts.push(TextPart {
                text: concatenated_text(items),
            }),
            None => kept.push(message.clone()),
        }
    }
    if parts.is_empty() {
        return SystemExtraction {
            instruction: None,
            messages: kept,
        };
    }
    if kept.is_empty() {
        kept.push(Message {
            role: USER.to_owned(),
            content: Some(Content::Text(SYSTEM_ONLY_USER_TEXT.to_owned())),
            name: None,
            provider_specific_fields: Object::new(),
        });
    }
    SystemExtraction {
        instruction: Some(SystemInstruction { parts }),
        messages: kept,
    }
}

fn concatenated_text(items: &[ContentPart]) -> String {
    let mut text = String::new();
    for item in items {
        if let Some(found) = item.fields.get(TEXT).and_then(Json::as_str) {
            text.push_str(found);
        }
    }
    text
}
