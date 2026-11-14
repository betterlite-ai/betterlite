use std::future::Future;
use std::time::Duration;

use betterlite_context::local_ttl_cache::CacheValue;

use crate::api::batch_enqueued_reserve::store::{
    BatchEnqueuedReserveError, EnqueuedTokenCounters, LocalTokenCounters, ReservationScripts,
};

use super::BatchEnqueuedRefundError;

// the save and pop scripts a Redis client registers beside the reserve and refund ones
pub trait ReservationRecordScripts: ReservationScripts {
    fn save(
        &self,
        record_key: &str,
        record: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn pop(
        &self,
        record_key: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, Self::Error>> + Send;
}

// the in-memory tier the source reaches through InternalUsageCache: counters, plus the records a save leaves behind
pub trait EnqueuedLocalStore: Send + Sync {
    fn counter(&mut self, counter_key: &str) -> Result<i64, BatchEnqueuedRefundError>;

    fn set_counter(
        &mut self,
        counter_key: &str,
        value: i64,
        ttl_seconds: i64,
    ) -> Result<(), BatchEnqueuedRefundError>;

    fn delete(&mut self, key: &str);

    fn record(&mut self, record_key: &str) -> Result<Option<Vec<u8>>, BatchEnqueuedRefundError>;

    fn set_record(
        &mut self,
        record_key: &str,
        record: &str,
        ttl_seconds: i64,
    ) -> Result<(), BatchEnqueuedRefundError>;
}

impl EnqueuedLocalStore for LocalTokenCounters {
    fn counter(&mut self, counter_key: &str) -> Result<i64, BatchEnqueuedRefundError> {
        Ok(EnqueuedTokenCounters::get(self, counter_key)?.unwrap_or(0))
    }

    fn set_counter(
        &mut self,
        counter_key: &str,
        value: i64,
        ttl_seconds: i64,
    ) -> Result<(), BatchEnqueuedRefundError> {
        EnqueuedTokenCounters::set(self, counter_key, value, ttl_seconds)?;
        Ok(())
    }

    fn delete(&mut self, key: &str) {
        self.cache.delete(key);
    }

    fn record(&mut self, record_key: &str) -> Result<Option<Vec<u8>>, BatchEnqueuedRefundError> {
        match self.cache.get(record_key)? {
            Some(CacheValue::Bytes(bytes)) => Ok(Some(bytes)),
            // a credential never lands under a reservation key, so it is no record at all
            Some(CacheValue::Credential(_)) => Ok(None),
            Some(value) => serde_json::to_vec(&value).map(Some).map_err(|source| {
                BatchEnqueuedRefundError::Encode {
                    source: Box::new(source),
                }
            }),
            None => Ok(None),
        }
    }

    fn set_record(
        &mut self,
        record_key: &str,
        record: &str,
        ttl_seconds: i64,
    ) -> Result<(), BatchEnqueuedRefundError> {
        let ttl = cache_ttl(ttl_seconds)?;
        self.cache
            .set(record_key, CacheValue::Text(record.to_owned()), Some(ttl))?;
        Ok(())
    }
}

fn cache_ttl(ttl_seconds: i64) -> Result<f64, BatchEnqueuedRefundError> {
    let seconds = u64::try_from(ttl_seconds)
        .map_err(|source| BatchEnqueuedReserveError::TtlOutOfRange { source })?;
    Ok(Duration::from_secs(seconds).as_secs_f64())
}
