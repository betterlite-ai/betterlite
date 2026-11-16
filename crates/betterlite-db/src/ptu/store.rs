use chrono::{DateTime, Utc};

use crate::postgres_store::errors::DbError;

pub mod models;
pub mod prune;
pub mod sentinel;
pub mod sleeper;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the stored PTU deployments could not be read")]
    Scan(#[source] DbError),
    #[error("the existing PTU sentinel rows could not be read")]
    Lookup(#[source] DbError),
    #[error("the PTU sentinel row could not be written")]
    Write(#[source] DbError),
    #[error("the stale PTU sentinel rows could not be deleted")]
    Prune(#[source] DbError),
    #[error(
        "the prune cutoff for a run started at {run_started} is before the representable range"
    )]
    GraceUnderflow { run_started: DateTime<Utc> },
}
