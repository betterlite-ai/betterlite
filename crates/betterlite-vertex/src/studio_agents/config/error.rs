use betterlite_core::api::agents::collection::AgentFailure;
use betterlite_core::api::agents::item::error::ProviderFailure;
use betterlite_http::interactions::base::ResponseHeaders;
use thiserror::Error;

use crate::studio_agents::error::StudioAgentError;

// a plain ValueError reaches exception_type() as APIConnectionError, and dispatch takes that path only below 400
const NO_STATUS: u16 = 0;
// docs/compatibility.md rejects a litellm_params value outside its declared type with a 400
const BAD_REQUEST: u16 = 400;

#[derive(Debug, Error)]
pub enum GeminiAgentError {
    #[error(transparent)]
    Studio(#[from] StudioAgentError),
    #[error("Gemini answered with status {status}: {message}")]
    Status {
        status: u16,
        message: String,
        headers: ResponseHeaders,
    },
    #[error("litellm_params.{field} does not have the type the Gemini agents API declares")]
    Params { field: &'static str },
}

impl GeminiAgentError {
    // exception_type() reads the message attribute: the provider body for a status error, the display text otherwise
    fn text(&self) -> String {
        if let Self::Status { message, .. } = self {
            return message.clone();
        }
        if let Self::Studio(StudioAgentError::Gemini { body, .. }) = self {
            return body.clone();
        }
        self.to_string()
    }

    const fn status(&self) -> Option<u16> {
        match self {
            Self::Studio(StudioAgentError::Gemini { status, .. }) | Self::Status { status, .. } => {
                Some(*status)
            }
            Self::Studio(
                StudioAgentError::CustomBaseWithoutKey
                | StudioAgentError::MissingApiKey
                | StudioAgentError::ResponseTooLarge { .. }
                | StudioAgentError::Shape { .. },
            )
            | Self::Params { .. } => None,
        }
    }
}

impl AgentFailure for GeminiAgentError {
    fn status_code(&self) -> Option<u16> {
        self.status()
    }

    fn message(&self) -> String {
        self.text()
    }
}

impl ProviderFailure for GeminiAgentError {
    fn status_code(&self) -> u16 {
        if matches!(self, Self::Params { .. }) {
            return BAD_REQUEST;
        }
        self.status().unwrap_or(NO_STATUS)
    }

    fn message(&self) -> String {
        self.text()
    }
}
