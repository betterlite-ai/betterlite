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
    // the embedding path raises WatsonXAIError only for credentials and URL failures; every reply failure stays untyped
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Watsonx(error) => match error.as_ref() {
                WatsonxError::Api { status_code, .. } => Some(*status_code),
                WatsonxError::IamApiKeyRequired
                | WatsonxError::InvalidField { .. }
                | WatsonxError::InvalidHeader { .. }
                | WatsonxError::IamUrl(_)
                | WatsonxError::IamStatus { .. }
                | WatsonxError::IamTransport(_)
                | WatsonxError::IamReply(_)
                | WatsonxError::IamReplyField { .. }
                | WatsonxError::Cache(_) => None,
            },
            Self::UnsupportedInput
            | Self::InvalidField { .. }
            | Self::InvalidResponse { .. }
            | Self::TooManyResults => None,
        }
    }
}
