use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::python_text::python_value_text;

pub const DEFAULT_STREAM_ERROR_MESSAGE: &str = "Response API in-stream error";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamErrorFields {
    pub message: String,
    pub error_type: Option<String>,
    pub code: Option<String>,
}

// an in-stream error arrives either as the decoded payload or as a provider object with attributes
pub enum StreamErrorObject<'a> {
    Mapping(&'a IndexMap<String, Json>),
    Attributes(StreamErrorAttributes<'a>),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StreamErrorAttributes<'a> {
    pub message: Option<&'a Json>,
    pub error_type: Option<&'a Json>,
    pub code: Option<&'a Json>,
}

pub fn error_event_fields(object: StreamErrorObject<'_>) -> StreamErrorFields {
    let (raw_message, raw_type, raw_code) = match object {
        StreamErrorObject::Mapping(fields) => (
            fields.get("message"),
            fields.get("type"),
            fields.get("code"),
        ),
        StreamErrorObject::Attributes(attributes) => {
            (attributes.message, attributes.error_type, attributes.code)
        }
    };
    StreamErrorFields {
        message: present(raw_message).map_or_else(
            || DEFAULT_STREAM_ERROR_MESSAGE.to_owned(),
            python_value_text,
        ),
        error_type: present(raw_type).and_then(|value| match value {
            Json::Text(text) => Some(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        }),
        code: present(raw_code).and_then(error_code_text),
    }
}

fn present(value: Option<&Json>) -> Option<&Json> {
    match value {
        Some(Json::Null) | None => None,
        Some(other) => Some(other),
    }
}

// code keeps str and int, so bools, floats, nulls and containers drop out
fn error_code_text(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => integer_text(number),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn integer_text(number: &Number) -> Option<String> {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        return None;
    }
    Some(if text == "-0" { "0".to_owned() } else { text })
}

pub fn stream_error_status_and_message(object: StreamErrorObject<'_>) -> (u16, String) {
    let fields = error_event_fields(object);
    let status = status_code_for_error_fields(fields.error_type.as_deref(), fields.code.as_deref());
    (status, fields.message)
}

pub fn status_code_for_error_fields(error_type: Option<&str>, error_code: Option<&str>) -> u16 {
    let fields = [error_code, error_type];
    let rate_limited = fields
        .iter()
        .flatten()
        .any(|field| field.starts_with("rate_limit") || *field == "insufficient_quota");
    if rate_limited {
        return 429;
    }
    fields
        .iter()
        .flatten()
        .find_map(|field| status_code_for_error_field(field))
        .unwrap_or(500)
}

// code is read before type, and only an ASCII decimal in 400..=599 counts as a status
pub fn status_code_for_error_field(field: &str) -> Option<u16> {
    if !field.is_empty() && field.bytes().all(|byte| byte.is_ascii_digit()) {
        let number = field.parse::<u16>().ok()?;
        if (400..=599).contains(&number) {
            return Some(number);
        }
    }
    error_code_status(field)
}

fn error_code_status(field: &str) -> Option<u16> {
    match field {
        "server_error" => Some(500),
        "rate_limit_exceeded" | "insufficient_quota" => Some(429),
        "vector_store_timeout" => Some(504),
        "invalid_prompt"
        | "invalid_image"
        | "invalid_image_format"
        | "invalid_base64_image"
        | "invalid_image_url"
        | "image_too_large"
        | "image_too_small"
        | "image_parse_error"
        | "image_content_policy_violation"
        | "invalid_image_mode"
        | "image_file_too_large"
        | "unsupported_image_media_type"
        | "empty_image_file"
        | "failed_to_download_image"
        | "image_file_not_found"
        | "invalid_request_error"
        | "context_length_exceeded"
        | "content_policy_violation"
        | "model_not_found" => Some(400),
        _ => None,
    }
}
