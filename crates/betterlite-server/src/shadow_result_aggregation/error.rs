use betterlite_db::postgres_store::errors::DbError;
use thiserror::Error;

use crate::shadow_eval_job_loading::jobs::ShadowJobError;

#[derive(Debug, Error)]
pub enum ShadowResultsError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    InvalidJob(#[from] ShadowJobError),
    #[error("target type {target_type} is not key, team or user")]
    UnknownTargetType { target_type: String },
    #[error("the {name} value has no Decimal representation")]
    Unrepresentable { name: &'static str },
    #[error("a shadow eval job has at least one leg")]
    NoLegs,
    #[error("the attempt totals read returned no row")]
    NoTotalsRow,
    #[error("attempts name leg {leg_id}, which is not part of the job")]
    UnknownLeg { leg_id: String },
}
