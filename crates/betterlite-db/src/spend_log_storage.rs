pub mod batch_claim;
pub mod poison;
pub mod postgres;
pub mod rows;
pub mod sql;

#[cfg(test)]
mod tests;

use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use crate::redis_spend::log_row::SpendLogRow;

pub type SpendLogFuture<'call, Value> = Pin<Box<dyn Future<Output = Value> + Send + 'call>>;

// PrismaDBExceptionHandler: what a failed spend log statement means for the flush retry and for poison isolation.
pub trait SpendLogWriteError: StdError + Send + Sync + 'static {
    // is_database_transport_error or is_deadlock_error: the write may succeed when it runs again
    fn is_transient(&self) -> bool;

    // the base prisma DataError: Postgres read the statement and refused the data, so one row may be poison
    fn is_data_rejection(&self) -> bool;

    // the database could not answer at all, so an outage must never be mistaken for a bad row
    fn is_service_unavailable(&self) -> bool;

    // P2034 and 40P01: a concurrent writer won, which only a retry settles
    fn is_deadlock(&self) -> bool;

    // is_prisma_error: the failure came from the database layer rather than from encoding or the external writer
    fn is_database(&self) -> bool;
}

// create_many(skip_duplicates=True): rows that already exist are skipped, and the answer counts what was inserted.
pub trait SpendLogsWriter: Send + Sync {
    type Error: SpendLogWriteError;

    fn insert_rows<'call>(
        &'call self,
        rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, Self::Error>>;
}
