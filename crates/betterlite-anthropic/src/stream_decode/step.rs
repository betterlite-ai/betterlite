use std::error::Error as StdError;

use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;

use crate::stream_events::StreamEventConverter;
use crate::stream_state::json_fragment_buffer::AccumulatedJson as _;
use crate::usage::ReasoningTokenCounter;

use super::error::{DecodeError, ReceiveError};
use super::{ChunkItem, DecodedChunk, TransportChunk, parse_data};

pub(super) enum Progress {
    Produced(DecodedChunk),
    Skipped,
    Ended,
}

pub(super) fn step<SourceFailure, Counter>(
    converter: &mut StreamEventConverter<Counter>,
    incoming: Option<ChunkItem<SourceFailure>>,
) -> Result<Progress, DecodeError<SourceFailure, Counter::Failure>>
where
    SourceFailure: StdError + 'static,
    Counter: ReasoningTokenCounter,
{
    let Some(polled) = incoming else {
        return drained(converter);
    };
    match polled {
        Ok(chunk) => received_line(converter, chunk),
        Err(ReceiveError::Value { source }) => Err(DecodeError::Receiving { source }),
        Err(ReceiveError::Other { source }) => Err(DecodeError::Other { source }),
    }
}

// At end of stream LiteLLM drains one complete value per call, and only while the buffer holds text.
fn drained<SourceFailure, Counter>(
    converter: &mut StreamEventConverter<Counter>,
) -> Result<Progress, DecodeError<SourceFailure, Counter::Failure>>
where
    SourceFailure: StdError + 'static,
    Counter: ReasoningTokenCounter,
{
    if converter.state.json_buffer.snapshot().is_empty() {
        return Ok(Progress::Ended);
    }
    match converter.handle_accumulated_json_chunk("", true) {
        Ok(Some(parsed)) => Ok(Progress::Produced(DecodedChunk::Event(Box::new(parsed)))),
        Ok(None) => Ok(Progress::Ended),
        Err(error) => Err(DecodeError::Event(error)),
    }
}

fn received_line<SourceFailure, Counter>(
    converter: &mut StreamEventConverter<Counter>,
    chunk: TransportChunk,
) -> Result<Progress, DecodeError<SourceFailure, Counter::Failure>>
where
    SourceFailure: StdError + 'static,
    Counter: ReasoningTokenCounter,
{
    let (text, scanned) = match chunk {
        TransportChunk::Text(text) => (text, false),
        TransportChunk::Bytes(bytes) => match String::from_utf8(bytes) {
            Ok(text) => (text, true),
            Err(source) => {
                return Err(DecodeError::Parsing {
                    source: source.utf8_error(),
                    chunk: source.as_bytes().to_vec(),
                });
            }
        },
    };
    // LiteLLM slices a byte chunk from its first data: but requires a text chunk to start with it.
    let window = if scanned {
        text.find("data:")
            .map_or(text.as_str(), |index| text.split_at(index).1)
    } else {
        text.as_str()
    };
    let Some(data) = window.strip_prefix("data:") else {
        return Ok(Progress::Produced(DecodedChunk::Generic(Box::new(
            GenericChunk {
                text: String::new(),
                tool_use: None,
                is_finished: false,
                finish_reason: String::new(),
                usage: None,
                index: Some(0),
                provider_specific_fields: None,
            },
        ))));
    };
    match parse_data(converter, data) {
        Ok(Some(parsed)) => Ok(Progress::Produced(DecodedChunk::Event(Box::new(parsed)))),
        Ok(None) => Ok(Progress::Skipped),
        Err(error) => Err(DecodeError::Event(error)),
    }
}
