pub mod adapters;
pub mod chunk;
pub mod error;

mod buffer;
mod embedded;
mod parser;

mod tests;

use std::str;

use http::header::HeaderMap;
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::gemini_response::complete::SERVICE_TIER_HEADER;
use crate::gemini_response_parts::projection::CallMode;

use buffer::FragmentBuffer;
use chunk::StreamChunk;
use error::StreamError;

#[derive(Debug, Clone, Copy)]
pub struct StreamOptions {
    pub now: i64,
    pub call_mode: CallMode,
}

pub trait IdSource: Send + Sync {
    fn next_id(&self) -> Uuid;
}

pub struct UuidV4;

impl IdSource for UuidV4 {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkType {
    ValidJson,
    AccumulatedJson,
}

pub struct GeminiStreamDecoder {
    options: StreamOptions,
    ids: Box<dyn IdSource>,
    service_tier: Option<String>,
    chunk_type: ChunkType,
    buffer: FragmentBuffer,
    pending: Vec<u8>,
    cumulative_tool_call_index: u32,
    has_seen_tool_calls: bool,
}

impl GeminiStreamDecoder {
    pub fn new(options: StreamOptions, headers: &HeaderMap, ids: Box<dyn IdSource>) -> Self {
        Self {
            options,
            ids,
            service_tier: stream_service_tier(headers),
            chunk_type: ChunkType::ValidJson,
            buffer: FragmentBuffer::default(),
            pending: Vec::new(),
            cumulative_tool_call_index: 0,
            has_seen_tool_calls: false,
        }
    }

    pub fn push(&mut self, fragment: &[u8]) -> Result<Option<StreamChunk>, StreamError> {
        let text = self.decode_fragment(fragment)?;
        if text.is_empty() {
            return Ok(None);
        }
        self.parse_fragment(&text)
    }

    pub fn finish(&mut self) -> Result<Option<StreamChunk>, StreamError> {
        if !self.pending.is_empty() {
            return Err(StreamError::IncompleteUtf8);
        }
        if self.chunk_type != ChunkType::AccumulatedJson || !self.buffer.has_buffered_content() {
            return Ok(None);
        }
        self.accumulated("", true)
    }

    fn decode_fragment(&mut self, fragment: &[u8]) -> Result<String, StreamError> {
        let mut bytes = std::mem::take(&mut self.pending);
        bytes.extend_from_slice(fragment);
        let error = match str::from_utf8(&bytes) {
            Ok(text) => return Ok(text.to_owned()),
            Err(error) => error,
        };
        if error.error_len().is_some() {
            return Err(StreamError::Utf8 { source: error });
        }
        let Some((valid, incomplete)) = bytes.split_at_checked(error.valid_up_to()) else {
            return Err(StreamError::Utf8 { source: error });
        };
        self.pending = incomplete.to_vec();
        String::from_utf8(valid.to_vec()).map_err(|error| StreamError::Utf8 {
            source: error.utf8_error(),
        })
    }

    fn parse_fragment(&mut self, fragment: &str) -> Result<Option<StreamChunk>, StreamError> {
        let chunk = strip_sse_data(fragment);
        if chunk.is_empty() {
            return Ok(None);
        }
        match self.chunk_type {
            ChunkType::ValidJson => self.valid_json(chunk),
            ChunkType::AccumulatedJson => self.accumulated(chunk, false),
        }
    }

    fn valid_json(&mut self, chunk: &str) -> Result<Option<StreamChunk>, StreamError> {
        let trimmed = chunk.trim();
        match serde_json::from_str::<Box<RawValue>>(trimmed) {
            Ok(value) => self.chunk_parser(value.get().as_bytes()).map(Some),
            Err(_) => {
                self.chunk_type = ChunkType::AccumulatedJson;
                self.accumulated(trimmed, false)
            }
        }
    }

    fn accumulated(
        &mut self,
        chunk: &str,
        is_final: bool,
    ) -> Result<Option<StreamChunk>, StreamError> {
        let message = strip_sse_data(chunk).replace("\n\n", "");
        self.buffer.append(&message)?;
        if !is_final && !self.buffer.could_close {
            return Ok(None);
        }
        loop {
            let Some(value) = self.buffer.pop_next_value() else {
                return Ok(None);
            };
            if value.get().trim_start().starts_with('{') {
                return self.chunk_parser(value.get().as_bytes()).map(Some);
            }
        }
    }
}

fn strip_sse_data(chunk: &str) -> &str {
    match chunk.strip_prefix("data: ") {
        Some(stripped) => stripped,
        None => chunk.strip_prefix("data:").unwrap_or(chunk),
    }
}

fn stream_service_tier(headers: &HeaderMap) -> Option<String> {
    let mut tier = String::new();
    for value in headers.get_all(SERVICE_TIER_HEADER) {
        if !tier.is_empty() {
            tier.push_str(", ");
        }
        tier.extend(value.as_bytes().iter().map(|byte| char::from(*byte)));
    }
    match tier.to_lowercase().as_str() {
        "" => None,
        "standard" => Some("default".to_owned()),
        other => Some(other.to_owned()),
    }
}
