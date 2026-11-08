use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use serde::Deserialize;

pub type ErrorFields = BTreeMap<String, JsonValue>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SseFailure {
    pub status_code: Option<u16>,
    pub code: Option<String>,
    pub error_type: Option<String>,
    pub param: Option<String>,
    pub detail: Option<Box<JsonValue>>,
    pub provider_specific_fields: Option<ErrorFields>,
}

#[must_use]
pub fn exception_detail_parts(detail: &JsonValue) -> (String, Option<ErrorFields>) {
    match detail {
        JsonValue::String(text) => (text.clone(), None),
        JsonValue::Object(fields) => {
            let structured: ErrorFields = fields.clone().into_iter().collect();
            if let Some(JsonValue::String(text)) = fields.get("error") {
                return (text.clone(), Some(structured));
            }
            if let Some(JsonValue::Object(nested)) = fields.get("error")
                && let Some(JsonValue::String(text)) = nested.get("message")
            {
                return (text.clone(), Some(structured));
            }
            if let Some(JsonValue::String(text)) = fields.get("message") {
                return (text.clone(), Some(structured));
            }
            (detail.to_string(), Some(structured))
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => {
            (detail.to_string(), None)
        }
    }
}

#[must_use]
pub fn sse_error_payload(failure: &SseFailure) -> (u16, ErrorFields) {
    let status = resolved_status(failure);
    let (message, structured) = failure.detail.as_deref().map_or_else(
        || (String::from("Error processing stream start"), None),
        exception_detail_parts,
    );
    let mut error: ErrorFields = ErrorFields::new();
    error.insert(String::from("message"), JsonValue::String(message));
    error.insert(
        String::from("type"),
        JsonValue::String(resolved_error_type(failure, status).to_owned()),
    );
    error.insert(String::from("param"), resolved_param(failure));
    error.insert(String::from("code"), JsonValue::String(status.to_string()));
    if let Some(merged) = merged_fields(failure, structured) {
        error.insert(
            String::from("provider_specific_fields"),
            JsonValue::Object(merged.into_iter().collect()),
        );
    }
    (status, error)
}

fn resolved_status(failure: &SseFailure) -> u16 {
    if let Some(status) = failure.status_code {
        return status;
    }
    let Some(code) = failure.code.as_deref() else {
        return 500;
    };
    if code.is_empty() || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return 500;
    }
    code.parse::<u16>().unwrap_or(500)
}

fn resolved_error_type(failure: &SseFailure, status: u16) -> &str {
    match failure.error_type.as_deref() {
        Some(text) if text != "None" => text,
        _ => match status {
            401 => "authentication_error",
            403 => "permission_error",
            429 => "rate_limit_error",
            below_five_hundred if below_five_hundred < 500 => "invalid_request_error",
            _ => "internal_server_error",
        },
    }
}

fn resolved_param(failure: &SseFailure) -> JsonValue {
    match failure.param.as_deref() {
        Some(text) if text != "None" => JsonValue::String(text.to_owned()),
        _ => JsonValue::Null,
    }
}

fn merged_fields(failure: &SseFailure, structured: Option<ErrorFields>) -> Option<ErrorFields> {
    let mut merged = failure.provider_specific_fields.clone().unwrap_or_default();
    if let Some(fields) = structured {
        merged.extend(fields);
    }
    (!merged.is_empty()).then_some(merged)
}

#[must_use]
pub fn sse_error_frames(error: &ErrorFields) -> [String; 2] {
    let mut envelope: ErrorFields = ErrorFields::new();
    envelope.insert(
        String::from("error"),
        JsonValue::Object(
            error
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
    );
    let payload = JsonValue::Object(envelope.into_iter().collect()).to_string();
    [
        format!("data: {payload}\n\n"),
        String::from("data: [DONE]\n\n"),
    ]
}

mod tests;
