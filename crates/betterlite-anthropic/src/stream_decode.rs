use std::convert::Infallible;
use std::str;

use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;

use crate::stream_events::StreamEventConverter;
use crate::stream_events::chunk::StreamEventChunk;
use crate::stream_events::error::StreamEventError;
use crate::stream_state::state::ChunkType;
use crate::usage::ReasoningTokenCounter;

pub mod error;
pub mod passthrough;
mod step;
mod stream;
mod tests;

use error::{DecodeError, ReceiveError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportChunk {
    Bytes(Vec<u8>),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedChunk {
    Event(Box<StreamEventChunk>),
    Generic(Box<GenericChunk>),
}

// The source hands over transport chunks; the decoder owns the converter and the exhausted flag.
pub struct ChunkStream<Source, Counter> {
    pub converter: StreamEventConverter<Counter>,
    pub source: Source,
    pub source_ended: bool,
}

// The shared framing parser hands over one event payload, so this path decodes and parses without a data: prefix.
pub fn decode_anthropic_event<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    payload: &[u8],
) -> Result<Option<StreamEventChunk>, DecodeError<Infallible, Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let text = str::from_utf8(payload).map_err(|source| DecodeError::Parsing {
        source,
        chunk: payload.to_vec(),
    })?;
    parse_data(converter, text).map_err(DecodeError::Event)
}

// LiteLLM tries one JSON value first and keeps accumulating after the first failure, permanently.
fn parse_data<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    data: &str,
) -> Result<Option<StreamEventChunk>, StreamEventError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    if converter.state.chunk_type == ChunkType::AccumulatedJson {
        return converter.handle_accumulated_json_chunk(data, false);
    }
    if let Ok(value) = serde_json::from_str::<Json>(data) {
        return converter.chunk_parser(&value).map(Some);
    }
    converter.state.chunk_type = ChunkType::AccumulatedJson;
    converter.handle_accumulated_json_chunk(data, false)
}

type ChunkItem<SourceFailure> = Result<TransportChunk, ReceiveError<SourceFailure>>;
