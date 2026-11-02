#![cfg(test)]

use super::*;
use crate::arguments::json::Object;

fn content_part(kind: &str, fields: Object) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields,
    }
}

mod detects_encrypted_reasoning_blocks;
mod detects_unsignable_thinking_blocks;
mod mints_and_reads_the_signature_prefix;
