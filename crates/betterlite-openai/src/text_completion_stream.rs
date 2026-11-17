// The text-route stream shapes: provider chunks turned into the chat chunks the text wrapper reads,
// and chat chunks turned into the text chunks the client reads, all written with LiteLLM's JSON.

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

// One SSE event of the client's answer.
#[must_use]
pub fn event_frame(value: &Value) -> String {
    format!("data: {value}\n\n")
}

pub const DONE_FRAME: &str = "data: [DONE]\n\n";

// The chat chunk the text wrapper converts: the provider's usage counters gain the detail block
// litellm's Usage validator fills in, and the route names the model the client asked for.
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

// The chat chunk a text endpoint's chunk becomes on the chat route: the wrapper carries the first
// delta's role, the content and the finish reason, and the usage chunk only when it was asked for.
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

// A chat endpoint's SSE bytes read as the text route's chunks: B005's framing, then the chunk parse
// the chat route already trusts, with the client's usage choice.
#[derive(Debug)]
pub struct ChatTextStream {
    parser: SseParser,
    model: String,
    include_usage: bool,
}

impl ChatTextStream {
    #[must_use]
    pub fn new(model: String, include_usage: bool) -> Self {
        Self {
            parser: SseParser::default(),
            model,
            include_usage,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Json>, TextStreamDecodeError> {
        let events = self.parser.push(bytes)?;
        self.chunks(&events)
    }

    pub fn finish(&mut self) -> Result<Vec<Json>, TextStreamDecodeError> {
        let events = self.parser.finish();
        self.chunks(&events)
    }

    fn chunks(&self, events: &[Vec<u8>]) -> Result<Vec<Json>, TextStreamDecodeError> {
        let mut chunks = Vec::new();
        for event in events {
            if event.trim_ascii() == b"[DONE]" {
                continue;
            }
            if let Some(chunk) = Chunk::parse(event, &self.model, self.include_usage)? {
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
}

// The whole-buffer form of ChatTextStream, for the committed corpus and the fuzz target.
pub fn decode_chat_events(
    bytes: &[u8],
    model: &str,
    include_usage: bool,
) -> Result<Vec<Json>, TextStreamDecodeError> {
    let mut stream = ChatTextStream::new(model.to_owned(), include_usage);
    let mut chunks = stream.push(bytes)?;
    chunks.extend(stream.finish()?);
    Ok(chunks)
}

mod tests;
