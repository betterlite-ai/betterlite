use thiserror::Error;

use crate::native_compaction::error::NativeCompactionError;

#[derive(Debug, Error)]
pub enum DispatchError<RouterError> {
    #[error(transparent)]
    Payload {
        #[from]
        source: NativeCompactionError,
    },
    #[error("router call failed: {source}")]
    Router {
        #[source]
        source: RouterError,
    },
}
