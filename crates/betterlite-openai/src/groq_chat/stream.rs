use serde_json::Value;
use uuid::Uuid;

use crate::chat_choices::stream::{StreamReply, chunk_parser as base_chunk_parser};
use crate::error::{OpenAiError, ProviderError, ProviderErrorParts};

use super::error::GroqChatError;

// the Groq check runs before the base parser, so an integer code is passed on without the base 400..600 clamp
pub fn chunk_parser(
    chunk: &Value,
    identifier: fn() -> Uuid,
    clock: fn() -> i64,
) -> Result<StreamReply, GroqChatError> {
    if let Some(error) = chunk.get("error").filter(|value| !is_blank(value)) {
        return Err(GroqChatError::Provider(OpenAiError::Provider(Box::new(
            provider_error(error),
        ))));
    }
    // the base parser renames delta.reasoning to delta.reasoning_content and builds the chunk envelope
    base_chunk_parser(chunk, identifier, clock).map_err(GroqChatError::from)
}

fn provider_error(error: &Value) -> ProviderError {
    let status = error
        .get("code")
        .and_then(Value::as_i64)
        .and_then(|code| u16::try_from(code).ok());
    ProviderError::from(ProviderErrorParts {
        status_code: status.unwrap_or(500),
        status_code_is_synthesized: status.is_none(),
        message: error_message(error),
        headers: None,
        request: None,
        response: None,
        body: error.as_object().cloned(),
    })
}

fn error_message(error: &Value) -> String {
    match error.get("message") {
        Some(Value::String(text)) => text.clone(),
        _ => match error {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        },
    }
}

fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
        Value::Number(number) => number.as_f64() == Some(0.0_f64),
        Value::Bool(true) => false,
    }
}
