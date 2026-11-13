use thiserror::Error;

use crate::error::{ErrorRequest, ProviderError, ProviderErrorParts};
use crate::openai_base_config::Headers;

const REQUEST_METHOD: &str = "POST";
const REQUEST_URL: &str = "https://api.fireworks.ai";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct FireworksError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
}

impl From<FireworksError> for ProviderError {
    fn from(error: FireworksError) -> Self {
        Self::from(ProviderErrorParts {
            status_code: error.status_code,
            message: error.message,
            headers: error
                .headers
                .map(|headers| headers.into_iter().collect::<Vec<(String, String)>>()),
            request: Some(ErrorRequest {
                method: REQUEST_METHOD.to_owned(),
                url: REQUEST_URL.to_owned(),
            }),
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("FIREWORKS_API_KEY is not set")]
pub struct MissingApiKey;
