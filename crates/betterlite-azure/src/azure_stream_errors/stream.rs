use std::collections::{BTreeMap, VecDeque};
use std::pin::Pin;
use std::str;

use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, HeaderSource, process_response_headers,
};
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::stream::SseParser;
use betterlite_openai::text_completion_transport::logging::StreamDefaults;
use betterlite_openai::text_completion_transport::sdk::{ByteStream, SdkError};
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::provider_chunk_decode::wrapper::{StreamSetup, StreamWrapper};
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChunk, StreamOptions,
};
use futures_util::{Stream, StreamExt as _, stream};
use http::HeaderMap;
use indexmap::IndexMap;

use super::error::{AzureStreamFailure, Raised, StreamingError};
use super::records::{StreamCall, StreamRuntime};

pub const AZURE_PROVIDER: &str = "azure";
// LiteLLM's REPEATED_STREAMING_CHUNK_LIMIT default
pub const REPEATED_STREAMING_CHUNK_LIMIT: usize = 100;

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<StreamChunk, StreamingError>> + Send>>;

// What CustomStreamWrapper was given as _response_headers: processed on the sync path, raw on the async one
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrapperHeaders {
    Processed(BTreeMap<String, String>),
    Raw(IndexMap<String, String>),
}

// The wrapper azure.py returns, with the state a caller can read before it is iterated
pub struct AzureChatStream {
    pub model: String,
    pub custom_llm_provider: &'static str,
    pub stream_options: Option<StreamOptions>,
    pub response_headers: WrapperHeaders,
    pub additional_headers: AdditionalHeaders,
    pub chunks: ChatStream,
}

// httpx decodes header bytes as ascii, then utf-8, then latin-1
fn header_text(value: &[u8]) -> String {
    str::from_utf8(value).map_or_else(
        |_| value.iter().map(|byte| char::from(*byte)).collect(),
        str::to_owned,
    )
}

// headers = dict(raw_response.headers): names lowercased, a repeated name's values joined by ", "
#[must_use]
pub fn raw_headers(headers: &HeaderMap) -> IndexMap<String, String> {
    let mut joined: IndexMap<String, String> = IndexMap::new();
    for (name, value) in headers {
        let text = header_text(value.as_bytes());
        match joined.get_mut(name.as_str()) {
            Some(existing) => {
                existing.push_str(", ");
                existing.push_str(&text);
            }
            None => {
                joined.insert(name.as_str().to_owned(), text);
            }
        }
    }
    joined
}

// CustomStreamWrapper(stream_options=data.get("stream_options") or logging_obj.stream_options)
#[must_use]
pub fn requested_stream_options(
    data: &Extra,
    defaults: Option<StreamOptions>,
) -> Option<StreamOptions> {
    match data.get("stream_options") {
        Some(Json::Object(fields)) if !fields.is_empty() => Some(StreamOptions {
            include_usage: matches!(fields.get("include_usage"), Some(Json::Bool(true))),
        }),
        Some(
            Json::Object(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_),
        )
        | None => defaults,
    }
}

fn shaped_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::String(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(shaped_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), shaped_value(field)))
                .collect(),
        ),
    }
}

fn shaped_headers(headers: &AdditionalHeaders) -> JsonObject {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), shaped_value(value)))
        .collect()
}

#[must_use]
pub fn wrap_stream(
    bytes: ByteStream,
    call: &StreamCall<'_>,
    defaults: StreamDefaults,
    headers: WrapperHeaders,
    runtime: StreamRuntime,
) -> AzureChatStream {
    let additional_headers = match &headers {
        WrapperHeaders::Processed(fields) => {
            let raw: IndexMap<String, String> = fields
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            process_response_headers(HeaderSource::Upstream(&raw))
        }
        WrapperHeaders::Raw(fields) => process_response_headers(HeaderSource::Upstream(fields)),
    };
    let stream_options = requested_stream_options(call.data, defaults.stream_options.clone());
    let mut base_hidden_params = defaults.base_hidden_params;
    base_hidden_params.insert(
        "additional_headers".to_owned(),
        JsonValue::Object(shaped_headers(&additional_headers)),
    );
    let setup = StreamSetup {
        model: call.model.to_owned(),
        stream_options: stream_options.clone(),
        merge_reasoning_in_choices: defaults.merge_reasoning_in_choices,
        is_function_call: defaults.is_function_call,
        base_hidden_params,
        repeated_chunk_limit: REPEATED_STREAMING_CHUNK_LIMIT,
        now: runtime.now,
        next_id: runtime.next_id,
    };
    AzureChatStream {
        model: call.model.to_owned(),
        custom_llm_provider: AZURE_PROVIDER,
        stream_options,
        response_headers: headers,
        additional_headers,
        chunks: chat_stream(bytes, setup),
    }
}

struct StreamState {
    bytes: ByteStream,
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

type Item = Result<StreamChunk, StreamingError>;

fn provider_chunk(payload: &[u8]) -> Result<ProviderChunk, ProviderChunkError> {
    let value: JsonValue = serde_json::from_slice(payload)
        .map_err(|source| ProviderChunkError::InvalidChunk { source })?;
    Ok(ProviderChunk::Json(Box::new(value)))
}

// None means the payload shaped no chunk and the next one is read; Some carries the item to yield.
fn process_payload(state: &mut StreamState, payload: &[u8]) -> Option<Item> {
    let chunk = match provider_chunk(payload) {
        Ok(chunk) => chunk,
        Err(error) => return Some(Err(StreamingError::Chunk(error))),
    };
    state
        .wrapper
        .push(&chunk)
        .map_err(StreamingError::from)
        .transpose()
}

// A byte read that is dropped here still advances the byte stream, so the loop in step ends.
fn absorb(state: &mut StreamState, read: Result<Vec<u8>, SdkError>) -> Result<(), StreamingError> {
    let bytes = read.map_err(|error| {
        StreamingError::Azure(Box::new(AzureStreamFailure::raised(Raised::sdk(&error))))
    })?;
    let events = state.parser.push(&bytes).map_err(|error| {
        StreamingError::Azure(Box::new(AzureStreamFailure::raised(Raised::sdk(
            &SdkError::Sse(error),
        ))))
    })?;
    state.pending.extend(events);
    Ok(())
}

enum Advance {
    Next,
    Item(Box<Item>),
    Closing,
}

async fn advance(state: &mut StreamState) -> Advance {
    if state.phase == Phase::Done {
        return Advance::Closing;
    }
    if let Some(payload) = state.pending.pop_front() {
        if payload.trim_ascii() == b"[DONE]" {
            state.phase = Phase::Done;
            return Advance::Next;
        }
        return process_payload(state, &payload)
            .map_or(Advance::Next, |item| Advance::Item(Box::new(item)));
    }
    if state.phase == Phase::Exhausted {
        return Advance::Closing;
    }
    let Some(read) = state.bytes.next().await else {
        state.pending.extend(state.parser.finish());
        state.phase = Phase::Exhausted;
        return Advance::Next;
    };
    match absorb(state, read) {
        Ok(()) => Advance::Next,
        Err(error) => Advance::Item(Box::new(Err(error))),
    }
}

async fn step(mut state: StreamState) -> Option<(Item, StreamState)> {
    loop {
        if state.phase == Phase::Failed {
            return None;
        }
        let item = match advance(&mut state).await {
            Advance::Next => continue,
            Advance::Item(item) => *item,
            Advance::Closing => state
                .wrapper
                .finish()
                .map(|chunk| chunk.map_err(StreamingError::from))?,
        };
        if item.is_err() {
            state.phase = Phase::Failed;
        }
        return Some((item, state));
    }
}

#[must_use]
pub fn chat_stream(bytes: ByteStream, setup: StreamSetup) -> ChatStream {
    let state = StreamState {
        bytes,
        parser: SseParser::default(),
        pending: VecDeque::new(),
        wrapper: StreamWrapper::new(AZURE_PROVIDER, setup),
        phase: Phase::Reading,
    };
    Box::pin(stream::unfold(state, step))
}
