use betterlite_models::decisions::Json;
use serde_json::{Map, Value};
use thiserror::Error;

use betterlite_streaming::stream_chunk_shaping::decision::is_delta_empty;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonValue, StreamChoice, StreamChunk, TokenDetails, Usage,
};

use crate::stream::SseError;
use crate::stream::chunk::{Chunk, ChunkError};

#[derive(Debug, Error)]
pub enum TextStreamDecodeError {
    #[error("a text stream event exceeds the framing limit")]
    Framing(#[from] SseError),
    #[error("a text stream chunk is invalid: {source}")]
    Chunk {
        #[from]
        source: ChunkError,
    },
}

// pydantic's exclude_none: a null field of the answer is left out of the client's JSON.
#[must_use]
pub fn without_nulls(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(without_nulls).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(_, item)| !item.is_null())
                .map(|(name, item)| (name, without_nulls(item)))
                .collect(),
        ),
        other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => other,
    }
}

pub const DONE_FRAME: &str = "data: [DONE]\n\n";

// one entry of a JSON object; an absent option is the null without_nulls drops from the client's JSON
pub(crate) fn field(name: &str, value: Option<Value>) -> (String, Value) {
    let item = value.unwrap_or(Value::Null);
    (name.to_owned(), item)
}

// The chat chunk the text wrapper converts: usage gains the detail block litellm fills in, model restamped
pub fn shaped_chunk(mut chunk: StreamChunk, model: &str) -> Result<Value, serde_json::Error> {
    if !model.is_empty() {
        model.clone_into(&mut chunk.model);
    }
    if let Some(usage) = &mut chunk.usage {
        *usage = with_details(usage.clone());
    }
    serde_json::to_value(chunk)
}

// LiteLLM's Usage validator fills the completion detail block when the provider sent none.
fn with_details(mut usage: Usage) -> Usage {
    if usage.completion_tokens_details.is_none() {
        let mut details = TokenDetails::default();
        let fields = &mut details.extra;
        let reasoning = JsonValue::Number(serde_json::Number::from(0_u8));
        let tokens = JsonValue::Number(serde_json::Number::from(usage.completion_tokens));
        fields.insert(String::from("reasoning_tokens"), reasoning);
        fields.insert(String::from("text_tokens"), tokens);
        usage.completion_tokens_details = Some(details);
    }
    usage
}

pub fn chat_chunk(
    chunk: &StreamChunk,
    model: &str,
    include_usage: bool,
) -> Result<Option<Value>, serde_json::Error> {
    let silent = chunk
        .choices
        .iter()
        .all(|choice| is_delta_empty(&choice.delta) && choice.finish_reason.is_none());
    if silent && (chunk.usage.is_none() || !include_usage) {
        return Ok(None);
    }
    let given = Some(model).filter(|name| !name.is_empty());
    let named = given.unwrap_or(chunk.model.as_str());
    let asked = chunk.usage.clone().map(with_details);
    let usage = serde_json::to_value(asked.filter(|_| include_usage))?;
    let choices = chunk.choices.iter().map(chat_choice).collect();
    let fields = Value::Object(Map::from_iter([
        field("id", Some(Value::from(chunk.id.as_str()))),
        field("object", Some(Value::from(chunk.object.as_str()))),
        field("created", Some(Value::from(chunk.created))),
        field("model", Some(Value::from(named))),
        field("choices", Some(Value::Array(choices))),
        field("usage", Some(usage)),
    ]));
    Ok(Some(without_nulls(fields)))
}

fn chat_choice(choice: &StreamChoice) -> Value {
    let delta = Value::Object(Map::from_iter([
        field("role", choice.delta.role.as_deref().map(Value::from)),
        field("content", choice.delta.content.as_deref().map(Value::from)),
    ]));
    let finish = choice.finish_reason.as_deref();
    Value::Object(Map::from_iter([
        field("index", Some(Value::from(choice.index))),
        field("delta", Some(delta)),
        field("finish_reason", finish.map(Value::from)),
    ]))
}

// A chat endpoint's SSE events read as the text route's chunks: the chat route's chunk parse, DONE skipped
pub fn decode_chat_events(
    events: &[Vec<u8>],
    model: &str,
    include_usage: bool,
) -> Result<Vec<Json>, TextStreamDecodeError> {
    let mut chunks = Vec::new();
    for event in events {
        if event.trim_ascii() == b"[DONE]" {
            continue;
        }
        if let Some(chunk) = Chunk::parse(event, model, include_usage)? {
            let value = serde_json::to_value(&chunk).map_err(ChunkError::from)?;
            chunks.push(serde_json::from_value(value).map_err(ChunkError::from)?);
        }
    }
    Ok(chunks)
}

mod tests;
