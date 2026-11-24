use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::response_synthetic_events::model_dump::HasModelDumpJson;

// One item of the async stream litellm.aresponses(stream=True) returns.
pub enum ProviderChunk {
    // the stream yielded None
    Empty,
    // a JSON chunk: an object serializes as JSON, any other value as its Python str()
    Json(Json),
    // a response object that dumps itself, with the event type it declares before serialization
    Object {
        kind: Option<String>,
        dump: Box<dyn HasModelDumpJson + Send>,
    },
}

// litellm.aresponses' exception string, which the error frame reports as its message
#[derive(Debug, Error)]
#[error("{message}")]
pub struct ProviderCallError {
    pub message: String,
}

// the chunk sequence of one provider call: a chunk or a stream failure
#[async_trait]
pub trait ResponsesStream: Send {
    async fn next_chunk(&mut self) -> Result<Option<ProviderChunk>, ProviderCallError>;
}

// B932's asynchronous public aresponses call, reduced to the stream this lifecycle consumes
#[async_trait]
pub trait ResponsesStreaming: Send + Sync {
    type Stream: ResponsesStream;

    async fn stream(
        &self,
        model: &str,
        call_kwargs: &IndexMap<String, Json>,
    ) -> Result<Self::Stream, ProviderCallError>;
}
