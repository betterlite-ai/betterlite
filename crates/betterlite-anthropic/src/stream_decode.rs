use std::convert::Infallible;
use std::error::Error as StdError;
use std::str;

use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use serde_json::error::Category;

use crate::stream_events::StreamEventConverter;
use crate::stream_events::chunk::StreamEventChunk;
use crate::stream_events::error::StreamEventError;
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

// The source hands over transport chunks; the decoder owns the converter, the exhausted flag, the buffered
// byte count and the event bound.
pub struct ChunkStream<Source, Counter> {
    pub converter: StreamEventConverter<Counter>,
    pub source: Source,
    pub source_ended: bool,
    // Bytes appended since the decoder last produced a value; the event bound reads this, not the buffer.
    pub accumulated_bytes: usize,
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
    // The shared framing parser hands over one payload per call and owns no running count, so the bound
    // covers the payload; the iterator's accumulated count covers a value split across its fragments.
    let mut accumulated_bytes = 0usize;
    parse_data::<Infallible, Counter>(converter, text, max_event_bytes, &mut accumulated_bytes)
}

// LiteLLM tries one JSON value first and keeps accumulating after the first failure, permanently.
fn parse_data<SourceFailure, Counter>(
    converter: &mut StreamEventConverter<Counter>,
    data: &str,
    max_event_bytes: usize,
    accumulated_bytes: &mut usize,
) -> Result<Option<StreamEventChunk>, DecodeError<SourceFailure, Counter::Failure>>
where
    SourceFailure: StdError + 'static,
    Counter: ReasoningTokenCounter,
{
    if converter.state.chunk_type != ChunkType::AccumulatedJson {
        if let Ok(value) = serde_json::from_str::<Json>(data) {
            return converter
                .chunk_parser(&value)
                .map(Some)
                .map_err(DecodeError::Event);
        }
        converter.state.chunk_type = ChunkType::AccumulatedJson;
    }
    // LiteLLM buffers an unterminated value for the rest of the stream; the shared max_event_bytes bounds it.
    // The bound reads a running count, so a deferred fragment costs its append and not a buffer snapshot.
    let appended = accumulated_bytes.saturating_add(data.len());
    if appended > max_event_bytes {
        return Err(DecodeError::TooLarge {
            limit: max_event_bytes,
        });
    }
    let parsed = converter
        .handle_accumulated_json_chunk(data, false)
        .map_err(DecodeError::Event)?;
    if parsed.is_some() {
        // A produced value drained the front, so the next bound counts from an empty buffer.
        *accumulated_bytes = 0;
        return Ok(parsed);
    }
    *accumulated_bytes = appended;
    // The accumulator defers a fragment whose last byte cannot close a value; dropping then would rescan
    // a buffer LiteLLM leaves alone.
    if !converter.json_could_close {
        return Ok(None);
    }
    discard_unparseable_prefix(converter, accumulated_bytes).map_err(DecodeError::Event)
}

// LiteLLM keeps a failed front in the buffer for good and loses every later event; drop only that value.
// A value that only lacks its tail keeps accumulating, so only a syntax or data failure is broken.
fn discard_unparseable_prefix<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    accumulated_bytes: &mut usize,
) -> Result<Option<StreamEventChunk>, StreamEventError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let buffered = converter.state.json_buffer.snapshot();
    let mut rest = buffered.as_str();
    let mut dropped = false;
    loop {
        let trimmed = rest.trim_start();
        let front = serde_json::Deserializer::from_str(trimmed)
            .into_iter::<Json>()
            .next();
        let end = match front {
            Some(Err(error)) if error.classify() != Category::Eof => {
                prefix_end(trimmed).unwrap_or(trimmed.len())
            }
            Some(_) | None => break,
        };
        tracing::warn!(
            "dropping {end} unparseable bytes from the accumulated anthropic stream buffer"
        );
        rest = trimmed.get(end..).unwrap_or_default();
        dropped = true;
    }
    if !dropped {
        return Ok(None);
    }
    // One write and one parse per call keep a line of broken values linear in its length.
    converter.state.json_buffer.set(rest.to_owned());
    converter.json_could_close = true;
    let parsed = converter.handle_accumulated_json_chunk("", false);
    // The tail the pop leaves is at most the scanned remainder, so the count stays an upper bound.
    *accumulated_bytes = rest.len();
    parsed
}

// The broken value ends at its own closing bracket or quote, so a valid value glued behind it survives.
fn prefix_end(trimmed: &str) -> Option<usize> {
    match trimmed.as_bytes().first().copied()? {
        b'{' | b'[' => container_end(trimmed),
        b'"' => quoted_end(trimmed.as_bytes()),
        _ => bare_end(trimmed),
    }
}

// Counts brackets outside strings so the dropped prefix ends where the value does, not at the buffer end.
fn container_end(trimmed: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in trimmed.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
        } else if character == '"' {
            in_string = true;
        } else if matches!(character, '{' | '[') {
            depth = depth.checked_add(1)?;
        } else if matches!(character, '}' | ']') {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return index.checked_add(1);
            }
        }
    }
    None
}

// A broken quoted token ends after its next unescaped quote, so a valid value glued behind it survives.
fn quoted_end(bytes: &[u8]) -> Option<usize> {
    let mut position = 1usize;
    while let Some(&byte) = bytes.get(position) {
        if byte == b'\\' {
            position = position.checked_add(2)?;
            continue;
        }
        if byte == b'"' {
            return position.checked_add(1);
        }
        position = position.saturating_add(1);
    }
    None
}

// A broken bare token ends where the next value can start; one with no boundary runs to the end.
fn bare_end(trimmed: &str) -> Option<usize> {
    trimmed
        .char_indices()
        .skip(1)
        .find(|&(_, character)| {
            matches!(character, '{' | '[' | '"') || character.is_ascii_whitespace()
        })
        .map(|(index, _)| index)
}

type ChunkItem<SourceFailure> = Result<TransportChunk, ReceiveError<SourceFailure>>;
