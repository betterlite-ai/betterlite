#![cfg(test)]

use super::*;

fn message(role: &str, content: Option<Content>) -> Message {
    Message {
        role: role.to_owned(),
        content,
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn text_message(role: &str, text: &str) -> Message {
    message(role, Some(Content::Text(text.to_owned())))
}

fn text_part(text: &str) -> ContentPart {
    ContentPart {
        kind: "text".to_owned(),
        fields: Object::from_iter([("text".to_owned(), Json::Text(text.to_owned()))]),
    }
}

mod merges_consecutive_system_message_runs;
mod moves_instruction_messages_first;
