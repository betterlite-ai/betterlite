use crate::database_faults::DbError;
use crate::tool_records::decode::ToolRecordDecodeError;

#[derive(Debug, thiserror::Error)]
pub enum ToolPolicyStoreError {
    #[error("the tool policy read failed: {source}")]
    Read {
        #[source]
        source: DbError,
    },
    #[error("a stored tool policy row could not be read: {source}")]
    Record {
        #[source]
        source: ToolRecordDecodeError,
    },
}
