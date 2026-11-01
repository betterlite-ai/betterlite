#![cfg(test)]

use super::*;
use crate::arguments::json::{Json, Object};
use crate::message_content::ContentPart;

fn part(kind: &str, fields: Object) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields,
    }
}

fn message(role: &str, parts: Vec<ContentPart>) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn text_part(text: &str) -> ContentPart {
    part(
        "text",
        Object::from_iter([("text".to_owned(), Json::Text(text.to_owned()))]),
    )
}

mod drops_tool_reference_parts_from_tool_messages;
mod passes_through_without_tool_reference_parts;
