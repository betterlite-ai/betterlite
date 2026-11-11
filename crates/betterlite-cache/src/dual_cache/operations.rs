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
        let expiry = settings
            .batch_cache_expiry
            .unwrap_or(DEFAULT_BATCH_CACHE_EXPIRY);
        Self {
            in_memory_cache,
            redis_cache: RwLock::new(redis_cache),
            last_redis_batch_access_time: Mutex::new(LimitedSizeOrderedDict {
                entries: IndexMap::new(),
                max_size: settings.max_redis_batch_cache_size,
            }),
            redis_batch_cache_expiry: expiry,
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
        let mut held = lock_write(&self.redis_cache);
        if held.is_none() {
            *held = Some(tier);
            drop(held);
            if let Some(ttl) = redis_ttl {
                *lock_write(&self.default_redis_ttl) = Some(ttl);
            }
        }
    }

    pub(crate) fn redis(&self) -> Option<Arc<RedisCache>> {
        match self.redis_cache.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn get_cache(&self, key: &str, local_only: bool) -> Option<Json> {
        let memory = &*self.in_memory_cache;
        let ttl = *lock_read(&self.default_in_memory_ttl);
        match reads::memory_hit(memory, key) {
            Some(hit) => Some(hit),
            None if local_only => None,
            None => reads::redis_read(&self.redis()?, memory, ttl, key),
        }
    }

    pub async fn async_get_cache(&self, key: &str, local_only: bool) -> Option<Json> {
        let memory = &*self.in_memory_cache;
        let hit = reads::memory_hit(memory, key);
        if hit.is_some() || local_only {
            return hit;
        }
        let ttl = *lock_read(&self.default_in_memory_ttl);
        reads::async_redis_read(&self.redis()?, memory, ttl, key).await
    }

    pub fn set_cache(&self, key: &str, value: &Json, options: CacheOptions) {
        let default = *lock_read(&self.default_in_memory_ttl);
        let ttl = options.ttl.or(default);
        let memory = &*self.in_memory_cache;
        if writes::memory_set(memory, key, value, ttl).is_ok() && !options.local_only {
            writes::redis_set(self.redis().as_ref(), key, value, ttl);
        }
    }

    pub async fn async_set_cache(&self, key: &str, value: &Json, options: CacheOptions) {
        let default = *lock_read(&self.default_in_memory_ttl);
        let ttl = options.ttl.or(default);
        let memory = &*self.in_memory_cache;
        if writes::memory_set(memory, key, value, ttl).is_ok() && !options.local_only {
            writes::async_redis_set(self.redis().as_ref(), key, value, ttl).await;
        }
    }

    pub async fn async_set_cache_pipeline(
        &self,
        entries: &[(String, Json)],
        options: CacheOptions,
    ) {
        let default = *lock_read(&self.default_in_memory_ttl);
        let ttl = options.ttl.or(default);
        let memory = &*self.in_memory_cache;
        if writes::memory_set_pipeline(memory, entries, ttl).is_err() || options.local_only {
            return;
        }
        writes::async_redis_set_pipeline(self.redis().as_ref(), entries, ttl).await;
    }

    pub fn increment_cache(
        &self,
        key: &str,
        value: f64,
        options: CacheOptions,
    ) -> Result<f64, DualCacheError> {
        let memory = increments::memory_increment(&*self.in_memory_cache, key, value, options.ttl)?;
        if options.local_only {
            return Ok(memory);
        }
        increments::redis_increment(self.redis().as_ref(), key, value, options.ttl, memory)
    }

    pub async fn async_increment_cache(
        &self,
        key: &str,
        value: f64,
        options: CacheOptions,
    ) -> Option<f64> {
        let memory = &*self.in_memory_cache;
        let incremented = match increments::memory_increment(memory, key, value, options.ttl) {
            Ok(incremented) => incremented,
            Err(error) => {
                tracing::warn!(%error, "in-memory increment failed");
                return None;
            }
        };
        if options.local_only {
            return Some(incremented);
        }
        increments::async_redis_increment(
            self.redis().as_ref(),
            increments::RedisIncrement {
                key,
                value,
                ttl: options.ttl,
                memory: Some(incremented),
                refresh_ttl: options.refresh_ttl,
            },
        )
        .await
    }

    pub async fn async_increment_cache_pipeline(
        &self,
        operations: &[RedisPipelineIncrement],
        local_only: bool,
    ) -> Option<Vec<f64>> {
        let memory = &*self.in_memory_cache;
        let results = match increments::memory_increment_pipeline(memory, operations) {
            Ok(results) => Some(results),
            Err(error) => {
                tracing::warn!(%error, "in-memory increment failed");
                return None;
            }
        };
        if local_only {
            return results;
        }
        increments::async_redis_increment_pipeline(self.redis().as_ref(), operations, results).await
    }

    pub async fn increment_cache_post_call(&self, key: &str, value: f64, ttl: Option<i64>) {
        let operations = [RedisPipelineIncrement {
            key: key.to_owned(),
            increment_value: value,
            ttl,
        }];
        self.pipeline_increment_post_call(&operations).await;
    }

    pub async fn pipeline_increment_post_call(&self, operations: &[RedisPipelineIncrement]) {
        let active = self
            .redis()
            .and_then(|redis| active_post_call_redis_batch(&redis));
        if let Some(batch) = active {
            increments::queue_post_call_increments(&*self.in_memory_cache, &batch, operations);
            return;
        }
        let _results = self.async_increment_cache_pipeline(operations, false).await;
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
        let effective = ttl.or_else(|| *lock_read(&self.default_in_memory_ttl));
        deferred::set_on_batch(&*self.in_memory_cache, &batch, key, value, effective)
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
        deferred::delete_on_batch(&*self.in_memory_cache, &batch, key)
    }

    pub async fn async_set_cache_sadd(
        &self,
        key: &str,
        values: &[String],
        options: CacheOptions,
    ) -> Result<(), DualCacheError> {
        writes::add_to_memory_set(&*self.in_memory_cache, key, values, options.ttl)?;
        if options.local_only {
            return Ok(());
        }
        lifecycle::redis_set_add(self.redis().as_ref(), key, values, options.ttl).await
    }

    pub fn flush_cache(&self) -> Result<(), DualCacheError> {
        self.in_memory_cache
            .flush_cache()
            .map_err(DualCacheError::from)?;
        lifecycle::redis_flush(self.redis().as_ref())
    }

    pub fn delete_cache(&self, key: &str) -> Result<(), DualCacheError> {
        self.in_memory_cache
            .delete_cache(key)
            .map_err(DualCacheError::from)?;
        lifecycle::redis_delete(self.redis().as_ref(), key)
    }

    pub async fn async_delete_cache(&self, key: &str) -> Result<(), DualCacheError> {
        self.in_memory_cache
            .delete_cache(key)
            .map_err(DualCacheError::from)?;
        lifecycle::async_redis_delete(self.redis().as_ref(), key).await
    }

    pub async fn async_delete_cache_keys(&self, keys: &[String]) -> Result<(), DualCacheError> {
        if keys.is_empty() {
            return Ok(());
        }
        for key in keys {
            self.in_memory_cache
                .delete_cache(key)
                .map_err(DualCacheError::from)?;
        }
        lifecycle::redis_delete_keys(self.redis().as_ref(), keys).await
    }

    pub async fn async_get_ttl(&self, key: &str) -> Result<Option<f64>, DualCacheError> {
        let entries = self
            .in_memory_cache
            .ttl_entries()
            .map_err(DualCacheError::from)?;
        if let Some((_name, expiry)) = entries.into_iter().find(|(name, _expiry)| name == key) {
            return Ok(Some(expiry));
        }
        lifecycle::redis_ttl(self.redis().as_ref(), key).await
    }
}
