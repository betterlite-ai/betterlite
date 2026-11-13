use thiserror::Error;

use crate::stream_chunk_shaping::records::JsonValue;
use records::Chunk;

pub mod provider_metadata;
pub mod records;
pub mod response;
mod tests;

// LiteLLM indexes chunks[0] after sorting, so an empty list has no first chunk to build from.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StreamBaseAssemblyError {
    #[error("the chunk list is empty")]
    NoChunks,
    #[error("the first chunk has no object")]
    MissingObject,
    #[error("the first chunk has no created timestamp")]
    MissingCreated,
    #[error("the first chunk has no model")]
    MissingModel,
}

// The sorted chunks plus the original first chunk, which stays the response's metadata source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamedChunks {
    pub chunks: Vec<Chunk>,
    pub first_chunk: Chunk,
}

impl StreamedChunks {
    pub fn new(chunks: Vec<Chunk>) -> Result<Self, StreamBaseAssemblyError> {
        let Some(first_chunk) = chunks.first().cloned() else {
            return Err(StreamBaseAssemblyError::NoChunks);
        };
        Ok(Self {
            chunks: sort_chunks(chunks),
            first_chunk,
        })
    }
}

fn sort_chunks(mut chunks: Vec<Chunk>) -> Vec<Chunk> {
    let Some(first) = chunks.first() else {
        return chunks;
    };
    let Some(created_at) = first
        .hidden_object()
        .and_then(|hidden| hidden.get("created_at"))
    else {
        return chunks;
    };
    // Only a set created_at on the original first chunk turns sorting on; 0 keeps the input order.
    if !is_set(created_at) {
        return chunks;
    }
    chunks.sort_by(|left, right| sort_key(left).total_cmp(&sort_key(right)));
    chunks
}

fn sort_key(chunk: &Chunk) -> f64 {
    match chunk
        .hidden_object()
        .and_then(|hidden| hidden.get("created_at"))
    {
        Some(JsonValue::Number(number)) => number.as_f64().unwrap_or(f64::INFINITY),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::String(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => f64::INFINITY,
    }
}

fn is_set(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number.as_f64().is_some_and(|amount| amount != 0.0_f64),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}
