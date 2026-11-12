use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretString;

use super::tracker::{DrainTracker, TransactionHold};

pub type PoolFuture<'borrowed, T, E> =
    Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'borrowed>>;

pub type TransactionFuture<'borrowed, E> =
    Pin<Box<dyn Future<Output = Result<(), E>> + Send + 'borrowed>>;

// A native replacement may refine the pool shape; the transport is absent unless the caller supplies one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PoolTransport {
    pub max_size: Option<usize>,
}

pub trait PoolHandle: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn connect(&self, timeout: Option<Duration>) -> PoolFuture<'_, (), Self::Error>;

    fn close(&self);

    fn is_connected(&self) -> bool;

    fn query<'borrowed>(
        &'borrowed self,
        sql: &'borrowed str,
    ) -> PoolFuture<'borrowed, u64, Self::Error>;
}

pub trait PoolProvider: Send + Sync {
    type Handle: PoolHandle<Error = Self::Error>;
    type Error: Error + Send + Sync + 'static;

    fn build(
        &self,
        url: &SecretString,
        transport: Option<&PoolTransport>,
    ) -> Result<Self::Handle, Self::Error>;
}

pub trait TransactionControl {
    type Error: Error + Send + Sync + 'static;

    fn commit<'borrowed>(self) -> TransactionFuture<'borrowed, Self::Error>
    where
        Self: 'borrowed;

    fn rollback<'borrowed>(self) -> TransactionFuture<'borrowed, Self::Error>
    where
        Self: 'borrowed;
}

impl TransactionControl for deadpool_postgres::Transaction<'_> {
    type Error = tokio_postgres::Error;

    fn commit<'borrowed>(self) -> TransactionFuture<'borrowed, Self::Error>
    where
        Self: 'borrowed,
    {
        Box::pin(deadpool_postgres::Transaction::commit(self))
    }

    fn rollback<'borrowed>(self) -> TransactionFuture<'borrowed, Self::Error>
    where
        Self: 'borrowed,
    {
        Box::pin(deadpool_postgres::Transaction::rollback(self))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionEnd {
    Commit,
    Rollback,
}

pub struct OperationHandle<H> {
    pub handle: Arc<H>,
    pub tracker: Option<Arc<DrainTracker>>,
}

pub struct TrackedTransaction<T> {
    pub value: T,
    pub hold: Option<TransactionHold>,
}

impl<H> OperationHandle<H>
where
    H: PoolHandle,
{
    pub async fn query(&self, sql: &str) -> Result<u64, H::Error> {
        let _operation = self.tracker.as_ref().map(DrainTracker::begin_operation);
        self.handle.query(sql).await
    }

    pub async fn begin_transaction<T, E, F>(&self, begin: F) -> Result<TrackedTransaction<T>, E>
    where
        F: Future<Output = Result<T, E>>,
    {
        let hold = self.tracker.as_ref().map(DrainTracker::begin_transaction);
        let value = begin.await?;
        Ok(TrackedTransaction { value, hold })
    }

    pub async fn finish_transaction<T>(
        &self,
        transaction: TrackedTransaction<T>,
        end: TransactionEnd,
    ) -> Result<(), T::Error>
    where
        T: TransactionControl,
    {
        let _operation = self.tracker.as_ref().map(DrainTracker::begin_operation);
        let TrackedTransaction { value, hold: _hold } = transaction;
        match end {
            TransactionEnd::Commit => value.commit().await,
            TransactionEnd::Rollback => value.rollback().await,
        }
    }
}
