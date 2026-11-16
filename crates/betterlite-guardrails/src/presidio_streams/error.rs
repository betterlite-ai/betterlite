use std::error::Error as StdError;

use thiserror::Error;
use tokio::sync::mpsc::error::SendError;

use super::StreamEvent;
use super::assembly::StreamAssembly;
use crate::cato::error::ModelStreamError;
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_response::error::PresidioResponseError;

// The error of a hook run whose helpers are Assembly
pub type StreamError<Assembly> = PresidioStreamError<<Assembly as StreamAssembly>::Error>;

#[derive(Debug, Error)]
pub enum PresidioStreamError<Failure>
where
    Failure: StdError + 'static,
{
    #[error("the upstream stream failed")]
    Upstream {
        #[source]
        source: ModelStreamError,
    },
    #[error("a streamed response could not be assembled or replayed")]
    Assembly {
        #[source]
        source: Failure,
    },
    #[error("a streamed event could not be read or written as JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Response(#[from] PresidioResponseError),
    #[error(transparent)]
    Actions(#[from] PresidioActionsError),
    // The reader dropped the stream, which ends LiteLLM's generator without an error
    #[error("the stream reader went away")]
    ConsumerClosed,
}

impl<Failure> PresidioStreamError<Failure>
where
    Failure: StdError + 'static,
{
    // LiteLLM's BlockedPiiEntityError, the one failure a stream never swallows
    #[must_use]
    pub const fn is_blocked(&self) -> bool {
        matches!(
            self,
            Self::Actions(PresidioActionsError::BlockedEntity { .. })
                | Self::Response(PresidioResponseError::Actions(
                    PresidioActionsError::BlockedEntity { .. }
                ))
        )
    }

    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Actions(source) => source.status(),
            Self::Response(source) => source.status(),
            Self::Upstream { .. }
            | Self::Assembly { .. }
            | Self::Json { .. }
            | Self::ConsumerClosed => 500,
        }
    }
}

impl<Failure> From<SendError<StreamEvent>> for PresidioStreamError<Failure>
where
    Failure: StdError + 'static,
{
    fn from(_closed: SendError<StreamEvent>) -> Self {
        Self::ConsumerClosed
    }
}
