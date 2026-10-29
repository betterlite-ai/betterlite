use std::error::Error as StdError;

use thiserror::Error;

use crate::cursor::CursorError;

#[derive(Debug, Error)]
pub enum StorageError<E>
where
    E: StdError,
{
    #[error(transparent)]
    Cursor(#[from] CursorError),
    #[error("trace storage query failed")]
    Query(#[source] E),
}
