use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use crate::api::messages_chat_output::output::ContentBlock;

use super::state::BlockKind;

// B966's derives are the one source of each wire shape, so an event copies the typed value through serde
pub(super) fn json_from_serialize<T>(value: &T) -> Result<Json, serde_json::Error>
where
    T: Serialize + ?Sized,
{
    match serde_json::to_string(value) {
        Ok(text) => serde_json::from_str(&text),
        Err(error) => Err(error),
    }
}

// The synthesized content_block_start is the object form of B966's block, since every Anthropic frame is an object
pub(super) fn start_object(block: &ContentBlock) -> Result<Object, serde_json::Error> {
    let text = serde_json::to_string(block)?;
    let start: Object = serde_json::from_str(&text)?;
    Ok(start)
}

// The pinned translate step reports these three names, and the adapter's own fallback is a text block
pub(super) const fn block_kind(block: &ContentBlock) -> BlockKind {
    match block {
        ContentBlock::Text { .. } | ContentBlock::RedactedThinking { .. } => BlockKind::Text,
        ContentBlock::Thinking { .. } => BlockKind::Thinking,
        ContentBlock::ToolUse { .. } => BlockKind::ToolUse,
    }
}
