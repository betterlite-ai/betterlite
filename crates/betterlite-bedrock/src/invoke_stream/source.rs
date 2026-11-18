use async_trait::async_trait;
use betterlite_error::error::details::ApiDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Extra;
use betterlite_sdk::chat_stream::ports::{ChunkSource, SourceFailure, StreamSourceError};
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, StreamChoice, StreamChunk, StreamDelta,
};
use futures_util::StreamExt as _;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::bedrock_http_stream::stream::{ChunkStream, StreamFailure};
use crate::converse_events::ChunkProjection;
use crate::converse_events::response::{ConverseChunk, ConverseDelta};
use crate::invoke_events::InvokeChunk;

// B691's chunks read through a mutex: its boxed body stream is Send but not Sync.
pub struct InvokeChunks<'deps> {
    pub(super) chunks: Mutex<ChunkStream<'deps>>,
    pub(super) now: fn() -> i64,
}

#[async_trait]
impl ChunkSource for InvokeChunks<'_> {
    async fn next_chunk(&mut self) -> Result<Option<ProviderChunk>, StreamSourceError> {
        let mut chunks = self.chunks.lock().await;
        let chunk = match chunks.next().await {
            None => return Ok(None),
            Some(Ok(chunk)) => chunk,
            Some(Err(failure)) => return Err(source_failure(&failure)),
        };
        let projected = match chunk {
            InvokeChunk::Base(projection) => match *projection {
                ChunkProjection::Generic(generic) => ProviderChunk::Generic(Box::new(generic)),
                ChunkProjection::Converse(converse) => {
                    ProviderChunk::Json(Box::new(convert(&converse_stream(converse, self.now)?)?))
                }
            },
            InvokeChunk::Anthropic(event) => {
                ProviderChunk::Json(Box::new(convert(event.as_ref())?))
            }
            InvokeChunk::DeepSeek(reply) | InvokeChunk::OpenAiCompatible(reply) => {
                ProviderChunk::Json(Box::new(convert(reply.as_ref())?))
            }
        };
        Ok(Some(projected))
    }

    async fn close(&mut self) -> Result<(), StreamSourceError> {
        Ok(())
    }
}

fn converse_stream(
    chunk: ConverseChunk,
    now: fn() -> i64,
) -> Result<StreamChunk, StreamSourceError> {
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
        usage: usage.map(|usage| convert(&usage)).transpose()?,
        provider_specific_fields: extra_object(Some(&provider_specific_fields))?,
        hidden_params: JsonObject::new(),
        extra: JsonObject::new(),
    })
}

// The delta ModelResponseStream carries, with an empty provider-specific map kept out of the chunk.
fn converse_delta(delta: ConverseDelta) -> Result<StreamDelta, StreamSourceError> {
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

fn extra_object(value: Option<&Extra>) -> Result<Option<JsonObject>, StreamSourceError> {
    let Some(extra) = value.filter(|extra| !extra.is_empty()) else {
        return Ok(None);
    };
    convert(extra).map(Some)
}

fn convert<T, U>(value: &T) -> Result<U, StreamSourceError>
where
    T: Serialize,
    U: DeserializeOwned,
{
    serde_json::from_value(serde_json::to_value(value).map_err(encode_failure)?)
        .map_err(encode_failure)
}

fn encode_failure(error: serde_json::Error) -> StreamSourceError {
    let message = error.to_string();
    StreamSourceError {
        failure: SourceFailure::Other,
        class: ErrorClass::InternalServer(Box::new(context(message.clone()))),
        message,
    }
}

// httpx raises ReadError for a body that fails; a timeout is litellm.Timeout, everything else a connection error.
fn source_failure(failure: &StreamFailure) -> StreamSourceError {
    match failure {
        StreamFailure::Read(error) => {
            let message = error.to_string();
            let failure = if matches!(error, HttpError::Timeout) {
                SourceFailure::Timeout
            } else {
                SourceFailure::Read
            };
            StreamSourceError {
                failure,
                class: ErrorClass::ApiConnection(Box::new(context(message.clone()))),
                message,
            }
        }
        // A provider error event keeps its status; a chunk that cannot be parsed is the connection failing.
        StreamFailure::Decode(error) => {
            let message = error.to_string();
            let class = match error.status() {
                Some(status) => ErrorClass::Api(Box::new(ApiDetails {
                    context: context(message.clone()),
                    status,
                })),
                None => ErrorClass::ApiConnection(Box::new(context(message.clone()))),
            };
            StreamSourceError {
                failure: SourceFailure::Other,
                class,
                message,
            }
        }
    }
}

fn context(message: String) -> ErrorContext {
    ErrorContext {
        message,
        model: None,
        provider: Some(super::CUSTOM_LLM_PROVIDER.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}
