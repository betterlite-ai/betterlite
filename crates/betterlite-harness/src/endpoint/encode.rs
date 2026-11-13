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
    match route {
        ROUTE_MESSAGES => encode_anthropic(chunk),
        ROUTE_CHAT => encode_chat(chunk),
        ROUTE_RESPONSES => encode_responses(chunk),
        _ => Err(EncodeError::UnknownRoute {
            route: route.to_owned(),
        }),
    }
}

fn encode_anthropic(chunk: Chunk<'_>) -> Result<Vec<u8>, EncodeError> {
    match chunk {
        Chunk::Bytes(bytes) => Ok(bytes.to_vec()),
        Chunk::Text(text) => Ok(text.as_bytes().to_vec()),
        Chunk::Json(event) => framed(event),
    }
}

fn encode_chat(chunk: Chunk<'_>) -> Result<Vec<u8>, EncodeError> {
    match chunk {
        Chunk::Json(object) => {
            let data =
                serde_json::to_string(object).map_err(|source| EncodeError::Json { source })?;
            Ok(format!("data: {data}\n\n").into_bytes())
        }
        Chunk::Text(text) => {
            let data =
                serde_json::to_string(text).map_err(|source| EncodeError::Json { source })?;
            Ok(format!("data: {data}\n\n").into_bytes())
        }
        Chunk::Bytes(_) => Err(EncodeError::BytesNotJson { route: ROUTE_CHAT }),
    }
}

fn encode_responses(chunk: Chunk<'_>) -> Result<Vec<u8>, EncodeError> {
    match chunk {
        Chunk::Json(event) => framed(event),
        Chunk::Text(text) => {
            let data =
                serde_json::to_string(text).map_err(|source| EncodeError::Json { source })?;
            Ok(format!("event: message\ndata: {data}\n\n").into_bytes())
        }
        Chunk::Bytes(_) => Err(EncodeError::BytesNotJson {
            route: ROUTE_RESPONSES,
        }),
    }
}

fn framed(event: &JsonObject) -> Result<Vec<u8>, EncodeError> {
    let event_type = event
        .get("type")
        .and_then(|value| value.as_str())
        .filter(|name| !name.is_empty() && !name.contains('\n') && !name.contains('\r'))
        .unwrap_or("message");
    let data = serde_json::to_string(event).map_err(|source| EncodeError::Json { source })?;
    Ok(format!("event: {event_type}\ndata: {data}\n\n").into_bytes())
}
