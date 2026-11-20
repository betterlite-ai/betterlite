use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::compatible_wire::{ReplyOptions, decode_compatible_chat_reply};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use indexmap::IndexMap;
use uuid::Uuid;

use super::error::AzureChatError;

// The projected answer plus the hidden parameters and headers convert_to_model_response_object stores
pub struct ChatReply {
    pub response: ChatResponse,
    pub hidden_params: Extra,
    pub response_headers: IndexMap<String, String>,
}

// the four OpenAI headers get_response_headers forwards bare
pub const RATE_LIMIT_HEADERS: [&str; 4] = [
    "x-ratelimit-limit-requests",
    "x-ratelimit-remaining-requests",
    "x-ratelimit-limit-tokens",
    "x-ratelimit-remaining-tokens",
];

// convert_to_model_response_object: the OpenAI-compatible headers, then the provider's own under llm_provider-
#[must_use]
pub fn additional_headers(headers: &SecretHeaders) -> Extra {
    let mut provider = Extra::new();
    for (name, value) in &headers.0 {
        let key = if name.contains("llm_provider") {
            name.clone()
        } else {
            format!("llm_provider-{name}")
        };
        provider.insert(key, Json::Text(value.clone()));
    }
    for name in RATE_LIMIT_HEADERS {
        if let Some(value) = headers.0.get(name) {
            provider.insert(name.to_owned(), Json::Text(value.clone()));
        }
    }
    provider
}

#[must_use]
pub fn raw_headers(headers: &SecretHeaders) -> IndexMap<String, String> {
    headers
        .0
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// convert_to_model_response_object with B004's decoder: the async path also keeps the raw headers in hidden_params
pub fn project(
    body: &Json,
    headers: &SecretHeaders,
    json_mode: bool,
    with_raw_headers: bool,
    identifier: fn() -> Uuid,
    clock: fn() -> i64,
) -> Result<ChatReply, AzureChatError> {
    let bytes =
        serde_json::to_vec(body).map_err(|error| AzureChatError::azure(500, error.to_string()))?;
    let options = ReplyOptions {
        json_mode,
        provider: None,
        base_model: None,
        identifier,
        clock,
    };
    let decoded = decode_compatible_chat_reply(&bytes, &options)
        .map_err(|error| AzureChatError::azure(500, error.to_string()))?;
    let raw = raw_headers(headers);
    let mut hidden_params = Extra::new();
    hidden_params.insert(
        "additional_headers".to_owned(),
        Json::Object(additional_headers(headers)),
    );
    if with_raw_headers {
        hidden_params.insert(
            "headers".to_owned(),
            Json::Object(
                raw.iter()
                    .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
                    .collect(),
            ),
        );
    }
    Ok(ChatReply {
        response: decoded.response,
        hidden_params,
        response_headers: raw,
    })
}
