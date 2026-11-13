use std::error::Error;
use std::future::Future;
use std::num::TryFromIntError;
use std::time::Duration;

use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};
use thiserror::Error;

// the reserve script answers {1, updated} on a grant and {0, current} past the limit
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReserveScriptReply {
    pub status: i64,
    pub count: i64,
}

pub trait ReservationScripts: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn reserve(
        &self,
        counter_key: &str,
        tokens: i64,
        ttl_seconds: i64,
        limit: i64,
    ) -> impl Future<Output = Result<ReserveScriptReply, Self::Error>> + Send;

    fn refund(
        &self,
        counter_key: &str,
        tokens: i64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

#[derive(Debug, Error)]
pub enum BatchEnqueuedReserveError {
    #[error("the local enqueued-token counter cache failed")]
    CounterCache(#[from] CacheError),
    #[error("the stored enqueued-token counter is not an integer")]
    StoredCounter,
    #[error("the enqueued-token counter ttl is out of range")]
    TtlOutOfRange {
        #[source]
        source: TryFromIntError,
    },
}

pub trait EnqueuedTokenCounters: Send + Sync {
    fn get(&mut self, counter_key: &str) -> Result<Option<i64>, BatchEnqueuedReserveError>;

    fn set(
        &mut self,
        counter_key: &str,
        value: i64,
        ttl_seconds: i64,
    ) -> Result<(), BatchEnqueuedReserveError>;
}

// the in-memory tier of the source's internal usage cache, keyed by counter and expired after the ttl
pub struct LocalTokenCounters {
    pub cache: LocalTtlCache,
}

impl EnqueuedTokenCounters for LocalTokenCounters {
    fn get(&mut self, counter_key: &str) -> Result<Option<i64>, BatchEnqueuedReserveError> {
        match self.cache.get(counter_key)? {
            Some(CacheValue::Integer(value)) => Ok(Some(value)),
            Some(_) => Err(BatchEnqueuedReserveError::StoredCounter),
            None => Ok(None),
        }
    }

    fn set(
        &mut self,
        counter_key: &str,
        value: i64,
        ttl_seconds: i64,
    ) -> Result<(), BatchEnqueuedReserveError> {
        let seconds = u64::try_from(ttl_seconds)
            .map_err(|source| BatchEnqueuedReserveError::TtlOutOfRange { source })?;
        let ttl = Duration::from_secs(seconds).as_secs_f64();
        self.cache
            .set(counter_key, CacheValue::Integer(value), Some(ttl))?;
        Ok(())
    }
}
