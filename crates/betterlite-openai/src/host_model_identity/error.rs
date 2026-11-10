use betterlite_models::provider_defaults_a::url::UrlError;
use thiserror::Error;

use crate::openai_base_config::Headers;

#[derive(Debug, Error)]
pub enum HostModelIdentityError {
    #[error("the {host} reply is {size} bytes, over the {limit} byte limit")]
    TooLarge {
        host: &'static str,
        size: usize,
        limit: usize,
    },
    #[error("the {host} reply is not valid JSON")]
    Json {
        host: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {host} reply is not a chat completion object")]
    Malformed { host: &'static str },
    #[error("the {host} reply does not match the chat completion fields")]
    Reply {
        host: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Clarifai model {model} is not user_id.app_id.model_id")]
    ModelName { model: String },
    #[error("the DataRobot api base {api_base} cannot be parsed")]
    ApiBase {
        api_base: String,
        #[source]
        source: UrlError,
    },
    // LiteLLM's Clarifai transform rethrows a json failure as the provider's OpenAIError, headers and all
    #[error("{message}")]
    Api {
        status_code: u16,
        headers: Headers,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct HostApiError {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}

// get_error_class builds the native OpenAIError for both hosts and copies the response headers into it
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> HostApiError {
    HostApiError {
        status_code,
        message: message.to_owned(),
        headers: headers.clone(),
    }
}
