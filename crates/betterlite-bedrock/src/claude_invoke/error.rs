use std::error::Error as StdError;

use betterlite_anthropic::request::error::RequestError;
use betterlite_prompts::media::error::MediaError;

use crate::bedrock_model_capabilities::CapabilityError;

use super::beta::InvokeBetaError;
use super::documents::DocumentError;
use super::tools::ToolSearchToolsError;

#[derive(Debug, thiserror::Error)]
pub enum ClaudeInvokeError<FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Request(#[from] RequestError<FileFailure>),
    #[error(transparent)]
    Media(#[from] MediaError),
    #[error(transparent)]
    Beta(#[from] InvokeBetaError),
    #[error(transparent)]
    Documents(#[from] DocumentError),
    #[error(transparent)]
    Tools(#[from] ToolSearchToolsError),
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error("the request body does not round-trip as JSON")]
    Body(#[source] serde_json::Error),
}

impl<FileFailure> ClaudeInvokeError<FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    // A failed media fetch is LiteLLM's ImageFetchError, a 400; the rest keep their own status or none
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Request(error) => error.status_code(),
            Self::Media(_) => Some(400),
            Self::Beta(error) => error.status_code(),
            Self::Documents(error) => error.status_code(),
            Self::Tools(error) => Some(error.status_code()),
            Self::Capability(_) | Self::Body(_) => None,
        }
    }
}
