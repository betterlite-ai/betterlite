use thiserror::Error;

use crate::openai_base_config::Headers;

// LiteLLM's OpenRouterException adds nothing to its native BaseLLMException
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct OpenRouterError {
    pub message: String,
    pub status_code: u16,
    pub headers: Headers,
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: Headers) -> OpenRouterError {
    OpenRouterError {
        message: message.to_owned(),
        status_code,
        headers,
    }
}

// the KeyError branch of the stream parser names the key and prints the chunk as compact JSON
#[must_use]
pub fn key_error(key: &str, chunk: &serde_json::Value) -> OpenRouterError {
    let printed = serde_json::to_string(chunk).unwrap_or_else(|_| String::from("null"));
    OpenRouterError {
        message: format!("KeyError: '{key}', Got unexpected response from OpenRouter: {printed}"),
        status_code: 400,
        headers: Headers::from([(
            String::from("Content-Type"),
            String::from("application/json"),
        )]),
    }
}
