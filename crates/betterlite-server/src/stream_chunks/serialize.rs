use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{ChunkKind, StreamChunk};

// the declared model fields a chunk body can nest models under; any other key holds an
// arbitrary value, and pydantic's exclude_none leaves entries of those dictionaries alone
#[derive(Clone, Copy)]
enum Shape {
    Value,
    Model(&'static [(&'static str, Shape)]),
    List(&'static [(&'static str, Shape)]),
}

const EMPTY: &[(&str, Shape)] = &[];
const TOOL_CALL: &[(&str, Shape)] = &[
    ("custom", Shape::Model(EMPTY)),
    ("function", Shape::Model(EMPTY)),
];
const THINKING: &[(&str, Shape)] = &[];
const REASONING: &[(&str, Shape)] = &[("summary", Shape::List(EMPTY))];
const IMAGE: &[(&str, Shape)] = &[("image_url", Shape::Model(EMPTY))];
const ANNOTATION: &[(&str, Shape)] = &[("url_citation", Shape::Model(EMPTY))];
const LOGPROB: &[(&str, Shape)] = &[("top_logprobs", Shape::List(EMPTY))];
const LOGPROBS: &[(&str, Shape)] = &[("content", Shape::List(LOGPROB))];
const DELTA: &[(&str, Shape)] = &[
    ("function_call", Shape::Model(EMPTY)),
    ("reasoning_items", Shape::List(REASONING)),
    ("thinking_blocks", Shape::List(THINKING)),
    ("tool_calls", Shape::List(TOOL_CALL)),
];
const STREAM_CHOICE: &[(&str, Shape)] = &[
    ("delta", Shape::Model(DELTA)),
    ("logprobs", Shape::Model(LOGPROBS)),
];
const STREAM_BODY: &[(&str, Shape)] = &[
    ("choices", Shape::List(STREAM_CHOICE)),
    ("usage", Shape::Model(EMPTY)),
];
const MESSAGE: &[(&str, Shape)] = &[
    ("annotations", Shape::List(ANNOTATION)),
    ("audio", Shape::Model(EMPTY)),
    ("function_call", Shape::Model(EMPTY)),
    ("images", Shape::List(IMAGE)),
    ("reasoning_items", Shape::List(REASONING)),
    ("thinking_blocks", Shape::List(THINKING)),
    ("tool_calls", Shape::List(TOOL_CALL)),
];
const FULL_CHOICE: &[(&str, Shape)] = &[
    ("logprobs", Shape::Model(LOGPROBS)),
    ("message", Shape::Model(MESSAGE)),
];
const FULL_BODY: &[(&str, Shape)] = &[
    ("choices", Shape::List(FULL_CHOICE)),
    ("usage", Shape::Model(EMPTY)),
];

#[must_use]
pub fn serialize_chunk(chunk: &StreamChunk) -> String {
    let body = Json::Object(chunk.body.clone());
    let written = match chunk.kind {
        ChunkKind::Plain => body,
        ChunkKind::ModelStream => without_nulls(&body, Shape::Model(STREAM_BODY)),
        ChunkKind::ModelFull => without_nulls(&body, Shape::Model(FULL_BODY)),
        ChunkKind::Event => without_nulls(&body, Shape::Model(EMPTY)),
    };
    // Json always serializes; an impossible encoding failure must not lose the stream
    serde_json::to_string(&written).unwrap_or_default()
}

// model_dump_json(exclude_none=True): a null model field goes, a null entry of a dictionary field stays
fn without_nulls(value: &Json, shape: Shape) -> Json {
    match shape {
        Shape::Value => value.clone(),
        Shape::Model(fields) => strip_model_nulls(value, fields),
        Shape::List(fields) => strip_list_nulls(value, fields),
    }
}

fn strip_model_nulls(value: &Json, fields: &[(&str, Shape)]) -> Json {
    let Json::Object(entries) = value else {
        return value.clone();
    };
    let mut kept = JsonObject::new();
    for (name, item) in entries {
        if !matches!(item, Json::Null) {
            kept.insert(name.clone(), without_nulls(item, field_shape(fields, name)));
        }
    }
    Json::Object(kept)
}

// a null list item is data, so it stays; every other item is a model at this level
fn strip_list_nulls(value: &Json, fields: &[(&str, Shape)]) -> Json {
    let Json::List(items) = value else {
        return value.clone();
    };
    let mut kept = Vec::new();
    for item in items {
        kept.push(item_without_nulls(item, fields));
    }
    Json::List(kept)
}

fn item_without_nulls(item: &Json, fields: &[(&str, Shape)]) -> Json {
    match item {
        Json::Null => Json::Null,
        other => without_nulls(other, Shape::Model(fields)),
    }
}

fn field_shape(fields: &[(&str, Shape)], name: &str) -> Shape {
    fields
        .iter()
        .find(|(field, _shape)| *field == name)
        .map_or(Shape::Value, |(_field, shape)| *shape)
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
