use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::chat_response_stream_tools::fields::string_value;
use crate::api::response_synthetic_events::model_dump::{HasModelDump, HasModelDumpJson};

use super::{FrameSink, ManagedResponsesWebSocket};

pub const ERROR_EVENT: &str = "error";
pub const INVALID_JSON_MESSAGE: &str = "Invalid JSON in response.create event";
pub const INVALID_REQUEST_ERROR: &str = "invalid_request_error";
pub const SERVER_ERROR: &str = "server_error";

#[derive(Clone, Copy)]
pub enum Chunk<'chunk> {
    ModelDumpJson(&'chunk dyn HasModelDumpJson),
    ModelDump(&'chunk dyn HasModelDump),
    Json(&'chunk Json),
}

pub type ChunkWriter = dyn for<'chunk> Fn(Chunk<'chunk>) -> Option<String> + Send + Sync;

#[must_use]
pub fn serialize_chunk(chunk: Chunk<'_>) -> Option<String> {
    match chunk {
        Chunk::ModelDumpJson(value) => match value.model_dump_json() {
            Ok(text) => Some(text),
            Err(error) => {
                tracing::debug!("ManagedResponsesWS: failed to serialize chunk: {error}");
                None
            }
        },
        Chunk::ModelDump(value) => match value.model_dump() {
            Ok(fields) => Some(dumps(&Json::Object(fields.into_iter().collect()))),
            Err(error) => {
                tracing::debug!("ManagedResponsesWS: failed to serialize chunk: {error}");
                None
            }
        },
        Chunk::Json(value) => match value {
            Json::Object(_) => Some(dumps(value)),
            other @ (Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)) => Some(dumps(&Json::Text(string_value(other)))),
        },
    }
}

pub async fn send_error<Sink>(
    handler: &ManagedResponsesWebSocket<Sink>,
    message: &str,
    error_type: Option<&str>,
) where
    Sink: FrameSink,
{
    let mut error = IndexMap::new();
    let class = Json::Text(error_type.unwrap_or(SERVER_ERROR).to_owned());
    error.insert("type".to_owned(), class);
    error.insert("message".to_owned(), Json::Text(message.to_owned()));
    let mut frame = IndexMap::new();
    frame.insert("type".to_owned(), Json::Text(ERROR_EVENT.to_owned()));
    frame.insert("error".to_owned(), Json::Object(error));
    let text = dumps(&Json::Object(frame));
    // LiteLLM drops a failed error frame; recovery for real sends belongs to B945
    let _failure = handler.websocket.send_text(&text).await;
}
