use std::convert::Infallible;
use std::error::Error as StdError;
use std::str;

use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;

use crate::stream_events::StreamEventConverter;
use crate::stream_events::chunk::StreamEventChunk;
use crate::stream_state::json_fragment_buffer::AccumulatedJson as _;
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

// The source hands over transport chunks; the decoder owns the converter, the exhausted flag and the event bound.
pub struct ChunkStream<Source, Counter> {
    pub converter: StreamEventConverter<Counter>,
    pub source: Source,
    pub source_ended: bool,
    pub max_event_bytes: usize,
}

// The shared framing parser hands over one event payload, so this path decodes and parses without a data: prefix.
pub fn decode_anthropic_event<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    payload: &[u8],
    max_event_bytes: usize,
) -> Result<Option<StreamEventChunk>, DecodeError<Infallible, Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let text = str::from_utf8(payload).map_err(|source| DecodeError::Parsing {
        source,
        chunk: payload.to_vec(),
    })?;
    parse_data::<Infallible, Counter>(converter, text, max_event_bytes)
}

// LiteLLM tries one JSON value first and keeps accumulating after the first failure, permanently.
fn parse_data<SourceFailure, Counter>(
    converter: &mut StreamEventConverter<Counter>,
    data: &str,
    max_event_bytes: usize,
) -> Result<Option<StreamEventChunk>, DecodeError<SourceFailure, Counter::Failure>>
where
    SourceFailure: StdError + 'static,
    Counter: ReasoningTokenCounter,
{
    if converter.state.chunk_type == ChunkType::AccumulatedJson {
        return accumulate(converter, data, max_event_bytes);
    }
    if let Ok(value) = serde_json::from_str::<Json>(data) {
        return converter
            .chunk_parser(&value)
            .map(Some)
            .map_err(DecodeError::Event);
    }
    converter.state.chunk_type = ChunkType::AccumulatedJson;
    accumulate(converter, data, max_event_bytes)
}

// LiteLLM buffers an unterminated value for the rest of the stream; the shared max_event_bytes bounds it.
fn accumulate<SourceFailure, Counter>(
    converter: &mut StreamEventConverter<Counter>,
    data: &str,
    max_event_bytes: usize,
) -> Result<Option<StreamEventChunk>, DecodeError<SourceFailure, Counter::Failure>>
where
    SourceFailure: StdError + 'static,
    Counter: ReasoningTokenCounter,
{
    let buffered = converter.state.json_buffer.snapshot();
    if buffered.len().saturating_add(data.len()) > max_event_bytes {
        return Err(DecodeError::TooLarge {
            limit: max_event_bytes,
        });
    }
    converter
        .handle_accumulated_json_chunk(data, false)
        .map_err(DecodeError::Event)
}

type ChunkItem<SourceFailure> = Result<TransportChunk, ReceiveError<SourceFailure>>;
