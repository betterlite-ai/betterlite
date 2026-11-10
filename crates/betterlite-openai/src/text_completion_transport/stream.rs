use std::collections::VecDeque;
use std::pin::Pin;

use betterlite_streaming::provider_chunk_decode::chunk::{
    DecodeState, ProviderChunk, ProviderKind, RequestOptions,
};
use betterlite_streaming::provider_chunk_decode::creator::chunk_creator;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChunk, StreamOptions,
};
use betterlite_streaming::stream_chunk_shaping::tokens::check_send_stream_usage;
use futures_util::{Stream, StreamExt as _, stream};
use thiserror::Error;
use uuid::Uuid;

use super::error::{ErrorSource, TextCompletionError};
use super::sdk::{ByteStream, SdkError};
use crate::stream::SseParser;

// The OpenAI SDK stops after the [DONE] event, so later payloads are never shaped
pub type CompletionStream =
    Pin<Box<dyn Stream<Item = Result<StreamChunk, StreamItemError>> + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorMode {
    // The sync wrapper maps every iteration failure to litellm's OpenAIError, as streaming() does
    Mapped,
    // async_streaming only catches during iteration and keeps the original SDK error
    Raw,
}

#[derive(Debug, Error)]
pub enum StreamItemError {
    #[error(transparent)]
    Mapped(#[from] TextCompletionError),
    #[error(transparent)]
    Sdk(#[from] SdkError),
    #[error(transparent)]
    Chunk(#[from] ProviderChunkError),
}

pub struct StreamSetup {
    pub model: String,
    pub stream_options: Option<StreamOptions>,
    pub merge_reasoning_in_choices: bool,
    pub is_function_call: bool,
    pub base_hidden_params: JsonObject,
    pub repeated_chunk_limit: usize,
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
    pub mode: ErrorMode,
}

enum StreamSource {
    Sdk(SdkError),
    Chunk(ProviderChunkError),
}

struct StreamState {
    bytes: ByteStream,
    parser: SseParser,
    pending: VecDeque<Vec<u8>>,
    shaper: ChunkShaper,
    decode: DecodeState,
    mode: ErrorMode,
    done: bool,
    exhausted: bool,
}

impl StreamState {
    fn failure(&self, source: StreamSource) -> StreamItemError {
        match (self.mode, source) {
            (ErrorMode::Raw, StreamSource::Sdk(error)) => StreamItemError::Sdk(error),
            (ErrorMode::Raw, StreamSource::Chunk(error)) => StreamItemError::Chunk(error),
            (ErrorMode::Mapped, StreamSource::Sdk(error)) => StreamItemError::Mapped(
                TextCompletionError::wrapped(ErrorSource::Sdk(Box::new(error))),
            ),
            (ErrorMode::Mapped, StreamSource::Chunk(error)) => {
                StreamItemError::Mapped(TextCompletionError::wrapped(ErrorSource::Chunk(error)))
            }
        }
    }
}

fn provider_chunk(payload: &[u8]) -> Result<ProviderChunk, ProviderChunkError> {
    let value: JsonValue = serde_json::from_slice(payload)
        .map_err(|source| ProviderChunkError::InvalidChunk { source })?;
    Ok(ProviderChunk::Json(Box::new(value)))
}

// None means the payload shaped no chunk and the next one is read; Some carries the item to yield.
fn process_payload(
    state: &mut StreamState,
    payload: &[u8],
) -> Option<Result<StreamChunk, StreamItemError>> {
    let chunk = match provider_chunk(payload) {
        Ok(chunk) => chunk,
        Err(error) => return Some(Err(state.failure(StreamSource::Chunk(error)))),
    };
    match chunk_creator(&mut state.shaper, &mut state.decode, &chunk) {
        Ok(transformed) => transformed.map(Ok),
        Err(error) => Some(Err(state.failure(StreamSource::Chunk(error)))),
    }
}

async fn step(
    mut state: StreamState,
) -> Option<(Result<StreamChunk, StreamItemError>, StreamState)> {
    loop {
        if state.done {
            return None;
        }
        if let Some(payload) = state.pending.pop_front() {
            if payload.trim_ascii() == b"[DONE]" {
                state.done = true;
                continue;
            }
            if let Some(item) = process_payload(&mut state, &payload) {
                return Some((item, state));
            }
            continue;
        }
        if state.exhausted {
            return None;
        }
        match state.bytes.next().await {
            Some(Ok(bytes)) => match state.parser.push(&bytes) {
                Ok(events) => state.pending.extend(events),
                Err(error) => {
                    let failure = state.failure(StreamSource::Sdk(SdkError::Other {
                        message: error.to_string(),
                    }));
                    return Some((Err(failure), state));
                }
            },
            Some(Err(error)) => {
                let failure = state.failure(StreamSource::Sdk(error));
                return Some((Err(failure), state));
            }
            None => {
                state.pending.extend(state.parser.finish());
                state.exhausted = true;
            }
        }
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
        mode: setup.mode,
        done: false,
        exhausted: false,
    };
    Box::pin(stream::unfold(state, step))
}
