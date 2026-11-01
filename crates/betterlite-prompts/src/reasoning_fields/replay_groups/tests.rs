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

fn thinking_block(text: &str) -> ContentPart {
    thinking_part(Some(text), "anthropic-signed")
}

fn encrypted_block(encrypted_content: &str) -> ContentPart {
    thinking_part(
        None,
        &format!("litellm_encrypted_reasoning:{encrypted_content}"),
    )
}

mod collapses_a_run_of_thinking_blocks;
mod drops_empty_summary_only_groups;
mod gives_each_encrypted_block_its_own_item;
mod interleaves_encrypted_and_summary_groups;
