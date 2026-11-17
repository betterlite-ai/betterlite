use betterlite_anthropic::anthropic_http_stream::completion::WrappedStream;
use betterlite_anthropic::anthropic_http_stream::transport::StreamPost;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::fixture::{FixtureCounter, FixtureFailure, LoggerCall, Recorded};

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

fn selected(pairs: &IndexMap<String, String>, names: &[String]) -> Value {
    let mut fields: IndexMap<&str, Option<&str>> = IndexMap::new();
    for name in names {
        fields.insert(name.as_str(), pairs.get(name.as_str()).map(String::as_str));
    }
    json!(fields)
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

pub(crate) fn transform_value(record: &Recorded, names: &[String]) -> Value {
    json!({
        "model": record.model,
        "messages": record.messages,
        "optional_params": record.optional_params,
        "litellm_params": record.litellm_params,
        "headers": selected(&record.headers, names),
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

pub(crate) fn stream_value(
    wrapper: &WrappedStream<FixtureFailure, FixtureCounter>,
    names: &[String],
) -> Value {
    let state = &wrapper.completion_stream.converter.state;
    json!({
        "model": wrapper.model,
        "provider": wrapper.custom_llm_provider,
        "options": {
            "json_mode": state.json_mode,
            "speed": state.speed,
            "tool_name_reverse_map": state.tool_name_reverse_map,
        },
        "response_headers": selected(&wrapper.response_headers, names),
    })
}

fn detail<'details>(args: Option<&'details Json>, key: &str) -> Option<&'details Json> {
    match args {
        Some(Json::Object(fields)) => fields.get(key),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

pub(crate) fn pre_call_value(details: &IndexMap<String, Json>) -> Value {
    let args = details.get("additional_args");
    json!({
        "input": details.get("input"),
        "body": detail(args, "complete_input_dict"),
        "api_base": detail(args, "api_base"),
    })
}

pub(crate) fn post_call_value(details: &IndexMap<String, Json>) -> Value {
    let args = details.get("additional_args");
    json!({
        "input": details.get("input"),
        "original_response": details.get("original_response"),
        "body": detail(args, "complete_input_dict"),
    })
}

// the async buffered failure logs str(exception), which no Rust text equals; the request body still does
pub(crate) fn failure_post_call(details: &IndexMap<String, Json>) -> Value {
    if !matches!(details.get("log_event_type"), Some(Json::Text(kind)) if kind == "post_api_call") {
        return Value::Null;
    }
    let args = details.get("additional_args");
    json!({
        "input": details.get("input"),
        "body": detail(args, "complete_input_dict"),
    })
}

// LiteLLM's async make_call hands post_call the live iterator; the record keeps "lines" for it
const ITERATOR_RESPONSE: &str = "lines";

pub(crate) fn logger_value(call: &LoggerCall) -> Value {
    let response =
        (call.original_response != ITERATOR_RESPONSE).then_some(call.original_response.as_str());
    json!({
        "input": call.input,
        "original_response": response,
        "body": serde_json::from_str::<Value>(&call.complete_input_dict).unwrap_or(Value::Null),
    })
}
