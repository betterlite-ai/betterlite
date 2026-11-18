use std::time::Duration;

use crate::database_faults::{DbError, ErrorType};
use crate::postgres_store::errors::DbError as NativeDbError;
use crate::tool_records::decode::ToolRecordDecodeError;

#[derive(Debug, thiserror::Error)]
pub enum ToolPolicyStoreError {
    #[error("the tool policy read failed: {source}")]
    Read {
        error_type: ErrorType,
        #[source]
        source: NativeDbError,
        reconnect: Option<Box<DbError>>,
    },
    #[error("the tool policy read failed with only its classification: {source}")]
    ClassifiedRead {
        #[source]
        source: DbError,
    },
    #[error("the {lookup} lookup did not finish within {deadline:?}")]
    Deadline {
        lookup: &'static str,
        deadline: Duration,
    },
    #[error("a stored tool policy row could not be read: {source}")]
    Record {
        #[source]
        source: ToolRecordDecodeError,
    },
}
