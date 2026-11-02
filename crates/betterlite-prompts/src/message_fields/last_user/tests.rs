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

mod finds_the_trailing_user_block;
mod replaces_the_trailing_user_block;
