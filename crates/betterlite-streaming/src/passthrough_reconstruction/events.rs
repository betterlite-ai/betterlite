use std::str::from_utf8;

use betterlite_models::decisions::Json;

use crate::passthrough_reconstruction::error::EventError;

// A relayed chunk is the text the provider sent or the bytes it arrived as; both decode the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawSseChunk {
    Text(String),
    Bytes(Vec<u8>),
}

impl RawSseChunk {
    pub fn text(&self) -> Result<&str, EventError> {
        match self {
            Self::Text(text) => Ok(text),
            Self::Bytes(bytes) => from_utf8(bytes).map_err(EventError::from),
        }
    }
}

pub fn split_sse_chunk_into_events(chunk: &RawSseChunk) -> Result<Vec<String>, EventError> {
    Ok(chunk
        .text()?
        .split("\n\n")
        .map(str::trim)
        .filter(|event| !event.is_empty())
        .map(str::to_owned)
        .collect())
}

// Only the first data: line is read, and an empty, [DONE] or non-JSON payload answers None.
#[must_use]
pub fn extract_sse_data(event: &str) -> Option<Json> {
    let line = event
        .lines()
        .find(|line| line.trim().starts_with("data:"))?;
    let payload = line.trim().strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    serde_json::from_str(payload).ok()
}

pub(crate) fn object_field<'value>(value: &'value Json, key: &str) -> Option<&'value Json> {
    let Json::Object(fields) = value else {
        return None;
    };
    match fields.get(key) {
        Some(field @ Json::Object(_)) => Some(field),
        _ => None,
    }
}

pub(crate) fn text_field<'value>(value: &'value Json, key: &str) -> Option<&'value str> {
    let Json::Object(fields) = value else {
        return None;
    };
    match fields.get(key)? {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(crate) fn integer_field(value: &Json, key: &str) -> Option<i64> {
    let Json::Object(fields) = value else {
        return None;
    };
    match fields.get(key)? {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}
