use std::mem;

use thiserror::Error;

use super::event::RunEvent;
use crate::watsonx_transform::extract::extract_text_from_wxo_result;

mod tests;

pub const MAX_LINE_BYTES: usize = 0x10_0000;
pub const MAX_TEXT_BYTES: usize = 0x40_0000;

#[derive(Debug, Error)]
pub enum SseError {
    #[error("a watsonx Orchestrate SSE line passed {max} bytes")]
    LineTooLong { max: usize },
    #[error("the watsonx Orchestrate SSE answer text passed {max} bytes")]
    TextTooLong { max: usize },
}

#[derive(Debug, Default)]
pub struct LineAccumulator {
    buffer: Vec<u8>,
    pending_cr: bool,
}

impl LineAccumulator {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, SseError> {
        let mut lines = Vec::new();
        for &byte in chunk {
            if byte == b'\n' && self.pending_cr {
                self.pending_cr = false;
                continue;
            }
            if byte == b'\n' || byte == b'\r' {
                self.pending_cr = byte == b'\r';
                self.end_line(&mut lines);
                continue;
            }
            self.pending_cr = false;
            self.buffer.push(byte);
            if self.buffer.len() > MAX_LINE_BYTES {
                return Err(SseError::LineTooLong {
                    max: MAX_LINE_BYTES,
                });
            }
        }
        Ok(lines)
    }

    fn end_line(&mut self, lines: &mut Vec<String>) {
        let bytes = mem::take(&mut self.buffer);
        // httpx decodes a line before it reaches LiteLLM; bytes that are not text skip it
        if let Ok(line) = String::from_utf8(bytes) {
            lines.push(line);
        }
    }

    pub fn finish(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            return None;
        }
        let bytes = mem::take(&mut self.buffer);
        String::from_utf8(bytes).ok()
    }
}

pub fn accumulate_text(lines: &[String]) -> Result<String, SseError> {
    let mut text = String::new();
    for line in lines {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = data.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(event) = serde_json::from_str::<RunEvent>(payload) else {
            continue;
        };
        text.push_str(&extract_text_from_wxo_result(&event.run));
        if text.len() > MAX_TEXT_BYTES {
            return Err(SseError::TextTooLong {
                max: MAX_TEXT_BYTES,
            });
        }
    }
    Ok(text)
}
