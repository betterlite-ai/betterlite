use betterlite_anthropic::count_tokens::error::CountTokensError;
use betterlite_azure::credentials::selection::AzureTokenError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProcessingError {
    #[error(transparent)]
    Request(#[from] CountTokensError),
    #[error(transparent)]
    Environment(#[from] AzureTokenError),
}

// LiteLLM re-raises AnthropicError untouched and wraps every other exception in a 500 AnthropicError
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("{text}")]
    Upstream { status_code: u16, text: String },
    #[error("CountTokens processing error: {source}")]
    Processing {
        #[from]
        source: ProcessingError,
    },
}

impl ApiError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Upstream { status_code, .. } => *status_code,
            Self::Processing { .. } => 500,
        }
    }
}

impl From<CountTokensError> for ApiError {
    fn from(source: CountTokensError) -> Self {
        Self::Processing {
            source: ProcessingError::Request(source),
        }
    }
}

impl From<AzureTokenError> for ApiError {
    fn from(source: AzureTokenError) -> Self {
        Self::Processing {
            source: ProcessingError::Environment(source),
        }
    }
}
