use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_prompts::error::FactoryError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WatsonxTextError {
    #[error(transparent)]
    Watsonx(Box<WatsonxError>),
    #[error(transparent)]
    Prompt(#[from] FactoryError),
    #[error("{field} must be a JSON object")]
    NotAMapping {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
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
    #[error("the watsonx_text reply has no results")]
    NoResults,
    #[error("the created_at value is not a timestamp")]
    CreatedAt(#[source] chrono::ParseError),
}

impl From<WatsonxError> for WatsonxTextError {
    fn from(error: WatsonxError) -> Self {
        Self::Watsonx(Box::new(error))
    }
}

impl WatsonxTextError {
    // LiteLLM raises WatsonXAIError(500) for an unusable reply and lets prompt failures escape as server errors
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
            Self::Prompt(_) | Self::NotAMapping { .. } | Self::InvalidField { .. } => None,
            Self::InvalidResponse { .. } | Self::NoResults | Self::CreatedAt(_) => Some(500),
        }
    }
}
