use async_trait::async_trait;
use betterlite_models::decisions::Json;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct StreamError {
    pub message: String,
}

// The SDK protocols in handler.py; the native transport implements these and forwards the raw event JSON
#[async_trait]
pub trait BedrockInputStream: Send + Sync {
    async fn send(&self, event: Json) -> Result<(), StreamError>;

    async fn close(&self) -> Result<(), StreamError>;
}

pub trait BedrockPayloadPart: Send + Sync {
    fn bytes(&self) -> Option<&[u8]>;
}

pub trait BedrockOutputChunk: Send + Sync {
    fn value(&self) -> Option<&dyn BedrockPayloadPart>;
}

#[async_trait]
pub trait BedrockOutputStream: Send + Sync {
    async fn receive(&self) -> Result<Option<Box<dyn BedrockOutputChunk>>, StreamError>;
}

#[async_trait]
pub trait BedrockBidirectionalStream: Send + Sync {
    fn input_stream(&self) -> &dyn BedrockInputStream;

    async fn await_output(&self) -> Result<(Json, Box<dyn BedrockOutputStream>), StreamError>;
}

#[async_trait]
pub trait ClosableBedrockRuntimeClient: Send + Sync {
    async fn close(&self) -> Result<(), StreamError>;
}
