use std::collections::BTreeMap;

use betterlite_openai::output_limits::raw_send::RawChatError;
use betterlite_oracle_helper_api::error::HelperError;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

// python's dispatch answers every failure with type, param and code present and null
pub(crate) fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

pub(crate) fn raw_failure(error: RawChatError) -> Box<HelperError> {
    match error {
        RawChatError::BadRequest { message, .. } => failure("BadRequestError", Some(400), message),
        RawChatError::Timeout { message } => failure("APITimeoutError", None, message),
        RawChatError::Other { message } => failure("RawChatError", None, message),
    }
}

pub(crate) fn encoded<T>(value: &T) -> Result<Value, Box<HelperError>>
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|error| failure("Encode", None, error.to_string()))
}

pub(crate) fn header_map(headers: &BTreeMap<String, String>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::try_from(value.as_str()),
        ) {
            map.insert(name, value);
        }
    }
    map
}

pub(crate) fn header_of(headers: &HeaderMap, name: &str) -> Value {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map_or(Value::Null, |value| Value::String(value.to_owned()))
}

pub(crate) fn body_of(body: &Value, data: &Map<String, Value>) -> Value {
    if matches!(data.get("stream"), Some(Value::Bool(true))) {
        projected_chunk(body)
    } else {
        projected_completion(body)
    }
}

pub(crate) fn projected_completion(value: &Value) -> Value {
    json!({
        "object": value.get("object"),
        "model": value.get("model"),
        "choices": projected_choices(value, "message"),
        "usage": projected_usage(value),
    })
}

pub(crate) fn projected_chunk(value: &Value) -> Value {
    json!({
        "object": value.get("object"),
        "model": value.get("model"),
        "choices": projected_choices(value, "delta"),
        "usage": projected_usage(value),
    })
}

fn projected_choices(value: &Value, body_key: &str) -> Value {
    let choices = value.get("choices").and_then(Value::as_array);
    let mut projected = Vec::new();
    if let Some(choices) = choices {
        for choice in choices {
            let body = choice.get(body_key);
            let mut fields = Map::new();
            fields.insert(
                String::from("index"),
                choice.get("index").cloned().unwrap_or(Value::Null),
            );
            fields.insert(
                String::from("finish_reason"),
                choice.get("finish_reason").cloned().unwrap_or(Value::Null),
            );
            fields.insert(
                body_key.to_owned(),
                json!({
                    "role": body.and_then(|body| body.get("role")),
                    "content": body.and_then(|body| body.get("content")),
                }),
            );
            projected.push(Value::Object(fields));
        }
    }
    Value::Array(projected)
}

fn projected_usage(value: &Value) -> Value {
    let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) else {
        return Value::Null;
    };
    json!({
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
    })
}
