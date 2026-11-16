use super::items::MessagePart;
use super::records::{Block, Content};

pub(super) fn system_parts(content: Option<&Content>) -> Vec<MessagePart> {
    match content {
        None => Vec::new(),
        Some(Content::Text(text)) => text_parts(text),
        Some(Content::Blocks(blocks)) => block_parts(blocks),
    }
}

fn text_parts(text: &str) -> Vec<MessagePart> {
    if text.is_empty() {
        return Vec::new();
    }
    vec![MessagePart::Text {
        text: text.to_owned(),
        prompt_cache_breakpoint: None,
    }]
}

// Only a non-empty text block reads as mid-turn system content; every other block shape is dropped.
fn block_parts(blocks: &[Block]) -> Vec<MessagePart> {
    let mut parts = Vec::new();
    for block in blocks {
        let Block::Text(text) = block else {
            continue;
        };
        if text.text.is_empty() {
            continue;
        }
        parts.push(MessagePart::Text {
            text: text.text.clone(),
            prompt_cache_breakpoint: text.prompt_cache_breakpoint,
        });
    }
    parts
}
