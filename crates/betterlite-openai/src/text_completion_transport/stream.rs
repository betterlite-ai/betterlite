use std::collections::VecDeque;
use std::pin::Pin;

use betterlite_streaming::provider_chunk_decode::chunk::{
    DecodeState, ProviderChunk, ProviderKind, RequestOptions,
};
use betterlite_streaming::provider_chunk_decode::creator::chunk_creator;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::creation::{StreamInstant, build_chunk};
use betterlite_streaming::stream_chunk_shaping::decision::is_delta_empty;
use betterlite_streaming::stream_chunk_shaping::records::{
    ChunkArguments, JsonObject, JsonValue, StreamChunk, StreamOptions,
};
use betterlite_streaming::stream_chunk_shaping::tokens::{
    check_send_stream_usage, finish_reason_handler,
};
use betterlite_streaming::stream_chunk_shaping::usage_totals::calculate_total_usage;
use futures_util::{Stream, StreamExt as _, stream};
use uuid::Uuid;

use super::error::{ErrorSource, TextCompletionError};
use super::sdk::{ByteStream, SdkError};
use crate::stream::SseParser;

// The OpenAI SDK stops after the [DONE] event, so later payloads are never shaped
pub type CompletionStream =
    Pin<Box<dyn Stream<Item = Result<StreamChunk, TextCompletionError>> + Send>>;

pub struct StreamSetup {
    pub model: String,
    pub stream_options: Option<StreamOptions>,
    pub merge_reasoning_in_choices: bool,
    pub is_function_call: bool,
    pub base_hidden_params: JsonObject,
    pub repeated_chunk_limit: usize,
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
}

struct StreamState {
    bytes: ByteStream,
    parser: SseParser,
    pending: VecDeque<Vec<u8>>,
    shaper: ChunkShaper,
    decode: DecodeState,
    has_stream_options: bool,
    sent_stream_usage: bool,
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

// The running total goes on the chunk's hidden params, where litellm's wrapper puts it.
fn attach_usage(state: &StreamState, chunk: &mut StreamChunk) -> Result<(), TextCompletionError> {
    let usage: JsonValue = serde_json::to_value(calculate_total_usage(&state.shaper.chunks))
        .and_then(serde_json::from_value)
        .map_err(|source| TextCompletionError::wrapped(ErrorSource::Serialize(source)))?;
    chunk.hidden_params.insert("usage".to_owned(), usage);
    Ok(())
}

// The wrapper keeps every chunk for the totals and sends provider usage only in its closing chunk.
fn record(
    state: &mut StreamState,
    mut chunk: StreamChunk,
) -> Result<Option<StreamChunk>, TextCompletionError> {
    state.shaper.chunks.push(chunk.clone());
    let had_usage = chunk.usage.take().is_some();
    let silent = chunk.provider_specific_fields.is_none()
        && chunk
            .choices
            .iter()
            .all(|choice| is_delta_empty(&choice.delta) && choice.finish_reason.is_none());
    if had_usage && silent {
        return Ok(None);
    }
    if state.shaper.progress.sent_last_chunk && !state.has_stream_options {
        attach_usage(state, &mut chunk)?;
    }
    Ok(Some(chunk))
}

// None means the payload shaped no chunk and the next one is read; Some carries the item to yield.
fn process_payload(state: &mut StreamState, payload: &[u8]) -> Option<Item> {
    let chunk = match provider_chunk(payload) {
        Ok(chunk) => chunk,
        Err(error) => return Some(Err(TextCompletionError::wrapped(ErrorSource::Chunk(error)))),
    };
    match chunk_creator(&mut state.shaper, &mut state.decode, &chunk) {
        Ok(Some(transformed)) => record(state, transformed).transpose(),
        Ok(None) => None,
        Err(error) => Some(Err(TextCompletionError::wrapped(ErrorSource::Chunk(error)))),
    }
}

// The closing chunks: the finish reason when none went out yet, then the usage chunk when asked for.
fn finalize(state: &mut StreamState) -> Option<Item> {
    if state.shaper.progress.sent_last_chunk {
        if state.sent_stream_usage || !state.decode.options.send_stream_usage {
            return None;
        }
        state.sent_stream_usage = true;
        let mut response = build_chunk(&mut state.shaper, ChunkArguments::default(), None);
        response.usage = Some(calculate_total_usage(&state.shaper.chunks));
        return Some(Ok(response));
    }
    state.shaper.progress.sent_last_chunk = true;
    let mut chunk = finish_reason_handler(&mut state.shaper);
    if !state.has_stream_options
        && let Err(error) = attach_usage(state, &mut chunk)
    {
        return Some(Err(error));
    }
    Some(Ok(chunk))
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
            Advance::Closing => finalize(&mut state)?,
        };
        if item.is_err() {
            state.phase = Phase::Failed;
        }
        return Some((item, state));
    }
}

#[must_use]
pub fn completion_stream(bytes: ByteStream, setup: StreamSetup) -> CompletionStream {
    let mut shaper = ChunkShaper::new(
        setup.model.clone(),
        Some("text-completion-openai".to_owned()),
        setup.merge_reasoning_in_choices,
        setup.base_hidden_params,
    );
    shaper.now = setup.now;
    shaper.next_id = setup.next_id;
    let decode = DecodeState {
        model: setup.model,
        kind: ProviderKind::Builtin,
        received_finish_reason: None,
        intermittent_finish_reason: None,
        sent_first_chunk: false,
        complete_response: String::new(),
        completion_stream: None,
        options: RequestOptions {
            send_stream_usage: check_send_stream_usage(setup.stream_options.as_ref()),
            is_function_call: setup.is_function_call,
        },
        repeated_chunk_limit: setup.repeated_chunk_limit,
        next_id: setup.next_id,
    };
    let state = StreamState {
        bytes,
        parser: SseParser::default(),
        pending: VecDeque::new(),
        shaper,
        decode,
        has_stream_options: setup.stream_options.is_some(),
        sent_stream_usage: false,
        phase: Phase::Reading,
    };
    Box::pin(stream::unfold(state, step))
}
