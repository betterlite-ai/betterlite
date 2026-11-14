use betterlite_roi::estimator::extraction::ExtractionError;
use betterlite_roi::estimator::json::JsonError;
use thiserror::Error;

use crate::roi_github::error::SourceError;

#[derive(Debug, Error)]
#[error(
    "The estimator did not return valid hours and reasoning. Check the selected model and prompt."
)]
pub struct EstimatorError {
    #[source]
    pub(super) source: CompletionFailure,
}

#[derive(Debug, Error)]
pub enum CompletionFailure {
    #[error("the completion callback failed")]
    Callback {
        #[source]
        source: SourceError,
    },
    #[error("the estimate evidence could not be serialized")]
    Evidence {
        #[source]
        source: JsonError,
    },
    #[error("the completion response does not match the completion schema")]
    Response {
        #[source]
        source: serde_json::Error,
    },
    #[error("the completion response has no choices")]
    NoChoices,
    #[error("the completion response is incomplete")]
    Incomplete,
    #[error("the completion content could not be searched for a JSON object")]
    Extraction {
        #[source]
        source: ExtractionError,
    },
    #[error("the completion content does not match the estimate schema")]
    Estimate {
        #[source]
        source: serde_json::Error,
    },
}
