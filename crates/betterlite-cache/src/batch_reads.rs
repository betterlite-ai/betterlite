mod apply;
mod async_read;
mod declared;
mod prepare;
mod reserve;
mod shared;
mod sync_read;
mod value;

#[path = "batch_reads/tests.rs"]
mod tests;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redis::batch::MgetValues;
use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use thiserror::Error;

use crate::backend::BackendError;
use crate::backend::memory::MemoryCacheOps;
use crate::dual_cache::DualCache;

// the memory half of a batch read, with its Redis keys reserved but not fetched yet
#[derive(Clone, Debug)]
pub struct PendingBatchRead {
    pub keys: Vec<String>,
    pub result: Vec<Option<Json>>,
    pub redis_keys: Vec<String>,
    pub previous_access_times: BTreeMap<String, Option<f64>>,
}

// a batch read split in two so its MGET rides the request pipeline's next round trip
pub struct DeclaredBatchRead {
    pub keys: Vec<String>,
    pub pending: PendingBatchRead,
    pub result: Option<BatchResult<MgetValues>>,
}

// LiteLLM passes these as keyword arguments; a struct keeps two booleans out of the call signatures
#[derive(Clone, Copy, Debug)]
pub struct BatchReadOptions {
    pub local_only: bool,
    pub throttle_redis: bool,
}

#[derive(Debug, Error)]
pub enum BatchReadsError {
    #[error("the in-memory cache tier failed")]
    Memory(#[from] BackendError),
    #[error("this cache value has no JSON representation")]
    UnrepresentableValue,
    #[error(transparent)]
    Redis(#[from] RedisCacheError),
    #[error(transparent)]
    RedisValues(#[from] RedisCacheValuesError),
    #[error(transparent)]
    Batch(Arc<BatchError>),
}

impl BatchReadsError {
    // LiteLLM treats an open circuit breaker as a memory-only read and every other failure as no read at all
    pub const fn circuit_open(&self) -> bool {
        matches!(
            self,
            Self::Redis(RedisCacheError::Breaker(_))
                | Self::RedisValues(RedisCacheValuesError::BreakerOpen(_))
        )
    }
}

// the workspace denies clippy::multiple_inherent_impl and B4293 owns DualCache's one inherent impl, so these live on a trait
pub trait BatchReads {
    fn reserve_redis_batch_keys(
        &self,
        current_time: f64,
        keys: &[String],
        result: &[Option<Json>],
    ) -> (Vec<String>, BTreeMap<String, Option<f64>>);

    fn rollback_redis_batch_keys(&self, previous_access_times: &BTreeMap<String, Option<f64>>);

    fn reserve_redis_batch_reads(
        &self,
        keys: &[String],
    ) -> (Vec<String>, BTreeMap<String, Option<f64>>);

    fn batch_get_cache(&self, keys: &[String], local_only: bool) -> Option<Vec<Option<Json>>>;

    fn prepare_batch_get(
        &self,
        keys: &[String],
        options: BatchReadOptions,
    ) -> Result<PendingBatchRead, BatchReadsError>;

    fn apply_batch_get(
        &self,
        pending: &PendingBatchRead,
        redis_result: Option<&BTreeMap<String, Option<Json>>>,
    ) -> Result<Vec<Option<Json>>, BatchReadsError>;

    fn async_batch_get_cache(
        &self,
        keys: &[String],
        options: BatchReadOptions,
    ) -> impl Future<Output = Option<Vec<Option<Json>>>> + Send;

    fn declare_batch_get(
        &self,
        keys: &[String],
        batch: &Arc<RedisBatch>,
    ) -> Result<DeclaredBatchRead, BatchReadsError>;

    fn async_resolve_batch_get(
        &self,
        declared: DeclaredBatchRead,
    ) -> impl Future<Output = Result<Vec<Option<Json>>, BatchReadsError>> + Send;

    fn async_batch_get_cache_shared(
        reads: &[(&Self, &[String])],
    ) -> impl Future<Output = Vec<Option<Vec<Option<Json>>>>> + Send;
}

impl<Memory> BatchReads for DualCache<Memory>
where
    Memory: MemoryCacheOps,
{
    fn reserve_redis_batch_keys(
        &self,
        current_time: f64,
        keys: &[String],
        result: &[Option<Json>],
    ) -> (Vec<String>, BTreeMap<String, Option<f64>>) {
        reserve::reserve_redis_batch_keys(self, current_time, keys, result)
    }

    fn rollback_redis_batch_keys(&self, previous_access_times: &BTreeMap<String, Option<f64>>) {
        reserve::rollback_redis_batch_keys(self, previous_access_times);
    }

    fn reserve_redis_batch_reads(
        &self,
        keys: &[String],
    ) -> (Vec<String>, BTreeMap<String, Option<f64>>) {
        reserve::reserve_redis_batch_reads(self, keys)
    }

    fn batch_get_cache(&self, keys: &[String], local_only: bool) -> Option<Vec<Option<Json>>> {
        sync_read::batch_get_cache(self, keys, local_only)
    }

    fn prepare_batch_get(
        &self,
        keys: &[String],
        options: BatchReadOptions,
    ) -> Result<PendingBatchRead, BatchReadsError> {
        prepare::prepare_batch_get(self, keys, options)
    }

    fn apply_batch_get(
        &self,
        pending: &PendingBatchRead,
        redis_result: Option<&BTreeMap<String, Option<Json>>>,
    ) -> Result<Vec<Option<Json>>, BatchReadsError> {
        apply::apply_batch_get(self, pending, redis_result)
    }

    async fn async_batch_get_cache(
        &self,
        keys: &[String],
        options: BatchReadOptions,
    ) -> Option<Vec<Option<Json>>> {
        async_read::async_batch_get_cache(self, keys, options).await
    }

    fn declare_batch_get(
        &self,
        keys: &[String],
        batch: &Arc<RedisBatch>,
    ) -> Result<DeclaredBatchRead, BatchReadsError> {
        declared::declare_batch_get(self, keys, batch)
    }

    async fn async_resolve_batch_get(
        &self,
        declared: DeclaredBatchRead,
    ) -> Result<Vec<Option<Json>>, BatchReadsError> {
        declared::async_resolve_batch_get(self, declared).await
    }

    async fn async_batch_get_cache_shared(
        reads: &[(&Self, &[String])],
    ) -> Vec<Option<Vec<Option<Json>>>> {
        shared::async_batch_get_cache_shared(reads).await
    }
}
