use std::collections::VecDeque;
use std::pin::Pin;

use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::provider_chunk_decode::wrapper::{
    StreamSetup as WrapperStreamSetup, StreamWrapper, WrapperError,
};
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamChunk};
use futures_util::{Stream, StreamExt as _, stream};

use super::error::{ErrorSource, TextCompletionError};
use super::sdk::{ByteStream, SdkError};
use crate::stream::SseParser;

// clippy::pub_use is denied and no expect may be added, so an alias keeps the moved type nameable here
pub type StreamSetup = WrapperStreamSetup;

// The OpenAI SDK stops after the [DONE] event, so later payloads are never shaped
pub type CompletionStream =
    Pin<Box<dyn Stream<Item = Result<StreamChunk, TextCompletionError>> + Send>>;

struct StreamState {
    bytes: ByteStream,
    parser: SseParser,
    pending: VecDeque<Vec<u8>>,
    wrapper: StreamWrapper,
    phase: Phase,
}

// Exhausted means the bytes ended but parsed events may remain; Done means the [DONE] event was read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Reading,
    Exhausted,
    Done,
    Failed,
}

type Item = Result<StreamChunk, TextCompletionError>;

fn provider_chunk(payload: &[u8]) -> Result<ProviderChunk, ProviderChunkError> {
    let value: JsonValue = serde_json::from_slice(payload)
        .map_err(|source| ProviderChunkError::InvalidChunk { source })?;
    Ok(ProviderChunk::Json(Box::new(value)))
}

// The wrapper's errors are the two sources this path has always reported.
fn wrapper_error(error: WrapperError) -> TextCompletionError {
    let source = match error {
        WrapperError::Chunk(chunk) => ErrorSource::Chunk(chunk),
        WrapperError::Usage(usage) => ErrorSource::Serialize(usage),
    };
    TextCompletionError::wrapped(source)
}

// None means the payload shaped no chunk and the next one is read; Some carries the item to yield.
fn process_payload(state: &mut StreamState, payload: &[u8]) -> Option<Item> {
    let chunk = match provider_chunk(payload) {
        Ok(chunk) => chunk,
        Err(error) => return Some(Err(TextCompletionError::wrapped(ErrorSource::Chunk(error)))),
    };
    state
        .wrapper
        .push(&chunk)
        .map_err(wrapper_error)
        .transpose()
}

// A byte read that is dropped here still advances the byte stream, so the loop in step ends.
fn absorb(
    state: &mut StreamState,
    read: Result<Vec<u8>, SdkError>,
) -> Result<(), TextCompletionError> {
    let bytes =
        read.map_err(|error| TextCompletionError::wrapped(ErrorSource::Sdk(Box::new(error))))?;
    let events = state.parser.push(&bytes).map_err(|error| {
        TextCompletionError::wrapped(ErrorSource::Sdk(Box::new(SdkError::Sse(error))))
    })?;
    state.pending.extend(events);
    Ok(())
}

enum Advance {
    Next,
    Item(Box<Item>),
    Closing,
}

async fn advance(state: &mut StreamState) -> Advance {
    if state.phase == Phase::Done {
        return Advance::Closing;
    }
    if let Some(payload) = state.pending.pop_front() {
        if payload.trim_ascii() == b"[DONE]" {
            state.phase = Phase::Done;
            return Advance::Next;
        }
        return process_payload(state, &payload)
            .map_or(Advance::Next, |item| Advance::Item(Box::new(item)));
    }
    if state.phase == Phase::Exhausted {
        return Advance::Closing;
    }
    let Some(read) = state.bytes.next().await else {
        state.pending.extend(state.parser.finish());
        state.phase = Phase::Exhausted;
        return Advance::Next;
    };
    match absorb(state, read) {
        Ok(()) => Advance::Next,
        Err(error) => Advance::Item(Box::new(Err(error))),
    }
}

async fn step(mut state: StreamState) -> Option<(Item, StreamState)> {
    loop {
        if state.phase == Phase::Failed {
            return None;
        }
        let item = match advance(&mut state).await {
            Advance::Next => continue,
            Advance::Item(item) => *item,
            Advance::Closing => state
                .wrapper
                .finish()
                .map(|chunk| chunk.map_err(wrapper_error))?,
        };
        if item.is_err() {
            state.phase = Phase::Failed;
        }
        return Some((item, state));
    }
}

#[must_use]
pub fn completion_stream(bytes: ByteStream, setup: StreamSetup) -> CompletionStream {
    let state = StreamState {
        bytes,
        parser: SseParser::default(),
        pending: VecDeque::new(),
        wrapper: StreamWrapper::new("text-completion-openai", setup),
        phase: Phase::Reading,
    };
    Box::pin(stream::unfold(state, step))
}
