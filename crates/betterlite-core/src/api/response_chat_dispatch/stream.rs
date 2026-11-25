use std::collections::VecDeque;
use std::str::FromStr as _;

use async_trait::async_trait;
use betterlite_context::chat_config_selection::selection::{
    ChatConfigError, ChatConfigRequest, select_chat_config,
};
use betterlite_context::chat_config_selection::table::ProviderConfigTable;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::provider_chunk_decode::wrapper::{StreamSetup, StreamWrapper};
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChoice, StreamChunk,
};
use serde::Serialize;

use super::error::{BridgeStreamError, PostStreamError};
use super::input::StreamStamps;
use super::port::{
    BridgeStream, ChatResponse, PostStreamDeps, ResponsesEventStream, TypedChunkStream,
};
use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream::chunk::wrapper_view;
use crate::api::responses_to_chat_stream::stream::{
    ParsedChatLine, ResponsesChatStream, get_model_response_iterator,
};

// LiteLLM's REPEATED_STREAMING_CHUNK_LIMIT default, which CustomStreamWrapper counts against
const REPEATED_CHUNK_LIMIT: usize = 100;

// What get_model_response_iterator and CustomStreamWrapper are built with
pub struct StreamConfig<'config> {
    pub provider: &'config str,
    pub model: &'config str,
    pub json_mode: Option<bool>,
    pub limits: ApiLimits,
    pub stamps: StreamStamps,
}

// B920's iterator over the Responses events, pushed through B005's wrapper one chunk at a time
pub struct TranslatedStream {
    events: Box<dyn ResponsesEventStream>,
    parser: ResponsesChatStream,
    wrapper: StreamWrapper,
    limits: ApiLimits,
    pending: VecDeque<Json>,
    finished: bool,
}

impl TranslatedStream {
    #[must_use]
    pub fn new(events: Box<dyn ResponsesEventStream>, config: &StreamConfig<'_>) -> Self {
        Self {
            events,
            parser: get_model_response_iterator(
                config.json_mode,
                config.stamps.next_id,
                config.stamps.clock,
            ),
            wrapper: wrapper(config.provider, config.model, config.stamps),
            limits: config.limits,
            pending: VecDeque::new(),
            finished: false,
        }
    }
}

#[async_trait]
impl TypedChunkStream for TranslatedStream {
    async fn next_chunk(&mut self) -> Result<Option<Json>, BridgeStreamError> {
        loop {
            if let Some(chunk) = self.pending.pop_front() {
                return Ok(Some(chunk));
            }
            if self.finished {
                return Ok(None);
            }
            match self.events.next_event().await {
                None => {
                    self.finished = true;
                    close(&mut self.wrapper, &mut self.pending)?;
                }
                Some(Err(failure)) => {
                    self.finished = true;
                    return Err(BridgeStreamError::Upstream { failure });
                }
                Some(Ok(line)) => {
                    let parsed = self.parser.parse_line(&line, &self.limits)?;
                    if let ParsedChatLine::Chunk(chunk) = parsed {
                        push(&mut self.wrapper, &mut self.pending, chunk)?;
                    }
                }
            }
        }
    }
}

// _completed_response_as_stream: MockResponseIterator replays the completed ModelResponse as one converted chunk
pub struct MockStream {
    response: Box<ChatResponse>,
    wrapper: StreamWrapper,
    pending: VecDeque<Json>,
    filled: bool,
}

impl MockStream {
    #[must_use]
    pub fn new(
        response: Box<ChatResponse>,
        provider: &str,
        model: &str,
        stamps: StreamStamps,
    ) -> Self {
        Self {
            response,
            wrapper: wrapper(provider, model, stamps),
            pending: VecDeque::new(),
            filled: false,
        }
    }

    // the replay is finite and synchronous, so its one chunk goes through the wrapper once
    fn fill(&mut self) -> Result<(), BridgeStreamError> {
        if self.filled {
            return Ok(());
        }
        push(
            &mut self.wrapper,
            &mut self.pending,
            replay_chunk(&self.response)?,
        )?;
        close(&mut self.wrapper, &mut self.pending)?;
        self.filled = true;
        Ok(())
    }
}

// convert_model_response_to_streaming: the whole message becomes each choice's delta, and its finish reason and logprobs survive
fn replay_chunk(response: &ChatResponse) -> Result<StreamChunk, BridgeStreamError> {
    let choices = response
        .choices
        .iter()
        .map(|choice| {
            Ok(StreamChoice {
                index: choice.index,
                delta: serde_json::from_value(serde_json::to_value(&choice.message)?)?,
                finish_reason: choice.finish_reason.clone(),
                logprobs: choice
                    .logprobs
                    .as_ref()
                    .map(|logprobs| {
                        serde_json::to_value(logprobs).and_then(serde_json::from_value::<JsonValue>)
                    })
                    .transpose()?,
                extra: choice.extra.clone(),
            })
        })
        .collect::<Result<Vec<_>, BridgeStreamError>>()?;
    // the wrapper rebuilds the chunk's object and clock, so the replay carries only what it reads back
    Ok(StreamChunk {
        id: response.id.clone(),
        object: String::from("chat.completion.chunk"),
        created: 0,
        model: response.model.clone().unwrap_or_default(),
        system_fingerprint: None,
        choices,
        usage: response.usage.clone(),
        provider_specific_fields: None,
        hidden_params: JsonObject::new(),
        extra: JsonObject::new(),
    })
}

#[async_trait]
impl TypedChunkStream for MockStream {
    async fn next_chunk(&mut self) -> Result<Option<Json>, BridgeStreamError> {
        self.fill()?;
        Ok(self.pending.pop_front())
    }
}

fn wrapper(provider: &str, model: &str, stamps: StreamStamps) -> StreamWrapper {
    StreamWrapper::new(
        provider,
        StreamSetup {
            model: model.to_owned(),
            stream_options: None,
            merge_reasoning_in_choices: false,
            is_function_call: false,
            base_hidden_params: JsonObject::new(),
            repeated_chunk_limit: REPEATED_CHUNK_LIMIT,
            now: stamps.now,
            next_id: stamps.next_id,
        },
    )
}

// The wrapper reads one chunk as provider JSON, so the bridge's typed records serialize at this handoff only
fn push<T>(
    wrapper: &mut StreamWrapper,
    pending: &mut VecDeque<Json>,
    chunk: T,
) -> Result<(), BridgeStreamError>
where
    T: Serialize,
{
    let value = serde_json::to_value(chunk).and_then(serde_json::from_value)?;
    if let Some(shaped) = wrapper.push(&ProviderChunk::Json(Box::new(value)))? {
        pending.push_back(wrapper_view(&shaped)?);
    }
    Ok(())
}

// The wrapper's own end-of-stream chunk, for a stream that never sent a finish reason itself
fn close(
    wrapper: &mut StreamWrapper,
    pending: &mut VecDeque<Json>,
) -> Result<(), BridgeStreamError> {
    let Some(shaped) = wrapper.finish() else {
        return Ok(());
    };
    pending.push_back(wrapper_view(&shaped?)?);
    Ok(())
}

// _apply_post_stream_processing: the provider chat config's own stream, or the stream unchanged
pub fn post_process(
    stream: BridgeStream,
    model: &str,
    provider: &str,
    deps: Option<&PostStreamDeps<'_>>,
) -> Result<BridgeStream, PostStreamError> {
    let Some(config_deps) = deps else {
        return Ok(stream);
    };
    let Ok(provider_id) = ProviderId::from_str(provider) else {
        return Ok(stream);
    };
    let mut table = ProviderConfigTable { rows: None };
    let request = ChatConfigRequest {
        model,
        provider: provider_id,
        base_model: config_deps.base_model,
        families: config_deps.families,
        bedrock: config_deps.bedrock,
        vertex: config_deps.vertex,
        json_providers: config_deps.json_providers,
    };
    match select_chat_config(&request, &mut table) {
        Ok(Some(selected)) => config_deps
            .processing
            .post_stream_processing(selected.tag, stream),
        Ok(None) | Err(ChatConfigError::MissingJsonRecord { .. }) => Ok(stream),
    }
}
