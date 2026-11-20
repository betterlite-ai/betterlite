use thiserror::Error;

use crate::postgres_store::errors::{DbError, backend};
use crate::window_queue::WindowQueueError;

#[derive(Debug, Error)]
pub enum WindowSpendError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Stamp(#[from] WindowQueueError),
    #[error("the budget window seed arithmetic overflows the decimal range")]
    Overflow,
    #[error("the budget window upserts did not finish within {seconds} seconds")]
    TransactionTimeout { seconds: u64 },
}

pub(super) fn database(source: tokio_postgres::Error) -> WindowSpendError {
    WindowSpendError::Database(backend(source))
}
