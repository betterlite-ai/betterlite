use betterlite_bedrock::invoke_stream::{
    HAS_CUSTOM_STREAM_WRAPPER, SUPPORTS_STREAM_PARAM_IN_BODY, StreamTransport, WrappedInvokeStream,
};
use betterlite_models::decisions::Json;
use serde_json::{Value, json};

use super::support::sorted_pairs;

// What the wrapper handed make_call/make_sync_call, as LiteLLM's own call sites spell it.
pub(super) fn arguments_view(transport: &StreamTransport<'_>) -> Value {
    json!({
        "model": transport.model,
        "api_base": transport.api_base,
        "headers": sorted_pairs(&transport.headers),
        "data": transport.data,
        "signed_json_body": transport.signed_json_body,
        "messages": transport.messages,
        "fake_stream": transport.fake_stream,
        "json_mode": transport.json_mode,
        "provider": transport.provider,
        "sync": transport.sync,
        "stream_chunk_size": transport.stream_chunk_size,
    })
}

// The identity B004's CustomStreamWrapper keeps: the model, the provider it was told, the returned headers.
pub(super) fn wrapper_view(wrapped: &WrappedInvokeStream<'_>) -> Value {
    json!({
        "model": wrapped.model,
        "custom_llm_provider": wrapped.custom_llm_provider,
        "response_headers": sorted_pairs(&wrapped.response_headers),
    })
}

pub(super) fn config_view() -> Value {
    json!({
        "has_custom_stream_wrapper": HAS_CUSTOM_STREAM_WRAPPER,
        "supports_stream_param_in_request_body": SUPPORTS_STREAM_PARAM_IN_BODY,
    })
}

pub(super) fn log_view(details: &Json) -> Value {
    let fields = match details {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Value::Null;
        }
    };
    let input = fields.get("input").cloned().unwrap_or(Json::Null);
    let original = fields
        .get("original_response")
        .cloned()
        .unwrap_or(Json::Null);
    let data = fields
        .get("additional_args")
        .and_then(|args| match args {
            Json::Object(inner) => inner.get("complete_input_dict").cloned(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .unwrap_or(Json::Null);
    json!({
        "input": input,
        "original_response": original,
        "complete_input_dict": data,
    })
}
