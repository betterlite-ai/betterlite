use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{ChunkKind, StreamChunk};

mod tool_shapes;

// these keys hold declared models that exclude_none descends into; other values are dictionary entries
#[derive(Clone, Copy)]
enum Shape {
    Value,
    Fields(&'static [(&'static str, Shape)]),
    Tagged(&'static [(&'static str, Shape)]),
    Tools(&'static [(&'static str, Shape)]),
}

const EMPTY: &[(&str, Shape)] = &[];
const TOOL_CALL: &[(&str, Shape)] = &[
    ("custom", Shape::Fields(EMPTY)),
    ("function", Shape::Fields(EMPTY)),
];
const REASONING: &[(&str, Shape)] = &[("summary", Shape::Fields(EMPTY))];
const IMAGE: &[(&str, Shape)] = &[("image_url", Shape::Fields(EMPTY))];
const ANNOTATION: &[(&str, Shape)] = &[("url_citation", Shape::Fields(EMPTY))];
const LOGPROB: &[(&str, Shape)] = &[("top_logprobs", Shape::Fields(EMPTY))];
const LOGPROBS: &[(&str, Shape)] = &[("content", Shape::Fields(LOGPROB))];
const TOKEN_DETAILS: &[(&str, Shape)] = &[
    ("cache_creation_token_details", Shape::Fields(EMPTY)),
    ("cached_tokens_details", Shape::Fields(EMPTY)),
];
const USAGE: &[(&str, Shape)] = &[
    ("completion_tokens_details", Shape::Fields(EMPTY)),
    ("prompt_tokens_details", Shape::Fields(TOKEN_DETAILS)),
    ("server_tool_use", Shape::Fields(EMPTY)),
];
const DELTA: &[(&str, Shape)] = &[
    ("function_call", Shape::Fields(EMPTY)),
    ("reasoning_items", Shape::Fields(REASONING)),
    ("thinking_blocks", Shape::Fields(EMPTY)),
    ("tool_calls", Shape::Fields(TOOL_CALL)),
];
const STREAM_CHOICE: &[(&str, Shape)] = &[
    ("delta", Shape::Fields(DELTA)),
    ("logprobs", Shape::Fields(LOGPROBS)),
];
const STREAM_BODY: &[(&str, Shape)] = &[
    ("choices", Shape::Fields(STREAM_CHOICE)),
    ("usage", Shape::Fields(USAGE)),
];
const MESSAGE: &[(&str, Shape)] = &[
    ("annotations", Shape::Fields(ANNOTATION)),
    ("audio", Shape::Fields(EMPTY)),
    ("function_call", Shape::Fields(EMPTY)),
    ("images", Shape::Fields(IMAGE)),
    ("reasoning_items", Shape::Fields(REASONING)),
    ("thinking_blocks", Shape::Fields(EMPTY)),
    ("tool_calls", Shape::Fields(TOOL_CALL)),
];
const FULL_CHOICE: &[(&str, Shape)] = &[
    ("logprobs", Shape::Fields(LOGPROBS)),
    ("message", Shape::Fields(MESSAGE)),
];
const FULL_BODY: &[(&str, Shape)] = &[
    ("choices", Shape::Fields(FULL_CHOICE)),
    ("usage", Shape::Fields(USAGE)),
];
const INPUT_DETAILS: &[(&str, Shape)] = &[("cached_tokens_details", Shape::Fields(EMPTY))];
const RESPONSE_USAGE: &[(&str, Shape)] = &[
    ("input_tokens_details", Shape::Fields(INPUT_DETAILS)),
    ("output_tokens_details", Shape::Fields(EMPTY)),
];
const TEXT: &[(&str, Shape)] = &[("format", Shape::Fields(EMPTY))];
const RESPONSE: &[(&str, Shape)] = &[
    ("incomplete_details", Shape::Fields(EMPTY)),
    ("text", Shape::Fields(TEXT)),
    ("tool_choice", Shape::Fields(EMPTY)),
    ("tools", Shape::Tools(tool_shapes::TOOL_VARIANTS)),
    ("usage", Shape::Fields(RESPONSE_USAGE)),
];
const RESPONSE_EVENT: &[(&str, Shape)] = &[("response", Shape::Fields(RESPONSE))];
const ITEM_EVENT: &[(&str, Shape)] = &[("item", Shape::Fields(EMPTY))];
const PART_EVENT: &[(&str, Shape)] = &[("part", Shape::Fields(EMPTY))];
const CONTENT_PART: &[(&str, Shape)] = &[
    ("annotations", Shape::Fields(EMPTY)),
    ("logprobs", Shape::Fields(LOGPROB)),
];
const TEXT_PART_EVENT: &[(&str, Shape)] = &[("part", Shape::Fields(CONTENT_PART))];
const ERROR_EVENT: &[(&str, Shape)] = &[("error", Shape::Fields(EMPTY))];
const EVENT_FIELDS: &[(&str, &[(&str, Shape)])] = &[
    ("error", ERROR_EVENT),
    ("response.completed", RESPONSE_EVENT),
    ("response.content_part.added", PART_EVENT),
    ("response.content_part.done", TEXT_PART_EVENT),
    ("response.created", RESPONSE_EVENT),
    ("response.failed", RESPONSE_EVENT),
    ("response.in_progress", RESPONSE_EVENT),
    ("response.incomplete", RESPONSE_EVENT),
    ("response.output_item.added", ITEM_EVENT),
    ("response.output_item.done", ITEM_EVENT),
    ("response.reasoning_summary_part.added", EMPTY),
    ("response.reasoning_summary_part.done", PART_EVENT),
];

#[must_use]
pub fn serialize_chunk(chunk: &StreamChunk) -> String {
    let body = Json::Object(chunk.body.clone());
    let written = without_nulls(&body, body_shape(chunk.kind, &chunk.body));
    // Json always serializes; an impossible encoding failure must not lose the stream
    serde_json::to_string(&written).unwrap_or_default()
}

fn body_shape(kind: ChunkKind, body: &JsonObject) -> Shape {
    match kind {
        ChunkKind::Plain => Shape::Value,
        ChunkKind::ModelStream => Shape::Fields(STREAM_BODY),
        ChunkKind::ModelFull => Shape::Fields(FULL_BODY),
        ChunkKind::Event => event_shape(body),
    }
}

fn event_shape(body: &JsonObject) -> Shape {
    let Some(Json::Text(kind)) = body.get("type") else {
        return Shape::Fields(EMPTY);
    };
    EVENT_FIELDS
        .iter()
        .find(|(event, _fields)| *event == kind.as_str())
        .map_or(Shape::Fields(EMPTY), |(_event, fields)| {
            Shape::Fields(*fields)
        })
}

// model_dump_json(exclude_none=True): a null model field goes, a null entry of a dictionary field stays
fn without_nulls(value: &Json, shape: Shape) -> Json {
    match shape {
        Shape::Value => value.clone(),
        Shape::Fields(fields) => strip_nulls(value, fields),
        Shape::Tagged(variants) => tagged_object(value, variants),
        Shape::Tools(variants) => tool_shapes::tools_without_nulls(value, variants),
    }
}

// a tagged union member is a model when its type matches, and anything else stays a dictionary
fn tagged_object(value: &Json, variants: &[(&str, Shape)]) -> Json {
    let Json::Object(entries) = value else {
        return match value {
            Json::List(items) => Json::List(
                items
                    .iter()
                    .map(|item| tagged_object(item, variants))
                    .collect(),
            ),
            other => other.clone(),
        };
    };
    let Some(Json::Text(kind)) = entries.get("type") else {
        return value.clone();
    };
    variants
        .iter()
        .find(|(name, _shape)| *name == kind.as_str())
        .map_or_else(
            || value.clone(),
            |(_name, shape)| without_nulls(value, *shape),
        )
}

// exclude_none drops a model's null fields; an array keeps null items and walks every other item
fn strip_nulls(value: &Json, fields: &[(&str, Shape)]) -> Json {
    match value {
        Json::Object(entries) => Json::Object(
            entries
                .iter()
                .filter(|(_name, item)| !matches!(*item, Json::Null))
                .map(|(name, item)| (name.clone(), without_nulls(item, field_shape(fields, name))))
                .collect(),
        ),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| item_without_nulls(item, fields))
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => value.clone(),
    }
}

fn item_without_nulls(item: &Json, fields: &[(&str, Shape)]) -> Json {
    match item {
        Json::Null => Json::Null,
        other => without_nulls(other, Shape::Fields(fields)),
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
