use std::error::Error as StdError;

use thiserror::Error;

use crate::cursor::CursorError;

#[derive(Debug, Error)]
pub enum StorageError<E: StdError> {
    #[error(transparent)]
    Cursor(#[from] CursorError),
    #[error("trace storage query failed: {0}")]
    Query(E),
}
