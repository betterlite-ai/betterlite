use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};

use super::params::ParamsError;
use super::submit::SubmitError;
use crate::api::bfl::common::BflError;
use crate::api::bfl::poll::PollError;

#[derive(Debug, thiserror::Error)]
pub enum GenerationCallError {
    #[error(transparent)]
    Bfl(#[from] BflError),
    #[error(transparent)]
    Params(#[from] ParamsError),
    #[error("the BFL extra header {name} cannot be carried")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the BFL extra header {name} holds a value a header cannot carry")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the BFL extra header {name} holds a value that is not ASCII")]
    HeaderNotAscii { name: String },
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("the BFL submission request failed")]
    Submission {
        #[source]
        source: SubmitError,
    },
    #[error("the BFL submission answered HTTP {status}")]
    SubmissionStatus { status: u16 },
    #[error(transparent)]
    Poll(#[from] PollError),
}

impl GenerationCallError {
    // LiteLLM wraps every submission failure in a BFL error with status 500, whatever the transport or provider answered
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Bfl(error) => error.status(),
            Self::Poll(error) => error.status(),
            Self::Submission { .. }
            | Self::SubmissionStatus { .. }
            | Self::HeaderName { .. }
            | Self::HeaderValue { .. }
            | Self::HeaderNotAscii { .. } => Some(500),
            Self::Params(_) | Self::Logging(_) => None,
        }
    }
}
