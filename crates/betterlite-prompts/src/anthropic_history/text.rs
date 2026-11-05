use crate::arguments::json::{Json, Object};

const EMPTY_TEXT: &str = "[System: Empty message content sanitised to satisfy protocol]";

#[must_use]
pub fn sanitize_empty_text_content(message: &Json) -> Json {
    let Some(message_fields) = message.as_object() else {
        return message.clone();
    };
    if !is_text_role(message_fields) {
        return message.clone();
    }
    match message_fields.get("content") {
        Some(Json::Text(content)) if content.trim().is_empty() => {
            let mut sanitized = message_fields.clone();
            sanitized.insert("content".to_owned(), Json::Text(EMPTY_TEXT.to_owned()));
            Json::Object(sanitized)
        }
        Some(Json::List(blocks)) => sanitize_text_blocks(message, message_fields, blocks),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => message.clone(),
    }
}

fn is_text_role(message_fields: &Object) -> bool {
    matches!(
        message_fields.get("role").and_then(Json::as_str),
        Some("user" | "assistant")
    )
}

fn sanitize_text_blocks(message: &Json, message_fields: &Object, blocks: &[Json]) -> Json {
    let mut rewritten = false;
    let content = blocks
        .iter()
        .map(|block| replace_empty_text_block(block, &mut rewritten))
        .collect::<Vec<_>>();
    if rewritten {
        let mut sanitized = message_fields.clone();
        sanitized.insert("content".to_owned(), Json::List(content));
        Json::Object(sanitized)
    } else {
        message.clone()
    }
}

fn replace_empty_text_block(block: &Json, rewritten: &mut bool) -> Json {
    if !is_empty_text_block(block) {
        return block.clone();
    }
    *rewritten = true;
    let Some(block_fields) = block.as_object() else {
        return block.clone();
    };
    let mut replaced = block_fields.clone();
    replaced.insert("text".to_owned(), Json::Text(EMPTY_TEXT.to_owned()));
    Json::Object(replaced)
}

fn is_empty_text_block(block: &Json) -> bool {
    let Some(block_fields) = block.as_object() else {
        return false;
    };
    let is_text = block_fields.get("type").and_then(Json::as_str) == Some("text");
    let is_empty = block_fields
        .get("text")
        .and_then(Json::as_str)
        .is_none_or(|text| text.trim().is_empty());
    is_text && is_empty
}
