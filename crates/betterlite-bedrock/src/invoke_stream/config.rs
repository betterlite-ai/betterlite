use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{HeaderSource, process_response_headers};
use betterlite_models::decisions::Json;
use betterlite_sdk::chat_stream::config::{CallMode, ChatStreamConfig, StreamLoggingFlags};
use betterlite_streaming::provider_chunk_decode::chunk::ProviderKind;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamOptions};
use betterlite_streaming::stream_chunk_shaping::tokens::check_send_stream_usage;
use http::HeaderMap;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::agentcore_stream::adapter::header_pairs;
use crate::invoke_stream::{CUSTOM_LLM_PROVIDER, StreamGlobals, StreamTransport};

// CustomStreamWrapper.__init__: what the wrapper reads off logging_obj, plus the globals the call layer resolved.
pub(super) fn of(
    transport: &StreamTransport<'_>,
    state: &LoggingState,
    headers: &HeaderMap,
    globals: &StreamGlobals,
) -> ChatStreamConfig {
    let stream_options = stream_options(state.stream_options.as_ref());
    ChatStreamConfig {
        model: transport.model.to_owned(),
        provider: Some(CUSTOM_LLM_PROVIDER.to_owned()),
        logging_provider: state.custom_llm_provider.clone(),
        send_stream_usage: check_send_stream_usage(stream_options.as_ref()),
        stream_options,
        merge_reasoning_in_choices: matches!(
            state
                .litellm_params
                .get("merge_reasoning_content_in_choices"),
            Some(Json::Bool(true))
        ),
        optional_params: convert(&state.optional_params),
        hidden_params: hidden_params(state, headers),
        mcp_metadata: None,
        max_streaming_duration: globals.max_streaming_duration,
        repeated_chunk_limit: globals.repeated_chunk_limit,
        mode: if transport.sync {
            CallMode::Blocking
        } else {
            CallMode::Async
        },
        provider_kind: ProviderKind::Builtin,
        completion_stream: None,
        logging: StreamLoggingFlags {
            disable: globals.logging_disable,
        },
    }
}

// getattr(logging_obj, "stream_options", None), read through the dict's include_usage flag.
fn stream_options(value: Option<&Json>) -> Option<StreamOptions> {
    let Json::Object(fields) = value? else {
        return None;
    };
    Some(StreamOptions {
        include_usage: matches!(fields.get("include_usage"), Some(Json::Bool(true))),
    })
}

fn hidden_params(state: &LoggingState, headers: &HeaderMap) -> JsonObject {
    let mut hidden = JsonObject::new();
    hidden.insert("model_id".to_owned(), model_id(state));
    hidden.insert("api_base".to_owned(), api_base(state));
    hidden.insert("additional_headers".to_owned(), additional_headers(headers));
    hidden
}

// litellm_params.model_info.get("id", None).
fn model_id(state: &LoggingState) -> JsonValue {
    let Some(Json::Object(params)) = state.litellm_params.get("model_info") else {
        return JsonValue::Null;
    };
    params
        .get("id")
        .and_then(convert)
        .unwrap_or(JsonValue::Null)
}

// get_api_base: litellm_params.api_base when it names one, else the provider default, which bedrock has none of.
fn api_base(state: &LoggingState) -> JsonValue {
    match state.litellm_params.get("api_base") {
        Some(value @ Json::Text(_)) => convert(value).unwrap_or(JsonValue::Null),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => JsonValue::Null,
    }
}

// process_response_headers(_response_headers or {}): the OPENAI_RESPONSE_HEADERS and llm_provider- copies.
fn additional_headers(headers: &HeaderMap) -> JsonValue {
    let upstream: IndexMap<String, String> = header_pairs(headers).into_iter().collect();
    let projected = process_response_headers(HeaderSource::Upstream(&upstream));
    convert(&projected).unwrap_or(JsonValue::Null)
}

fn convert<T, U>(value: &T) -> Option<U>
where
    T: Serialize,
    U: DeserializeOwned,
{
    serde_json::to_value(value).map_or_else(|_| None, |plain| serde_json::from_value(plain).ok())
}
