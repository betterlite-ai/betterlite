use std::mem::take;
use std::str::from_utf8;

use serde::Deserialize;
use serde_json::value::RawValue;

pub const MAX_LINE_BYTES: usize = 0x0100_0000;

#[derive(Debug, thiserror::Error)]
pub enum StreamLineError {
    #[error("the stream line is not valid UTF-8")]
    NotUtf8,
    #[error("the stream chunk has a shape the proxy does not send")]
    Chunk(#[source] serde_json::Error),
    #[error("the stream line is larger than {limit} bytes")]
    TooLarge { limit: usize },
}

#[derive(Debug, Eq, PartialEq)]
pub enum LineEvent {
    Ignored,
    Content(String),
    Done,
}

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Default, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

pub fn decode_line(line: &[u8]) -> Result<LineEvent, StreamLineError> {
    let Ok(text) = from_utf8(line) else {
        return Err(StreamLineError::NotUtf8);
    };
    if text.is_empty() {
        return Ok(LineEvent::Ignored);
    }
    let Some(payload) = text.strip_prefix("data: ") else {
        return Ok(LineEvent::Ignored);
    };
    if payload.trim() == "[DONE]" {
        return Ok(LineEvent::Done);
    }
    let Ok(raw) = serde_json::from_str::<&RawValue>(payload) else {
        return Ok(LineEvent::Ignored);
    };
    let chunk: StreamChunk = serde_json::from_str(raw.get()).map_err(StreamLineError::Chunk)?;
    let Some(choice) = chunk.choices.into_iter().next() else {
        return Ok(LineEvent::Ignored);
    };
    let Some(content) = choice.delta.content else {
        return Ok(LineEvent::Ignored);
    };
    if content.is_empty() {
        return Ok(LineEvent::Ignored);
    }
    Ok(LineEvent::Content(content))
}

#[derive(Default)]
pub struct StreamLines {
    pending: Vec<u8>,
}

impl StreamLines {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<LineEvent>, StreamLineError> {
        let mut events = Vec::new();
        let mut rest = chunk;
        while let Some(position) = rest.iter().position(|byte| *byte == b'\n') {
            let (head, tail) = rest.split_at(position);
            self.pending.extend_from_slice(head);
            if self.pending.len() > MAX_LINE_BYTES {
                return Err(StreamLineError::TooLarge {
                    limit: MAX_LINE_BYTES,
                });
            }
            let line = take(&mut self.pending);
            events.push(decode_line(line.strip_suffix(b"\r").unwrap_or(&line))?);
            rest = tail.get(1..).unwrap_or_default();
        }
        self.pending.extend_from_slice(rest);
        if self.pending.len() > MAX_LINE_BYTES {
            return Err(StreamLineError::TooLarge {
                limit: MAX_LINE_BYTES,
            });
        }
        Ok(events)
    }

    pub fn finish(&mut self) -> Result<Option<LineEvent>, StreamLineError> {
        if self.pending.is_empty() {
            return Ok(None);
        }
        let line = take(&mut self.pending);
        let event = decode_line(line.strip_suffix(b"\r").unwrap_or(&line))?;
        Ok(Some(event))
    }
}
