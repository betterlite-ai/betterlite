use std::str::from_utf8;

use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, EventBoundary};

// background_streaming.py's _sse_frame_data over one body-iterator chunk
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollFrame {
    NoData,
    Done,
    Event(Json),
    Malformed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PollFrameError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the provider frame is not utf-8")]
    Utf8,
}

// the source decodes the chunk, takes the first line starting with "data: ", strips it and parses JSON
pub fn decode_poll_stream_frame(
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<PollFrame, PollFrameError> {
    limits.check(ApiBoundariesInput::Event(EventBoundary {
        known_bytes: bytes.len(),
    }))?;
    let text = from_utf8(bytes).map_err(|_error| PollFrameError::Utf8)?;
    Ok(select_frame_data(text))
}

// CPython's _PyUnicode_IsWhitespace table; char::is_whitespace() misses U+001C-U+001F
const PYTHON_WHITESPACE: [char; 29] = [
    '\t', '\n', '\u{b}', '\u{c}', '\r', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', ' ', '\u{85}',
    '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}',
    '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}',
    '\u{205f}', '\u{3000}',
];

const LINE_BREAKS: [char; 10] = [
    '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
];

fn select_frame_data(frame: &str) -> PollFrame {
    let mut remainder = frame;
    // every line takes at least one byte, so this bound only stops a broken splitter
    for _ in 0..=frame.len() {
        let (line, rest) = next_line(remainder);
        if let Some(payload) = line
            .strip_prefix("data: ")
            .map(|payload| payload.trim_matches(|character| PYTHON_WHITESPACE.contains(&character)))
        {
            return classify_payload(payload);
        }
        if rest.is_empty() {
            return PollFrame::NoData;
        }
        remainder = rest;
    }
    PollFrame::NoData
}

// python's str.splitlines reads a break as the end of a line; a "\r\n" pair leaves one empty line
// more, and an empty line never selects a payload either way
fn next_line(text: &str) -> (&str, &str) {
    let Some((offset, character)) = text
        .char_indices()
        .find(|(_, character)| LINE_BREAKS.contains(character))
    else {
        return (text, "");
    };
    let Some((line, rest)) = text.split_at_checked(offset) else {
        return (text, "");
    };
    (line, rest.get(character.len_utf8()..).unwrap_or(rest))
}

fn classify_payload(payload: &str) -> PollFrame {
    if payload == "[DONE]" {
        return PollFrame::Done;
    }
    match serde_json::from_str::<Json>(payload) {
        Ok(event) => PollFrame::Event(event),
        Err(_error) => PollFrame::Malformed,
    }
}
