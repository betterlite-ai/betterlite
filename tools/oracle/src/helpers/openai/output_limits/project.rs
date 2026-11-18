use std::collections::BTreeMap;

use betterlite_openai::output_limits::raw_send::{BadRequestReply, RawChatError, StatusReply};
use betterlite_oracle_helper_api::error::HelperError;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
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
        RawChatError::BadRequest(rejected) => {
            let BadRequestReply { message, .. } = *rejected;
            failure("BadRequestError", Some(400), message)
        }
        RawChatError::Status(reply) => {
            let StatusReply {
                status, message, ..
            } = *reply;
            failure("APIStatusError", Some(status), message)
        }
        RawChatError::Timeout { message } => failure("APITimeoutError", None, message),
        RawChatError::Other { message } => failure("RawChatError", None, message),
        RawChatError::Count { source } => failure("PromptCountError", None, source.to_string()),
        RawChatError::Encode { source } => failure("Encode", None, source.to_string()),
    }
}

pub(crate) fn header_map(headers: &BTreeMap<String, String>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        if let (Ok(parsed_name), Ok(parsed_value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::try_from(value.as_str()),
        ) {
            map.insert(parsed_name, parsed_value);
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

// the two entries every case reports for an answered request: its chosen header and the projected body
pub(crate) fn reply_report<T>(
    headers: &HeaderMap,
    header_name: &str,
    body: &T,
    data: &Map<String, Value>,
) -> Result<Map<String, Value>, Box<HelperError>>
where
    T: Serialize,
{
    let mut report = Map::new();
    report.insert(String::from("header"), header_of(headers, header_name));
    report.insert(String::from("body"), body_of(body, data)?);
    Ok(report)
}

// the reply the case's request asked for: a chunk when it streamed, a completion otherwise
pub(crate) fn body_of<T>(body: &T, data: &Map<String, Value>) -> Result<Value, Box<HelperError>>
where
    T: Serialize,
{
    let encoded =
        serde_json::to_value(body).map_err(|error| failure("Encode", None, error.to_string()))?;
    let body_key = if matches!(data.get("stream"), Some(Value::Bool(true))) {
        "delta"
    } else {
        "message"
    };
    Ok(projected_body(&encoded, body_key))
}

fn projected_body(value: &Value, body_key: &str) -> Value {
    json!({
        "object": value.get("object"),
        "model": value.get("model"),
        "choices": projected_choices(value, body_key),
        "usage": projected_usage(value),
    })
}

fn projected_choices(value: &Value, body_key: &str) -> Value {
    let choices = value.get("choices").and_then(Value::as_array);
    let mut projected = Vec::new();
    if let Some(entries) = choices {
        for choice in entries {
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
                    "role": body.and_then(|message| message.get("role")),
                    "content": body.and_then(|message| message.get("content")),
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
    let mut projected = Map::new();
    for name in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        projected.insert(
            name.to_owned(),
            usage.get(name).cloned().unwrap_or(Value::Null),
        );
    }
    Value::Object(projected)
}
