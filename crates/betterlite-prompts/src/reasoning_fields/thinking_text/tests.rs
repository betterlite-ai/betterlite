#![cfg(test)]

use super::*;
use crate::arguments::json::{Json, Object};

fn content_part(kind: &str, fields: Object) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields,
    }
}

mod flattens_thinking_blocks_into_reasoning_content;
mod reads_thinking_text_from_a_block;
