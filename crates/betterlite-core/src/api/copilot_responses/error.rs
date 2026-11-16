use betterlite_openai::copilot::api_key::error::ApiKeyError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CopilotResponsesError {
    #[error("GitHub Copilot API key is required. Please authenticate via OAuth Device Flow.")]
    MissingKey { model: String },
    // LiteLLM turns GetAPIKeyError into AuthenticationError and keeps its message
    #[error("{source}")]
    Authentication {
        model: String,
        #[source]
        source: ApiKeyError,
    },
    // a failed GitHub login is not a GetAPIKeyError, so it keeps its own status
    #[error("the Copilot login failed")]
    Login(#[source] ApiKeyError),
}

impl CopilotResponsesError {
    #[must_use]
    pub fn status(&self) -> u16 {
        match self {
            Self::MissingKey { .. } | Self::Authentication { .. } => 401,
            Self::Login(source) => source.status(),
        }
    }
}
