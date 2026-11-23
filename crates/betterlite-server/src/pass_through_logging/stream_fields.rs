use std::str::Utf8Error;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

use super::batch_attribution::optional_str;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chunk {
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Error)]
pub enum StreamFieldError {
    #[error("the chunk at index {index} is not valid UTF-8: {source}")]
    Utf8 {
        index: usize,
        #[source]
        source: Utf8Error,
    },
}

// The field readers of the Anthropic message_start event: the first non-empty string wins
#[must_use]
pub fn extract_message_start_field(
    chunks: &[Chunk],
    field: &str,
) -> Result<Option<String>, StreamFieldError> {
    for (index, chunk) in chunks.iter().enumerate() {
        let text = chunk_text(chunk, index)?;
        if let Some(found) = field_in_text(text, field) {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

#[must_use]
pub fn resolve_logged_model(
    call_log: &LoggingState,
    request_body: &JsonObject,
    all_chunks: &[Chunk],
) -> Result<String, StreamFieldError> {
    let request_model = request_body
        .get("model")
        .and_then(optional_str)
        .filter(|model| !model.is_empty());
    let logged_model =
        request_model.map_or_else(|| logged_model_from_details(call_log), str::to_owned);
    if !logged_model.is_empty() && logged_model != "unknown" {
        return Ok(logged_model);
    }
    Ok(extract_message_start_field(all_chunks, "model")?.unwrap_or(logged_model))
}

fn chunk_text(chunk: &Chunk, index: usize) -> Result<&str, StreamFieldError> {
    match chunk {
        Chunk::Text(text) => Ok(text),
        Chunk::Bytes(bytes) => {
            str::from_utf8(bytes).map_err(|source| StreamFieldError::Utf8 { index, source })
        }
    }
}

fn field_in_text(text: &str, field: &str) -> Option<String> {
    for line in split_lines(text) {
        if let Some(found) = field_in_line(line, field) {
            return Some(found);
        }
    }
    None
}

fn field_in_line(line: &str, field: &str) -> Option<String> {
    let data = line.strip_prefix("data:")?.trim_matches(is_strip_space);
    let parsed: Json = serde_json::from_str(data).ok()?;
    let Json::Object(fields) = parsed else {
        return None;
    };
    let is_message_start =
        matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "message_start");
    if !is_message_start {
        return None;
    }
    let Some(Json::Object(message)) = fields.get("message") else {
        return None;
    };
    match message.get(field) {
        Some(Json::Text(value)) if !value.is_empty() => Some(value.clone()),
        None | Some(_) => None,
    }
}

fn logged_model_from_details(call_log: &LoggingState) -> String {
    call_log
        .model_call_details
        .get("model")
        .and_then(optional_str)
        .unwrap_or_default()
        .to_owned()
}

// CPython's str.splitlines boundaries, which are more than Rust's lines() breaks on
const fn is_line_boundary(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r'
            | '\u{0b}'
            | '\u{0c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

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

fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0_usize;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if !is_line_boundary(character) {
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

mod tests;
