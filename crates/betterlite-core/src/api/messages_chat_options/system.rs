use betterlite_prompts::arguments::json::{Json, Object};

use crate::api::messages_content_primitives::markers::{
    carry_cache_breakpoint, copy_cache_control,
};

const SYSTEM: &str = "system";
const TEXT_BLOCK: &str = "text";
const TYPE: &str = "type";
const TEXT: &str = "text";

fn system_message(content: Json) -> Object {
    let mut message = Object::new();
    message.insert("role".to_owned(), Json::Text(SYSTEM.to_owned()));
    message.insert("content".to_owned(), content);
    message
}

fn text_object(block: &Object, model: Option<&str>) -> Object {
    let text = block.get(TEXT).and_then(Json::as_str).unwrap_or_default();
    let mut part = Object::new();
    part.insert(TYPE.to_owned(), Json::Text(TEXT_BLOCK.to_owned()));
    part.insert(TEXT.to_owned(), Json::Text(text.to_owned()));
    copy_cache_control(block, &mut part, model);
    carry_cache_breakpoint(block, &part)
}

// An in-sequence system turn keeps its role and position, and empty text leaves no turn at all.
#[must_use]
pub fn mid_turn_system_message(message: &Object, model: Option<&str>) -> Option<Object> {
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        if text.is_empty() {
            return None;
        }
        return Some(system_message(Json::Text(text.to_owned())));
    }
    let Json::List(blocks) = content else {
        return None;
    };
    let mut parts = Vec::new();
    for block in blocks {
        let Some(block) = block.as_object() else {
            continue;
        };
        if block.get(TYPE).and_then(Json::as_str) != Some(TEXT_BLOCK) {
            continue;
        }
        let empty = block
            .get(TEXT)
            .and_then(Json::as_str)
            .is_none_or(str::is_empty);
        if empty {
            continue;
        }
        parts.push(Json::Object(text_object(block, model)));
    }
    if parts.is_empty() {
        return None;
    }
    Some(system_message(Json::List(parts)))
}

// The top-level system prompt is one turn inserted at index 0; unlike an in-sequence block it keeps empty text.
pub fn insert_system_message(messages: &mut Vec<Json>, request: &Object) {
    let Some(system) = request.get(SYSTEM).filter(|value| value.is_truthy()) else {
        return;
    };
    if let Some(text) = system.as_str() {
        messages.insert(0, Json::Object(system_message(Json::Text(text.to_owned()))));
        return;
    }
    let Json::List(blocks) = system else {
        return;
    };
    let model = request.get("model").and_then(Json::as_str);
    let mut parts = Vec::new();
    for block in blocks {
        let Some(block) = block.as_object() else {
            continue;
        };
        if block.get(TYPE).and_then(Json::as_str) != Some(TEXT_BLOCK) {
            continue;
        }
        parts.push(Json::Object(text_object(block, model)));
    }
    if parts.is_empty() {
        return;
    }
    messages.insert(0, Json::Object(system_message(Json::List(parts))));
}
