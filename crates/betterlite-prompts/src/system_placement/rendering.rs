use crate::arguments::json::Json;
use crate::message_content::{Content, ContentPart, Message};
use crate::reasoning_fields::encrypted_signature::is_unsignable_thinking_block;

const RENDERED_PART_TYPES: [&str; 4] = ["text", "image_url", "document", "file"];
const RENDERED_ASSISTANT_PART_TYPES: [&str; 2] = ["text", "server_tool_use"];
const THINKING_BLOCK_TYPES: [&str; 2] = ["thinking", "redacted_thinking"];

// A text part always renders: the converter pads empty text with a placeholder.
#[must_use]
pub fn assistant_part_renders(part: &ContentPart) -> bool {
    if part.kind == "thinking" {
        let has_text = part
            .fields
            .get("thinking")
            .and_then(Json::as_str)
            .is_some_and(|text| !text.is_empty());
        // Only a thinking block Anthropic can verify renders; never a bridged encrypted one.
        return has_text
            && THINKING_BLOCK_TYPES.contains(&part.kind.as_str())
            && !is_unsignable_thinking_block(part);
    }
    RENDERED_ASSISTANT_PART_TYPES.contains(&part.kind.as_str())
        || part.kind.ends_with("_tool_result")
}

// thinking_blocks reach the wire only when no inline thinking-type part is present, rendered or not.
fn separate_thinking_renders(message: &Message, parts: &[ContentPart]) -> bool {
    if parts
        .iter()
        .any(|part| THINKING_BLOCK_TYPES.contains(&part.kind.as_str()))
    {
        return false;
    }
    thinking_blocks_of(message).iter().any(|block| {
        THINKING_BLOCK_TYPES.contains(&block.kind.as_str()) && !is_unsignable_thinking_block(block)
    })
}

fn thinking_blocks_of(message: &Message) -> Vec<ContentPart> {
    let Some(Json::List(blocks)) = message.provider_specific_fields.get("thinking_blocks") else {
        return Vec::new();
    };
    blocks.iter().filter_map(json_to_content_part).collect()
}

fn json_to_content_part(value: &Json) -> Option<ContentPart> {
    let Json::Object(fields) = value else {
        return None;
    };
    let kind = fields.get("type").and_then(Json::as_str)?.to_owned();
    let mut rest = fields.clone();
    rest.shift_remove("type");
    Some(ContentPart { kind, fields: rest })
}

fn field_is_set(message: &Message, key: &str) -> bool {
    message
        .provider_specific_fields
        .get(key)
        .is_some_and(Json::is_truthy)
}

// LiteLLM's own provider_specific_fields dict carries compaction_blocks, nested inside betterlite's flattened catch-all.
fn has_compaction_blocks(message: &Message) -> bool {
    message
        .provider_specific_fields
        .get("provider_specific_fields")
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("compaction_blocks"))
        .is_some_and(Json::is_truthy)
}

fn assistant_renders_parts(message: &Message, parts: &[ContentPart]) -> bool {
    parts.iter().any(assistant_part_renders)
        || separate_thinking_renders(message, parts)
        || field_is_set(message, "tool_calls")
        || field_is_set(message, "function_call")
        || has_compaction_blocks(message)
}

// An assistant message renders if it has string content, a rendering part, tool calls, a function call or compaction blocks; otherwise it vanishes from the wire.
#[must_use]
pub fn assistant_renders(message: &Message) -> bool {
    match &message.content {
        Some(Content::Text(_)) => true,
        Some(Content::Parts(parts)) => assistant_renders_parts(message, parts),
        None => assistant_renders_parts(message, &[]),
    }
}

// A tool message always renders; a user message renders through string content or a part type the converter emits.
#[must_use]
pub fn renders(message: &Message) -> bool {
    if matches!(message.role.as_str(), "tool" | "function") {
        return true;
    }
    if message.role == "assistant" {
        return assistant_renders(message);
    }
    match &message.content {
        Some(Content::Text(_)) => true,
        Some(Content::Parts(parts)) => parts
            .iter()
            .any(|part| RENDERED_PART_TYPES.contains(&part.kind.as_str())),
        None => false,
    }
}

mod tests;
