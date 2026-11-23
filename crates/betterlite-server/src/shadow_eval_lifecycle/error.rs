use std::error::Error as StdError;

use thiserror::Error;

use crate::shadow_eval_lifecycle::body::ShadowBodyError;
use crate::shadow_eval_models::ShadowEvalModelError;
use crate::shadow_eval_validation::error::{AccessError, ShadowEvalValidationError};
use crate::shadow_result_aggregation::error::ShadowResultsError;

// the detail LiteLLM's CommonProxyErrors.db_not_connected_error carries
pub const DB_NOT_CONNECTED: &str = "DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys";

#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error(transparent)]
    Access(#[from] AccessError),
    #[error(transparent)]
    Validation(#[from] ShadowEvalValidationError),
    #[error(transparent)]
    Body(#[from] ShadowBodyError),
    #[error(transparent)]
    Model(#[from] ShadowEvalModelError),
    #[error(transparent)]
    Results(#[from] ShadowResultsError),
    #[error("the caller's identity could not be read")]
    Caller(#[source] serde_json::Error),
    #[error("{}", DB_NOT_CONNECTED)]
    DatabaseUnavailable,
    #[error("shadow eval storage failed: {0}")]
    Database(#[source] Box<dyn StdError + Send + Sync>),
    #[error("Already in an active {direction} shadow eval job: {targets}. Stop it first.")]
    AlreadyClaimed { direction: String, targets: String },
    #[error(
        "A requested target was claimed by another {direction} shadow eval job concurrently. Stop it first."
    )]
    ConcurrentClaim { direction: String },
    #[error("No shadow eval job {job_id}")]
    JobNotFound { job_id: String },
    #[error("Job {job_id} is already {status}")]
    NotStoppable { job_id: String, status: String },
    #[error("the shadow eval window does not fit the clock")]
    WindowOverflow,
    #[error("the shadow eval job id source is exhausted")]
    IdsExhausted,
}

impl LifecycleError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Access(error) => error.status_code(),
            Self::Validation(error) => error.status_code(),
            Self::NotStoppable { .. } | Self::Body(_) => 400,
            Self::AlreadyClaimed { .. } | Self::ConcurrentClaim { .. } => 409,
            Self::JobNotFound { .. } => 404,
            Self::Model(_)
            | Self::Results(_)
            | Self::Caller(_)
            | Self::DatabaseUnavailable
            | Self::Database(_)
            | Self::WindowOverflow
            | Self::IdsExhausted => 500,
        }
    }
}
