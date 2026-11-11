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
    #[error("`fallback_sap_modules` must be a list")]
    FallbackModules,
    #[error("each entry in `fallback_sap_modules` must be an object")]
    FallbackEntry,
    #[error("Each entry in `fallback_sap_modules` must include a 'model' key.")]
    FallbackModel,
    #[error("`fallback_sap_modules` messages must be a list")]
    FallbackMessages,
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
        match error {
            wrapped @ (SapAuthError::NoCredentials
            | SapAuthError::MissingCredentials
            | SapAuthError::IncompleteCredentials) => Self::Orchestration(SapOrchestrationError {
                status_code: CREDENTIALS_STATUS,
                message: wrapped.to_string(),
            }),
            other @ (SapAuthError::ConfigFileNotFound { .. }
            | SapAuthError::ConfigNotJson { .. }
            | SapAuthError::ConfigRead { .. }
            | SapAuthError::TokenRequest { .. }
            | SapAuthError::NoOrchestrationDeployment
            | SapAuthError::DeploymentEntry
            | SapAuthError::ConfigurationLookup { .. }
            | SapAuthError::InvalidHeader { .. }) => Self::Auth(other),
        }
    }

    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Orchestration(error) => Some(error.status_code),
            Self::Auth(_)
            | Self::Model(_)
            | Self::FallbackModules
            | Self::FallbackEntry
            | Self::FallbackModel
            | Self::FallbackMessages
            | Self::Tools
            | Self::ResponseFormat
            | Self::PlaceholderDefaults
            | Self::StreamOptions
            | Self::Response(_) => None,
        }
    }
}
