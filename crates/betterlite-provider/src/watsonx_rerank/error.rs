use betterlite_openai::watsonx_auth::error::WatsonxError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WatsonxRerankError {
    #[error(transparent)]
    Watsonx(Box<WatsonxError>),
    #[error("the rerank parameters are not valid JSON: {source}")]
    Json(#[from] serde_json::Error),
    #[error("Failed to parse response: {source}")]
    Parse(#[source] serde_json::Error),
    #[error("the rerank reply does not match the record shape: {source}")]
    Reply(#[source] serde_json::Error),
    #[error("the reply field {field} has the wrong shape")]
    ReplyField { field: &'static str },
    #[error("{field} must be a string")]
    NotText { field: &'static str },
    #[error("documents must be a list of strings or objects")]
    InvalidDocuments,
    #[error("No results found in the response={body}")]
    NoResults { body: String },
}

impl From<WatsonxError> for WatsonxRerankError {
    fn from(error: WatsonxError) -> Self {
        Self::Watsonx(Box::new(error))
    }
}

impl WatsonxRerankError {
    // only the shared credential and URL failures carry a status; the rerank path leaves reply failures untyped
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
            Self::Json(_)
            | Self::Parse(_)
            | Self::Reply(_)
            | Self::ReplyField { .. }
            | Self::NotText { .. }
            | Self::InvalidDocuments
            | Self::NoResults { .. } => None,
        }
    }
}
