use std::error::Error;
use std::future::Future;

use rust_decimal::Decimal;

use super::values::ReseedCacheValue;

pub mod dual;

pub trait ReseedCache: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn redis_configured(&self) -> bool;

    fn redis_get(
        &self,
        counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, Self::Error>> + Send;

    fn redis_set_nx(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<Option<bool>, Self::Error>> + Send;

    fn memory_get(
        &self,
        counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, Self::Error>> + Send;

    fn memory_set(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn increment_local(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, Self::Error>> + Send;
}

// B2316's request-scoped batch: Some(Missing) is an authoritative Redis miss, None means no answer
pub trait ActiveSpendBatch: Send + Sync {
    fn read(
        &self,
        counter_key: &str,
    ) -> impl Future<Output = Result<Option<CounterValue>, SpendCounterBatchError>> + Send;

    fn record(&self, counter_key: &str, value: Decimal);
}
