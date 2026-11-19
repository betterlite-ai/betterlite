use betterlite_openai::copilot::api_key::error::ApiKeyError;
use betterlite_openai::error::OpenAiError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CopilotEmbeddingError {
    // LiteLLM raises AuthenticationError 401 for an absent or falsey key
    #[error("GitHub Copilot API key is required. Please authenticate via OAuth Device Flow.")]
    MissingKey { model: String },
    // every GetAPIKeyError becomes LiteLLM's AuthenticationError, which carries 401
    #[error("{source}")]
    Authentication {
        model: String,
        #[source]
        source: ApiKeyError,
    },
    #[error(transparent)]
    OpenAi(#[from] OpenAiError),
    #[error("the embeddings reply could not be decoded: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

impl CopilotEmbeddingError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::MissingKey { .. } | Self::Authentication { .. } => Some(401),
            Self::OpenAi(OpenAiError::Provider(error)) => Some(error.status_code),
            Self::OpenAi(_) | Self::Decode { .. } => None,
        }
    }
}
