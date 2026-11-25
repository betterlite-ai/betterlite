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
use betterlite_streaming::provider_chunk_decode::wrapper::{
    StreamSetup, StreamWrapper, WrapperError,
};
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChunk};
use indexmap::IndexMap;

use super::error::{PostStreamError, StreamStepError};
use super::input::StreamStamps;
use super::port::{PostStreamDeps, ResponsesEventStream};
use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream::chunk::wrapper_view;
use crate::api::chat_response_stream::source::AsyncChunkStream;
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::response_chat_result::records::ModelResponse;
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
impl AsyncChunkStream for TranslatedStream {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
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
                    return Err(failure);
                }
                Some(Ok(line)) => {
                    let parsed = self
                        .parser
                        .parse_line(&line, &self.limits)
                        .map_err(StreamStepError::from)?;
                    if let ParsedChatLine::Chunk(chunk) = parsed {
                        let value = serde_json::to_value(chunk).map_err(StreamStepError::from)?;
                        let row =
                            serde_json::from_value::<Json>(value).map_err(StreamStepError::from)?;
                        push_json(&mut self.wrapper, &mut self.pending, row)?;
                    }
                }
            }
        }
    }
}

// _completed_response_as_stream: MockResponseIterator replays the completed ModelResponse as one converted chunk
pub struct MockStream {
    response: Box<ModelResponse>,
    wrapper: StreamWrapper,
    pending: VecDeque<Json>,
    filled: bool,
}

impl MockStream {
    #[must_use]
    pub fn new(
        response: Box<ModelResponse>,
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
    fn fill(&mut self) -> Result<(), StreamFailure> {
        if self.filled {
            return Ok(());
        }
        let chunk = replay_chunk(&self.response).map_err(StreamStepError::from)?;
        push_json(&mut self.wrapper, &mut self.pending, chunk)?;
        close(&mut self.wrapper, &mut self.pending)?;
        self.filled = true;
        Ok(())
    }
}

// convert_model_response_to_streaming: the whole message becomes each choice's delta, and its finish reason survives
fn replay_chunk(response: &ModelResponse) -> Result<Json, serde_json::Error> {
    let mut choices = Vec::with_capacity(response.choices.len());
    for choice in &response.choices {
        let delta = serde_json::from_value(serde_json::to_value(&choice.message)?)?;
        choices.push(Json::Object(IndexMap::from([
            (
                String::from("index"),
                Json::Number(serde_json::Number::from(choice.index)),
            ),
            (String::from("delta"), delta),
            (
                String::from("finish_reason"),
                Json::Text(choice.finish_reason.clone()),
            ),
        ])));
    }
    let model = response.model.clone().map_or(Json::Null, Json::Text);
    let mut chunk = IndexMap::from([
        (String::from("id"), Json::Text(response.id.clone())),
        (
            String::from("object"),
            Json::Text(String::from("chat.completion.chunk")),
        ),
        (
            String::from("created"),
            Json::Number(serde_json::Number::from(response.created)),
        ),
        (String::from("model"), model),
        (String::from("choices"), Json::List(choices)),
    ]);
    if let Some(usage) = &response.usage {
        chunk.insert(
            String::from("usage"),
            serde_json::from_value(usage.clone())?,
        );
    }
    Ok(Json::Object(chunk))
}

#[async_trait]
impl AsyncChunkStream for MockStream {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
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

fn push_json(
    wrapper: &mut StreamWrapper,
    pending: &mut VecDeque<Json>,
    chunk: Json,
) -> Result<(), StreamStepError> {
    let value = serde_json::to_value(chunk)?;
    let json = serde_json::from_value::<JsonValue>(value)?;
    hold(pending, wrapper.push(&ProviderChunk::Json(Box::new(json))))
}

fn hold(
    pending: &mut VecDeque<Json>,
    shaped: Result<Option<StreamChunk>, WrapperError>,
) -> Result<(), StreamStepError> {
    if let Some(chunk) = shaped? {
        pending.push_back(wrapper_view(&chunk)?);
    }
    Ok(())
}

// The wrapper's own end-of-stream chunk, for a stream that never sent a finish reason itself
fn close(wrapper: &mut StreamWrapper, pending: &mut VecDeque<Json>) -> Result<(), StreamStepError> {
    if let Some(shaped) = wrapper.finish() {
        pending.push_back(wrapper_view(&shaped?)?);
    }
    Ok(())
}

// _apply_post_stream_processing: the provider chat config's own stream, or the stream unchanged
pub fn post_process(
    stream: Box<dyn AsyncChunkStream + Send>,
    model: &str,
    provider: &str,
    deps: Option<&PostStreamDeps<'_>>,
) -> Result<Box<dyn AsyncChunkStream + Send>, PostStreamError> {
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
