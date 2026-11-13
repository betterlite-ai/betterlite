use serde::Serialize;
use thiserror::Error;

use crate::types::JsonObject;

use super::route::{ROUTE_CHAT, ROUTE_MESSAGES, ROUTE_RESPONSES};

pub enum Chunk<'chunk> {
    Bytes(&'chunk [u8]),
    Text(&'chunk str),
    Json(&'chunk JsonObject),
}

#[derive(Debug, Error)]
pub enum EncodeError {
    #[error("the {route} stream has no chunk encoder")]
    UnknownRoute { route: String },
    #[error("a bytes chunk cannot be framed as a JSON event on the {route} route")]
    BytesNotJson { route: &'static str },
    #[error("the chunk could not be encoded")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

#[must_use]
pub fn stream_trailer(route: &str) -> Option<&'static [u8]> {
    if route == ROUTE_CHAT {
        return Some(b"data: [DONE]\n\n");
    }
    None
}

pub fn encode_chunk(route: &str, chunk: Chunk<'_>) -> Result<Vec<u8>, EncodeError> {
    if route == ROUTE_MESSAGES {
        return encode_anthropic(chunk);
    }
    if route == ROUTE_CHAT {
        return encode_chat(chunk);
    }
    if route == ROUTE_RESPONSES {
        return encode_responses(chunk);
    }
    Err(EncodeError::UnknownRoute {
        route: route.to_owned(),
    })
}

fn encode_anthropic(chunk: Chunk<'_>) -> Result<Vec<u8>, EncodeError> {
    match chunk {
        Chunk::Bytes(bytes) => Ok(bytes.to_vec()),
        Chunk::Text(text) => Ok(text.as_bytes().to_vec()),
        Chunk::Json(event) => event_frame(event),
    }
}

fn encode_chat(chunk: Chunk<'_>) -> Result<Vec<u8>, EncodeError> {
    match chunk {
        Chunk::Bytes(_) => Err(EncodeError::BytesNotJson { route: ROUTE_CHAT }),
        Chunk::Text(text) => Ok(frame_event(None, &json(text)?)),
        Chunk::Json(object) => Ok(frame_event(None, &json(object)?)),
    }
}

fn encode_responses(chunk: Chunk<'_>) -> Result<Vec<u8>, EncodeError> {
    match chunk {
        Chunk::Bytes(_) => Err(EncodeError::BytesNotJson {
            route: ROUTE_RESPONSES,
        }),
        Chunk::Text(text) => Ok(frame_event(Some("message"), &json(text)?)),
        Chunk::Json(event) => event_frame(event),
    }
}

fn event_frame(event: &JsonObject) -> Result<Vec<u8>, EncodeError> {
    let event_type = event
        .get("type")
        .and_then(|value| value.as_str())
        .filter(|name| !name.is_empty() && !name.contains('\n') && !name.contains('\r'))
        .unwrap_or("message");
    let data = json(event)?;
    Ok(frame_event(Some(event_type), &data))
}

fn frame_event(event_type: Option<&str>, data: &str) -> Vec<u8> {
    match event_type {
        Some(name) => format!("event: {name}\ndata: {data}\n\n").into_bytes(),
        None => format!("data: {data}\n\n").into_bytes(),
    }
}

fn json<T: Serialize>(value: &T) -> Result<String, EncodeError> {
    match serde_json::to_string(value) {
        Ok(data) => Ok(data),
        Err(source) => Err(EncodeError::Json { source }),
    }
}
