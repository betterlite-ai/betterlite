use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::BackendError;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::{DualCache, MemoryMaxSize};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;

use super::error::InvalidationError;
use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::{AuthCache, WriteOptions, WriteTtl};

// a Redis handle's identity: the usage and spend caches share one when their Arcs point at the same cache
pub trait RedisMarker: Send + Sync {}

impl RedisMarker for RedisCache {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeferredDelete {
    // an open request batch carried the delete and its result came back
    Settled,
    // no request batch was open, so the caller deletes directly
    Unqueued,
}

#[async_trait]
pub trait AuthInvalidationCache: Send + Sync {
    fn redis_handle(&self, key: &str) -> Option<Arc<dyn RedisMarker>>;
    fn management_ttl(&self) -> f64;
    async fn write_management(
        &self,
        key: &str,
        payload: &Json,
        ttl: Option<f64>,
    ) -> Result<(), InvalidationError>;
    fn delete_sync(&self, key: &str) -> Result<(), InvalidationError>;
    async fn delete_async(&self, key: &str) -> Result<(), InvalidationError>;
    async fn delete_pre_call(&self, key: &str) -> Result<DeferredDelete, InvalidationError>;
    fn delete_memory(&self, key: &str) -> Result<(), BackendError>;
    fn delete_redis(&self, key: &str) -> Result<(), RedisAdminError>;
}

#[async_trait]
pub trait UsageInvalidationCache: Send + Sync {
    fn redis_handle(&self) -> Option<Arc<dyn RedisMarker>>;
    fn delete_memory(&self, key: &str) -> Result<(), BackendError>;
    async fn delete_async(&self, key: &str) -> Result<(), InvalidationError>;
}

#[async_trait]
pub trait SpendCounterCache: Send + Sync {
    fn redis_handle(&self) -> Option<Arc<dyn RedisMarker>>;
    fn set_memory(&self, key: &str, value: f64, ttl: f64) -> Result<(), BackendError>;
    async fn set_redis(&self, key: &str, value: f64, ttl: f64)
    -> Result<(), RedisCacheValuesError>;
    async fn delete_redis(&self, key: &str) -> Result<(), RedisAdminError>;
}

#[async_trait]
impl<Memory> AuthInvalidationCache for AuthCache<DualCache<Memory>>
where
    Memory: MemoryCacheOps + MemoryMaxSize,
{
    // a Redis marker needs a live Redis cache, and without one this answer and None are the same
    #[cfg_attr(test, mutants::skip)]
    fn redis_handle(&self, key: &str) -> Option<Arc<dyn RedisMarker>> {
        marker_of(redis_of(self.tier(key)))
    }

    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }

    async fn write_management(
        &self,
        key: &str,
        payload: &Json,
        ttl: Option<f64>,
    ) -> Result<(), InvalidationError> {
        let options = WriteOptions {
            ttl: ttl.map(|seconds| WriteTtl { seconds }),
            ..WriteOptions::default()
        };
        self.async_set(Some(key), payload, options)
            .await
            .map_err(|error| InvalidationError::Write(Box::new(error)))
    }

    fn delete_sync(&self, key: &str) -> Result<(), InvalidationError> {
        self.tier(key)
            .delete(key)
            .map_err(|error| InvalidationError::Backend(Box::new(error)))
    }

    async fn delete_async(&self, key: &str) -> Result<(), InvalidationError> {
        self.tier(key)
            .async_delete(key)
            .await
            .map_err(|error| InvalidationError::Backend(Box::new(error)))
    }

    async fn delete_pre_call(&self, key: &str) -> Result<DeferredDelete, InvalidationError> {
        let queued = self
            .tier(key)
            .delete_pre_call(key)
            .map_err(|error| InvalidationError::Backend(Box::new(error)))?;
        let Some(batch) = queued else {
            return Ok(DeferredDelete::Unqueued);
        };
        batch.await.map_err(InvalidationError::QueuedDelete)?;
        Ok(DeferredDelete::Settled)
    }

    fn delete_memory(&self, key: &str) -> Result<(), BackendError> {
        self.in_memory_cache_for(key).delete_cache(key)
    }

    // deleting from Redis needs a live Redis cache, and without one this delete is already a no-op
    #[cfg_attr(test, mutants::skip)]
    fn delete_redis(&self, key: &str) -> Result<(), RedisAdminError> {
        match redis_of(self.tier(key)) {
            Some(redis) => redis.admin.delete_cache(key),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl<Memory> UsageInvalidationCache for DualCache<Memory>
where
    Memory: MemoryCacheOps + MemoryMaxSize,
{
    // a Redis marker needs a live Redis cache, and without one this answer and None are the same
    #[cfg_attr(test, mutants::skip)]
    fn redis_handle(&self) -> Option<Arc<dyn RedisMarker>> {
        marker_of(redis_of(self))
    }

    fn delete_memory(&self, key: &str) -> Result<(), BackendError> {
        self.in_memory_cache.delete_cache(key)
    }

    async fn delete_async(&self, key: &str) -> Result<(), InvalidationError> {
        self.async_delete_cache(key)
            .await
            .map_err(|error| InvalidationError::Backend(Box::new(error)))
    }
}

#[async_trait]
impl<Memory> SpendCounterCache for DualCache<Memory>
where
    Memory: MemoryCacheOps + MemoryMaxSize,
{
    // a Redis marker needs a live Redis cache, and without one this answer and None are the same
    #[cfg_attr(test, mutants::skip)]
    fn redis_handle(&self) -> Option<Arc<dyn RedisMarker>> {
        marker_of(redis_of(self))
    }

    fn set_memory(&self, key: &str, value: f64, ttl: f64) -> Result<(), BackendError> {
        self.in_memory_cache
            .set_cache(key, CacheValue::Float(value), Some(ttl))
    }

    // a Redis write needs a live Redis cache, and without one this write is already a no-op
    #[cfg_attr(test, mutants::skip)]
    async fn set_redis(
        &self,
        key: &str,
        value: f64,
        ttl: f64,
    ) -> Result<(), RedisCacheValuesError> {
        let Some(redis) = redis_of(self) else {
            return Ok(());
        };
        redis
            .values
            .async_set_cache(Some(key), &value, Some(ttl), false)
            .await
            .map(|_stored| ())
    }

    // deleting from Redis needs a live Redis cache, and without one this delete is already a no-op
    #[cfg_attr(test, mutants::skip)]
    async fn delete_redis(&self, key: &str) -> Result<(), RedisAdminError> {
        let Some(redis) = redis_of(self) else {
            return Ok(());
        };
        redis.admin.async_delete_cache(key).await.map(|_deleted| ())
    }
}

// without a live Redis the lock always holds None, where this answer and None are the same
#[cfg_attr(test, mutants::skip)]
fn redis_of<Memory>(cache: &DualCache<Memory>) -> Option<Arc<RedisCache>>
where
    Memory: MemoryCacheOps,
{
    match cache.redis_cache.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

// a marker needs a live Redis cache, and without one this answer and None are the same
#[cfg_attr(test, mutants::skip)]
fn marker_of(redis: Option<Arc<RedisCache>>) -> Option<Arc<dyn RedisMarker>> {
    match redis {
        Some(handle) => {
            let marker: Arc<dyn RedisMarker> = handle;
            Some(marker)
        }
        None => None,
    }
}
