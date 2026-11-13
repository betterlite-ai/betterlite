use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_redact::secret::Redactor;
use bytes::{Bytes, BytesMut};
use futures_util::Stream;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::converse_events::ChunkProjection;
use crate::converse_events::ConverseStreamState;
use crate::event_frames::shape::ResponseStreamShape;
use crate::event_frames::tally::EventStreamTally;
use crate::invoke_events::error::InvokeStreamError;
use crate::invoke_events::framing::{FramedEventStream, FramedEvents, FramedState};
use crate::invoke_events::ids::IdFunction;

mod anthropic;
mod deepseek;
pub mod error;
pub mod fake;
pub mod framing;
mod ids;
mod openai_compatible;

mod tests;

#[derive(Clone, Copy)]
pub struct DecoderClock {
    pub next_id: fn() -> Uuid,
    pub now: fn() -> i64,
}

pub struct DecoderOptions<'model> {
    pub model: &'model str,
    pub json_mode: Option<bool>,
    pub sync_stream: bool,
    pub clock: DecoderClock,
}

pub enum InvokeChunk {
    Base(Box<ChunkProjection>),
    Anthropic(Box<StreamEventChunk>),
    DeepSeek(Box<StreamReply>),
    OpenAiCompatible(Box<StreamReply>),
}

pub struct InvokeStreamDecoder {
    base: ConverseStreamState,
    projection: Projection,
    clock: DecoderClock,
}

enum Projection {
    Base,
    Anthropic(Box<StreamEventConverter<anthropic::ZeroCounter>>),
    DeepSeek { has_finished_thinking: bool },
    OpenAiCompatible,
}

// AWSEventStreamDecoder.__init__: the content blocks, response id and JSON-mode flag every provider shares.
fn base_state(model: &str, json_mode: bool, clock: DecoderClock) -> ConverseStreamState {
    ConverseStreamState {
        content_blocks: Vec::new(),
        tool_calls_index: None,
        response_id: None,
        current_tool_name: None,
        thinking_ran: false,
        provider_reasoning_tokens: None,
        json_mode,
        model: model.to_owned(),
        mappings: IndexMap::new(),
        ids: Arc::new(IdFunction {
            next: clock.next_id,
        }),
    }
}

#[must_use]
pub fn base_decoder(options: &DecoderOptions<'_>) -> InvokeStreamDecoder {
    InvokeStreamDecoder {
        base: base_state(
            options.model,
            options.json_mode.unwrap_or(false),
            options.clock,
        ),
        projection: Projection::Base,
        clock: options.clock,
    }
}

#[must_use]
pub fn anthropic_decoder(options: &DecoderOptions<'_>) -> InvokeStreamDecoder {
    InvokeStreamDecoder {
        base: base_state(options.model, false, options.clock),
        projection: Projection::Anthropic(Box::new(anthropic::converter(
            options.sync_stream,
            options.json_mode,
            options.clock.next_id,
        ))),
        clock: options.clock,
    }
}

#[must_use]
pub fn deepseek_decoder(options: &DecoderOptions<'_>) -> InvokeStreamDecoder {
    InvokeStreamDecoder {
        base: base_state(options.model, false, options.clock),
        projection: Projection::DeepSeek {
            has_finished_thinking: false,
        },
        clock: options.clock,
    }
}

#[must_use]
pub fn openai_compatible_decoder(options: &DecoderOptions<'_>) -> InvokeStreamDecoder {
    InvokeStreamDecoder {
        base: base_state(options.model, false, options.clock),
        projection: Projection::OpenAiCompatible,
        clock: options.clock,
    }
}

impl InvokeStreamDecoder {
    pub fn chunk_parser(&mut self, chunk: &Json) -> Result<InvokeChunk, InvokeStreamError> {
        match &mut self.projection {
            Projection::Base => self
                .base
                .chunk_parser(chunk)
                .map(|projection| InvokeChunk::Base(Box::new(projection)))
                .map_err(InvokeStreamError::Converse),
            Projection::Anthropic(converter) => anthropic::chunk_parser(converter, chunk)
                .map(|chunk| InvokeChunk::Anthropic(Box::new(chunk))),
            Projection::DeepSeek {
                has_finished_thinking,
            } => deepseek::chunk_parser(
                has_finished_thinking,
                chunk,
                self.clock.next_id,
                self.clock.now,
            )
            .map(|chunk| InvokeChunk::DeepSeek(Box::new(chunk))),
            Projection::OpenAiCompatible => {
                openai_compatible::chunk_parser(chunk, self.clock.next_id, self.clock.now)
                    .map(|chunk| InvokeChunk::OpenAiCompatible(Box::new(chunk)))
            }
        }
    }

    pub fn iter_bytes<'decoder, Chunks>(
        &'decoder mut self,
        chunks: Chunks,
        response_headers: Option<BTreeMap<String, String>>,
        redactor: &'decoder Redactor,
    ) -> FramedEvents<'decoder, Chunks>
    where
        Chunks: Iterator<Item = Bytes>,
    {
        let state = FramedState {
            decoder: self,
            buffer: BytesMut::new(),
            tally: EventStreamTally::default(),
            shape: ResponseStreamShape::bedrock_runtime(),
            response_headers,
            done: false,
        };
        FramedEvents {
            state,
            chunks,
            pending: VecDeque::new(),
            redactor,
        }
    }

    pub fn aiter_bytes<'decoder, Chunks>(
        &'decoder mut self,
        chunks: Chunks,
        response_headers: Option<BTreeMap<String, String>>,
        redactor: &'decoder Redactor,
    ) -> FramedEventStream<'decoder, Chunks>
    where
        Chunks: Stream<Item = Bytes> + Unpin,
    {
        let state = FramedState {
            decoder: self,
            buffer: BytesMut::new(),
            tally: EventStreamTally::default(),
            shape: ResponseStreamShape::bedrock_runtime(),
            response_headers,
            done: false,
        };
        FramedEventStream {
            state,
            chunks,
            pending: VecDeque::new(),
            redactor,
        }
    }
}

#[must_use]
pub fn stream_decoder(provider: Option<&str>, options: &DecoderOptions<'_>) -> InvokeStreamDecoder {
    match provider {
        Some("anthropic") => anthropic_decoder(options),
        Some("deepseek_r1") => deepseek_decoder(options),
        Some("moonshot") => openai_compatible_decoder(options),
        Some(_) | None => base_decoder(options),
    }
}

#[must_use]
pub fn bedrock_event_stream_decoder(
    provider: Option<&str>,
    options: &DecoderOptions<'_>,
) -> InvokeStreamDecoder {
    match provider {
        Some("anthropic") => anthropic_decoder(options),
        Some("deepseek_r1") => deepseek_decoder(options),
        // the legacy export has no Moonshot branch and drops json_mode for every other provider
        Some(_) | None => InvokeStreamDecoder {
            base: base_state(options.model, false, options.clock),
            projection: Projection::Base,
            clock: options.clock,
        },
    }
}
