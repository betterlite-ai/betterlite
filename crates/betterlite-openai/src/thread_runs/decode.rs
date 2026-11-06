use std::mem;
use std::str;

use serde::de::DeserializeOwned;
use serde_json::error::Category;

use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::records::{RunErrorObject, RunEvent};

// The provider names the event on the frame's `event:` line; the proxy's own frames carry it inside the data.
pub fn decode_run_event(bytes: &[u8]) -> Result<Option<RunEvent>, ThreadRunsError> {
    let (event, data) = event_and_data(bytes);
    if data.starts_with(b"[DONE]") {
        return Ok(None);
    }
    event.map_or_else(
        || decode_json(&data).map(Some),
        |name| decode_named(&name, &data).map(Some),
    )
}

fn decode_named(name: &str, data: &[u8]) -> Result<RunEvent, ThreadRunsError> {
    match name {
        "thread.run.created" => decode_json(data).map(RunEvent::Created),
        "thread.run.queued" => decode_json(data).map(RunEvent::Queued),
        "thread.run.in_progress" => decode_json(data).map(RunEvent::InProgress),
        "thread.run.requires_action" => decode_json(data).map(RunEvent::RequiresAction),
        "thread.run.completed" => decode_json(data).map(RunEvent::Completed),
        "thread.run.incomplete" => decode_json(data).map(RunEvent::Incomplete),
        "thread.run.failed" => decode_json(data).map(RunEvent::Failed),
        "thread.run.cancelling" => decode_json(data).map(RunEvent::Cancelling),
        "thread.run.cancelled" => decode_json(data).map(RunEvent::Cancelled),
        "thread.run.expired" => decode_json(data).map(RunEvent::Expired),
        "error" => decode_json::<RunErrorObject>(data).map(RunEvent::Error),
        other => Err(ThreadRunsError::UnsupportedEvent {
            event: other.to_owned(),
        }),
    }
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

fn event_and_data(frame: &[u8]) -> (Option<String>, Vec<u8>) {
    let mut event = None;
    let mut data = Vec::new();
    for line in lines(frame) {
        if let Some(rest) = line.strip_prefix(b"event:") {
            event = str::from_utf8(rest.trim_ascii()).ok().map(str::to_owned);
        }
        if let Some(rest) = line.strip_prefix(b"data:") {
            if !data.is_empty() {
                data.push(b'\n');
            }
            data.extend_from_slice(rest.strip_prefix(b" ").unwrap_or(rest));
        }
    }
    (event, data)
}

// One frame ends at a blank line whatever line terminator wrote it; a body without a final blank line keeps its tail.
pub(crate) fn run_event_frames(body: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut frame: Vec<u8> = Vec::new();
    for line in lines(body) {
        if line.is_empty() {
            if !frame.is_empty() {
                frames.push(mem::take(&mut frame));
            }
            continue;
        }
        if !frame.is_empty() {
            frame.push(b'\n');
        }
        frame.extend_from_slice(line);
    }
    if !frame.is_empty() {
        frames.push(frame);
    }
    frames
}

pub fn decode_json<T>(bytes: &[u8]) -> Result<T, ThreadRunsError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|source| match source.classify() {
        Category::Eof => ThreadRunsError::Truncated { source },
        Category::Data => ThreadRunsError::Shape { source },
        Category::Syntax | Category::Io => ThreadRunsError::Malformed { source },
    })
}
