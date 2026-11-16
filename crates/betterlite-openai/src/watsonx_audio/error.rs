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
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Watsonx(error) => api_status(error),
            Self::NotText { .. } | Self::InvalidReply { .. } | Self::MissingFields { .. } => None,
        }
    }
}

// only the shared credential and URL failures carry a status in the four watsonx endpoint families
#[must_use]
pub fn api_status(error: &WatsonxError) -> Option<u16> {
    if let WatsonxError::Api { status_code, .. } = error {
        Some(*status_code)
    } else {
        None
    }
}
