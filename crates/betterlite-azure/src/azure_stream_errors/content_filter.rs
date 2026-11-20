use thiserror::Error;

use crate::azure_content_policy::{JsonValue, extract_azure_error};

pub const CONTENT_FILTER: &str = "content_filter";
pub const CONTENT_POLICY_VIOLATION: &str = "content_policy_violation";
pub const RESPONSIBLE_AI_CODE: &str = "ResponsibleAIPolicyViolation";

// The Azure content-filter error body a streamed error event can carry
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct ContentFilterError {
    pub message: String,
    pub code: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub inner_code: Option<String>,
}

#[derive(Debug, Error)]
pub enum ContentFilterDecodeError {
    #[error("the Azure error body is not JSON: {0}")]
    Body(#[from] serde_json::Error),
}

fn text_of(value: Option<&JsonValue>) -> Option<String> {
    match value {
        Some(JsonValue::String(text)) => Some(text.clone()),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}

fn inner_error_code(inner: Option<&JsonValue>) -> Option<String> {
    match inner {
        Some(JsonValue::Object(fields)) => text_of(fields.get("code")),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_),
        )
        | None => None,
    }
}

#[must_use]
pub fn is_content_filter(code: &str, error_type: &str, inner_code: Option<&str>) -> bool {
    code == CONTENT_FILTER
        || code == CONTENT_POLICY_VIOLATION
        || error_type == CONTENT_FILTER
        || inner_code == Some(RESPONSIBLE_AI_CODE)
}

// The body's error object, whether the wire put it under "error" or at the top level
pub fn decode_content_filter_error(
    body: &[u8],
) -> Result<Option<ContentFilterError>, ContentFilterDecodeError> {
    let value: JsonValue = serde_json::from_slice(body)?;
    let (error, inner) = extract_azure_error(&value);
    let code = text_of(error.get("code")).unwrap_or_default();
    let error_type = text_of(error.get("type")).unwrap_or_default();
    let inner_code = inner_error_code(inner.as_ref());
    if !is_content_filter(&code, &error_type, inner_code.as_deref()) {
        return Ok(None);
    }
    Ok(Some(ContentFilterError {
        message: text_of(error.get("message")).unwrap_or_default(),
        code,
        error_type,
        inner_code,
    }))
}
