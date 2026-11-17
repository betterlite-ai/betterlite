use betterlite_anthropic::anthropic_http_stream::completion::WrappedStream;
use betterlite_anthropic::anthropic_http_stream::transport::StreamPost;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::fixture::{FixtureCounter, FixtureFailure, LoggerCall};

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

fn selected_http(headers: &HeaderMap, names: &[String]) -> Value {
    let mut fields: IndexMap<&str, Option<&str>> = IndexMap::new();
    for name in names {
        fields.insert(
            name.as_str(),
            headers
                .get(name.as_str())
                .and_then(|value| value.to_str().ok()),
        );
    }
    json!(fields)
}

pub(crate) fn request_value(post: &StreamPost, names: &[String]) -> Value {
    let body = serde_json::from_slice::<Value>(&post.body).unwrap_or(Value::Null);
    json!({
        "url": post.url.as_str(),
        "body": body,
        "headers": selected_http(&post.headers, names),
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

// the response mapper's post_call drops api_base from the details, so the sent post supplies it
pub(crate) fn pre_call_value(details: &IndexMap<String, Json>, api_base: Option<&str>) -> Value {
    let body = detail(details.get("additional_args"), "complete_input_dict");
    let base = api_base.map_or(Value::Null, |value| Value::String(value.to_owned()));
    json!({
        "input": details.get("input"),
        "body": body,
        "api_base": base,
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
