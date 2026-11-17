use betterlite_models::decisions::Json;

use super::{ChunkKind, StreamChunk};

#[must_use]
pub fn serialize_chunk(chunk: &StreamChunk) -> String {
    let body = Json::Object(chunk.body.clone());
    let written = match chunk.kind {
        ChunkKind::Plain => body,
        ChunkKind::ModelStream | ChunkKind::ModelFull | ChunkKind::Event => without_nulls(&body),
    };
    // Json always serializes; an impossible encoding failure must not lose the stream
    serde_json::to_string(&written).unwrap_or_default()
}

// model_dump_json(exclude_none=True) drops a null field at any depth; a null array item stays
fn without_nulls(value: &Json) -> Json {
    match value {
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => value.clone(),
        Json::List(items) => Json::List(items.iter().map(without_nulls).collect()),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .filter_map(|(name, item)| match item {
                    Json::Null => None,
                    other => Some((name.clone(), without_nulls(other))),
                })
                .collect(),
        ),
    }
}

#[must_use]
pub fn is_usage_artifact(chunk: &StreamChunk) -> bool {
    if chunk.kind != ChunkKind::ModelStream {
        return false;
    }
    if !matches!(
        chunk.body.get("provider_specific_fields"),
        None | Some(Json::Null)
    ) {
        return false;
    }
    match chunk.body.get("choices") {
        None | Some(Json::Null) => true,
        Some(Json::List(choices)) => choices.iter().all(empty_choice),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => false,
    }
}

fn empty_choice(choice: &Json) -> bool {
    let Json::Object(fields) = choice else {
        return false;
    };
    let unfinished = matches!(fields.get("finish_reason"), None | Some(Json::Null));
    let without_logprobs = matches!(fields.get("logprobs"), None | Some(Json::Null));
    if !unfinished || !without_logprobs {
        return false;
    }
    match fields.get("delta") {
        None | Some(Json::Null) => true,
        Some(Json::Object(delta)) => delta.values().all(|value| matches!(value, Json::Null)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => false,
    }
}

#[must_use]
pub fn chunk_response_text(chunk: &StreamChunk) -> String {
    let container = match chunk.kind {
        ChunkKind::ModelStream => "delta",
        ChunkKind::ModelFull => "message",
        ChunkKind::Event | ChunkKind::Plain => return String::new(),
    };
    let Some(Json::List(choices)) = chunk.body.get("choices") else {
        return String::new();
    };
    let mut text = String::new();
    for choice in choices {
        let Json::Object(fields) = choice else {
            continue;
        };
        let Some(Json::Object(message)) = fields.get(container) else {
            continue;
        };
        match message.get("content") {
            None | Some(Json::Null) => {}
            Some(Json::Text(part)) => text.push_str(part),
            // any other content is joined as its JSON text
            Some(other) => text.push_str(&serde_json::to_string(other).unwrap_or_default()),
        }
    }
    text
}
