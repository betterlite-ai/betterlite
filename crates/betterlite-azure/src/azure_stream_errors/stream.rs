use std::collections::BTreeMap;
use std::str;

use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, HeaderSource, process_response_headers,
};
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::StreamDefaults;
use betterlite_openai::text_completion_transport::sdk::ByteStream;
use betterlite_openai::text_completion_transport::stream::{
    ProviderStream, PumpError, provider_stream,
};
use betterlite_streaming::provider_chunk_decode::wrapper::StreamSetup;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamOptions};
use http::HeaderMap;
use indexmap::IndexMap;

use super::error::{AzureStreamFailure, Raised, StreamingError};
use super::records::{StreamCall, StreamRuntime};

pub const AZURE_PROVIDER: &str = "azure";
// LiteLLM's REPEATED_STREAMING_CHUNK_LIMIT default
pub const REPEATED_STREAMING_CHUNK_LIMIT: usize = 100;

pub type ChatStream = ProviderStream<StreamingError>;

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
        chunks: provider_stream(AZURE_PROVIDER, bytes, setup, streaming_error),
    }
}

// The classes the Azure stream has always reported: a chunk error, a usage write error, or an SDK read error.
fn streaming_error(error: PumpError) -> StreamingError {
    match error {
        PumpError::Chunk(chunk) => StreamingError::Chunk(chunk),
        PumpError::Usage(usage) => StreamingError::Usage(usage),
        PumpError::Sdk(source) => {
            StreamingError::Azure(Box::new(AzureStreamFailure::raised(Raised::sdk(&source))))
        }
    }
}
