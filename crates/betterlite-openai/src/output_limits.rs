pub mod build;
pub mod raw_send;
pub mod tokens;

use std::str;

use serde_json::{Map, Value};
use thiserror::Error;

// LiteLLM's _OUTPUT_TOKEN_LIMIT_ERROR_MARKER: the provider sentence, not a fragment of it
pub const OUTPUT_TOKEN_LIMIT_MARKER: &str =
    "could not finish the message because max_tokens or model output limit was reached";

#[derive(Debug, Clone, Copy)]
pub enum ErrorBody<'body> {
    // the openai SDK's exception.body: the error object, or the whole body when it has no error key
    Sdk(&'body Value),
    // an httpx.HTTPStatusError's response bytes
    Http(&'body [u8]),
}

impl ErrorBody<'_> {
    // LiteLLM's is_output_token_limit_error: the marker is matched inside the whole exception text
    #[must_use]
    pub fn reaches_output_limit(&self) -> bool {
        let text = match self {
            Self::Sdk(value) => value.as_str().unwrap_or_default(),
            Self::Http(bytes) => str::from_utf8(bytes).unwrap_or_default(),
        };
        text.to_lowercase().contains(OUTPUT_TOKEN_LIMIT_MARKER)
    }
}

#[derive(Debug, Error)]
pub enum RejectedFieldsError {
    #[error("the error response is not JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the error response is not a JSON object")]
    ResponseNotObject,
    #[error("the error body's message is not a JSON object")]
    MessageNotObject,
    #[error("detail entry {index} is not a JSON object")]
    DetailEntryNotObject { index: usize },
}

// LiteLLM's drop_params_from_unprocessable_entity_error: the rejected field names of a 422, as loc[1] of each detail entry
pub fn decode_rejected_fields(body: ErrorBody<'_>) -> Result<Vec<Value>, RejectedFieldsError> {
    match body {
        ErrorBody::Sdk(value) => decode_body(value),
        ErrorBody::Http(bytes) => {
            let parsed: Value = serde_json::from_slice(bytes)
                .map_err(|source| RejectedFieldsError::Json { source })?;
            let Value::Object(response) = &parsed else {
                return Err(RejectedFieldsError::ResponseNotObject);
            };
            response
                .get("error")
                .map_or_else(|| Ok(Vec::new()), decode_body)
        }
    }
}

fn decode_body(error_body: &Value) -> Result<Vec<Value>, RejectedFieldsError> {
    let Value::Object(fields) = error_body else {
        return Ok(Vec::new());
    };
    let Some(raw_message) = fields.get("message").filter(|value| present(value)) else {
        return Ok(Vec::new());
    };
    let Some(decoded) = decoded_message(raw_message) else {
        return Ok(Vec::new());
    };
    let Value::Object(message) = decoded else {
        return Err(RejectedFieldsError::MessageNotObject);
    };
    match message.get("detail") {
        Some(Value::Array(detail)) => rejected_fields(detail),
        _ => Ok(Vec::new()),
    }
}

// a string message is JSON-decoded, and one that is not JSON reads as {detail: message}, which names nothing
fn decoded_message(message: &Value) -> Option<Value> {
    match message {
        Value::String(text) => serde_json::from_str::<Value>(text).ok(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            Some(message.clone())
        }
    }
}

fn rejected_fields(detail: &[Value]) -> Result<Vec<Value>, RejectedFieldsError> {
    let Some(first) = detail.first() else {
        return Ok(Vec::new());
    };
    if !first.is_object() {
        return Ok(Vec::new());
    }
    let mut rejected = Vec::new();
    for (index, entry) in detail.iter().enumerate() {
        let Value::Object(fields) = entry else {
            return Err(RejectedFieldsError::DetailEntryNotObject { index });
        };
        if let Some(Value::Array(location)) = fields.get("loc")
            && location.len() == 2
        {
            rejected.push(location.get(1).cloned().unwrap_or(Value::Null));
        }
    }
    Ok(rejected)
}

// LiteLLM re-reads the caller's data as a new dict, so the caller's map is never changed
pub fn without_rejected_fields(
    body: ErrorBody<'_>,
    data: &Map<String, Value>,
) -> Result<Map<String, Value>, RejectedFieldsError> {
    let rejected = decode_rejected_fields(body)?;
    let mut kept = data.clone();
    kept.retain(|name, _value| {
        !rejected
            .iter()
            .any(|field| field.as_str() == Some(name.as_str()))
    });
    Ok(kept)
}

// Python's truthiness of a decoded JSON value: the checks decide whether the message and detail fields are read at all
fn present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_none_or(|float| float != 0.0_f64),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

mod tests;
