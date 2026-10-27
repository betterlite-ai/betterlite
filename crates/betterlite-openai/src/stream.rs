pub mod chunk;

use std::mem;
use std::num::NonZeroUsize;

use thiserror::Error;

const MAX_BUFFER_BYTES: usize = 0x0010_0000;

#[derive(Debug, Error)]
pub enum SseError {
    #[error("the provider SSE event exceeds the byte limit")]
    TooLarge,
}

#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, SseError> {
        if self.buffer.len().saturating_add(bytes.len()) > MAX_BUFFER_BYTES {
            return Err(SseError::TooLarge);
        }
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = event_end(&self.buffer) {
            let event = self.buffer.drain(..end.get()).collect::<Vec<_>>();
            let payload = event_payload(&event);
            if !payload.is_empty() {
                events.push(payload);
            }
        }
        Ok(events)
    }

    pub fn finish(&mut self) -> Vec<Vec<u8>> {
        let remainder = mem::take(&mut self.buffer);
        let payload = event_payload(&remainder);
        if payload.is_empty() {
            Vec::new()
        } else {
            vec![payload]
        }
    }
}

fn event_end(buffer: &[u8]) -> Option<NonZeroUsize> {
    let mut line_start = 0;
    let mut skip_lf = false;
    for (position, byte) in buffer.iter().enumerate() {
        if skip_lf && *byte == b'\n' {
            line_start = position.checked_add(1)?;
            skip_lf = false;
            continue;
        }
        skip_lf = *byte == b'\r';
        if !matches!(*byte, b'\r' | b'\n') {
            continue;
        }
        let end = position.checked_add(1)?;
        if position == line_start {
            let length = if skip_lf && buffer.get(end) == Some(&b'\n') {
                end.checked_add(1)?
            } else {
                end
            };
            return NonZeroUsize::new(length);
        }
        line_start = end;
    }
    None
}

fn event_payload(event: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    for line in event.split(|byte| *byte == b'\n' || *byte == b'\r') {
        let Some(data_line) = line.strip_prefix(b"data:") else {
            continue;
        };
        let line_payload = data_line.strip_prefix(b" ").unwrap_or(data_line);
        if !payload.is_empty() {
            payload.push(b'\n');
        }
        payload.extend_from_slice(line_payload);
    }
    payload
}

mod tests;
