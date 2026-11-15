use betterlite_openai::api::image_contracts::records::ImageHeaders;
use thiserror::Error;

use crate::api::runway::tasks::error::RunwayTaskError;

#[derive(Debug, Error)]
pub enum RunwayImageError {
    #[error(transparent)]
    Task(#[from] RunwayTaskError),
    #[error("the RunwayML image submission answer is not a task")]
    Submission {
        status: u16,
        headers: ImageHeaders,
        #[source]
        source: RunwayTaskError,
    },
    #[error("RunwayML response missing task ID")]
    MissingTaskId,
    #[error("Parameter {parameter} is not supported for model {model}")]
    UnsupportedParameter { parameter: String, model: String },
    #[error("the size option must be a string")]
    InvalidSize,
}

impl RunwayImageError {
    // Only a submission that fails to decode carries the inherited image error's own HTTP status and headers; every later poll failure keeps its task cause instead.
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Submission { status, .. } => Some(*status),
            Self::Task(_)
            | Self::MissingTaskId
            | Self::UnsupportedParameter { .. }
            | Self::InvalidSize => None,
        }
    }

    #[must_use]
    pub const fn headers(&self) -> Option<&ImageHeaders> {
        match self {
            Self::Submission { headers, .. } => Some(headers),
            Self::Task(_)
            | Self::MissingTaskId
            | Self::UnsupportedParameter { .. }
            | Self::InvalidSize => None,
        }
    }
}
