use std::convert::Infallible;

use crate::api::messages_context_clear::error::ContextManagementError;
use crate::api::summary_execution::extract::ExtractionError;
use crate::api::summary_history::error::SummaryHistoryError;
use crate::api::summary_model_access::error::SummaryAccessError;
use crate::api::summary_model_limits::error::SummaryLimitsError;

// an editor failure, split by the caller each check's error came from; only the trigger error is the client's 400
#[derive(Debug, thiserror::Error)]
pub enum SummaryExecutionError<HookError> {
    #[error(transparent)]
    Trigger(#[from] ContextManagementError),
    #[error("the summary model access check could not read the principal")]
    Access(#[source] SummaryAccessError),
    #[error("the summary model budget check could not read the principal")]
    Budget(#[source] SummaryLimitsError<Infallible>),
    #[error("the summary rate-limit check failed")]
    Rate(#[source] SummaryLimitsError<HookError>),
    #[error(transparent)]
    History(#[from] SummaryHistoryError),
    #[error(transparent)]
    Extraction(#[from] ExtractionError),
    #[error("the principal's allowed model region could not be read")]
    Principal(#[source] serde_json::Error),
    #[error("the summary result could not be encoded")]
    Encode(#[source] serde_json::Error),
}
