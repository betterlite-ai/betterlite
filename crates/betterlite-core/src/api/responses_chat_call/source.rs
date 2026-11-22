use std::collections::VecDeque;
use std::fmt::Display;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::stream::SseParser;
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::provider_chunk_decode::wrapper::{
    StreamSetup as WrapperSetup, StreamWrapper,
};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChunk, StreamOptions,
};
use futures_util::StreamExt as _;
use uuid::Uuid;

use crate::api::chat_response_stream::chunk::wrapper_view;
use crate::api::chat_response_stream::source::AsyncChunkStream;
use crate::api::messages_stream_split::errors::StreamFailure;

use super::completions::CompletionBody;

const DONE_SENTINEL: &[u8] = b"[DONE]";
// LiteLLM's REPEATED_STREAMING_CHUNK_LIMIT default
const REPEATED_CHUNK_LIMIT: usize = 100;

// What CustomStreamWrapper is built with when handler.py hands its stream to the Responses iterator
pub struct ChunkSourceConfig {
    pub provider: String,
    pub model: String,
    pub stream_options: Option<StreamOptions>,
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
}

// B014's SSE bytes as B924 reads them: CustomStreamWrapper's decode loop, one ModelResponseStream per item
pub struct ResponsesChunkSource {
    body: CompletionBody,
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

impl ResponsesChunkSource {
    #[must_use]
    pub fn new(config: ChunkSourceConfig, body: CompletionBody) -> Self {
        let wrapper = StreamWrapper::new(
            &config.provider,
            WrapperSetup {
                model: config.model,
                stream_options: config.stream_options,
                merge_reasoning_in_choices: false,
                is_function_call: false,
                base_hidden_params: JsonObject::new(),
                repeated_chunk_limit: REPEATED_CHUNK_LIMIT,
                now: config.now,
                next_id: config.next_id,
            },
        );
        Self {
            body,
            parser: SseParser::default(),
            pending: VecDeque::new(),
            wrapper,
            phase: Phase::Reading,
        }
    }

    async fn advance(&mut self) -> Result<Option<Json>, StreamFailure> {
        loop {
            if let Some(payload) = self.pending.pop_front() {
                if payload.trim_ascii() == DONE_SENTINEL {
                    self.phase = Phase::Done;
                    continue;
                }
                match self.shape(&payload)? {
                    Some(chunk) => return Ok(Some(chunk)),
                    None => continue,
                }
            }
            if self.phase == Phase::Done || self.phase == Phase::Exhausted {
                return self.closing();
            }
            let Some(read) = self.body.0.next().await else {
                self.pending.extend(self.parser.finish());
                self.phase = Phase::Exhausted;
                continue;
            };
            let bytes = read.map_err(|error| unclassified(&error))?;
            let events = self
                .parser
                .push(&bytes)
                .map_err(|error| unclassified(&error))?;
            self.pending.extend(events);
        }
    }

    // None means the payload shaped no chunk, after the wrapper stripped a usage-only one.
    fn shape(&mut self, payload: &[u8]) -> Result<Option<Json>, StreamFailure> {
        let value: JsonValue =
            serde_json::from_slice(payload).map_err(|source| unclassified(&source))?;
        let shaped = self
            .wrapper
            .push(&ProviderChunk::Json(Box::new(value)))
            .map_err(|error| unclassified(&error))?;
        let Some(chunk) = shaped else {
            return Ok(None);
        };
        view_chunk(&chunk)
    }

    // The wrapper's closing chunks: the finish reason when none went out, then the usage chunk it was asked for.
    fn closing(&mut self) -> Result<Option<Json>, StreamFailure> {
        let Some(closing) = self.wrapper.finish() else {
            return Ok(None);
        };
        let chunk = closing.map_err(|error| unclassified(&error))?;
        view_chunk(&chunk)
    }
}

fn unclassified(error: &dyn Display) -> StreamFailure {
    StreamFailure::Unclassified {
        text: error.to_string(),
    }
}

fn view_chunk(chunk: &StreamChunk) -> Result<Option<Json>, StreamFailure> {
    wrapper_view(chunk)
        .map(Some)
        .map_err(|error| unclassified(&error))
}

#[async_trait]
impl AsyncChunkStream for ResponsesChunkSource {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
        if self.phase == Phase::Failed {
            return Ok(None);
        }
        let result = self.advance().await;
        if result.is_err() {
            self.phase = Phase::Failed;
        }
        result
    }
}
