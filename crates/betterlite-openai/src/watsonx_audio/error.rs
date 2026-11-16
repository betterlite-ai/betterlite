use crate::watsonx_auth::error::WatsonxError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WatsonxAudioError {
    #[error(transparent)]
    Watsonx(Box<WatsonxError>),
    #[error("{field} must be a string")]
    NotText { field: &'static str },
    #[error("Error transforming response to json: {source}")]
    InvalidReply {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "Invalid response format. Received response does not match the expected format. Got: {body}"
    )]
    MissingFields { body: String },
}

impl From<WatsonxError> for WatsonxAudioError {
    fn from(error: WatsonxError) -> Self {
        Self::Watsonx(Box::new(error))
    }
}

impl WatsonxAudioError {
    // the audio config puts IBMWatsonXMixin first, so credentials and URL failures carry its status and the rest stay untyped
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
            Self::NotText { .. } | Self::InvalidReply { .. } | Self::MissingFields { .. } => None,
        }
    }
}
