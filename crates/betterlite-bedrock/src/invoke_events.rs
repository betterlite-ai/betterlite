use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use betterlite_anthropic::stream_events::StreamEventConverter;
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_redact::secret::Redactor;
use bytes::BytesMut;
use indexmap::IndexMap;
use uuid::Uuid;

use crate::converse_events::ChunkProjection;
use crate::converse_events::ConverseStreamState;
use crate::event_frames::shape::ResponseStreamShape;
use crate::event_frames::tally::EventStreamTally;
use crate::invoke_events::error::InvokeStreamError;
use crate::invoke_events::framing::{FramedEvents, FramedState};
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
fn new_decoder(
    projection: Projection,
    json_mode: bool,
    options: &DecoderOptions<'_>,
) -> InvokeStreamDecoder {
    InvokeStreamDecoder {
        base: ConverseStreamState {
            content_blocks: Vec::new(),
            tool_calls_index: None,
            response_id: None,
            current_tool_name: None,
            thinking_ran: false,
            provider_reasoning_tokens: None,
            json_mode,
            model: options.model.to_owned(),
            mappings: IndexMap::new(),
            ids: Arc::new(IdFunction {
                next: options.clock.next_id,
            }),
        },
        projection,
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

    // AWSEventStreamDecoder.iter_bytes and aiter_bytes: the frames pull from a sync iterator or an async stream.
    pub fn framed<'decoder, Chunks>(
        &'decoder mut self,
        chunks: Chunks,
        response_headers: Option<BTreeMap<String, String>>,
        redactor: &'decoder Redactor,
    ) -> FramedEvents<'decoder, Chunks> {
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
}

#[must_use]
pub fn stream_decoder(provider: Option<&str>, options: &DecoderOptions<'_>) -> InvokeStreamDecoder {
    match provider {
        Some("anthropic") => {
            let converter = anthropic::converter(
                options.sync_stream,
                options.json_mode,
                options.clock.next_id,
            );
            new_decoder(Projection::Anthropic(Box::new(converter)), false, options)
        }
        Some("deepseek_r1") => new_decoder(
            Projection::DeepSeek {
                has_finished_thinking: false,
            },
            false,
            options,
        ),
        Some("moonshot") => new_decoder(Projection::OpenAiCompatible, false, options),
        Some(_) | None => new_decoder(
            Projection::Base,
            options.json_mode.unwrap_or(false),
            options,
        ),
    }
}

#[must_use]
pub fn bedrock_event_stream_decoder(
    provider: Option<&str>,
    options: &DecoderOptions<'_>,
) -> InvokeStreamDecoder {
    match provider {
        Some("anthropic" | "deepseek_r1") => stream_decoder(provider, options),
        // the legacy export has no Moonshot branch and drops json_mode for every other provider
        Some(_) | None => new_decoder(Projection::Base, false, options),
    }
}
