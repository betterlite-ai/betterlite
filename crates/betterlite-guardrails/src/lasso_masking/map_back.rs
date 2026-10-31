use std::collections::BTreeMap;

use crate::bedrock_message_selection::wire::{Content, Role};

use super::index::{self, TextCursor};
use super::tool_calls::update_tool_calls_from_masked;
use super::wire::{LassoMessage, Message, ToolUseBlock};

// A tool message takes its content from the masked tool result by call id; every other message with text content takes the next masked text in order, and an assistant message's tool_calls are rewritten whenever present, regardless of whether its own content changed.
#[must_use]
pub fn map_masked_messages_back(original: &[Message], masked: &[LassoMessage]) -> Vec<Message> {
    let index::ToolIndex {
        tool_use,
        tool_result,
    } = index::index_tool_blocks(masked);
    let texts = index::collect_texts(masked);
    let original_text_count = original
        .iter()
        .filter(|message| message.role != Role::Tool && has_text_content(message))
        .count();
    let mut cursor = TextCursor {
        texts: &texts,
        index: 0,
        enabled: original_text_count == texts.len(),
    };
    original
        .iter()
        .map(|message| map_one_message(message, &mut cursor, &tool_use, &tool_result))
        .collect()
}

fn map_one_message(
    message: &Message,
    cursor: &mut TextCursor<'_>,
    tool_use: &BTreeMap<String, ToolUseBlock>,
    tool_result: &BTreeMap<String, String>,
) -> Message {
    let mut updated = message.clone();
    if message.role == Role::Tool {
        if let Some(text) = message
            .tool_call_id
            .as_ref()
            .and_then(|id| tool_result.get(id))
        {
            updated.content = Some(Content::Text(text.clone()));
        }
        return updated;
    }
    if has_text_content(message)
        && let Some(text) = cursor.next_text()
    {
        updated.content = Some(Content::Text(text));
    }
    if message.role == Role::Assistant
        && let Some(calls) = message.tool_calls.as_ref()
    {
        updated.tool_calls = Some(update_tool_calls_from_masked(calls, tool_use));
    }
    updated
}

// A string takes the cursor only when non-empty; a multimodal list takes it unconditionally, mirroring Lasso's own isinstance(content, list) check.
const fn has_text_content(message: &Message) -> bool {
    match message.content.as_ref() {
        Some(Content::Text(text)) => !text.is_empty(),
        Some(Content::List(_)) => true,
        None => false,
    }
}
