use std::mem;

use serde::Deserialize;
use thiserror::Error;

pub const MAX_BUFFER_BYTES: usize = 0x0010_0000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StreamEvent {
    pub generated_text: String,
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("the NLP Cloud stream line is not a generated_text event")]
    Malformed {
        #[source]
        source: serde_json::Error,
    },
    #[error("the NLP Cloud stream buffer exceeds the byte limit")]
    TooLarge,
}

#[derive(Debug, Default)]
pub struct StreamCleaner {
    buffer: String,
}

impl StreamCleaner {
    // LiteLLM yields a buffer once its NUL separators are gone; an empty line is skipped rather than ending the stream
    pub fn push(&mut self, line: &str) -> Result<Option<String>, StreamError> {
        if self.buffer.len().saturating_add(line.len()) > MAX_BUFFER_BYTES {
            return Err(StreamError::TooLarge);
        }
        self.buffer.push_str(line);
        if !self.buffer.contains('\0') {
            return Ok(None);
        }
        let cleaned = self.buffer.replace('\0', "");
        self.buffer.clear();
        if cleaned.is_empty() {
            return Ok(None);
        }
        Ok(Some(cleaned))
    }

    pub fn finish(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            return None;
        }
        Some(mem::take(&mut self.buffer))
    }
}

pub fn decode_stream_line(line: &str) -> Result<StreamEvent, StreamError> {
    match serde_json::from_str(line) {
        Ok(event) => Ok(event),
        Err(source) => Err(StreamError::Malformed { source }),
    }
}
