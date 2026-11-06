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

fn event_and_data(frame: &[u8]) -> (Option<String>, Vec<u8>) {
    let mut event = None;
    let mut data = Vec::new();
    for line in frame.split(|byte| matches!(*byte, b'\n' | b'\r')) {
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

// One frame ends at a blank line; the trailing remainder is still a frame when the body ends without one.
pub(crate) fn run_event_frames(body: &[u8]) -> Vec<&[u8]> {
    let mut frames = Vec::new();
    let (mut start, mut cursor) = (0_usize, 0_usize);
    while let Some(rest) = body.get(cursor..) {
        let boundary = if rest.starts_with(b"\r\n\r\n") {
            4
        } else if rest.starts_with(b"\n\n") || rest.starts_with(b"\r\r") {
            2
        } else {
            cursor = cursor.saturating_add(1);
            continue;
        };
        if let Some(frame) = body.get(start..cursor).filter(|frame| !frame.is_empty()) {
            frames.push(frame);
        }
        cursor = cursor.saturating_add(boundary);
        start = cursor;
    }
    if let Some(tail) = body.get(start..).filter(|tail| !tail.is_empty()) {
        frames.push(tail);
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
