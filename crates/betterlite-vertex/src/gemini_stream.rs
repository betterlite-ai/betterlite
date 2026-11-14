pub mod adapters;
mod buffer;
pub mod chunk;
mod embedded;
pub mod error;
mod parser;
mod tests;
use crate::gemini_response::complete::SERVICE_TIER_HEADER;
use crate::gemini_response_parts::projection::CallMode;
use buffer::FragmentBuffer;
use chunk::StreamChunk;
use error::StreamError;
use http::header::HeaderMap;
use serde_json::value::RawValue;
use uuid::Uuid;
#[derive(Debug, Clone, Copy)]
pub struct StreamOptions {
    pub now: i64,
    pub call_mode: CallMode,
}
pub struct GeminiStreamDecoder {
    options: StreamOptions,
    ids: Box<dyn FnMut() -> Uuid + Send>,
    service_tier: Option<String>,
    accumulated: bool,
    buffer: FragmentBuffer,
    cumulative_tool_call_index: u32,
    has_seen_tool_calls: bool,
}
impl GeminiStreamDecoder {
    pub fn new(
        options: StreamOptions,
        headers: &HeaderMap,
        ids: Box<dyn FnMut() -> Uuid + Send>,
    ) -> Self {
        Self {
            options,
            ids,
            service_tier: stream_service_tier(headers),
            accumulated: false,
            buffer: FragmentBuffer::default(),
            cumulative_tool_call_index: 0,
            has_seen_tool_calls: false,
        }
    }
    pub fn push(&mut self, fragment: &[u8]) -> Result<Option<StreamChunk>, StreamError> {
        let text = std::str::from_utf8(fragment).map_err(|source| StreamError::Utf8 { source })?;
        if text.is_empty() {
            return Ok(None);
        }
        self.parse_fragment(text)
    }
    pub fn finish(&mut self) -> Result<Option<StreamChunk>, StreamError> {
        if !self.accumulated {
            return Ok(None);
        }
        self.accumulated_chunk("", true)
    }
    fn parse_fragment(&mut self, fragment: &str) -> Result<Option<StreamChunk>, StreamError> {
        let chunk = strip_sse_data(fragment);
        if chunk.is_empty() {
            return Ok(None);
        }
        if self.accumulated {
            return self.accumulated_chunk(chunk, false);
        }
        let trimmed = chunk.trim();
        match serde_json::from_str::<Box<RawValue>>(trimmed) {
            Ok(value) => self.chunk_parser(value.get().as_bytes()).map(Some),
            Err(_) => {
                self.accumulated = true;
                self.accumulated_chunk(trimmed, false)
            }
        }
    }
    fn accumulated_chunk(
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
