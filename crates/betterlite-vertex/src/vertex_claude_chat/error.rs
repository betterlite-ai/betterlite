use std::error::Error as StdError;

use betterlite_anthropic::request::error::RequestError;
use thiserror::Error;

use super::beta::BetaError;

#[derive(Debug, Error)]
pub enum ClaudeChatError<FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Request(#[from] RequestError<FileFailure>),
    #[error(transparent)]
    Beta(#[from] BetaError),
}

impl<FileFailure> ClaudeChatError<FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Request(error) => error.status_code(),
            Self::Beta(error) => error.status_code(),
        }
    }
}
