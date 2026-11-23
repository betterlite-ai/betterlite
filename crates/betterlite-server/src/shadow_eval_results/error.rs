use thiserror::Error;

use crate::shadow_eval_lifecycle::error::DB_NOT_CONNECTED;
use crate::shadow_eval_models::ShadowEvalModelError;
use crate::shadow_eval_validation::error::AccessError;
use crate::shadow_result_aggregation::error::ShadowResultsError;

use super::query::ShadowQueryError;

// the listing and detail endpoints' failures, each with the status LiteLLM answers
#[derive(Debug, Error)]
pub enum ShadowResultsRouteError {
    #[error(transparent)]
    Access(#[from] AccessError),
    #[error(transparent)]
    Query(#[from] ShadowQueryError),
    #[error(transparent)]
    Model(#[from] ShadowEvalModelError),
    #[error(transparent)]
    Results(#[from] ShadowResultsError),
    #[error("{}", DB_NOT_CONNECTED)]
    DatabaseUnavailable,
    #[error("target_type and target_id filter together; pass both or neither")]
    FilterPair,
    #[error("No shadow eval job {job_id}")]
    JobNotFound { job_id: String },
}

impl ShadowResultsRouteError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Access(error) => error.status_code(),
            Self::Query(_) => 422,
            Self::FilterPair => 400,
            Self::JobNotFound { .. } => 404,
            Self::Model(_) | Self::Results(_) | Self::DatabaseUnavailable => 500,
        }
    }
}
