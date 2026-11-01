#![cfg(test)]

use super::*;

fn text_message(role: &str, text: &str) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Text(text.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn text_part(text: &str) -> ContentPart {
    ContentPart {
        kind: "text".to_owned(),
        fields: Object::from_iter([("text".to_owned(), Json::Text(text.to_owned()))]),
    }
}

mod inserts_a_new_leading_system_message;
mod merges_into_an_existing_system_message;
mod skips_insertion_for_an_empty_prompt;
