use std::mem;

use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;

use super::records::parse_run;

pub(crate) const SSE_POLLER_KEY: &str = "tinyfish_sse_poller_spawned";

// LiteLLM decodes the frames with errors="replace" and splits them with Python's str.splitlines.
#[must_use]
pub fn run_id_from_sse_frames(frames: &[u8]) -> Option<String> {
    let mut text = String::with_capacity(frames.len());
    for chunk in frames.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    run_id_from_sse_lines(&split_lines(&text))
}

// The first frame with a non-empty run_id wins, as LiteLLM returns on the first match.
#[must_use]
pub fn run_id_from_sse_lines(lines: &[String]) -> Option<String> {
    for line in lines {
        let Some(raw) = line.strip_prefix("data:") else {
            continue;
        };
        // Python's strip also removes the information separators that the line split leaves in place.
        let trimmed = raw.trim_matches(|character: char| {
            character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
        });
        let Ok(value) = serde_json::from_str::<Json>(trimmed) else {
            continue;
        };
        let Some(run) = parse_run(&value) else {
            continue;
        };
        let run_id = run
            .run_id
            .as_ref()
            .and_then(Option::as_deref)
            .filter(|found| !found.is_empty());
        if let Some(found) = run_id {
            return Some(found.to_owned());
        }
    }
    None
}

pub fn mark_sse_poller_spawned(logging_obj: &StreamLoggingObject) {
    logging_obj.edit(&mut |state| {
        state
            .model_call_details
            .insert(SSE_POLLER_KEY.to_owned(), Json::Bool(true));
    });
}

// LiteLLM tests with `is True`, so only a JSON true counts as a spawned poller.
#[must_use]
pub fn sse_poller_spawned(logging_obj: &StreamLoggingObject) -> bool {
    logging_obj
        .read(&mut |state| {
            matches!(
                state.model_call_details.get(SSE_POLLER_KEY),
                Some(Json::Bool(true))
            )
        })
        .unwrap_or(false)
}

// Python splits a line at every one of these separators and adds no empty line after a trailing break.
pub(super) fn split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                characters.next_if_eq(&'\n');
                lines.push(mem::take(&mut line));
            }
            '\n' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}'
            | '\u{2029}' => lines.push(mem::take(&mut line)),
            other => line.push(other),
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
