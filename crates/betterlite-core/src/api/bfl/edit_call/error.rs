use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};

use crate::api::bfl::common::BflError;
use crate::api::bfl::edit::EditError;
use crate::api::bfl::generation_call::params::ParamsError;
use crate::api::bfl::generation_call::submit::SubmitError;
use crate::api::bfl::poll::PollError;

#[derive(Debug, thiserror::Error)]
pub enum EditCallError {
    #[error(transparent)]
    Bfl(#[from] BflError),
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error(transparent)]
    Params(#[from] ParamsError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    // BlackForestLabsImageEdit raises this before it transforms or logs, so neither a record nor a POST exists
    #[error("No image provided")]
    NoImageProvided,
    #[error("image_edit_optional_request_params.extra_headers must be an object")]
    InnerHeadersType,
    #[error("image_edit_optional_request_params.extra_headers.{name} must be a string")]
    InnerHeaderValue { name: String },
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
    #[error("the BFL submission request failed")]
    Submission {
        #[source]
        source: SubmitError,
    },
    #[error("the BFL submission answered HTTP {status}")]
    SubmissionStatus { status: u16 },
    #[error(transparent)]
    Poll(#[from] PollError),
    #[error("the BFL edit poll finished without a response to project")]
    MissingCompletedResponse,
}

impl EditCallError {
    // a submission failure is LiteLLM's 500 whatever the transport said; the poll and the edit config keep their own status
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Bfl(error) => error.status(),
            Self::Edit(error) => error.status(),
            Self::Poll(error) => error.status(),
            Self::NoImageProvided => Some(400),
            Self::HeaderName { .. }
            | Self::HeaderValue { .. }
            | Self::HeaderNotAscii { .. }
            | Self::Submission { .. }
            | Self::SubmissionStatus { .. }
            | Self::MissingCompletedResponse => Some(500),
            Self::Params(_)
            | Self::Logging(_)
            | Self::InnerHeadersType
            | Self::InnerHeaderValue { .. } => None,
        }
    }
}
