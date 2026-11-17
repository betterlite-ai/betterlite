use std::mem::take;

use betterlite_models::decisions::Json;

use super::MessagesStreamChunk;
use super::python_text::{decode_utf8_ignore, is_space, splitlines_keepends};

pub const MESSAGE_START_EVENT: &str = "message_start";
const MESSAGE_START_MARKER: &[u8] = b"message_start";
const SSE_DATA_FIELD: &str = "data:";
const SSE_DONE_PAYLOAD: &str = "[DONE]";
pub const MAX_HELD_BYTES: usize = 0x0001_0000;
const PING_MARKERS: [&[u8]; 3] = [b"event: ping", b"\"type\": \"ping\"", b"\"type\":\"ping\""];
const SSE_FRAME_ENDS: [&[u8]; 3] = [b"\r\n\r\n", b"\r\r", b"\n\n"];
const TYPE_FIELD: &str = "type";
const MODEL_FIELD: &str = "model";
const MESSAGE_FIELD: &str = "message";

// LiteLLM's restamp_anthropic_stream_chunk_model: only a chunk that mentions message_start is decoded, and a chunk with nothing to restamp comes back as it arrived
#[must_use]
pub fn restamp_chunk(chunk: &MessagesStreamChunk, requested_model: &str) -> MessagesStreamChunk {
    match chunk {
        MessagesStreamChunk::Event(event) => restamped_event(event, requested_model)
            .map_or_else(|| chunk.clone(), MessagesStreamChunk::Event),
        MessagesStreamChunk::Bytes(bytes) => {
            if contains_marker(bytes, MESSAGE_START_MARKER) {
                MessagesStreamChunk::Bytes(restamp_bytes(bytes, requested_model))
            } else {
                chunk.clone()
            }
        }
        MessagesStreamChunk::Text(text) => {
            if text.contains(MESSAGE_START_EVENT) {
                restamped_frame(text, requested_model)
                    .map_or_else(|| chunk.clone(), MessagesStreamChunk::Text)
            } else {
                chunk.clone()
            }
        }
    }
}

#[must_use]
pub fn is_ping_frame(frame: &[u8]) -> bool {
    PING_MARKERS
        .iter()
        .any(|marker| contains_marker(frame, marker))
}

// LiteLLM's per-stream restamper: complete SSE frames leave as their terminator closes them and torn tails wait in held
pub struct StreamModelRestamper {
    requested_model: String,
    held: Vec<u8>,
    armed: bool,
}

impl StreamModelRestamper {
    #[must_use]
    pub const fn new(requested_model: String) -> Self {
        Self {
            requested_model,
            held: Vec::new(),
            armed: true,
        }
    }

    #[must_use]
    pub fn process(&mut self, chunk: &MessagesStreamChunk) -> MessagesStreamChunk {
        if !self.armed {
            return chunk.clone();
        }
        match chunk {
            MessagesStreamChunk::Event(event) => {
                let restamped = restamp_chunk(chunk, &self.requested_model);
                if event_disarms(event) {
                    self.armed = false;
                }
                restamped
            }
            MessagesStreamChunk::Bytes(bytes) => {
                MessagesStreamChunk::Bytes(self.process_encoded(bytes))
            }
            MessagesStreamChunk::Text(text) => {
                MessagesStreamChunk::Bytes(self.process_encoded(text.as_bytes()))
            }
        }
    }

    #[must_use]
    pub fn flush(&mut self) -> MessagesStreamChunk {
        let held = take(&mut self.held);
        self.armed = false;
        if !contains_marker(&held, MESSAGE_START_MARKER) {
            return MessagesStreamChunk::Bytes(held);
        }
        MessagesStreamChunk::Bytes(restamp_bytes(&held, &self.requested_model))
    }

    fn process_encoded(&mut self, data: &[u8]) -> Vec<u8> {
        let mut combined = take(&mut self.held);
        combined.extend_from_slice(data);
        let boundaries = frame_end_positions(&combined);
        let Some(last) = boundaries.last().copied() else {
            if combined.len() > MAX_HELD_BYTES {
                self.armed = false;
                return combined;
            }
            self.held = combined;
            return Vec::new();
        };
        let closed = combined.get(..last).unwrap_or_default();
        let emitted = self.restamped_closed_block(closed);
        let tail = combined.get(last..).unwrap_or_default().to_vec();
        if !self.armed {
            return [emitted, tail].concat();
        }
        self.held = tail;
        emitted
    }

    fn restamped_closed_block(&mut self, closed: &[u8]) -> Vec<u8> {
        let boundaries = frame_end_positions(closed);
        let frames = frames_between(closed, &boundaries);
        let found_decider = frames.iter().copied().position(is_decider);
        let Some(decider) = found_decider else {
            return closed.to_vec();
        };
        self.armed = false;
        let frame = frames.get(decider).copied().unwrap_or_default();
        if !contains_marker(frame, MESSAGE_START_MARKER) {
            return closed.to_vec();
        }
        let Some(restamped) = restamped_frame(&decode_utf8_ignore(frame), &self.requested_model)
        else {
            return closed.to_vec();
        };
        let mut output = Vec::with_capacity(closed.len());
        for (index, part) in frames.iter().copied().enumerate() {
            if index == decider {
                output.extend_from_slice(restamped.as_bytes());
            } else {
                output.extend_from_slice(part);
            }
        }
        output
    }
}

// The first frame that proves what the stream carries: a message_start, or a data line that is no ping
fn is_decider(frame: &[u8]) -> bool {
    contains_marker(frame, MESSAGE_START_MARKER)
        || (contains_marker(frame, SSE_DATA_FIELD.as_bytes()) && !is_ping_frame(frame))
}

// LiteLLM disarms on any event whose type is neither absent nor null nor ping
fn event_disarms(event: &Json) -> bool {
    let Json::Object(fields) = event else {
        return false;
    };
    match fields.get(TYPE_FIELD) {
        Some(Json::Text(event_type)) => event_type != "ping",
        Some(Json::Null) | None => false,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => true,
    }
}

fn restamped_event(event: &Json, requested_model: &str) -> Option<Json> {
    let Json::Object(fields) = event else {
        return None;
    };
    let is_message_start = matches!(
        fields.get(TYPE_FIELD),
        Some(Json::Text(event_type)) if event_type == MESSAGE_START_EVENT
    );
    if !is_message_start {
        return None;
    }
    let Json::Object(message) = fields.get(MESSAGE_FIELD)? else {
        return None;
    };
    if matches!(message.get(MODEL_FIELD), Some(Json::Text(model)) if model == requested_model) {
        return None;
    }
    let mut restamped_message = message.clone();
    restamped_message.insert(
        MODEL_FIELD.to_owned(),
        Json::Text(requested_model.to_owned()),
    );
    let mut restamped = fields.clone();
    restamped.insert(MESSAGE_FIELD.to_owned(), Json::Object(restamped_message));
    Some(Json::Object(restamped))
}

fn restamped_data_line(line: &str, requested_model: &str) -> Option<String> {
    let payload = line
        .trim_matches(is_space)
        .strip_prefix(SSE_DATA_FIELD)?
        .trim_matches(is_space);
    if matches!(payload, "" | SSE_DONE_PAYLOAD) {
        return None;
    }
    let event: Json = serde_json::from_str(payload).ok()?;
    let restamped = restamped_event(&event, requested_model)?;
    let encoded = serde_json::to_string(&restamped).ok()?;
    let terminator = line.get(line.trim_end_matches(['\r', '\n']).len()..)?;
    Some(format!("data: {encoded}{terminator}"))
}

fn restamped_frame(frame: &str, requested_model: &str) -> Option<String> {
    let lines = splitlines_keepends(frame);
    let restamped: Vec<Option<String>> = lines
        .iter()
        .map(|line| restamped_data_line(line, requested_model))
        .collect();
    if restamped.iter().all(Option::is_none) {
        return None;
    }
    let mut output = String::with_capacity(frame.len());
    for (replacement, original) in restamped.into_iter().zip(lines) {
        output.push_str(replacement.as_deref().unwrap_or(original));
    }
    Some(output)
}

fn restamp_bytes(bytes: &[u8], requested_model: &str) -> Vec<u8> {
    restamped_frame(&decode_utf8_ignore(bytes), requested_model)
        .map_or_else(|| bytes.to_vec(), String::into_bytes)
}

fn contains_marker(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// The ends of LiteLLM's _SSE_FRAME_END_PATTERN matches, scanned leftmost first without overlap
fn frame_end_positions(bytes: &[u8]) -> Vec<usize> {
    let mut ends = Vec::new();
    let mut index = 0_usize;
    while let Some(rest) = bytes.get(index..).filter(|window| !window.is_empty()) {
        let found_width = SSE_FRAME_ENDS
            .iter()
            .find_map(|end| rest.starts_with(end).then_some(end.len()));
        let Some(width) = found_width else {
            index = index.saturating_add(1);
            continue;
        };
        let end = index.saturating_add(width);
        ends.push(end);
        index = end;
    }
    ends
}

fn frames_between<'frame>(closed: &'frame [u8], boundaries: &[usize]) -> Vec<&'frame [u8]> {
    let mut frames = Vec::with_capacity(boundaries.len());
    let mut start = 0_usize;
    for end in boundaries {
        if let Some(frame) = closed.get(start..*end) {
            frames.push(frame);
        }
        start = *end;
    }
    frames
}
