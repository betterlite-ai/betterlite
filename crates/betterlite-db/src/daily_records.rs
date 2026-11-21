pub mod baseline_targets;
pub mod callers;
pub mod metadata;
pub mod projection;

#[path = "daily_records/tests/support.rs"]
mod support;

use betterlite_models::savings_evidence::error::SavingsEvidenceError;

use crate::daily_spend_sql::tags::DailyTagsError;
use crate::spend_queues::SpendQueueError;

use self::metadata::DailyMetadataError;

// One logged request projected into the daily rollup tables: the source returns None where it skips a row and raises where the row cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum DailyRecordError {
    #[error("the request metadata could not be read")]
    Metadata {
        #[from]
        source: DailyMetadataError,
    },
    #[error("the request's savings evidence could not be read")]
    Evidence {
        #[from]
        source: SavingsEvidenceError,
    },
    #[error("the request tags could not be read")]
    Tags {
        #[from]
        source: DailyTagsError,
    },
    #[error("the daily record field {field} holds no value")]
    InvalidField { field: &'static str },
    #[error("the {entity} daily spend update could not be queued")]
    Enqueue {
        entity: &'static str,
        #[source]
        source: SpendQueueError,
    },
}
