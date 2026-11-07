use std::collections::BTreeMap;
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
    let (event, data, present) = event_and_data(bytes);
    if data.starts_with(b"[DONE]") {
        return Ok(FrameOutcome::End);
    }
    if !present {
        return Ok(FrameOutcome::Skip);
    }
    let value = serde_json::from_slice::<Value>(&data).map_err(classify)?;
    if let Some(name) = event.as_deref().filter(|name| name.starts_with("thread.")) {
        return Ok(FrameOutcome::Event(Box::new(typed_event(name, value)?)));
    }
    if let Some(error) = provider_error(&value) {
        return Err(error);
    }
    if let Some(name) = value.get("event").and_then(Value::as_str) {
        let payload = value.get("data").cloned().unwrap_or(Value::Null);
        return Ok(FrameOutcome::Event(Box::new(typed_event(name, payload)?)));
    }
    Ok(FrameOutcome::Event(Box::new(RunEvent::Bare(value))))
}
const MESSAGE_KEYS: &str = "id assistant_id attachments completed_at content created_at \
    incomplete_at incomplete_details metadata object role run_id status thread_id";
const STEP_KEYS: &str = "id assistant_id cancelled_at completed_at created_at expired_at \
    failed_at last_error metadata object run_id status step_details thread_id type usage";
// The SDK builds event models leniently, so a declared key the provider left out is null in its dump.
fn typed_event(name: &str, mut value: Value) -> Result<RunEvent, ThreadRunsError> {
    if RUN_EVENTS.split(' ').any(|known| known == name) {
        return Ok(RunEvent::Named {
            event: name.to_owned(),
            data: serde_json::to_value(reply_value(value)?).map_err(classify)?,
            extra: BTreeMap::new(),
        });
    }
    if name == "thread.created" {
        add_keys(&mut value, "id created_at metadata object tool_resources");
        if let Some(resources) = value.get_mut("tool_resources") {
            add_keys(resources, "code_interpreter file_search");
            fill_object(resources, "code_interpreter", "file_ids");
            fill_object(resources, "file_search", "vector_store_ids");
        }
        return Ok(RunEvent::Named {
            event: name.to_owned(),
            data: value,
            extra: BTreeMap::from([(String::from("enabled"), Value::Null)]),
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
                    fill_parts(content, true);
                }
            }
        }
        "thread.run.step.delta" => {
            if let Some(delta) = value.get_mut("delta") {
                add_keys(delta, "step_details");
                if let Some(details) = delta.get_mut("step_details") {
                    fill_step(details, true);
                }
            }
        }
        _ if name.starts_with("thread.message.") => {
            add_keys(value, MESSAGE_KEYS);
            if let Some(content) = value.get_mut("content") {
                fill_parts(content, false);
            }
            if let Some(Value::Array(items)) = value.get_mut("attachments") {
                for item in items {
                    add_keys(item, "file_id tools");
                }
            }
        }
        _ if name.starts_with("thread.run.step.") => {
            add_keys(value, STEP_KEYS);
            if let Some(details) = value.get_mut("step_details") {
                fill_step(details, false);
            }
        }
        _ => {}
    }
}
fn fill_parts(content: &mut Value, delta: bool) {
    let Value::Array(parts) = content else {
        return;
    };
    for part in parts {
        let kind = part.get("type").and_then(Value::as_str).unwrap_or_default();
        if matches!(kind, "text" | "image_file" | "image_url" | "refusal") {
            let key = kind.to_owned();
            add_keys(part, &key);
        }
        fill_object(part, "text", "annotations value");
        fill_object(part, "image_file", "file_id detail");
        fill_object(part, "image_url", "url detail");
        if delta {
            fill_annotations(part);
        }
    }
}
fn fill_annotations(part: &mut Value) {
    let Some(Value::Array(annotations)) = part.pointer_mut("/text/annotations") else {
        return;
    };
    for annotation in annotations {
        if annotation.get("type").and_then(Value::as_str) == Some("file_citation") {
            add_keys(annotation, "end_index file_citation start_index text");
            fill_object(annotation, "file_citation", "file_id quote");
        }
    }
}
fn fill_step(details: &mut Value, delta: bool) {
    fill_object(details, "message_creation", "message_id");
    let Value::Object(object) = details else {
        return;
    };
    let Some(Value::Array(calls)) = object.get_mut("tool_calls") else {
        return;
    };
    for call in calls {
        add_keys(call, "id");
        match call.get("type").and_then(Value::as_str) {
            Some("function") => fill_call(call, "function", "name arguments output"),
            Some("code_interpreter") => fill_call(call, "code_interpreter", "input outputs"),
            Some("file_search") if delta => add_keys(call, "file_search"),
            Some("file_search") => fill_call(call, "file_search", "ranking_options results"),
            _ => {}
        }
    }
}
fn fill_call(call: &mut Value, kind: &str, keys: &str) {
    add_keys(call, kind);
    fill_object(call, kind, keys);
}
pub(crate) fn fill_object(value: &mut Value, key: &str, keys: &str) {
    if let Some(object) = value.as_object_mut() {
        fill_map(object, key, keys);
    }
}
pub(crate) fn add_keys(target: &mut Value, keys: &str) {
    if let Some(object) = target.as_object_mut() {
        insert_keys(object, keys);
    }
}
pub(crate) fn fill_map(object: &mut Map<String, Value>, key: &str, keys: &str) {
    if let Some(Value::Object(inner)) = object.get_mut(key) {
        insert_keys(inner, keys);
    }
}
pub(crate) fn insert_keys(object: &mut Map<String, Value>, keys: &str) {
    for key in keys.split(' ') {
        object.entry(key).or_insert(Value::Null);
    }
}
fn provider_error(value: &Value) -> Option<ThreadRunsError> {
    let error = value.get("error")?;
    if matches!(error, Value::Null | Value::Bool(false)) {
        return None;
    }
    Some(ThreadRunsError::ProviderError {
        body: serde_json::to_vec(error).unwrap_or_default(),
    })
}
fn lines(body: &[u8]) -> impl Iterator<Item = &[u8]> {
    body.split(|byte| matches!(*byte, b'\r' | b'\n'))
        .filter(|line| !line.is_empty())
}
type Frame = (Option<String>, Vec<u8>, bool);
// Each event and id line replaces the last one, as the SDK does, while retry stays set once a line parses.
fn event_and_data(frame: &[u8]) -> Frame {
    let mut event = None;
    let mut data: Vec<u8> = Vec::new();
    let mut seen = false;
    let mut event_ok = false;
    let mut id_ok = false;
    let mut retry_ok = false;
    for line in lines(frame) {
        let at = line.iter().position(|byte| *byte == b':');
        let (name, rest) = at.map_or((line, &[][..]), |position| {
            let (name, rest) = line.split_at(position);
            (name, rest.get(1..).unwrap_or_default())
        });
        match name {
            b"event" => {
                let field = rest.trim_ascii();
                event = str::from_utf8(field).ok().map(str::to_owned);
                event_ok = !field.is_empty();
            }
            b"data" => {
                if seen {
                    data.push(b'\n');
                }
                data.extend_from_slice(rest.strip_prefix(b" ").unwrap_or(rest));
                seen = true;
            }
            b"id" if !rest.contains(&0) => id_ok = !rest.trim_ascii().is_empty(),
            b"retry" => {
                retry_ok |=
                    str::from_utf8(rest.trim_ascii()).is_ok_and(|text| text.parse::<i64>().is_ok());
            }
            _ => {}
        }
    }
    (event, data, seen || event_ok || id_ok || retry_ok)
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
    let (mut frame_start, mut line_start, mut cursor, mut end) =
        (0_usize, 0_usize, 0_usize, 0_usize);
    while let Some(byte) = buffer.get(cursor) {
        if !matches!(*byte, b'\r' | b'\n') {
            cursor = cursor.saturating_add(1);
            continue;
        }
        let next = cursor.saturating_add(1);
        let after = next.saturating_add(usize::from(
            *byte == b'\r' && buffer.get(next) == Some(&b'\n'),
        ));
        if line_start == cursor {
            if let Some(frame) = buffer.get(frame_start..line_start) {
                frames.extend((!frame.is_empty()).then(|| frame.to_vec()));
            }
            frame_start = after;
            end = after;
        }
        line_start = after;
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
