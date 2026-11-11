use std::num::TryFromIntError;

use betterlite_models::provider_defaults_a::url::UrlError;
use thiserror::Error;

use crate::openai_base_config::Headers;

use super::MAX_REPLY_BYTES;

#[derive(Debug, Error)]
pub enum HostModelIdentityError {
    #[error("the reply is over the {MAX_REPLY_BYTES} byte limit")]
    TooLarge,
    #[error("the {host} reply is not valid JSON")]
    Json {
        host: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {host} reply has no created time and the clock reading does not fit one")]
    Clock {
        host: &'static str,
        #[source]
        source: TryFromIntError,
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
    #[error("the Clarifai response body is not valid JSON")]
    ClarifaiJson {
        status_code: u16,
        headers: Headers,
        #[source]
        source: serde_json::Error,
    },
}

// the OpenAIError both get_error_class methods build, with the response headers copied in
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct HostApiError {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}
