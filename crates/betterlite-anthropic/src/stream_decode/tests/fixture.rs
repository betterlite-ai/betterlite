use std::convert::Infallible;

use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use futures_util::{StreamExt as _, stream};
use indexmap::IndexMap;
use tokio::runtime::Builder;
use uuid::Uuid;

use crate::stream_decode::error::{DecodeError, ReceiveError};
use crate::stream_decode::passthrough::{
    ConvertError, ConvertedChunk, convert_chunk_to_generic_chunk,
};
use crate::stream_decode::{ChunkStream, DecodedChunk, TransportChunk};
use crate::stream_events::StreamEventConverter;
use crate::stream_events::error::StreamEventError;
use crate::stream_state::state::{IdSource, StreamOptions, StreamState};
use crate::usage::ReasoningTokenCounter;

pub(super) const RESPONSE_UUID: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;
pub(super) const RESPONSE_ID: &str = "chatcmpl-01234567-89ab-4def-8123-456789abcdef";

struct FixedIds(Uuid);

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        self.0
    }
}

pub(super) struct Counter;

impl ReasoningTokenCounter for Counter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub(super) struct SourceFailure {
    pub(super) message: String,
}

pub(super) type Source = Result<TransportChunk, ReceiveError<SourceFailure>>;
pub(super) type Outcome = Result<DecodedChunk, DecodeError<SourceFailure, Infallible>>;
pub(super) type Conversion = Result<ConvertedChunk, ConvertError<Infallible>>;

pub(super) fn converter() -> StreamEventConverter<Counter> {
    let ids = FixedIds(Uuid::from_u128(RESPONSE_UUID));
    let state = StreamState::new(
        StreamOptions {
            sync_stream: false,
            json_mode: None,
            speed: None,
            tool_name_reverse_map: IndexMap::new(),
        },
        &ids,
    );
    StreamEventConverter {
        state,
        counter: Counter,
        json_could_close: false,
    }
}

pub(super) fn drive_sync(items: Vec<Source>) -> Vec<Outcome> {
    ChunkStream {
        converter: converter(),
        source: items.into_iter(),
        source_ended: false,
    }
    .collect()
}

pub(super) fn drive_async(items: Vec<Source>) -> Vec<Outcome> {
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let mut stream = ChunkStream {
                converter: converter(),
                source: stream::iter(items),
                source_ended: false,
            };
            let mut outcomes = Vec::new();
            while let Some(outcome) = stream.next().await {
                outcomes.push(outcome);
            }
            outcomes
        })
}

pub(super) fn convert(chunks: Vec<TransportChunk>) -> Vec<Conversion> {
    let mut converter = converter();
    chunks
        .into_iter()
        .map(|chunk| convert_chunk_to_generic_chunk(&mut converter, chunk))
        .collect()
}

pub(super) fn generic(outcome: &Outcome) -> &GenericChunk {
    match outcome {
        Ok(DecodedChunk::Generic(chunk)) => chunk,
        Ok(DecodedChunk::Event(_)) => panic!("a generic chunk was expected, not an event"),
        Err(error) => panic!("a generic chunk was expected, got the error {error}"),
    }
}

pub(super) fn failure(outcome: &Outcome) -> &DecodeError<SourceFailure, Infallible> {
    match outcome {
        Err(error) => error,
        Ok(DecodedChunk::Event(_) | DecodedChunk::Generic(_)) => panic!("an error was expected"),
    }
}

pub(super) fn stream_failure(
    error: &DecodeError<SourceFailure, Infallible>,
) -> &StreamEventError<Infallible> {
    match error {
        DecodeError::Event(inner) => inner,
        DecodeError::Receiving { source } | DecodeError::Other { source } => {
            panic!("an event error was expected, got {source}")
        }
        DecodeError::Parsing { source, .. } => panic!("an event error was expected, got {source}"),
    }
}
