use betterlite_openai::copilot::api_key::error::ApiKeyError;
use thiserror::Error;

use crate::api::anthropic_beta_filter::error::BetaFilterError;

#[derive(Debug, Error)]
pub enum CopilotMessagesError {
    // every GetAPIKeyError becomes LiteLLM's AuthenticationError, which carries 401
    #[error("{source}")]
    Authentication {
        model: String,
        #[source]
        source: ApiKeyError,
    },
    #[error(transparent)]
    Beta(#[from] BetaFilterError),
}

impl CopilotMessagesError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Authentication { .. } => Some(401),
            Self::Beta(_) => None,
        }
    }
}
