use std::collections::VecDeque;
use std::fmt::Display;
use std::str::FromStr as _;

use async_trait::async_trait;
use betterlite_context::chat_config_selection::selection::{
    ChatConfigError, ChatConfigRequest, select_chat_config,
};
use betterlite_context::chat_config_selection::table::ProviderConfigTable;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::compatible_wire::{NEW_ID, system_clock};
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::provider_chunk_decode::wrapper::{
    StreamSetup, StreamWrapper, WrapperError,
};
use betterlite_streaming::stream_chunk_shaping::creation::system_instant;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChunk};

use super::error::PostStreamError;
use super::port::{PostStreamDeps, ResponsesEventStream};
use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream::chunk::wrapper_view;
use crate::api::chat_response_stream::source::AsyncChunkStream;
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::responses_to_chat_stream::stream::{
    ParsedChatLine, ResponsesChatStream, get_model_response_iterator,
};
use crate::message_utilities::response_streams::StreamError;
use crate::message_utilities::response_streams::iterators::{DeltaIdentity, ModelResponseIterator};

// LiteLLM's REPEATED_STREAMING_CHUNK_LIMIT default, which CustomStreamWrapper counts against
const REPEATED_CHUNK_LIMIT: usize = 100;

// What get_model_response_iterator and CustomStreamWrapper are built with
pub struct StreamConfig<'config> {
    pub provider: &'config str,
    pub model: &'config str,
    pub json_mode: Option<bool>,
    pub limits: ApiLimits,
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
            parser: get_model_response_iterator(config.json_mode, NEW_ID, system_clock),
            wrapper: wrapper(config.provider, config.model),
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
                        .map_err(unclassified)?;
                    if let ParsedChatLine::Chunk(chunk) = parsed {
                        let value = serde_json::to_value(chunk).map_err(unclassified)?;
                        let row = serde_json::from_value::<Json>(value).map_err(unclassified)?;
                        push_json(&mut self.wrapper, &mut self.pending, row)?;
                    }
                }
            }
        }
    }
}

// _completed_response_as_stream: B010's MockResponseIterator replaying the ModelResponse as one delta
pub struct MockStream {
    inner: ModelResponseIterator,
    wrapper: StreamWrapper,
    pending: VecDeque<Json>,
    filled: bool,
}

impl MockStream {
    pub fn new(response: Json, provider: &str, model: &str) -> Result<Self, StreamError> {
        let id = text_field(&response, "id");
        let created = number_field(&response, "created");
        let identity = DeltaIdentity { id: &id, created };
        Ok(Self {
            inner: ModelResponseIterator::new(response, Some(identity))?,
            wrapper: wrapper(provider, model),
            pending: VecDeque::new(),
            filled: false,
        })
    }

    // the replay is finite and synchronous, so every chunk goes through the wrapper once
    fn fill(&mut self) -> Result<(), StreamFailure> {
        if self.filled {
            return Ok(());
        }
        while let Some(chunk) = self.inner.next() {
            push_json(&mut self.wrapper, &mut self.pending, chunk)?;
        }
        close(&mut self.wrapper, &mut self.pending)?;
        self.filled = true;
        Ok(())
    }
}

#[async_trait]
impl AsyncChunkStream for MockStream {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
        self.fill()?;
        Ok(self.pending.pop_front())
    }
}

fn wrapper(provider: &str, model: &str) -> StreamWrapper {
    StreamWrapper::new(
        provider,
        StreamSetup {
            model: model.to_owned(),
            stream_options: None,
            merge_reasoning_in_choices: false,
            is_function_call: false,
            base_hidden_params: JsonObject::new(),
            repeated_chunk_limit: REPEATED_CHUNK_LIMIT,
            now: system_instant,
            next_id: NEW_ID,
        },
    )
}

fn field<'json>(response: &'json Json, name: &str) -> Option<&'json Json> {
    match response {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

// B010 names the replayed chunk after the ModelResponse it came from
fn text_field(response: &Json, name: &str) -> String {
    let Some(Json::Text(text)) = field(response, name) else {
        return String::new();
    };
    text.clone()
}

fn number_field(response: &Json, name: &str) -> i64 {
    let Some(Json::Number(number)) = field(response, name) else {
        return 0;
    };
    number.as_i64().unwrap_or_default()
}

fn push_json(
    wrapper: &mut StreamWrapper,
    pending: &mut VecDeque<Json>,
    chunk: Json,
) -> Result<(), StreamFailure> {
    let value = serde_json::to_value(chunk).map_err(unclassified)?;
    let json = serde_json::from_value::<JsonValue>(value).map_err(unclassified)?;
    hold(pending, wrapper.push(&ProviderChunk::Json(Box::new(json))))
}

fn hold(
    pending: &mut VecDeque<Json>,
    shaped: Result<Option<StreamChunk>, WrapperError>,
) -> Result<(), StreamFailure> {
    if let Some(chunk) = shaped.map_err(unclassified)? {
        pending.push_back(wrapper_view(&chunk).map_err(unclassified)?);
    }
    Ok(())
}

// The wrapper's own end-of-stream chunk, for a stream that never sent a finish reason itself
fn close(wrapper: &mut StreamWrapper, pending: &mut VecDeque<Json>) -> Result<(), StreamFailure> {
    if let Some(shaped) = wrapper.finish() {
        let chunk = shaped.map_err(unclassified)?;
        pending.push_back(wrapper_view(&chunk).map_err(unclassified)?);
    }
    Ok(())
}

fn unclassified<Message>(error: Message) -> StreamFailure
where
    Message: Display,
{
    StreamFailure::Unclassified {
        text: error.to_string(),
    }
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
