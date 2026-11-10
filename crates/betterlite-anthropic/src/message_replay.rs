pub mod advisor;
pub mod cache_control;
pub mod provider_fields;
pub mod tool_ids;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::ContentPart;
use betterlite_prompts::reasoning_fields::encrypted_signature::is_encrypted_reasoning_block;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockStrip {
    Thinking,
    EncryptedReasoning,
    EmptyContent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyKind {
    Text,
    Thinking,
}

#[must_use]
pub fn is_invalid_thinking_error(error_text: &str) -> bool {
    if error_text.is_empty() {
        return false;
    }
    let lower = error_text.to_lowercase();
    if !lower.contains("thinking") {
        return false;
    }
    if lower.contains("signature") && (lower.contains("invalid") || lower.contains("valid string"))
    {
        return true;
    }
    lower.contains("must contain thinking")
}

impl BlockStrip {
    fn removes(self, block: &Json) -> bool {
        match self {
            Self::Thinking => is_thinking_block(block),
            Self::EncryptedReasoning => {
                as_content_part(block).is_some_and(|part| is_encrypted_reasoning_block(&part))
            }
            Self::EmptyContent => {
                is_empty_block(block, EmptyKind::Text) || is_empty_block(block, EmptyKind::Thinking)
            }
        }
    }
}

// LiteLLM's thinking strip drops a message whose content list was empty before it ran.
#[must_use]
pub fn strip_blocks(messages: &[Json], strip: BlockStrip) -> Vec<Json> {
    let mut stripped = Vec::with_capacity(messages.len());
    for message in messages {
        let Some(fields) = message.as_object() else {
            stripped.push(message.clone());
            continue;
        };
        let Some(Json::List(blocks)) = fields.get("content") else {
            stripped.push(message.clone());
            continue;
        };
        let kept: Vec<Json> = blocks
            .iter()
            .filter(|block| !strip.removes(block))
            .cloned()
            .collect();
        if kept.len() == blocks.len() {
            if blocks.is_empty() && strip == BlockStrip::Thinking {
                continue;
            }
            stripped.push(message.clone());
            continue;
        }
        if kept.is_empty() {
            continue;
        }
        let mut copy = fields.clone();
        copy.insert("content".to_owned(), Json::List(kept));
        stripped.push(Json::Object(copy));
    }
    stripped
}

fn is_thinking_block(block: &Json) -> bool {
    block.as_object().is_some_and(|fields| {
        matches!(
            fields.get("type").and_then(Json::as_str),
            Some("thinking" | "redacted_thinking")
        )
    })
}

fn as_content_part(block: &Json) -> Option<ContentPart> {
    let fields = block.as_object()?;
    let Json::Text(kind) = fields.get("type")? else {
        return None;
    };
    let mut rest = fields.clone();
    rest.shift_remove("type");
    Some(ContentPart {
        kind: kind.clone(),
        fields: rest,
    })
}

// LiteLLM repairs a request dict in place, which is why this takes the payload mutably.
pub fn strip_thinking_from_request(data: &mut Object) {
    if let Some(Json::List(messages)) = data.get("messages") {
        let stripped = strip_blocks(messages, BlockStrip::Thinking);
        data.insert("messages".to_owned(), Json::List(stripped));
    }
    if data.contains_key("thinking") {
        data.shift_remove("thinking");
    }
}

#[must_use]
pub fn is_empty_block(block: &Json, kind: EmptyKind) -> bool {
    let Some(fields) = block.as_object() else {
        return false;
    };
    let (type_name, text_field) = match kind {
        EmptyKind::Text => ("text", "text"),
        EmptyKind::Thinking => ("thinking", "thinking"),
    };
    if fields.get("type").and_then(Json::as_str) != Some(type_name) {
        return false;
    }
    match fields.get(text_field) {
        Some(Json::Text(text)) => text.trim().is_empty(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => true,
    }
}

#[must_use]
pub fn is_empty_unsigned_thinking(block: &Json) -> bool {
    let Some(fields) = block.as_object() else {
        return false;
    };
    is_empty_block(block, EmptyKind::Thinking)
        && !fields.get("signature").is_some_and(Json::is_truthy)
}

mod tests;
