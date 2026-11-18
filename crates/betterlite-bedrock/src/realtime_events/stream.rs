use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct StreamError<E> {
    pub message: String,
    #[source]
    pub source: Option<E>,
}

// The SDK protocols in handler.py; each native transport names the error type it reports
#[async_trait]
pub trait BedrockInputStream: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn send(&self, event: Json) -> Result<(), StreamError<Self::Error>>;

    async fn close(&self) -> Result<(), StreamError<Self::Error>>;
}

pub trait BedrockPayloadPart: Send + Sync {
    fn bytes(&self) -> Option<&[u8]>;
}

pub trait BedrockOutputChunk: Send + Sync {
    fn value(&self) -> Option<&dyn BedrockPayloadPart>;
}

#[async_trait]
pub trait BedrockOutputStream: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn receive(
        &self,
    ) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError<Self::Error>>;
}

#[async_trait]
pub trait BedrockBidirectionalStream: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn input_stream(&self) -> &dyn BedrockInputStream<Error = Self::Error>;

    async fn await_output(
        &self,
    ) -> Result<(Json, Box<dyn BedrockOutputStream<Error = Self::Error>>), StreamError<Self::Error>>;
}

#[async_trait]
pub trait ClosableBedrockRuntimeClient: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn close(&self) -> Result<(), StreamError<Self::Error>>;
}
