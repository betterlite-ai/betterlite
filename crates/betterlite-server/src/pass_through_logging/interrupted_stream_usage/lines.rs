use std::str::from_utf8;

use betterlite_models::decisions::Json;

use crate::pass_through_logging::stream_fields::{Chunk, StreamFieldError};

// CPython's str.splitlines boundaries, which are more than Rust's lines() breaks on
const LINE_BOUNDARIES: [char; 10] = [
    '\n', '\r', '\u{0b}', '\u{0c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
];

// str.strip whitespace at the pin, which char::is_whitespace does not match for \x1c to \x1f
const fn is_strip_space(character: char) -> bool {
    matches!(
        character,
        '\u{9}'..='\u{d}'
            | '\u{1c}'..='\u{20}'
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

pub(super) fn chunk_text(chunk: &Chunk, index: usize) -> Result<&str, StreamFieldError> {
    match chunk {
        Chunk::Text(text) => Ok(text),
        Chunk::Bytes(bytes) => {
            from_utf8(bytes).map_err(|source| StreamFieldError::Utf8 { index, source })
        }
    }
}

// The first event that decides the tail: message_delta completes it, the three block events were cut.
pub(super) fn tail_decision(line: &str) -> Option<bool> {
    let data = line.strip_prefix("data:")?.trim_matches(is_strip_space);
    let parsed: Json = serde_json::from_str(data).ok()?;
    let Json::Object(fields) = parsed else {
        return None;
    };
    let Some(Json::Text(kind)) = fields.get("type") else {
        return None;
    };
    match kind.as_str() {
        "message_delta" => Some(false),
        "content_block_delta" | "content_block_stop" | "message_start" => Some(true),
        _ => None,
    }
}

pub(super) fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0_usize;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if !LINE_BOUNDARIES.contains(&character) {
            continue;
        }
        let end = if character == '\r'
            && characters.peek().is_some_and(|(_, next)| *next == '\n')
            && let Some((next_index, next_character)) = characters.next()
        {
            next_index.saturating_add(next_character.len_utf8())
        } else {
            index.saturating_add(character.len_utf8())
        };
        if let Some(line) = text.get(start..index) {
            lines.push(line);
        }
        start = end;
    }
    if let Some(tail) = text.get(start..)
        && !tail.is_empty()
    {
        lines.push(tail);
    }
    lines
}
