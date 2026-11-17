use std::str;

use thiserror::Error;

use crate::watsonx_auth::error::WatsonxError;

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
pub const fn api_status(error: &WatsonxError) -> Option<u16> {
    if let WatsonxError::Api { status_code, .. } = error {
        Some(*status_code)
    } else {
        None
    }
}

// the body of a reply is shown as text only when it is UTF-8, and its length stands in otherwise
#[must_use]
pub fn body_text(bytes: &[u8]) -> String {
    let Ok(text) = str::from_utf8(bytes) else {
        return format!("{} bytes that are not UTF-8", bytes.len());
    };
    text.to_owned()
}
