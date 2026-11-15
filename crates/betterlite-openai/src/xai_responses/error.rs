use crate::xai_tokens::error::XaiOauthError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum XaiResponsesError {
    #[error(
        "XAI API key is required. Set api_key, litellm.xai_key, litellm.api_key, XAI_API_KEY, or use_xai_oauth=True."
    )]
    MissingApiKey,
    #[error("the xAI OAuth access token for {model} could not be obtained")]
    Authentication {
        model: String,
        #[source]
        source: XaiOauthError,
    },
}

impl XaiResponsesError {
    // LiteLLM's missing key is a bare ValueError while an OAuth failure is AuthenticationError
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingApiKey => None,
            Self::Authentication { .. } => Some(401),
        }
    }
}
