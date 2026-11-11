use std::sync::{Arc, Mutex, RwLock};

use betterlite_models::decisions::Json;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::scope::{
    active_post_call_redis_batch, active_request_redis_batch,
};
use indexmap::IndexMap;

use super::limited_size::LimitedSizeOrderedDict;
use super::{
    CacheOptions, DEFAULT_BATCH_CACHE_EXPIRY, DEFAULT_MAX_SIZE_IN_MEMORY, DualCache,
    DualCacheError, DualCacheSettings, MemoryMaxSize, deferred, increments, lifecycle, lock_read,
    lock_write, reads, writes,
};
use crate::backend::memory::MemoryCacheOps;
use crate::types::pipeline::RedisPipelineIncrement;

impl<Memory: MemoryCacheOps> DualCache<Memory> {
    pub fn new(
        in_memory_cache: Arc<Memory>,
        redis_cache: Option<Arc<RedisCache>>,
        settings: DualCacheSettings,
    ) -> Self {
        Self {
            in_memory_cache,
            redis_cache: RwLock::new(redis_cache),
            last_redis_batch_access_time: Mutex::new(LimitedSizeOrderedDict {
                entries: IndexMap::new(),
                max_size: settings.max_redis_batch_cache_size,
            }),
            redis_batch_cache_expiry: settings
                .batch_cache_expiry
                .unwrap_or(DEFAULT_BATCH_CACHE_EXPIRY),
            default_in_memory_ttl: RwLock::new(settings.default_in_memory_ttl),
            default_redis_ttl: RwLock::new(settings.default_redis_ttl),
        }
    }

    pub fn update_cache_ttl(&self, memory_ttl: Option<f64>, redis_ttl: Option<f64>) {
        if let Some(ttl) = memory_ttl {
            *lock_write(&self.default_in_memory_ttl) = Some(ttl);
        }
        if let Some(ttl) = redis_ttl {
            *lock_write(&self.default_redis_ttl) = Some(ttl);
        }
    }

    pub fn update_in_memory_max_size(&self, max_size: Option<usize>)
    where
        Memory: MemoryMaxSize,
    {
        match max_size {
            Some(limit) => self.in_memory_cache.set_max_size(limit),
            None => self
                .in_memory_cache
                .set_max_size(DEFAULT_MAX_SIZE_IN_MEMORY),
        }
    }

    pub fn attach_redis_cache(&self, redis: Option<Arc<RedisCache>>, redis_ttl: Option<f64>) {
        let Some(tier) = redis else {
            return;
        };
        lifecycle::attach_redis(self, tier, redis_ttl);
    }

    pub(crate) fn redis(&self) -> Option<Arc<RedisCache>> {
        match self.redis_cache.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn get_cache(&self, key: &str, local_only: bool) -> Option<Json> {
        if local_only {
            return reads::local_get(self, key, "LiteLLM Cache: exception in get_cache");
        }
        reads::get(self, key)
    }

    pub async fn async_get_cache(&self, key: &str, local_only: bool) -> Option<Json> {
        if local_only {
            return reads::local_get(self, key, "LiteLLM Cache: exception in async_get_cache");
        }
        reads::async_get(self, key).await
    }

    pub fn set_cache(&self, key: &str, value: &Json, options: CacheOptions) {
        let resolved = options
            .ttl
            .or_else(|| *lock_read(&self.default_in_memory_ttl));
        if let Err(error) = writes::memory_set(self, key, value, resolved) {
            tracing::debug!(%error, "LiteLLM Cache: exception in set_cache");
            return;
        }
        if options.local_only {
            return;
        }
        writes::redis_set(self, key, value, resolved);
    }

    pub async fn async_set_cache(&self, key: &str, value: &Json, options: CacheOptions) {
        let resolved = options
            .ttl
            .or_else(|| *lock_read(&self.default_in_memory_ttl));
        if let Err(error) = writes::memory_set(self, key, value, resolved) {
            tracing::error!(%error, "LiteLLM Cache: exception in async add_cache");
            return;
        }
        if options.local_only {
            return;
        }
        writes::async_redis_set(self, key, value, resolved).await;
    }

    pub async fn async_set_cache_pipeline(
        &self,
        entries: &[(String, Json)],
        options: CacheOptions,
    ) {
        let resolved = options
            .ttl
            .or_else(|| *lock_read(&self.default_in_memory_ttl));
        if let Err(error) = writes::memory_set_pipeline(self, entries, resolved) {
            tracing::error!(%error, "LiteLLM Cache: exception in async add_cache");
            return;
        }
        if options.local_only {
            return;
        }
        writes::async_redis_set_pipeline(self, entries, resolved).await;
    }

    pub fn increment_cache(
        &self,
        key: &str,
        value: f64,
        options: CacheOptions,
    ) -> Result<f64, DualCacheError> {
        let memory = increments::memory_increment(self, key, value, options.ttl)?;
        if options.local_only {
            return Ok(memory);
        }
        increments::redis_increment(self, key, value, options.ttl, memory)
    }

    pub async fn async_increment_cache(
        &self,
        key: &str,
        value: f64,
        options: CacheOptions,
    ) -> Option<f64> {
        let memory = match increments::memory_increment(self, key, value, options.ttl) {
            Ok(incremented) => Some(incremented),
            Err(error) => {
                tracing::warn!(%error, "in-memory increment failed");
                None
            }
        };
        if options.local_only {
            return memory;
        }
        let increment = increments::RedisIncrement {
            key,
            value,
            ttl: options.ttl,
            memory,
            refresh_ttl: options.refresh_ttl,
        };
        increments::async_redis_increment(self, increment).await
    }

    pub async fn async_increment_cache_pipeline(
        &self,
        increments: &[RedisPipelineIncrement],
        local_only: bool,
    ) -> Option<Vec<f64>> {
        let memory = match increments::memory_increment_pipeline(self, increments) {
            Ok(results) => Some(results),
            Err(error) => {
                tracing::warn!(%error, "in-memory increment failed");
                return None;
            }
        };
        if local_only {
            return memory;
        }
        increments::async_redis_increment_pipeline(self, increments, memory).await
    }

    pub async fn increment_cache_post_call(&self, key: &str, value: f64, ttl: Option<i64>) {
        let increments = [RedisPipelineIncrement {
            key: key.to_owned(),
            increment_value: value,
            ttl,
        }];
        self.pipeline_increment_post_call(&increments).await;
    }

    pub async fn pipeline_increment_post_call(&self, increments: &[RedisPipelineIncrement]) {
        let active = self
            .redis()
            .and_then(|redis| active_post_call_redis_batch(&redis));
        if let Some(batch) = active {
            increments::queue_post_call_increments(self, &batch, increments);
            return;
        }
        let _pipeline = self.async_increment_cache_pipeline(increments, false).await;
    }

    pub fn set_cache_pre_call(
        &self,
        key: &str,
        value: &Json,
        ttl: Option<f64>,
    ) -> Result<Option<BatchResult<()>>, DualCacheError> {
        let active = self
            .redis()
            .and_then(|redis| active_request_redis_batch(&redis));
        let Some(batch) = active else {
            return Ok(None);
        };
        deferred::set_on_batch(self, &batch, key, value, ttl)
    }

    pub fn delete_cache_pre_call(
        &self,
        key: &str,
    ) -> Result<Option<BatchResult<()>>, DualCacheError> {
        let active = self
            .redis()
            .and_then(|redis| active_request_redis_batch(&redis));
        let Some(batch) = active else {
            return Ok(None);
        };
        deferred::delete_on_batch(self, &batch, key)
    }

    pub async fn async_set_cache_sadd(
        &self,
        key: &str,
        values: &[String],
        options: CacheOptions,
    ) -> Result<(), DualCacheError> {
        writes::add_to_memory_set(self, key, values, options.ttl)?;
        if options.local_only {
            return Ok(());
        }
        lifecycle::redis_set_add(self, key, values, options.ttl).await
    }

    pub fn flush_cache(&self) -> Result<(), DualCacheError> {
        self.in_memory_cache
            .flush_cache()
            .map_err(|source| DualCacheError::Memory { source })?;
        lifecycle::redis_flush(self)
    }

    pub fn delete_cache(&self, key: &str) -> Result<(), DualCacheError> {
        self.in_memory_cache
            .delete_cache(key)
            .map_err(|source| DualCacheError::Memory { source })?;
        lifecycle::redis_delete(self, key)
    }

    pub async fn async_delete_cache(&self, key: &str) -> Result<(), DualCacheError> {
        self.in_memory_cache
            .delete_cache(key)
            .map_err(|source| DualCacheError::Memory { source })?;
        lifecycle::async_redis_delete(self, key).await
    }

    pub async fn async_delete_cache_keys(&self, keys: &[String]) -> Result<(), DualCacheError> {
        if keys.is_empty() {
            return Ok(());
        }
        for key in keys {
            self.in_memory_cache
                .delete_cache(key)
                .map_err(|source| DualCacheError::Memory { source })?;
        }
        lifecycle::redis_delete_keys(self, keys).await
    }

    pub async fn async_get_ttl(&self, key: &str) -> Result<Option<f64>, DualCacheError> {
        let entries = self
            .in_memory_cache
            .ttl_entries()
            .map_err(|source| DualCacheError::Memory { source })?;
        if let Some((_name, expiry)) = entries.into_iter().find(|(name, _expiry)| name == key) {
            return Ok(Some(expiry));
        }
        lifecycle::redis_ttl(self, key).await
    }
}
