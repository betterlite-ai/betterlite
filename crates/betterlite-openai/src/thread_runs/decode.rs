use std::collections::BTreeMap;
use std::mem;
use std::str;

use serde_json::error::Category;
use serde_json::{Map, Value};

use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::records::{RunEvent, reply_value};

const RUN_EVENTS: &str = "thread.run.created thread.run.queued thread.run.in_progress \
    thread.run.requires_action thread.run.completed thread.run.incomplete thread.run.failed \
    thread.run.cancelling thread.run.cancelled thread.run.expired";
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameOutcome {
    Event(Box<RunEvent>),
    End,
    Skip,
}
// The SDK wraps thread.* events and yields other payloads bare, raising when one carries an error key.
pub fn decode_run_event(bytes: &[u8]) -> Result<FrameOutcome, ThreadRunsError> {
    let frame = event_and_data(bytes);
    if frame.1.starts_with(b"[DONE]") {
        return Ok(FrameOutcome::End);
    }
    if !frame.2 {
        return Ok(FrameOutcome::Skip);
    }
    let value = serde_json::from_slice::<Value>(&frame.1).map_err(classify)?;
    let named = frame
        .0
        .as_deref()
        .filter(|name| name.starts_with("thread."));
    if let Some(name) = named {
        return Ok(FrameOutcome::Event(Box::new(typed_event(name, value)?)));
    }
    if let Some(error) = provider_error(&value) {
        return Err(error);
    }
    match value.get("event").and_then(Value::as_str) {
        Some(name) => {
            let data = value.get("data").cloned().unwrap_or(Value::Null);
            Ok(FrameOutcome::Event(Box::new(typed_event(name, data)?)))
        }
        None => Ok(FrameOutcome::Event(Box::new(RunEvent::Bare(value)))),
    }
}
const MESSAGE_KEYS: &str = "id assistant_id attachments completed_at content created_at \
    incomplete_at incomplete_details metadata object role run_id status thread_id";
const STEP_KEYS: &str = "id assistant_id cancelled_at completed_at created_at expired_at \
    failed_at last_error metadata object run_id status step_details thread_id type usage";
// The SDK builds event models leniently, so a declared key the provider left out is null in its dump.
fn typed_event(name: &str, mut value: Value) -> Result<RunEvent, ThreadRunsError> {
    if RUN_EVENTS.split(' ').any(|known| known == name) {
        let run = reply_value(value)?;
        let data = serde_json::to_value(run).map_err(classify)?;
        return Ok(RunEvent::Named {
            event: name.to_owned(),
            data,
            extra: BTreeMap::new(),
        });
    }
    if name == "thread.created" {
        add_keys(&mut value, "id created_at metadata object tool_resources");
        let extra = BTreeMap::from([(String::from("enabled"), Value::Null)]);
        return Ok(RunEvent::Named {
            event: name.to_owned(),
            data: value,
            extra,
        });
    }
    fill_event(name, &mut value);
    Ok(RunEvent::Named {
        event: name.to_owned(),
        data: value,
        extra: BTreeMap::new(),
    })
}
fn fill_event(name: &str, value: &mut Value) {
    match name {
        "thread.message.delta" => {
            if let Some(delta) = value.get_mut("delta") {
                add_keys(delta, "content role");
                if let Some(content) = delta.get_mut("content") {
                    fill_parts(content);
                }
            }
        }
        "thread.run.step.delta" => {
            if let Some(delta) = value.get_mut("delta") {
                add_keys(delta, "step_details");
                if let Some(details) = delta.get_mut("step_details") {
                    fill_calls(details, true);
                }
            }
        }
        _ if name.starts_with("thread.message.") => {
            add_keys(value, MESSAGE_KEYS);
            if let Some(Value::Array(items)) = value.get_mut("attachments") {
                for item in items {
                    add_keys(item, "file_id tools");
                }
            }
        }
        _ if name.starts_with("thread.run.step.") => {
            add_keys(value, STEP_KEYS);
            if let Some(details) = value.get_mut("step_details") {
                fill_calls(details, false);
            }
        }
        _ => {}
    }
}
fn fill_parts(content: &mut Value) {
    let Value::Array(parts) = content else {
        return;
    };
    for part in parts {
        if let Some(Value::Object(text)) = part.get_mut("text") {
            insert_keys(text, "annotations");
        }
    }
}
fn fill_calls(details: &mut Value, delta: bool) {
    let Value::Object(object) = details else {
        return;
    };
    let Some(Value::Array(calls)) = object.get_mut("tool_calls") else {
        return;
    };
    for call in calls {
        if delta {
            add_keys(call, "id");
        }
        match call.get("type").and_then(Value::as_str) {
            Some("code_interpreter") => {
                if let Some(Value::Object(inner)) = call.get_mut("code_interpreter") {
                    insert_keys(inner, "input outputs");
                }
            }
            Some("file_search") if !delta => {
                if let Some(Value::Object(inner)) = call.get_mut("file_search") {
                    insert_keys(inner, "ranking_options results");
                }
            }
            _ => {}
        }
    }
}
fn add_keys(target: &mut Value, keys: &str) {
    let Value::Object(object) = target else {
        return;
    };
    insert_keys(object, keys);
}
pub(crate) fn insert_keys(object: &mut Map<String, Value>, keys: &str) {
    for key in keys.split(' ') {
        object.entry(key).or_insert(Value::Null);
    }
}
fn provider_error(value: &Value) -> Option<ThreadRunsError> {
    let Value::Object(fields) = value else {
        return None;
    };
    match fields.get("error") {
        None | Some(Value::Null | Value::Bool(false)) => None,
        Some(error) => Some(ThreadRunsError::ProviderError {
            body: serde_json::to_vec(error).unwrap_or_default(),
        }),
    }
}
fn lines(body: &[u8]) -> Vec<&[u8]> {
    body.split(|byte| matches!(*byte, b'\r' | b'\n'))
        .filter(|line| !line.is_empty())
        .collect()
}
type Frame = (Option<String>, Vec<u8>, bool);
fn event_and_data(frame: &[u8]) -> Frame {
    let mut parsed: Frame = (None, Vec::new(), false);
    for line in lines(frame) {
        let at = line.iter().position(|byte| *byte == b':');
        let (name, rest) = at.map_or((line, &[][..]), |position| {
            let (name, rest) = line.split_at(position);
            (name, rest.get(1..).unwrap_or_default())
        });
        match name {
            b"event" => {
                let field = rest.trim_ascii();
                parsed.0 = str::from_utf8(field).ok().map(str::to_owned);
                parsed.2 = !field.is_empty();
            }
            b"data" => {
                if !parsed.1.is_empty() {
                    parsed.1.push(b'\n');
                }
                parsed
                    .1
                    .extend_from_slice(rest.strip_prefix(b" ").unwrap_or(rest));
                parsed.2 = true;
            }
            b"id" => parsed.2 = !rest.trim_ascii().is_empty() && !rest.contains(&0),
            b"retry" => {
                parsed.2 =
                    str::from_utf8(rest.trim_ascii()).is_ok_and(|text| text.parse::<u64>().is_ok());
            }
            _ => {}
        }
    }
    parsed
}
// Frames end at a blank line, so a tail the provider cut off before one is never dispatched.
#[derive(Debug, Default)]
pub(crate) struct RunFrames {
    buffer: Vec<u8>,
}
impl RunFrames {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, ThreadRunsError> {
        if self.buffer.len().saturating_add(bytes.len()) > 0x0010_0000 {
            return Err(ThreadRunsError::StreamTooLarge);
        }
        self.buffer.extend_from_slice(bytes);
        let (frames, end) = frames_in(&self.buffer);
        self.buffer.drain(..end);
        Ok(frames)
    }
}
fn frames_in(buffer: &[u8]) -> (Vec<Vec<u8>>, usize) {
    let mut frames = Vec::new();
    let mut frame: Vec<u8> = Vec::new();
    let (mut start, mut cursor, mut end) = (0_usize, 0_usize, 0_usize);
    while let Some(byte) = buffer.get(cursor) {
        if !matches!(*byte, b'\r' | b'\n') {
            cursor = cursor.saturating_add(1);
            continue;
        }
        let line = buffer.get(start..cursor).unwrap_or_default();
        let next = cursor.saturating_add(1);
        let after = if *byte == b'\r' && buffer.get(next) == Some(&b'\n') {
            next.saturating_add(1)
        } else {
            next
        };
        if line.is_empty() {
            frames.extend((!frame.is_empty()).then(|| mem::take(&mut frame)));
            end = after;
        } else {
            if !frame.is_empty() {
                frame.push(b'\n');
            }
            frame.extend_from_slice(line);
        }
        start = after;
        cursor = after;
    }
    (frames, end)
}
pub(crate) fn classify(source: serde_json::Error) -> ThreadRunsError {
    match source.classify() {
        Category::Eof => ThreadRunsError::Truncated(source),
        Category::Data => ThreadRunsError::Shape(source),
        Category::Syntax | Category::Io => ThreadRunsError::Malformed(source),
    }
}
