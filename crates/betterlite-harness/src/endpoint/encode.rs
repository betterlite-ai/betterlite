use serde::Serialize;
use thiserror::Error;

use crate::types::JsonObject;

use super::route::Route;

pub enum Chunk<'chunk> {
    Bytes(&'chunk [u8]),
    Text(&'chunk str),
    Json(&'chunk JsonObject),
}

#[derive(Debug, Error)]
pub enum EncodeError {
    #[error("a bytes chunk cannot be framed as a JSON event on the {route} route")]
    BytesNotJson { route: &'static str },
    #[error("the chunk could not be encoded")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

#[must_use]
pub const fn stream_trailer(route: Route) -> Option<&'static [u8]> {
    match route {
        Route::Chat => Some(b"data: [DONE]\n\n"),
        Route::Messages | Route::Responses => None,
    }
}

pub fn encode_chunk(route: Route, chunk: Chunk<'_>) -> Result<Vec<u8>, EncodeError> {
    match (route, chunk) {
        (Route::Messages, Chunk::Bytes(bytes)) => Ok(bytes.to_vec()),
        (Route::Messages, Chunk::Text(text)) => Ok(text.as_bytes().to_vec()),
        (Route::Chat | Route::Responses, Chunk::Bytes(_)) => Err(EncodeError::BytesNotJson {
            route: route.path(),
        }),
        (Route::Chat, Chunk::Text(text)) => Ok(frame_event(None, &json(text)?)),
        (Route::Chat, Chunk::Json(object)) => Ok(frame_event(None, &json(object)?)),
        (Route::Responses, Chunk::Text(text)) => Ok(frame_event(Some("message"), &json(text)?)),
        (Route::Messages | Route::Responses, Chunk::Json(event)) => event_frame(event),
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
    let head = event_type.map_or_else(String::new, |name| format!("event: {name}\n"));
    format!("{head}data: {data}\n\n").into_bytes()
}

fn json<T>(value: &T) -> Result<String, EncodeError>
where
    T: ?Sized + Serialize,
{
    match serde_json::to_string(value) {
        Ok(data) => Ok(data),
        Err(source) => Err(EncodeError::Json { source }),
    }
}
