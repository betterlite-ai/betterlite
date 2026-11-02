#![cfg(test)]

use super::*;
use crate::arguments::json::{Json, Object};

fn message(content: Option<Content>, provider_specific_fields: Object) -> Message {
    Message {
        role: "assistant".to_owned(),
        content,
        name: None,
        provider_specific_fields,
    }
}

mod falls_back_to_tag_parsing;
mod parses_reasoning_tags_from_text;
mod reasoning_key_precedence;
