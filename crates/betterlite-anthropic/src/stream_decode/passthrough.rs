use std::convert::Infallible;
use std::error::Error as StdError;
use std::str::Utf8Error;

use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::stream_events::StreamEventConverter;
use crate::stream_events::chunk::StreamEventChunk;
use crate::stream_events::error::StreamEventError;
use crate::usage::ReasoningTokenCounter;

use super::TransportChunk;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertedChunk {
    Event(Box<StreamEventChunk>),
    // ModelResponseStream(id=self.response_id) for a chunk with no data: line.
    Default { id: String },
}

#[derive(Debug, Error)]
pub enum ConvertError<CounterFailure = Infallible>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Utf8(#[from] Utf8Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Event(#[from] StreamEventError<CounterFailure>),
}

// The pass-through converter scans any chunk for data:, decodes the whole remainder once and never accumulates.
pub fn convert_chunk_to_generic_chunk<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    chunk: TransportChunk,
) -> Result<ConvertedChunk, ConvertError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let text = match chunk {
        TransportChunk::Text(text) => text,
        TransportChunk::Bytes(bytes) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(source) => return Err(ConvertError::Utf8(source.utf8_error())),
        },
    };
    let window = text
        .find("data:")
        .map_or(text.as_str(), |index| text.split_at(index).1);
    let Some(data) = window.strip_prefix("data:") else {
        return Ok(ConvertedChunk::Default {
            id: converter.state.response_id.clone(),
        });
    };
    let value = serde_json::from_str::<Json>(data)?;
    converter
        .chunk_parser(&value)
        .map(|parsed| ConvertedChunk::Event(Box::new(parsed)))
        .map_err(ConvertError::Event)
}
