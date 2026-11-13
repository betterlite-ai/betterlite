use betterlite_models::decisions::Json;
use thiserror::Error;

use super::{HeaderMap, RerankLimits};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: HeaderMap,
}

// LiteLLM picks {"detail": {"error": ...}} first, then a string detail, and swallows every parse or type failure
#[must_use]
pub fn decode_deepinfra_error(message: &str, limits: RerankLimits) -> String {
    if message.len() > limits.max_body_bytes {
        return message.to_owned();
    }
    let Ok(Json::Object(fields)) = serde_json::from_str::<Json>(message) else {
        return message.to_owned();
    };
    let selected = match fields.get("detail") {
        Some(Json::Object(detail)) => match detail.get("error") {
            Some(Json::Text(text)) => Some(text.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        },
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) | None => None,
    };
    selected.unwrap_or_else(|| message.to_owned())
}

#[must_use]
pub fn get_error_class(
    error_message: &str,
    status_code: u16,
    headers: HeaderMap,
    limits: RerankLimits,
) -> BaseLlmException {
    BaseLlmException {
        status_code,
        message: decode_deepinfra_error(error_message, limits),
        headers,
    }
}
