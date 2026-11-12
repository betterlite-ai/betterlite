use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_codex_tools::records::input_item_role;
use crate::api::responses_input_content::content_blocks;
use crate::api::responses_input_messages::Assembly;
use crate::api::responses_input_messages::content_message;
use crate::api::responses_input_messages::error::InputMessagesError;
use crate::api::responses_input_messages::thinking::decode_thinking_blocks;

pub(crate) fn reasoning_messages(
    fields: &IndexMap<String, Json>,
    assembly: &Assembly<'_>,
) -> Result<Vec<Value>, InputMessagesError> {
    if assembly.replay_reasoning {
        return replay_message(fields, assembly.limits);
    }
    inspection_message(fields)
}

// inspection keeps the text a guardrail has to read, so it stays visible message content
fn inspection_message(fields: &IndexMap<String, Json>) -> Result<Vec<Value>, InputMessagesError> {
    let content = fields
        .get("content")
        .filter(|value| !matches!(value, Json::Null));
    let inspectable = if reasoning_text_from_content(fields).is_some() {
        content.cloned()
    } else {
        reasoning_text_from_summary(fields)
            .map(Json::Text)
            .or_else(|| content.cloned())
    };
    let Some(visible) = inspectable else {
        return Ok(Vec::new());
    };
    let role = input_item_role(fields)?;
    let converted = content_blocks(&serde_json::to_value(&visible)?)?;
    Ok(vec![content_message(&role, converted)])
}

fn replay_message(
    fields: &IndexMap<String, Json>,
    limits: ApiLimits,
) -> Result<Vec<Value>, InputMessagesError> {
    let text = reasoning_text_from_content(fields).or_else(|| reasoning_text_from_summary(fields));
    let blocks = signed_blocks(fields, limits)?;
    if text.is_none() && blocks.is_none() {
        return Ok(Vec::new());
    }
    let mut message = Map::new();
    message.insert("role".to_owned(), Value::String("assistant".to_owned()));
    message.insert("content".to_owned(), Value::Null);
    if let Some(reasoning_text) = text {
        message.insert(
            "reasoning_content".to_owned(),
            Value::String(reasoning_text),
        );
    }
    if let Some(thinking_blocks) = blocks {
        message.insert("thinking_blocks".to_owned(), Value::Array(thinking_blocks));
    }
    Ok(vec![Value::Object(message)])
}

fn signed_blocks(
    fields: &IndexMap<String, Json>,
    limits: ApiLimits,
) -> Result<Option<Vec<Value>>, InputMessagesError> {
    let Some(Json::Text(encrypted)) = fields.get("encrypted_content") else {
        return Ok(None);
    };
    if encrypted.trim().is_empty() {
        return Ok(None);
    }
    Ok(decode_thinking_blocks(encrypted.as_bytes(), limits)?)
}

fn reasoning_text_from_content(fields: &IndexMap<String, Json>) -> Option<String> {
    match fields.get("content") {
        Some(Json::Text(text)) if !text.trim().is_empty() => Some(text.clone()),
        Some(Json::List(blocks)) => {
            let parts: Vec<&str> = blocks.iter().filter_map(plaintext_block).collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) | Json::Text(_)) => {
            None
        }
    }
}

fn reasoning_text_from_summary(fields: &IndexMap<String, Json>) -> Option<String> {
    let Some(Json::List(blocks)) = fields.get("summary") else {
        return None;
    };
    let parts: Vec<&str> = blocks.iter().filter_map(stripped_text).collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn plaintext_block(block: &Json) -> Option<&str> {
    let Json::Object(fields) = block else {
        return None;
    };
    let opaque = matches!(
        fields.get("type"),
        Some(Json::Text(kind)) if kind == "encrypted_content" || kind == "redacted_thinking"
    );
    if opaque {
        return None;
    }
    stripped_text(block)
}

fn stripped_text(block: &Json) -> Option<&str> {
    let Json::Object(fields) = block else {
        return None;
    };
    match fields.get("text") {
        Some(Json::Text(text)) if !text.trim().is_empty() => Some(text.trim()),
        None
        | Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Text(_),
        ) => None,
    }
}
