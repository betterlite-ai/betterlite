#![cfg(test)]

use super::*;
use crate::arguments::json::{Json, Object};

fn content_part(kind: &str, fields: Object) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields,
    }
}

fn thinking_part(thinking: Option<&str>, signature: &str) -> ContentPart {
    let mut fields = Object::new();
    if let Some(text) = thinking {
        fields.insert("thinking".to_owned(), Json::Text(text.to_owned()));
    }
    fields.insert("signature".to_owned(), Json::Text(signature.to_owned()));
    content_part("thinking", fields)
}

fn encrypted_block() -> ContentPart {
    thinking_part(None, "litellm_encrypted_reasoning:abc")
}

fn plain_block() -> ContentPart {
    thinking_part(Some("because"), "anthropic-signed")
}

fn message(content: Option<Content>) -> Message {
    Message {
        role: "assistant".to_owned(),
        content,
        name: None,
        provider_specific_fields: Object::new(),
    }
}

mod does_not_mutate_the_input;
mod drops_encrypted_blocks_unconditionally;
mod honors_the_should_strip_predicate;
mod leaves_non_list_content_untouched;
