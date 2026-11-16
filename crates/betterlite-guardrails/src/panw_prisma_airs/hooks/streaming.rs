use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use thiserror::Error;

use super::error::HookDataError;
use crate::guardrail_message_scope::StreamItem;
use crate::panw_prisma_airs::error_detail::GuardrailHttpError;
use crate::prisma_airs_masking::{MaskingError, Response};

pub mod assembly;
pub mod frames;
pub mod hook;
pub mod raw;

// LiteLLM routes on the first chunk: raw SSE bytes, /v1/responses events, or ModelResponse chunks.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    Bytes(Vec<u8>),
    Event(GuardrailValue),
    Model(GuardrailValue),
}

impl From<StreamChunk> for StreamItem {
    fn from(chunk: StreamChunk) -> Self {
        match chunk {
            StreamChunk::Bytes(bytes) => Self::Bytes(bytes),
            StreamChunk::Event(event) | StreamChunk::Model(event) => Self::Json(event),
        }
    }
}

#[must_use]
pub fn into_items(chunks: Vec<StreamChunk>) -> Vec<StreamItem> {
    let mut items = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        items.push(StreamItem::from(chunk));
    }
    items
}

// The chunks the caller collected plus the assembler U2 will provide.
pub struct StreamingResponse<'stream> {
    pub chunks: Vec<StreamChunk>,
    pub assembler: &'stream dyn StreamAssembler,
}

// B2227's stream_chunk_builder and U2's MockResponseIterator reach this hook through one seam.
pub trait StreamAssembler {
    fn assemble(&self, chunks: &[GuardrailValue]) -> Result<Option<Response>, StreamAssemblyError>;
    fn reassemble(&self, response: &Response) -> Result<Vec<StreamChunk>, StreamAssemblyError>;
}

#[derive(Debug, Error)]
#[error("the streamed response could not be assembled or rebuilt")]
pub struct StreamAssemblyError;

// Anything that is not an HTTPException becomes the generic frame; LiteLLM catches Exception there.
#[derive(Debug, Error)]
pub enum StreamError {
    #[error(transparent)]
    Http(#[from] GuardrailHttpError),
    #[error("the streamed response could not be scanned: {source}")]
    Data {
        #[source]
        source: HookDataError,
    },
    #[error("the streamed response could not be reassembled: {source}")]
    Assembly {
        #[source]
        source: StreamAssemblyError,
    },
    #[error(transparent)]
    Masking(#[from] MaskingError),
}

impl From<assembly::StreamScanError> for StreamError {
    fn from(error: assembly::StreamScanError) -> Self {
        match error {
            assembly::StreamScanError::Http(http) => Self::Http(http),
            assembly::StreamScanError::Data { source } => Self::Data { source },
            assembly::StreamScanError::Masking(source) => Self::Masking(source),
        }
    }
}
