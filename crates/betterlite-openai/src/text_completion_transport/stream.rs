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

// The provider name completion_stream shapes its chunks under.
pub const TEXT_COMPLETION_PROVIDER: &str = "text-completion-openai";

pub type ProviderStream<E> = Pin<Box<dyn Stream<Item = Result<StreamChunk, E>> + Send>>;

// The OpenAI SDK stops after the [DONE] event, so later payloads are never shaped
pub type CompletionStream = ProviderStream<TextCompletionError>;

// What the pump can meet while it reads: a chunk it cannot shape, usage it cannot write, or a byte read that failed.
#[derive(Debug)]
pub enum PumpError {
    Chunk(ProviderChunkError),
    Usage(serde_json::Error),
    Sdk(Box<SdkError>),
}

struct StreamState<E> {
    bytes: ByteStream,
    parser: SseParser,
    pending: VecDeque<Vec<u8>>,
    wrapper: StreamWrapper,
    phase: Phase,
    wrap: fn(PumpError) -> E,
}

// Exhausted means the bytes ended but parsed events may remain; Done means the [DONE] event was read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Reading,
    Exhausted,
    Done,
    Failed,
}

type Item<E> = Result<StreamChunk, E>;

fn provider_chunk(payload: &[u8]) -> Result<ProviderChunk, ProviderChunkError> {
    let value: JsonValue = serde_json::from_slice(payload)
        .map_err(|source| ProviderChunkError::InvalidChunk { source })?;
    Ok(ProviderChunk::Json(Box::new(value)))
}

fn wrapper_error<E>(wrap: fn(PumpError) -> E, error: WrapperError) -> E {
    match error {
        WrapperError::Chunk(chunk) => wrap(PumpError::Chunk(chunk)),
        WrapperError::Usage(usage) => wrap(PumpError::Usage(usage)),
    }
}

// None means the payload shaped no chunk and the next one is read; Some carries the item to yield.
fn process_payload<E>(state: &mut StreamState<E>, payload: &[u8]) -> Option<Item<E>> {
    let wrap = state.wrap;
    let chunk = match provider_chunk(payload) {
        Ok(chunk) => chunk,
        Err(error) => return Some(Err(wrap(PumpError::Chunk(error)))),
    };
    state
        .wrapper
        .push(&chunk)
        .map_err(|error| wrapper_error(wrap, error))
        .transpose()
}

// A byte read that is dropped here still advances the byte stream, so the loop in step ends.
fn absorb<E>(state: &mut StreamState<E>, read: Result<Vec<u8>, SdkError>) -> Result<(), E> {
    let wrap = state.wrap;
    let bytes = read.map_err(|error| wrap(PumpError::Sdk(Box::new(error))))?;
    let events = state
        .parser
        .push(&bytes)
        .map_err(|error| wrap(PumpError::Sdk(Box::new(SdkError::Sse(error)))))?;
    state.pending.extend(events);
    Ok(())
}

enum Advance<E> {
    Next,
    Item(Box<Item<E>>),
    Closing,
}

async fn advance<E>(state: &mut StreamState<E>) -> Advance<E> {
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

async fn step<E>(mut state: StreamState<E>) -> Option<(Item<E>, StreamState<E>)> {
    let wrap = state.wrap;
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
                .map(|chunk| chunk.map_err(|error| wrapper_error(wrap, error)))?,
        };
        if item.is_err() {
            state.phase = Phase::Failed;
        }
        return Some((item, state));
    }
}

// The one SSE loop both the text completion and the Azure chat streams run; wrap builds each path's error.
#[must_use]
pub fn provider_stream<E: Send + 'static>(
    provider: &str,
    bytes: ByteStream,
    setup: StreamSetup,
    wrap: fn(PumpError) -> E,
) -> ProviderStream<E> {
    let state = StreamState {
        bytes,
        parser: SseParser::default(),
        pending: VecDeque::new(),
        wrapper: StreamWrapper::new(provider, setup),
        phase: Phase::Reading,
        wrap,
    };
    Box::pin(stream::unfold(state, step::<E>))
}

#[must_use]
pub fn completion_stream(bytes: ByteStream, setup: StreamSetup) -> CompletionStream {
    provider_stream(
        TEXT_COMPLETION_PROVIDER,
        bytes,
        setup,
        text_completion_error,
    )
}

// handler.py wraps every iteration failure in OpenAIError, so the text path maps each pump error to ErrorSource.
#[must_use]
pub fn text_completion_error(error: PumpError) -> TextCompletionError {
    let source = match error {
        PumpError::Chunk(source) => ErrorSource::Chunk(source),
        PumpError::Usage(source) => ErrorSource::Serialize(source),
        PumpError::Sdk(source) => ErrorSource::Sdk(source),
    };
    TextCompletionError::wrapped(source)
}
