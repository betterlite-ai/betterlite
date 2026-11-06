use std::mem;
use std::str;

use serde_json::Value;
use serde_json::error::Category;

use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::events::{RunMessage, RunMessageDeltaEvent, RunStep, RunStepDeltaEvent};
use crate::thread_runs::records::{RunEvent, RunReply};

const MAX_FRAME_BYTES: usize = 0x0010_0000;
const RUN_EVENTS: [&str; 10] = [
    "thread.run.created",
    "thread.run.queued",
    "thread.run.in_progress",
    "thread.run.requires_action",
    "thread.run.completed",
    "thread.run.incomplete",
    "thread.run.failed",
    "thread.run.cancelling",
    "thread.run.cancelled",
    "thread.run.expired",
];
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameOutcome {
    Event(Box<RunEvent>),
    End,
    Skip,
}
// The SDK wraps thread.* events and yields other payloads bare, raising when one carries an error key.
pub fn decode_run_event(bytes: &[u8]) -> Result<FrameOutcome, ThreadRunsError> {
    let frame = event_and_data(bytes);
    if frame.data.starts_with(b"[DONE]") {
        return Ok(FrameOutcome::End);
    }
    if !frame.touched {
        return Ok(FrameOutcome::Skip);
    }
    let value = serde_json::from_slice::<Value>(&frame.data).map_err(classify)?;
    let named = frame
        .event
        .as_deref()
        .filter(|name| name.starts_with("thread."));
    if let Some(name) = named {
        return Ok(FrameOutcome::Event(Box::new(typed_event(name, value)?)));
    }
    if reports_error(&value) {
        return Err(provider_error(&value));
    }
    match value.get("event").and_then(Value::as_str) {
        Some(name) => {
            let data = value.get("data").cloned().unwrap_or(Value::Null);
            Ok(FrameOutcome::Event(Box::new(typed_event(name, data)?)))
        }
        None => Ok(FrameOutcome::Event(Box::new(RunEvent::Bare(value)))),
    }
}
fn typed_event(name: &str, value: Value) -> Result<RunEvent, ThreadRunsError> {
    let event = String::from(name);
    if RUN_EVENTS.contains(&name) {
        let run = serde_json::from_value::<RunReply>(value).map_err(classify)?;
        return Ok(RunEvent::Typed {
            event,
            data: Box::new(run),
        });
    }
    if name == "thread.message.delta" {
        let delta = serde_json::from_value::<RunMessageDeltaEvent>(value).map_err(classify)?;
        return Ok(RunEvent::MessageDelta {
            event,
            data: Box::new(delta),
        });
    }
    if name.starts_with("thread.message.") {
        let message = serde_json::from_value::<RunMessage>(value).map_err(classify)?;
        return Ok(RunEvent::Message {
            event,
            data: Box::new(message),
        });
    }
    if name == "thread.run.step.delta" {
        let delta = serde_json::from_value::<RunStepDeltaEvent>(value).map_err(classify)?;
        return Ok(RunEvent::StepDelta {
            event,
            data: Box::new(delta),
        });
    }
    if name.starts_with("thread.run.step.") {
        let step = serde_json::from_value::<RunStep>(value).map_err(classify)?;
        return Ok(RunEvent::Step {
            event,
            data: Box::new(step),
        });
    }
    Ok(RunEvent::Named { event, data: value })
}
fn reports_error(value: &Value) -> bool {
    let Value::Object(fields) = value else {
        return false;
    };
    !matches!(
        fields.get("error"),
        None | Some(Value::Null | Value::Bool(false))
    )
}
fn provider_error(value: &Value) -> ThreadRunsError {
    let body = value
        .get("error")
        .and_then(|error| serde_json::to_vec(error).ok())
        .unwrap_or_default();
    ThreadRunsError::ProviderError { body }
}
// The shared SseParser strips non-data fields and its event_end is private, so this keeps the event line and CRLF.
fn lines(body: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let (mut start, mut cursor) = (0_usize, 0_usize);
    while let Some(byte) = body.get(cursor) {
        if !matches!(*byte, b'\r' | b'\n') {
            cursor = cursor.saturating_add(1);
            continue;
        }
        if let Some(line) = body.get(start..cursor) {
            lines.push(line);
        }
        let next = cursor.saturating_add(1);
        cursor = if *byte == b'\r' && body.get(next) == Some(&b'\n') {
            next.saturating_add(1)
        } else {
            next
        };
        start = cursor;
    }
    if let Some(tail) = body.get(start..) {
        lines.push(tail);
    }
    lines
}
struct Frame {
    event: Option<String>,
    data: Vec<u8>,
    touched: bool,
}
fn event_and_data(frame: &[u8]) -> Frame {
    let mut parsed = Frame {
        event: None,
        data: Vec::new(),
        touched: false,
    };
    for line in lines(frame) {
        let (name, rest) = split_field(line);
        match name {
            b"event" => {
                parsed.event = str::from_utf8(rest.trim_ascii()).ok().map(str::to_owned);
                parsed.touched = true;
            }
            b"data" => {
                if !parsed.data.is_empty() {
                    parsed.data.push(b'\n');
                }
                parsed
                    .data
                    .extend_from_slice(rest.strip_prefix(b" ").unwrap_or(rest));
                parsed.touched = true;
            }
            b"id" | b"retry" => parsed.touched = true,
            _ => {}
        }
    }
    parsed
}
fn split_field(line: &[u8]) -> (&[u8], &[u8]) {
    line.iter()
        .position(|byte| *byte == b':')
        .map_or((line, &[]), |at| {
            let (name, rest) = line.split_at(at);
            (name, rest.get(1..).unwrap_or_default())
        })
}
// Frames end at a blank line, so a tail the provider cut off before one is never dispatched.
#[derive(Debug, Default)]
pub(crate) struct RunFrames {
    buffer: Vec<u8>,
}
impl RunFrames {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, ThreadRunsError> {
        if self.buffer.len().saturating_add(bytes.len()) > MAX_FRAME_BYTES {
            return Err(ThreadRunsError::StreamTooLarge);
        }
        self.buffer.extend_from_slice(bytes);
        let Some(end) = terminated_end(&self.buffer) else {
            return Ok(Vec::new());
        };
        let frames = frames_in(self.buffer.get(..end).unwrap_or_default());
        self.buffer.drain(..end);
        Ok(frames)
    }
}
// The byte length through the last blank-line terminator; anything after it is an unterminated tail.
fn terminated_end(buffer: &[u8]) -> Option<usize> {
    let (mut line_start, mut end, mut cursor) = (0_usize, None, 0_usize);
    while let Some(byte) = buffer.get(cursor) {
        if !matches!(*byte, b'\r' | b'\n') {
            cursor = cursor.saturating_add(1);
            continue;
        }
        let next = cursor.saturating_add(1);
        let after = if *byte == b'\r' && buffer.get(next) == Some(&b'\n') {
            next.saturating_add(1)
        } else {
            next
        };
        if cursor == line_start {
            end = Some(after);
        }
        line_start = after;
        cursor = after;
    }
    end
}
fn frames_in(block: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut frame: Vec<u8> = Vec::new();
    for line in lines(block) {
        if line.is_empty() {
            frames.extend((!frame.is_empty()).then(|| mem::take(&mut frame)));
            continue;
        }
        if !frame.is_empty() {
            frame.push(b'\n');
        }
        frame.extend_from_slice(line);
    }
    frames
}
pub(crate) fn classify(source: serde_json::Error) -> ThreadRunsError {
    match source.classify() {
        Category::Eof => ThreadRunsError::Truncated(source),
        Category::Data => ThreadRunsError::Shape(source),
        Category::Syntax | Category::Io => ThreadRunsError::Malformed(source),
    }
}
