use betterlite_streaming::streamed_response_assembly::records::{Chunk, Delta};

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::request::tool_argument_parts;
use crate::cisco_conversation::response::reasoning_parts;

pub(super) fn chunk_scan_text(chunks: &[Chunk]) -> String {
    let mut original_text = String::new();
    let mut argument_text = String::new();
    for chunk in chunks {
        for choice in &chunk.choices {
            let Some(delta) = &choice.delta else {
                continue;
            };
            if let Some(content) = &delta.content {
                original_text.push_str(content);
            }
            let fields = delta_fields(delta);
            let reasoning = reasoning_parts(&fields).join(" ");
            if !reasoning.is_empty() {
                original_text.push_str(&reasoning);
            }
            for arguments in tool_argument_parts(&fields) {
                argument_text.push_str(&arguments);
            }
        }
    }
    let mut parts = Vec::new();
    for part in [original_text, argument_text] {
        if !part.is_empty() {
            parts.push(part);
        }
    }
    parts.join(" ")
}

fn delta_fields(delta: &Delta) -> Fields {
    let Ok(text) = serde_json::to_string(delta) else {
        return Fields::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}
