use betterlite_anthropic::anthropic_http_stream::completion::WrappedStream;
use betterlite_anthropic::anthropic_http_stream::transport::StreamPost;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::fixture::runtime::FixtureCounter;
use super::fixture::{FixtureFailure, LoggerCall};

pub(crate) fn header_map(pairs: &IndexMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            HeaderName::try_from(name.as_str()).or_value_error()?,
            HeaderValue::try_from(value.as_str()).or_value_error()?,
        );
    }
    Ok(headers)
}

pub(crate) fn request_value(post: &StreamPost, names: &[String]) -> Value {
    let body = serde_json::from_slice::<Value>(&post.body).unwrap_or(Value::Null);
    let mut headers: IndexMap<&str, Option<&str>> = IndexMap::new();
    for name in names {
        headers.insert(
            name.as_str(),
            post.headers
                .get(name.as_str())
                .and_then(|value| value.to_str().ok()),
        );
    }
    json!({
        "url": post.url.as_str(),
        "body": body,
        "headers": headers,
    })
}

pub(crate) fn reply_value(envelope: &ResponseEnvelope) -> Value {
    let choice = envelope.choices.first();
    json!({
        "model": envelope.model,
        "content": choice.and_then(|item| item.message.content.as_deref()),
        "finish_reason": choice.and_then(|item| item.finish_reason.as_deref()),
    })
}

pub(crate) fn stream_value(wrapper: &WrappedStream<FixtureFailure, FixtureCounter>) -> Value {
    let state = &wrapper.completion_stream.converter.state;
    json!({
        "model": wrapper.model,
        "provider": wrapper.custom_llm_provider,
        "options": {
            "json_mode": state.json_mode,
            "speed": state.speed,
            "tool_name_reverse_map": state.tool_name_reverse_map,
        },
    })
}

fn detail<'details>(args: Option<&'details Json>, key: &str) -> Option<&'details Json> {
    match args {
        Some(Json::Object(fields)) => fields.get(key),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

// the hook logged the filtered headers and body, so the beta here is the filter's answer, not the transform's
pub(crate) fn pre_call_value(details: &IndexMap<String, Json>) -> Value {
    let additional = details.get("additional_args");
    let beta = detail(detail(additional, "headers"), "anthropic-beta");
    json!({
        "input": details.get("input"),
        "body": detail(additional, "complete_input_dict"),
        "api_base": detail(additional, "api_base"),
        "anthropic_beta": beta,
    })
}

pub(crate) fn post_call_value(details: &IndexMap<String, Json>) -> Value {
    json!({
        "input": details.get("input"),
        "original_response": details.get("original_response"),
        "body": detail(details.get("additional_args"), "complete_input_dict"),
    })
}

// the stream post_call body stays the JSON string LiteLLM sent, so the two sides compare it byte for byte
pub(crate) fn logger_value(call: &LoggerCall) -> Value {
    json!({
        "input": call.input,
        "body": call.complete_input_dict,
    })
}
