use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::local_ttl_cache::CacheValue;
use thiserror::Error;

use crate::types::call_type::CachingSupportedCallType;

mod tests;

#[path = "replay/tests/conversion.rs"]
mod conversion;
#[path = "replay/tests/deferral.rs"]
mod deferral;
#[path = "replay/tests/metadata.rs"]
mod metadata;
#[path = "replay/tests/responses_bridge.rs"]
mod responses_bridge;
#[path = "replay/tests/shared.rs"]
mod shared;
#[path = "replay/tests/streaming.rs"]
mod streaming;

pub const CACHED_STREAM_PROVIDER: &str = "cached_response";
// LiteLLM stamps this model on a replayed transcription even when another one was requested
pub const TRANSCRIPTION_CACHE_MODEL: &str = "whisper-1";
pub type CachedDict = BTreeMap<String, CacheValue>;

pub struct ReplayRequest<'request> {
    pub stream: bool,
    pub converted_stream_requested: bool,
    pub agentic_loop_depth: u32,
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub elapsed: Duration,
}

pub struct TranscriptionHidden<'provider> {
    pub model: &'static str,
    pub custom_llm_provider: Option<&'provider str>,
    pub cache_hit: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseKind {
    Chat,
    Text,
    Embedding,
    Rerank,
}

pub trait CacheHitMetadata {
    fn set_cache_hit(&mut self);
}

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("the cached result does not match the {expected} response shape")]
    Shape { expected: &'static str },
    #[error("the cached result could not be decoded: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

pub trait ReplayConversions {
    type Response: CacheHitMetadata;

    fn model_response(
        &self,
        cached: &CachedDict,
        kind: ResponseKind,
    ) -> Result<Self::Response, ReplayError>;

    fn transcription_response(
        &self,
        cached: &CachedDict,
        hidden: &TranscriptionHidden<'_>,
    ) -> Result<Self::Response, ReplayError>;

    fn responses_response(&self, cached: &CachedDict) -> Result<Self::Response, ReplayError>;

    fn responses_stream(
        &self,
        response: Self::Response,
        call_type: CachingSupportedCallType,
    ) -> Result<Self::Response, ReplayError>;

    fn anthropic_messages_stream(
        &self,
        cached: &CachedDict,
    ) -> Result<Option<Self::Response>, ReplayError>;

    fn stream_wrapper(
        &self,
        cached: &CachedDict,
        asynchronous: bool,
        model: &str,
        custom_llm_provider: &str,
    ) -> Result<Self::Response, ReplayError>;

    fn update_response_metadata(
        &self,
        result: &mut ReplayedBy<Self>,
        model: &str,
        elapsed: Duration,
    );
}

pub enum Replayed<Response> {
    Response(ResponseKind, Response),
    Transcription(Response),
    Responses(Response),
    Stream(Response),
    ResponsesStream(Response),
    MessagesStream(Response),
    Unchanged(CacheValue),
}

pub type ReplayedBy<C> = Replayed<<C as ReplayConversions>::Response>;

pub enum CacheHit<C: ReplayConversions> {
    Stored(CacheValue),
    Replayed(ReplayedBy<C>),
}

impl<Response> Replayed<Response>
where
    Response: CacheHitMetadata,
{
    #[must_use]
    pub const fn defers_success_callbacks(&self) -> bool {
        match self {
            Self::Stream(_) | Self::ResponsesStream(_) | Self::MessagesStream(_) => true,
            Self::Response(..)
            | Self::Transcription(_)
            | Self::Responses(_)
            | Self::Unchanged(_) => false,
        }
    }

    pub fn stamp_cache_hit(&mut self) {
        match self {
            Self::Response(_, response)
            | Self::Transcription(response)
            | Self::Responses(response)
            | Self::Stream(response)
            | Self::ResponsesStream(response)
            | Self::MessagesStream(response) => response.set_cache_hit(),
            Self::Unchanged(_) => {}
        }
    }
}

#[must_use]
pub fn is_chat_completion_cached_dict(cached: &CachedDict) -> bool {
    if let Some(CacheValue::Text(id)) = cached.get("id")
        && id.starts_with("chatcmpl")
    {
        return true;
    }
    if let Some(CacheValue::Text(object)) = cached.get("object") {
        return object.starts_with("chat.completion");
    }
    cached.contains_key("choices")
}

#[must_use]
pub const fn stream_replay_requested(request: &ReplayRequest<'_>) -> bool {
    if request.stream {
        return true;
    }
    request.converted_stream_requested && request.agentic_loop_depth == 0
}

#[must_use]
pub const fn streams_asynchronously(call_type: CachingSupportedCallType) -> bool {
    // the Responses bridge hands aresponses to the completion converter, so it replays async
    matches!(
        call_type,
        CachingSupportedCallType::Acompletion
            | CachingSupportedCallType::AtextCompletion
            | CachingSupportedCallType::Aresponses
    )
}

pub fn replay_cached_result<C>(
    cached_result: CacheHit<C>,
    call_type: CachingSupportedCallType,
    request: &ReplayRequest<'_>,
    conversions: &C,
) -> Result<ReplayedBy<C>, ReplayError>
where
    C: ReplayConversions,
{
    let value = match cached_result {
        CacheHit::Stored(value) => value,
        // a replayed result was wrapped and stamped once already, so it never converts again
        CacheHit::Replayed(replayed) => return Ok(replayed),
    };
    let mut outcome = if let CacheValue::Object(dict) = value {
        convert_dict(dict, call_type, request, conversions)?
    } else {
        Replayed::Unchanged(value)
    };
    outcome.stamp_cache_hit();
    conversions.update_response_metadata(&mut outcome, request.model, request.elapsed);
    Ok(outcome)
}

fn convert_dict<C>(
    cached: CachedDict,
    call_type: CachingSupportedCallType,
    request: &ReplayRequest<'_>,
    conversions: &C,
) -> Result<ReplayedBy<C>, ReplayError>
where
    C: ReplayConversions,
{
    match call_type {
        CachingSupportedCallType::Completion
        | CachingSupportedCallType::Acompletion
        | CachingSupportedCallType::TextCompletion
        | CachingSupportedCallType::AtextCompletion
        | CachingSupportedCallType::Responses
        | CachingSupportedCallType::Aresponses => {
            convert_chat_like(&cached, call_type, request, conversions)
        }
        CachingSupportedCallType::Embedding | CachingSupportedCallType::Aembedding => {
            let response = conversions.model_response(&cached, ResponseKind::Embedding)?;
            Ok(Replayed::Response(ResponseKind::Embedding, response))
        }
        CachingSupportedCallType::Rerank | CachingSupportedCallType::Arerank => {
            let response = conversions.model_response(&cached, ResponseKind::Rerank)?;
            Ok(Replayed::Response(ResponseKind::Rerank, response))
        }
        CachingSupportedCallType::Transcription | CachingSupportedCallType::Atranscription => {
            let hidden = TranscriptionHidden {
                model: TRANSCRIPTION_CACHE_MODEL,
                custom_llm_provider: request.custom_llm_provider,
                cache_hit: true,
            };
            let response = conversions.transcription_response(&cached, &hidden)?;
            Ok(Replayed::Transcription(response))
        }
        CachingSupportedCallType::AnthropicMessages
        | CachingSupportedCallType::AanthropicMessages => {
            let stream = conversions.anthropic_messages_stream(&cached)?;
            Ok(stream.map_or_else(
                || Replayed::Unchanged(CacheValue::Object(cached)),
                Replayed::MessagesStream,
            ))
        }
    }
}

fn convert_chat_like<C>(
    cached: &CachedDict,
    call_type: CachingSupportedCallType,
    request: &ReplayRequest<'_>,
    conversions: &C,
) -> Result<ReplayedBy<C>, ReplayError>
where
    C: ReplayConversions,
{
    let bridge = matches!(
        call_type,
        CachingSupportedCallType::Responses | CachingSupportedCallType::Aresponses
    );
    if bridge && !is_chat_completion_cached_dict(cached) {
        let mut response = conversions.responses_response(cached)?;
        response.set_cache_hit();
        if stream_replay_requested(request) {
            let stream = conversions.responses_stream(response, call_type)?;
            return Ok(Replayed::ResponsesStream(stream));
        }
        return Ok(Replayed::Responses(response));
    }
    if stream_replay_requested(request) {
        let asynchronous = streams_asynchronously(call_type);
        let stream = conversions.stream_wrapper(
            cached,
            asynchronous,
            request.model,
            CACHED_STREAM_PROVIDER,
        )?;
        return Ok(Replayed::Stream(stream));
    }
    let kind = if matches!(
        call_type,
        CachingSupportedCallType::TextCompletion | CachingSupportedCallType::AtextCompletion
    ) {
        ResponseKind::Text
    } else {
        ResponseKind::Chat
    };
    let response = conversions.model_response(cached, kind)?;
    Ok(Replayed::Response(kind, response))
}
