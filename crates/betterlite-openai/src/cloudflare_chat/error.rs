use thiserror::Error;

use crate::error::{ErrorRequest, ErrorResponse, ProviderError};
use crate::openai_base_config::Headers;

const REQUEST_METHOD: &str = "POST";
const REQUEST_URL: &str = "https://api.cloudflare.com";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct CloudflareError {
    pub status_code: u16,
    pub message: String,
}

impl From<CloudflareError> for ProviderError {
    fn from(error: CloudflareError) -> Self {
        let request = ErrorRequest {
            method: REQUEST_METHOD.to_owned(),
            url: REQUEST_URL.to_owned(),
        };
        Self {
            status_code: error.status_code,
            status_code_is_synthesized: false,
            message: error.message,
            headers: None,
            response: ErrorResponse {
                status_code: error.status_code,
                request: request.clone(),
            },
            request,
            body: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CloudflareChatError {
    #[error(
        "Missing CLOUDFLARE_ACCOUNT_ID - set CLOUDFLARE_ACCOUNT_ID in the environment or pass api_base explicitly"
    )]
    MissingAccountId,
    #[error(
        "Missing Cloudflare API Key - A call is being made to cloudflare but no key is set either in the environment variables or via params"
    )]
    MissingApiKey,
}

// LiteLLM's CloudflareError never carries headers, whatever the caller passed
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, _headers: &Headers) -> CloudflareError {
    CloudflareError {
        status_code,
        message: message.to_owned(),
    }
}
