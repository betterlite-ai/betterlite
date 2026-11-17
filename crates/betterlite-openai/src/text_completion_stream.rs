use betterlite_models::decisions::Json;
use serde_json::Value;
use thiserror::Error;

use betterlite_streaming::stream_chunk_shaping::decision::is_delta_empty;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonValue, StreamChoice, StreamChunk, TokenDetails, Usage,
};

use crate::stream::SseError;
use crate::stream::chunk::{Chunk, ChunkError};

// What a text stream decode can fail on: the B005 framing limits or a chunk of the wrong shape.
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
    let model = if model.is_empty() {
        chunk.model.as_str()
    } else {
        model
    };
    let usage = chunk.usage.clone().map(with_details);
    let usage = serde_json::to_value(usage.filter(|_| include_usage))?;
    let fields = serde_json::json!({
        "id": chunk.id,
        "object": chunk.object,
        "created": chunk.created,
        "model": model,
        "choices": chunk.choices.iter().map(chat_choice).collect::<Vec<_>>(),
        "usage": usage,
    });
    Ok(Some(without_nulls(fields)))
}

fn chat_choice(choice: &StreamChoice) -> Value {
    let delta = serde_json::json!({
        "role": choice.delta.role.as_deref(),
        "content": choice.delta.content.as_deref(),
    });
    serde_json::json!({
        "index": choice.index,
        "delta": delta,
        "finish_reason": choice.finish_reason.as_deref(),
    })
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
