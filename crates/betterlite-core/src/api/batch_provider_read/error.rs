use std::convert::Infallible;
use std::error::Error;

use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;
use crate::api::resource_sdk_settings::timeout::TimeoutError;

// the failures retrieve_batch raises itself before it reaches a provider
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadCallError {
    InvalidVertexCredentials,
    Unsupported { provider: String },
}

#[derive(Debug, Error)]
pub enum BatchProviderReadError<DispatchError = Infallible>
where
    DispatchError: Error + 'static,
{
    #[error(transparent)]
    Limits(#[from] ApiLimitError),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("vertex_credentials must be a string or an object")]
    InvalidVertexCredentials,
    #[error("retrieving a batch from provider `{provider}` needs a model, or is not supported")]
    Unsupported { provider: String },
    #[error(transparent)]
    Dispatch(DispatchError),
}

impl<DispatchError> From<ReadCallError> for BatchProviderReadError<DispatchError>
where
    DispatchError: Error + 'static,
{
    fn from(error: ReadCallError) -> Self {
        match error {
            ReadCallError::InvalidVertexCredentials => Self::InvalidVertexCredentials,
            ReadCallError::Unsupported { provider } => Self::Unsupported { provider },
        }
    }
}

// litellm's BadRequestError carries the status alone: openai's APIError leaves type, param and code null
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadErrorFields {
    pub status: u16,
}

impl<DispatchError> BatchProviderReadError<DispatchError>
where
    DispatchError: Error + 'static,
{
    // the status of the failures retrieve_batch raises itself; the provider's own answer keeps its own
    #[must_use]
    pub const fn error_fields(&self) -> Option<ReadErrorFields> {
        match self {
            Self::InvalidVertexCredentials | Self::Unsupported { .. } | Self::Timeout(_) => {
                Some(ReadErrorFields { status: 400 })
            }
            Self::Limits(error) => Some(ReadErrorFields {
                status: error.fields().status,
            }),
            Self::Dispatch(_) => None,
        }
    }
}
