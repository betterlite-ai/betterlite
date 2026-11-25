use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::provider::{ProviderCallError, ProviderChunk, ResponsesStream as _, ResponsesStreaming};
use crate::api::responses_ws_history::serialization::{Chunk, ChunkWriter};
use crate::api::responses_ws_history::{FrameSink, ManagedResponsesWebSocket};

const COMPLETED_EVENT: &str = "response.completed";

pub async fn stream_and_forward<Sink, Provider>(
    handler: &ManagedResponsesWebSocket<Sink>,
    provider: &Provider,
    model: &str,
    call_kwargs: &IndexMap<String, Json>,
) -> Result<Option<IndexMap<String, Json>>, ProviderCallError<Provider::Error>>
where
    Sink: FrameSink,
    Provider: ResponsesStreaming,
{
    let mut completed: Option<IndexMap<String, Json>> = None;
    let mut stream = provider.stream(model, call_kwargs).await?;
    while let Some(chunk) = stream.next_chunk().await? {
        let Some((kind, serialized)) = serialized_chunk(handler.serializer.as_ref(), chunk) else {
            continue;
        };
        // the discriminator is read before the send, so the first completion is the one history keeps
        if kind.as_deref() == Some(COMPLETED_EVENT) && completed.is_none() {
            completed = completed_object(&serialized);
        }
        if let Err(error) = handler.websocket.send_text(&serialized).await {
            tracing::debug!("ManagedResponsesWS: error sending chunk to client: {error}");
            return Ok(completed);
        }
    }
    Ok(completed)
}

fn serialized_chunk(
    writer: &ChunkWriter,
    chunk: ProviderChunk,
) -> Option<(Option<String>, String)> {
    match chunk {
        ProviderChunk::Empty => None,
        ProviderChunk::Json(value) => {
            let kind = json_event_type(&value);
            let serialized = writer(Chunk::Json(&value))?;
            Some((kind, serialized))
        }
        ProviderChunk::Object { kind, dump } => {
            let serialized = writer(Chunk::ModelDumpJson(dump.as_ref()))?;
            Some((kind, serialized))
        }
    }
}

// a dict chunk names its event in "type"; LiteLLM reads nothing else off a non-object chunk
fn json_event_type(value: &Json) -> Option<String> {
    match value {
        Json::Object(fields) => match fields.get("type") {
            Some(Json::Text(event_type)) => Some(event_type.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn completed_object(serialized: &str) -> Option<IndexMap<String, Json>> {
    let Ok(parsed) = serde_json::from_str::<Json>(serialized) else {
        return None;
    };
    match parsed {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
