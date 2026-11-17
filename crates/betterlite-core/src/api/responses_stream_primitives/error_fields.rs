use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::python_text::value_text;

pub const DEFAULT_STREAM_ERROR_MESSAGE: &str = "Response API in-stream error";

// LiteLLM's _ERROR_CODE_HTTP_STATUS table, in the order it declares the entries
const ERROR_CODE_STATUS: [(&str, u16); 23] = [
    ("server_error", 500),
    ("rate_limit_exceeded", 429),
    ("insufficient_quota", 429),
    ("vector_store_timeout", 504),
    ("invalid_prompt", 400),
    ("invalid_image", 400),
    ("invalid_image_format", 400),
    ("invalid_base64_image", 400),
    ("invalid_image_url", 400),
    ("image_too_large", 400),
    ("image_too_small", 400),
    ("image_parse_error", 400),
    ("image_content_policy_violation", 400),
    ("invalid_image_mode", 400),
    ("image_file_too_large", 400),
    ("unsupported_image_media_type", 400),
    ("empty_image_file", 400),
    ("failed_to_download_image", 400),
    ("image_file_not_found", 400),
    ("invalid_request_error", 400),
    ("context_length_exceeded", 400),
    ("content_policy_violation", 400),
    ("model_not_found", 400),
];

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
        message: present(raw_message)
            .map_or_else(|| DEFAULT_STREAM_ERROR_MESSAGE.to_owned(), value_text),
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

pub fn error_status_and_message(object: StreamErrorObject<'_>) -> (u16, String) {
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
    let decimal = !field.is_empty() && field.bytes().all(|byte| byte.is_ascii_digit());
    if decimal {
        return field
            .parse::<u16>()
            .ok()
            .filter(|number| (400..=599).contains(number));
    }
    error_code_status(field)
}

pub(super) fn error_code_status(field: &str) -> Option<u16> {
    ERROR_CODE_STATUS
        .iter()
        .find_map(|(code, status)| (*code == field).then_some(*status))
}
