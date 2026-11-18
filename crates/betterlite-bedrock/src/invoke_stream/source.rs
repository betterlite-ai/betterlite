use async_trait::async_trait;
use betterlite_anthropic::stream_events::error::StreamEventError;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Extra;
use betterlite_openai::chat_choices::stream::StreamError;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::chat_stream::ports::{ChunkSource, SourceFailure, StreamSourceError};
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChoice, StreamChunk, StreamDelta,
};
use futures_util::StreamExt as _;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::bedrock_http_stream::invoke::TIMEOUT_MESSAGE;
use crate::bedrock_http_stream::stream::{ChunkStream, StreamFailure};
use crate::converse_events::ChunkProjection;
use crate::converse_events::response::{ConverseChunk, ConverseDelta};
use crate::event_frames::error::EventFrameError;
use crate::invoke_events::InvokeChunk;
use crate::invoke_events::error::InvokeStreamError;
use crate::invoke_stream::CUSTOM_LLM_PROVIDER;

// This module's failures, each holding the typed cause until the shared wrapper's message-only boundary.
#[derive(Debug, Error)]
enum InvokeSourceError {
    #[error("the provider chunk could not be converted from this record")]
    Json(#[source] serde_json::Error),
    #[error("{0}")]
    Read(#[source] HttpError),
    #[error("{0}")]
    Decode(#[source] InvokeStreamError),
}

impl From<StreamFailure> for InvokeSourceError {
    fn from(failure: StreamFailure) -> Self {
        match failure {
            StreamFailure::Read(source) => Self::Read(source),
            StreamFailure::Decode(source) => Self::Decode(source),
        }
    }
}

// B691's chunks read through a mutex: its boxed body stream is Send but not Sync.
pub struct InvokeChunks<'deps> {
    pub(super) chunks: Mutex<ChunkStream<'deps>>,
    pub(super) model: String,
    pub(super) redactor: &'deps Redactor,
    pub(super) now: fn() -> i64,
}

#[async_trait]
impl ChunkSource for InvokeChunks<'_> {
    async fn next_chunk(&mut self) -> Result<Option<ProviderChunk>, StreamSourceError> {
        let outcome = self.next_provider_chunk().await;
        outcome.map_err(|error| self.source_error(&error))
    }

    async fn close(&mut self) -> Result<(), StreamSourceError> {
        Ok(())
    }
}

impl InvokeChunks<'_> {
    // &mut self is the only access this source has, so no guard can cross the await.
    async fn next_provider_chunk(&mut self) -> Result<Option<ProviderChunk>, InvokeSourceError> {
        let chunk = match self.chunks.get_mut().next().await {
            None => return Ok(None),
            Some(Ok(chunk)) => chunk,
            Some(Err(failure)) => return Err(InvokeSourceError::from(failure)),
        };
        let projected = match chunk {
            InvokeChunk::Base(projection) => match *projection {
                ChunkProjection::Generic(generic) => ProviderChunk::Generic(Box::new(generic)),
                ChunkProjection::Converse(converse) => {
                    let stream = converse_stream(converse, self.now)?;
                    ProviderChunk::Json(Box::new(encoded(&stream)?))
                }
            },
            InvokeChunk::Anthropic(event) => {
                ProviderChunk::Json(Box::new(encoded(event.as_ref())?))
            }
            InvokeChunk::DeepSeek(reply) | InvokeChunk::OpenAiCompatible(reply) => {
                ProviderChunk::Json(Box::new(encoded(reply.as_ref())?))
            }
        };
        Ok(Some(projected))
    }

    // CustomStreamWrapper raises exception_type()'s answer over the wrapper's own provider.
    fn source_error(&self, error: &InvokeSourceError) -> StreamSourceError {
        let read_timeout =
            matches!(error, InvokeSourceError::Read(source) if read_timed_out(source));
        let (error_str, status_code) = match (read_timeout, provider_failure(error)) {
            // B691's read_failure: a timed-out read is litellm.Timeout with status 408.
            (true, _) => (TIMEOUT_MESSAGE.to_owned(), Some(408)),
            (false, Some((message, status))) => (message.to_owned(), Some(status)),
            (false, None) => (error.to_string(), None),
        };
        let failure = match error {
            InvokeSourceError::Read(_) if read_timeout => SourceFailure::Timeout,
            InvokeSourceError::Read(_) => SourceFailure::Read,
            InvokeSourceError::Json(_) | InvokeSourceError::Decode(_) => SourceFailure::Other,
        };
        StreamSourceError {
            failure,
            class: self.classified(&error_str, status_code),
            message: self.redactor.redact_string(&error_str),
        }
    }

    fn classified(&self, error_str: &str, status_code: Option<u16>) -> ErrorClass {
        let raw = RawException {
            model: self.model.clone(),
            custom_llm_provider: CUSTOM_LLM_PROVIDER.to_owned(),
            error_str: error_str.to_owned(),
            status_code,
            status_code_is_synthesized: false,
            message: Some(error_str.to_owned()),
            body: None,
            exception_type: String::new(),
        };
        dispatch(&DispatchInput::Raw(Box::new(raw)), self.redactor)
    }
}

// A chunk with no model keeps provider_response_model out of the hidden params, as an absent field does.
fn encoded<T>(value: &T) -> Result<JsonValue, InvokeSourceError>
where
    T: Serialize,
{
    let mut json: JsonValue = convert(value)?;
    if let JsonValue::Object(fields) = &mut json
        && fields
            .get("model")
            .is_none_or(|model| matches!(model, JsonValue::Null))
    {
        fields.insert("model".to_owned(), JsonValue::String(String::new()));
    }
    Ok(json)
}

// The provider error a decode failure carries: the message and status exception_type() reads off it.
fn provider_failure(source_error: &InvokeSourceError) -> Option<(&str, u16)> {
    let InvokeSourceError::Decode(source) = source_error else {
        return None;
    };
    match source {
        InvokeStreamError::Frame(EventFrameError::Bedrock { status, error }) => {
            Some((error.error.message.as_str(), *status))
        }
        InvokeStreamError::Anthropic(StreamEventError::Anthropic(error)) => {
            Some((error.message.as_str(), error.status_code))
        }
        InvokeStreamError::OpenAi(StreamError::Provider { message, status }) => {
            Some((message.as_str(), *status))
        }
        InvokeStreamError::Frame(_)
        | InvokeStreamError::Json { .. }
        | InvokeStreamError::Chunk { .. }
        | InvokeStreamError::Encode { .. }
        | InvokeStreamError::Anthropic(_)
        | InvokeStreamError::OpenAi(_)
        | InvokeStreamError::Converse(_)
        | InvokeStreamError::Buffered { .. } => None,
    }
}

fn read_timed_out(error: &HttpError) -> bool {
    matches!(error, HttpError::Timeout)
        || matches!(error, HttpError::Request(source) if source.is_timeout())
}

fn converse_stream(
    chunk: ConverseChunk,
    now: fn() -> i64,
) -> Result<StreamChunk, InvokeSourceError> {
    let ConverseChunk {
        id,
        model,
        finish_reason,
        index,
        delta,
        usage,
        provider_specific_fields,
    } = chunk;
    Ok(StreamChunk {
        id,
        object: "chat.completion.chunk".to_owned(),
        created: now(),
        model,
        system_fingerprint: None,
        choices: vec![StreamChoice {
            index,
            delta: converse_delta(delta)?,
            finish_reason,
            logprobs: None,
            extra: JsonObject::new(),
        }],
        usage: usage.map(|usage_value| convert(&usage_value)).transpose()?,
        provider_specific_fields: extra_object(Some(&provider_specific_fields))?,
        hidden_params: JsonObject::new(),
        extra: JsonObject::new(),
    })
}

// The delta ModelResponseStream carries, with an empty provider-specific map kept out of the chunk.
fn converse_delta(delta: ConverseDelta) -> Result<StreamDelta, InvokeSourceError> {
    let ConverseDelta {
        content,
        role,
        tool_calls,
        provider_specific_fields,
        thinking_blocks,
        reasoning_content,
    } = delta;
    Ok(StreamDelta {
        content,
        role,
        function_call: None,
        tool_calls: tool_calls.map(|calls| convert(&calls)).transpose()?,
        audio: None,
        images: None,
        annotations: None,
        reasoning_content,
        thinking_blocks: thinking_blocks.map(|blocks| convert(&blocks)).transpose()?,
        provider_specific_fields: extra_object(provider_specific_fields.as_ref())?,
        extra: JsonObject::new(),
    })
}

fn extra_object(value: Option<&Extra>) -> Result<Option<JsonObject>, InvokeSourceError> {
    let Some(extra) = value.filter(|extra| !extra.is_empty()) else {
        return Ok(None);
    };
    convert(extra).map(Some)
}

fn convert<T, U>(value: &T) -> Result<U, InvokeSourceError>
where
    T: Serialize,
    U: DeserializeOwned,
{
    serde_json::from_value(serde_json::to_value(value).map_err(InvokeSourceError::Json)?)
        .map_err(InvokeSourceError::Json)
}
