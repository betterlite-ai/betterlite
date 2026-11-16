use super::CollectorClock;
use super::event::LogEventError;
use crate::converse_events::ChunkProjection;
use crate::converse_events::response::{ConverseChunk, ConverseDelta};
use crate::event_frames::error::EventFrameError;
use crate::invoke_events::error::InvokeStreamError;
use crate::invoke_events::{InvokeChunk, InvokeStreamDecoder};
use betterlite_models::decisions::{Extra, Json};
use betterlite_streaming::stream_chunk_shaping::extras::{
    convert_generic_chunk, generic_chunk_has_known_fields,
};
use betterlite_streaming::stream_chunk_shaping::records::{
    GenericChunk, JsonObject, JsonValue, StreamChoice, StreamChunk, StreamDelta, Usage,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum StreamLogError {
    #[error(transparent)]
    Frame(#[from] EventFrameError),
    #[error(transparent)]
    Event(#[from] LogEventError),
    #[error(transparent)]
    Invoke(#[from] InvokeStreamError),
    #[error("the passthrough log chunk could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the passthrough log chunk could not be decoded")]
    Decode(#[source] serde_json::Error),
    #[error("the stream decoder failed: {message}")]
    Decoder { message: String },
}
pub enum DecodedChunk {
    Chunk(Box<StreamChunk>),
    Generic(Json),
    Ignored,
}
pub trait EventChunkDecoder: Send {
    fn decode(
        &mut self,
        chunk: &Json,
        clock: CollectorClock,
    ) -> Result<DecodedChunk, StreamLogError>;
}
fn convert<T: Serialize, U: DeserializeOwned>(value: &T) -> Result<U, StreamLogError> {
    serde_json::from_str(&serde_json::to_string(value).map_err(StreamLogError::Encode)?)
        .map_err(StreamLogError::Decode)
}
fn extra_object(extra: &Extra) -> Result<Option<JsonObject>, StreamLogError> {
    if extra.is_empty() {
        return Ok(None);
    }
    convert(extra).map(Some)
}
impl EventChunkDecoder for InvokeStreamDecoder {
    fn decode(
        &mut self,
        chunk: &Json,
        clock: CollectorClock,
    ) -> Result<DecodedChunk, StreamLogError> {
        match self.chunk_parser(chunk)? {
            InvokeChunk::Base(projection) => match *projection {
                ChunkProjection::Converse(converse) => Ok(DecodedChunk::Chunk(Box::new(
                    converse_stream_chunk(converse, clock)?,
                ))),
                ChunkProjection::Generic(generic) => Ok(DecodedChunk::Generic(convert(&generic)?)),
            },
            InvokeChunk::Anthropic(event) => Ok(DecodedChunk::Chunk(convert(&event)?)),
            InvokeChunk::DeepSeek(reply) | InvokeChunk::OpenAiCompatible(reply) => {
                Ok(DecodedChunk::Chunk(convert(&reply)?))
            }
        }
    }
}
fn converse_stream_chunk(
    chunk: ConverseChunk,
    clock: CollectorClock,
) -> Result<StreamChunk, StreamLogError> {
    let delta = converse_delta(chunk.delta)?;
    let choice = StreamChoice {
        index: chunk.index,
        delta,
        finish_reason: chunk.finish_reason,
        logprobs: None,
        extra: JsonObject::new(),
    };
    let usage: Option<Usage> = chunk.usage.as_ref().map(convert).transpose()?;
    Ok(StreamChunk {
        id: chunk.id,
        object: "chat.completion.chunk".to_owned(),
        created: (clock.instant)().seconds,
        model: chunk.model,
        system_fingerprint: None,
        choices: vec![choice],
        usage,
        provider_specific_fields: extra_object(&chunk.provider_specific_fields)?,
        hidden_params: JsonObject::new(),
        extra: JsonObject::new(),
    })
}
fn converse_delta(chunk: ConverseDelta) -> Result<StreamDelta, StreamLogError> {
    let ConverseDelta {
        content,
        role,
        tool_calls,
        provider_specific_fields,
        thinking_blocks,
        reasoning_content,
    } = chunk;
    Ok(StreamDelta {
        content,
        role,
        function_call: None,
        tool_calls: tool_calls
            .as_ref()
            .map(convert::<_, JsonValue>)
            .transpose()?,
        audio: None,
        images: None,
        annotations: None,
        reasoning_content,
        thinking_blocks: thinking_blocks
            .as_ref()
            .map(convert::<_, JsonValue>)
            .transpose()?,
        provider_specific_fields: provider_specific_fields
            .as_ref()
            .map(extra_object)
            .transpose()?
            .flatten(),
        extra: JsonObject::new(),
    })
}
pub fn generic_chunk(
    value: &Json,
    clock: CollectorClock,
) -> Result<Option<StreamChunk>, StreamLogError> {
    let object: JsonObject = convert(value)?;
    if !generic_chunk_has_known_fields(&object) {
        return Ok(None);
    }
    let generic: GenericChunk = convert(value)?;
    Ok(Some(convert_generic_chunk(
        &generic,
        clock.instant,
        clock.next_id,
    )))
}
