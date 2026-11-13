use serde_json::{Map, Value};
use uuid::Uuid;

use super::wire::MistralContentBlock;
use crate::chat_choices::stream::{StreamError, chunk_parser as parent_chunk_parser};
use crate::chat_choices::wire::StreamReply;

pub const THINKING_SIGNATURE: &str = "mistral";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MistralChatResponseIterator {
    pub json_mode: bool,
}

#[must_use]
pub fn get_model_response_iterator(json_mode: bool) -> MistralChatResponseIterator {
    MistralChatResponseIterator { json_mode }
}

pub fn chunk_parser(
    chunk: &mut Value,
    identifier: fn() -> Uuid,
    clock: fn() -> i64,
) -> Result<StreamReply, StreamError> {
    normalize_choices(chunk);
    parent_chunk_parser(chunk, identifier, clock)
}

struct Normalized {
    text: Option<String>,
    thinking_blocks: Vec<Value>,
    reasoning: Option<String>,
}

fn normalize_choices(chunk: &mut Value) -> Option<()> {
    let choices = match chunk.get_mut("choices") {
        None => return Some(()),
        Some(Value::Array(choices)) => choices,
        Some(_) => return None,
    };
    for choice in choices {
        let Some(choice_fields) = choice.as_object_mut() else {
            return None;
        };
        let Some(delta) = choice_fields.get_mut("delta") else {
            continue;
        };
        let delta_fields = delta.as_object_mut()?;
        let Some(Value::Array(blocks)) = delta_fields.get("content") else {
            continue;
        };
        let normalized = normalize_content_blocks(blocks)?;
        delta_fields.insert(
            String::from("content"),
            normalized.text.map_or(Value::Null, Value::String),
        );
        if normalized.thinking_blocks.is_empty() {
            delta_fields.shift_remove("thinking_blocks");
            delta_fields.shift_remove("reasoning_content");
        } else {
            delta_fields.insert(
                String::from("thinking_blocks"),
                Value::Array(normalized.thinking_blocks),
            );
            delta_fields.insert(
                String::from("reasoning_content"),
                normalized.reasoning.map_or(Value::Null, Value::String),
            );
        }
    }
    Some(())
}

fn normalize_content_blocks(blocks: &[Value]) -> Option<Normalized> {
    let mut text_parts: Vec<String> = Vec::new();
    let mut thinking_blocks: Vec<Value> = Vec::new();
    let mut reasoning_parts: Vec<String> = Vec::new();
    for block in blocks {
        let decoded: MistralContentBlock = serde_json::from_value(block.clone()).ok()?;
        match decoded.kind.as_str() {
            "thinking" => {
                let text = decoded.thinking_text("");
                if !text.is_empty() {
                    reasoning_parts.push(text.clone());
                    thinking_blocks.push(thinking_block(&text));
                }
            }
            "text" => text_parts.push(decoded.text.unwrap_or_default()),
            _ => {}
        }
    }
    Some(Normalized {
        text: (!text_parts.is_empty()).then(|| text_parts.concat()),
        thinking_blocks,
        reasoning: (!reasoning_parts.is_empty()).then(|| reasoning_parts.join("\n")),
    })
}

fn thinking_block(text: &str) -> Value {
    let mut fields = Map::new();
    fields.insert(
        String::from("type"),
        Value::String(String::from("thinking")),
    );
    fields.insert(String::from("thinking"), Value::String(text.to_owned()));
    fields.insert(
        String::from("signature"),
        Value::String(String::from(THINKING_SIGNATURE)),
    );
    Value::Object(fields)
}
