use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::response_synthetic_events::model_dump::HasModelDumpJson;

pub enum ProviderChunk {
    Empty,
    Json(Json),
    Object {
        // the event type the response declares, read before the dump so a serializer cannot hide it
        kind: Option<String>,
        dump: Box<dyn HasModelDumpJson + Send>,
    },
}

// The stage a provider failed in; the error frame reports the wrapped cause's own text.
#[derive(Debug, Error)]
pub enum ProviderCallError<Source> {
    #[error("{0}")]
    Call(#[source] Source),
    #[error("{0}")]
    Stream(#[source] Source),
}

#[async_trait]
pub trait ResponsesStream: Send {
    type Error: Error + Send + Sync + 'static;

    async fn next_chunk(&mut self)
    -> Result<Option<ProviderChunk>, ProviderCallError<Self::Error>>;
}

#[async_trait]
pub trait ResponsesStreaming: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    type Stream: ResponsesStream<Error = Self::Error>;

    async fn stream(
        &self,
        model: &str,
        call_kwargs: &IndexMap<String, Json>,
    ) -> Result<Self::Stream, ProviderCallError<Self::Error>>;
}
