use thiserror::Error;

use crate::routing_state_store::error::StateStoreError;
use crate::routing_value_store::StoreError;

// the ways a least-busy answer stops before it names a deployment
#[derive(Debug, Error)]
pub enum LeastBusyError {
    #[error("deployment {index} has no model_info.id to count in-flight calls under")]
    MissingDeploymentId { index: usize },
    #[error("the in-flight counter key cannot be built")]
    Key {
        #[source]
        source: StoreError,
    },
    #[error("this worker's in-flight counts could not be read")]
    Local {
        #[source]
        source: StoreError,
    },
    #[error("the value under {key} is not an in-flight count")]
    NotACount { key: String },
}

// _warn_unreadable and _warn_unwritable: the request goes on, and the caller logs what the optional tier could not do
#[derive(Debug)]
pub enum LeastBusyWarning {
    Unreadable {
        model_group: String,
        cause: UnreadableCounts,
    },
    Unwritable {
        key: String,
        cause: UnwrittenCount,
    },
}

// why the shared tier's counts could not be used for this selection
#[derive(Debug)]
pub enum UnreadableCounts {
    Store { operation: &'static str },
    Refused(StateStoreError),
    NotACount { key: String },
}

// why this worker's counter, or the shared floor, did not take the write
#[derive(Debug)]
pub enum UnwrittenCount {
    Local(StoreError),
    Shared(StateStoreError),
    NotACount,
}
