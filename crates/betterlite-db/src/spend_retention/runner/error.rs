use crate::postgres_store::errors::DbError;
use crate::spend_partitions::manager::PartitionMaintenanceError;

// what escapes cleanup_old_spend_logs: the source's finally-block release and the session the job handle opens
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Release(#[from] LockReleaseError),
    #[error("the spend-log cleanup database session could not be opened")]
    Session {
        #[from]
        source: DbError,
    },
}

// a release that raises in the source's finally block replaces the run's result instead of being swallowed
#[derive(Debug, thiserror::Error)]
#[error("the spend-log cleanup lock could not be released")]
pub struct LockReleaseError;

// an error inside the run body: the source logs its type, counts the run as aborted and returns normally
#[derive(Debug, thiserror::Error)]
pub enum RunFailure {
    #[error("PartitionMaintenanceError: {0}")]
    Partition(#[from] PartitionMaintenanceError),
    #[error("the retention cutoff is not a representable date")]
    Cutoff,
}
