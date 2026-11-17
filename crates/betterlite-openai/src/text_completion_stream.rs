use betterlite_models::decisions::Json;
use serde_json::{Map, Value};
use thiserror::Error;

use betterlite_streaming::stream_chunk_shaping::decision::is_delta_empty;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonValue, StreamChunk, TokenDetails, Usage,
};

use crate::stream::SseError;
use crate::stream::SseParser;
use crate::stream::chunk::{Chunk, ChunkError};

// What a text stream decode can fail on: the B005 framing limits or a chunk of the wrong shape.
#[derive(Debug, Error)]
pub enum TextStreamDecodeError {
    #[error("a text stream event exceeds the framing limit")]
    Framing(#[from] SseError),
    #[error("a text stream chunk is invalid: {source}")]
    Chunk(#[from] ChunkError),
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
        other => other,
    }
}

pub const DONE_FRAME: &str = "data: [DONE]\n\n";

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
        details.extra.insert(
            String::from("reasoning_tokens"),
            JsonValue::Number(serde_json::Number::from(0_u8)),
        );
        details.extra.insert(
            String::from("text_tokens"),
            JsonValue::Number(serde_json::Number::from(usage.completion_tokens)),
        );
        usage.completion_tokens_details = Some(details);
    }
    usage
}

// The chat chunk a text chunk becomes on the chat route: role, content, finish reason, and usage only if asked
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
    let choices = chunk
        .choices
        .iter()
        .map(|choice| {
            let mut delta = Map::new();
            if let Some(role) = &choice.delta.role {
                delta.insert(String::from("role"), Value::from(role.as_str()));
            }
            if let Some(content) = &choice.delta.content {
                delta.insert(String::from("content"), Value::from(content.as_str()));
            }
            let mut converted = Map::new();
            converted.insert(String::from("index"), Value::from(choice.index));
            converted.insert(String::from("delta"), Value::Object(delta));
            if let Some(reason) = &choice.finish_reason {
                converted.insert(String::from("finish_reason"), Value::from(reason.as_str()));
            }
            Value::Object(converted)
        })
        .collect();
    let mut fields = Map::new();
    fields.insert(String::from("id"), Value::from(chunk.id.as_str()));
    fields.insert(String::from("object"), Value::from(chunk.object.as_str()));
    fields.insert(String::from("created"), Value::from(chunk.created));
    let model = if model.is_empty() {
        chunk.model.as_str()
    } else {
        model
    };
    fields.insert(String::from("model"), Value::from(model));
    fields.insert(String::from("choices"), Value::Array(choices));
    if include_usage && let Some(usage) = &chunk.usage {
        fields.insert(
            String::from("usage"),
            serde_json::to_value(with_details(usage.clone()))?,
        );
    }
    Ok(Some(without_nulls(Value::Object(fields))))
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
            chunks.push(
                serde_json::to_value(&chunk)
                    .map_err(|source| TextStreamDecodeError::Chunk(ChunkError::from(source)))
                    .and_then(|value| {
                        serde_json::from_value(value).map_err(|source| {
                            TextStreamDecodeError::Chunk(ChunkError::from(source))
                        })
                    })?,
            );
        }
    }
    Ok(chunks)
}

// The whole-buffer form, for the committed corpus and the fuzz target
pub fn decode_chat_bytes(
    bytes: &[u8],
    model: &str,
    include_usage: bool,
) -> Result<Vec<Json>, TextStreamDecodeError> {
    let mut parser = SseParser::default();
    let mut events = parser.push(bytes)?;
    events.extend(parser.finish());
    decode_chat_events(&events, model, include_usage)
}

mod tests;
