use serde_json::Error as JsonError;
use thiserror::Error;

use crate::sap_auth::error::SapAuthError;
use crate::sap_models::SapModelError;
use crate::sap_stream::SapOrchestrationError;

pub const CREDENTIALS_STATUS: u16 = 400;

#[derive(Debug, Error)]
pub enum SapChatError {
    #[error(transparent)]
    Auth(#[from] SapAuthError),
    #[error(transparent)]
    Model(#[from] SapModelError),
    #[error(transparent)]
    Orchestration(#[from] SapOrchestrationError),
    #[error("{0}")]
    Fallback(&'static str),
    #[error("`tools` must be a list")]
    Tools,
    #[error("`response_format` must be an object")]
    ResponseFormat,
    #[error("`placeholder_defaults` must be an object")]
    PlaceholderDefaults,
    #[error("`stream_options` must be an object")]
    StreamOptions,
    #[error("the SAP orchestration response is not the JSON a completion returns")]
    Response(#[source] JsonError),
}

impl SapChatError {
    // LiteLLM wraps a credentials ValueError as a 400 and lets every other setup failure through
    pub(crate) fn credentials(error: SapAuthError) -> Self {
        let wrapped = matches!(
            error,
            SapAuthError::NoCredentials
                | SapAuthError::MissingCredentials
                | SapAuthError::IncompleteCredentials
        );
        if wrapped {
            return Self::Orchestration(SapOrchestrationError {
                status_code: CREDENTIALS_STATUS,
                message: error.to_string(),
            });
        }
        Self::Auth(error)
    }

    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        if let Self::Orchestration(error) = self {
            return Some(error.status_code);
        }
        None
    }
}
