use std::collections::BTreeMap;

use super::index::{self, TextCursor};
use super::tool_calls::update_tool_calls_from_masked;
use super::wire::{LassoMessage, ModelResponseShape, ResponseMessage, ToolUseBlock};

// message.content takes the next masked text in order, and tool_calls[].function.arguments takes its masked tool_use block by id; a text-count mismatch leaves content untouched but tool_calls still rewrite.
#[must_use]
pub fn mask_model_response(
    response: &ModelResponseShape,
    masked: &[LassoMessage],
) -> ModelResponseShape {
    let tool_use = index::index_tool_blocks(masked).tool_use;
    let texts = index::collect_texts(masked);
    let original_text_count = response
        .choices
        .iter()
        .filter_map(|choice| choice.message.as_ref())
        .filter(|message| has_response_text(message))
        .count();
    let mut cursor = TextCursor {
        texts: &texts,
        index: 0,
        enabled: original_text_count == texts.len(),
    };
    let mut updated = response.clone();
    for choice in &mut updated.choices {
        if let Some(message) = choice.message.as_mut() {
            apply_message_masking(message, &mut cursor, &tool_use);
        }
    }
    updated
}

fn apply_message_masking(
    message: &mut ResponseMessage,
    cursor: &mut TextCursor<'_>,
    tool_use: &BTreeMap<String, ToolUseBlock>,
) {
    if has_response_text(message)
        && let Some(text) = cursor.next_text()
    {
        message.content = Some(text);
    }
    if let Some(calls) = message.tool_calls.as_ref() {
        message.tool_calls = Some(update_tool_calls_from_masked(calls, tool_use));
    }
}

fn has_response_text(message: &ResponseMessage) -> bool {
    let Some(text) = message.content.as_deref() else {
        return false;
    };
    !text.is_empty()
}
