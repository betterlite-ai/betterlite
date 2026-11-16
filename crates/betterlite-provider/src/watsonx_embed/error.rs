use betterlite_openai::watsonx_audio::error::api_status;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WatsonxEmbeddingError {
    #[error(transparent)]
    Watsonx(Box<WatsonxError>),
    #[error("WatsonX embeddings require a string or list of strings")]
    UnsupportedInput,
    #[error("{field} must be a string")]
    InvalidField {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("Error: Invalid response from Watsonx.ai API: {body}")]
    InvalidResponse {
        body: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the watsonx embeddings reply has more results than an index can hold")]
    TooManyResults,
}

impl From<WatsonxError> for WatsonxEmbeddingError {
    fn from(error: WatsonxError) -> Self {
        Self::Watsonx(Box::new(error))
    }
}

impl WatsonxEmbeddingError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Watsonx(error) => api_status(error),
            Self::UnsupportedInput
            | Self::InvalidField { .. }
            | Self::InvalidResponse { .. }
            | Self::TooManyResults => None,
        }
    }
}
