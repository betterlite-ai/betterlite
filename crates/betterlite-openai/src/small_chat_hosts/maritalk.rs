use thiserror::Error;

use crate::error::{ProviderError, ProviderErrorParts};
use crate::openai_base_config::Headers;

pub const SUPPORTED_PARAMS: [&str; 12] = [
    "frequency_penalty",
    "presence_penalty",
    "top_p",
    "top_k",
    "temperature",
    "max_tokens",
    "n",
    "stop",
    "stream",
    "stream_options",
    "tools",
    "tool_choice",
];

// LiteLLM's MaritalkError is a BaseLLMException, which B2202 projects onto ProviderError
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}", .error.message)]
pub struct MaritalkError {
    pub error: ProviderError,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MaritalkConfig;

impl MaritalkConfig {
    // the twelve params are the whole list, in this order
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - the config API fixes this name and the list is the whole body
    pub fn supported_params() -> Vec<&'static str> {
        SUPPORTED_PARAMS.to_vec()
    }

    // the reply that failed keeps its own status, message and headers
    #[must_use]
    pub fn get_error_class(
        message: &str,
        status_code: u16,
        headers: Option<&Headers>,
    ) -> MaritalkError {
        MaritalkError {
            error: ProviderError::from(ProviderErrorParts {
                status_code,
                message: message.to_owned(),
                headers: headers.map(|given| {
                    given
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect()
                }),
                request: None,
                response: None,
                body: None,
                status_code_is_synthesized: false,
            }),
        }
    }
}
