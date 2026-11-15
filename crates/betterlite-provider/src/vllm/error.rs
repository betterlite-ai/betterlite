use std::collections::BTreeMap;

use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::passthrough::transformation::types::PassthroughError;
use thiserror::Error;

// an in-process vllm engine cannot run in Rust, so its paths answer 400 not supported (docs/deviations/B8153.yaml)
pub const ENGINE_STATUS: u16 = 400;
pub const ENGINE_MISSING_MESSAGE: &str = "No module named 'vllm'";

// common_utils.py declares VLLMError with headers, and handler.py declares a second one with status 0, so both map onto this type
#[derive(Debug, Error)]
pub enum VllmError {
    #[error("{message}")]
    Api {
        status_code: u16,
        message: String,
        headers: BTreeMap<String, String>,
    },
    #[error("{message}")]
    Engine {
        status_code: u16,
        message: &'static str,
    },
    #[error(transparent)]
    Setup(#[from] VllmSetupError),
    #[error(transparent)]
    Passthrough(#[from] PassthroughError),
    #[error("{}", .0.message)]
    Provider(ProviderError),
}

impl VllmError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status_code, .. } | Self::Engine { status_code, .. } => Some(*status_code),
            Self::Provider(error) => Some(error.status_code),
            Self::Setup(_) | Self::Passthrough(_) => None,
        }
    }
}

impl From<ProviderError> for VllmError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VllmSetupError {
    #[error(
        "VLLM_API_BASE is not set. Please set the environment variable, to use VLLM's pass-through - `{{LITELLM_API_BASE}}/vllm/{{endpoint}}`."
    )]
    MissingApiBase,
    #[error(
        "VLLM_API_BASE or VLLM_API_KEY is not set. Please set the environment variable, to query VLLM's `/models` endpoint."
    )]
    MissingModelsCredentials,
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> VllmError {
    VllmError::Api {
        status_code,
        message: message.to_owned(),
        headers: headers.clone(),
    }
}
