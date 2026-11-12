use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::{CacheOptions, DualCache, DualCacheError, MemoryMaxSize};
use betterlite_models::decisions::Json;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::RedisCache;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreCall {
    // the backend queued the write on the request pipeline and its result is dropped unawaited
    Queued,
    // no request pipeline is open, so the caller takes the direct write
    Direct,
}

// the operations the auth cache routes into one partition; a second implementation records calls for tests
#[async_trait]
pub trait AuthCacheTier: Send + Sync {
    type Memory: MemoryCacheOps + MemoryMaxSize;

    fn in_memory(&self) -> &Arc<Self::Memory>;
    fn memory_ttl(&self) -> Option<f64>;
    fn update_cache_ttl(&self, memory_ttl: Option<f64>, redis_ttl: Option<f64>);
    fn update_in_memory_max_size(&self, max_size: Option<usize>);
    fn attach_redis_cache(&self, redis: Option<Arc<RedisCache>>, redis_ttl: Option<f64>);
    fn get(&self, key: &str, local_only: bool) -> Option<Json>;
    async fn async_get(&self, key: &str, local_only: bool) -> Option<Json>;
    fn set(&self, key: Option<&str>, value: &Json, options: CacheOptions);
    async fn async_set(&self, key: Option<&str>, value: &Json, options: CacheOptions);
    fn set_pre_call(
        &self,
        key: &str,
        value: &Json,
        ttl: Option<f64>,
    ) -> Result<PreCall, DualCacheError>;
    fn delete(&self, key: &str) -> Result<(), DualCacheError>;
    async fn async_delete(&self, key: &str) -> Result<(), DualCacheError>;
    fn delete_pre_call(&self, key: &str) -> Result<Option<BatchResult<()>>, DualCacheError>;
    async fn async_delete_keys(&self, keys: &[String]) -> Result<(), DualCacheError>;
    async fn async_set_pipeline(
        &self,
        entries: &[(String, Json)],
        options: CacheOptions,
    ) -> Result<(), DualCacheError>;
}

#[async_trait]
impl<Memory: MemoryCacheOps + MemoryMaxSize> AuthCacheTier for DualCache<Memory> {
    type Memory = Memory;

    fn in_memory(&self) -> &Arc<Memory> {
        &self.in_memory_cache
    }

    fn memory_ttl(&self) -> Option<f64> {
        match self.default_in_memory_ttl.read() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    fn update_cache_ttl(&self, memory_ttl: Option<f64>, redis_ttl: Option<f64>) {
        Self::update_cache_ttl(self, memory_ttl, redis_ttl);
    }

    fn update_in_memory_max_size(&self, max_size: Option<usize>) {
        Self::update_in_memory_max_size(self, max_size);
    }

    fn attach_redis_cache(&self, redis: Option<Arc<RedisCache>>, redis_ttl: Option<f64>) {
        Self::attach_redis_cache(self, redis, redis_ttl);
    }

    fn get(&self, key: &str, local_only: bool) -> Option<Json> {
        Self::get_cache(self, key, local_only)
    }

    async fn async_get(&self, key: &str, local_only: bool) -> Option<Json> {
        Self::async_get_cache(self, key, local_only).await
    }

    fn set(&self, key: Option<&str>, value: &Json, options: CacheOptions) {
        // a null key has no native slot: InMemoryCache would hold it where no string key can read it
        if let Some(name) = key {
            Self::set_cache(self, name, value, options);
        }
    }

    async fn async_set(&self, key: Option<&str>, value: &Json, options: CacheOptions) {
        if let Some(name) = key {
            Self::async_set_cache(self, name, value, options).await;
        }
    }

    fn set_pre_call(
        &self,
        key: &str,
        value: &Json,
        ttl: Option<f64>,
    ) -> Result<PreCall, DualCacheError> {
        let deferred = Self::set_cache_pre_call(self, key, value, ttl)?;
        Ok(deferred.map_or(PreCall::Direct, |queued| {
            drop(queued);
            PreCall::Queued
        }))
    }

    fn delete(&self, key: &str) -> Result<(), DualCacheError> {
        Self::delete_cache(self, key)
    }

    async fn async_delete(&self, key: &str) -> Result<(), DualCacheError> {
        Self::async_delete_cache(self, key).await
    }

    fn delete_pre_call(&self, key: &str) -> Result<Option<BatchResult<()>>, DualCacheError> {
        Self::delete_cache_pre_call(self, key)
    }

    async fn async_delete_keys(&self, keys: &[String]) -> Result<(), DualCacheError> {
        Self::async_delete_cache_keys(self, keys).await
    }

    async fn async_set_pipeline(
        &self,
        entries: &[(String, Json)],
        options: CacheOptions,
    ) -> Result<(), DualCacheError> {
        Self::async_set_cache_pipeline(self, entries, options).await;
        Ok(())
    }
}
