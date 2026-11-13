use std::sync::{Arc, Mutex, RwLock};

use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::scope::{
    active_post_call_redis_batch, active_request_redis_batch,
};
use indexmap::IndexMap;

use super::limited_size::LimitedSizeOrderedDict;
use super::{
    CacheOptions, DEFAULT_BATCH_CACHE_EXPIRY, DEFAULT_MAX_SIZE_IN_MEMORY, Deferred, DualCache,
    DualCacheError, DualCacheSettings, MemoryMaxSize, Ttl, increments, lock_read, lock_write,
    logged, reads, value, writes,
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
            redis_delete_chunk_size: settings.redis_delete_chunk_size,
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
        self.in_memory_cache
            .set_max_size(max_size.unwrap_or(DEFAULT_MAX_SIZE_IN_MEMORY));
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
        let message = "LiteLLM Cache: exception in get_cache";
        let memory = logged(reads::memory_hit(self, key), message)?;
        if memory.is_some() || local_only {
            return memory;
        }
        let hit = logged(self.redis()?.values.get_cache(key), message)??;
        reads::backfill(self, key, hit)
    }

    pub async fn async_get_cache(&self, key: &str, local_only: bool) -> Option<Json> {
        let message = "LiteLLM Cache: exception in async_get_cache";
        let memory = logged(reads::memory_hit(self, key), message)?;
        if memory.is_some() || local_only {
            return memory;
        }
        let redis = self.redis()?;
        if active_request_redis_batch(&redis).is_some_and(|batch| batch.read_as_missing(key)) {
            return None;
        }
        let hit = logged(redis.async_get_cache(key).await, message)??;
        reads::backfill(self, key, hit)
    }

    pub fn set_cache(&self, key: &str, value: &Json, options: CacheOptions) {
        let ttl = options
            .ttl
            .or_else(|| *lock_read(&self.default_in_memory_ttl));
        let written = writes::memory_set(self, key, value, ttl);
        if let Some(redis) = self
            .redis()
            .filter(|_| written.is_some() && !options.local_only)
        {
            redis.values.set_cache(key, value, ttl);
        }
    }

    pub async fn async_set_cache(&self, key: &str, value: &Json, options: CacheOptions) {
        let ttl = options
            .ttl
            .or_else(|| *lock_read(&self.default_in_memory_ttl));
        let written = writes::memory_set(self, key, value, ttl);
        let Some(redis) = self
            .redis()
            .filter(|_| written.is_some() && !options.local_only)
        else {
            return;
        };
        let write = redis.values.async_set_cache(Some(key), value, ttl, false);
        logged(write.await, "LiteLLM Cache: exception in async add_cache");
    }

    pub async fn async_set_cache_pipeline(
        &self,
        entries: &[(String, Json)],
        options: CacheOptions,
    ) {
        let ttl = options
            .ttl
            .or_else(|| *lock_read(&self.default_in_memory_ttl));
        let written = entries
            .iter()
            .all(|(key, value)| writes::memory_set(self, key, value, ttl).is_some());
        if let Some(redis) = self.redis().filter(|_| written && !options.local_only) {
            let stored = redis.values.async_set_cache_pipeline(entries, ttl).await;
            logged(stored, "LiteLLM Cache: exception in async add_cache");
        }
    }

    pub fn set_cache_pre_call(&self, key: &str, value: &Json, ttl: Option<f64>) -> Deferred {
        let Some(batch) = self
            .redis()
            .and_then(|redis| active_request_redis_batch(&redis))
        else {
            return Ok(None);
        };
        let expiry = ttl.or_else(|| *lock_read(&self.default_in_memory_ttl));
        self.in_memory_cache
            .set_cache(key, value::cache_from_json(value)?, expiry)?;
        Ok(Some(batch.set(key, value, expiry)))
    }

    pub fn delete_cache_pre_call(&self, key: &str) -> Deferred {
        self.redis()
            .and_then(|redis| active_request_redis_batch(&redis))
            .map(|queue| {
                self.in_memory_cache.delete_cache(key)?;
                Ok(queue.delete(key))
            })
            .transpose()
    }

    pub async fn async_set_cache_sadd(
        &self,
        key: &str,
        values: &[String],
        options: CacheOptions,
    ) -> Result<(), DualCacheError> {
        writes::add_to_memory_set(self, key, values, options.ttl)?;
        if let Some(redis) = self.redis().filter(|_| !options.local_only) {
            let added = redis.values.async_set_cache_sadd(key, values, options.ttl);
            added.await?;
        }
        Ok(())
    }

    pub fn flush_cache(&self) -> Result<(), DualCacheError> {
        self.in_memory_cache.flush_cache()?;
        if let Some(redis) = self.redis() {
            redis.admin.flush_cache()?;
        }
        Ok(())
    }

    pub fn delete_cache(&self, key: &str) -> Result<(), DualCacheError> {
        self.in_memory_cache.delete_cache(key)?;
        if let Some(redis) = self.redis() {
            redis.admin.delete_cache(key)?;
        }
        Ok(())
    }

    pub async fn async_delete_cache(&self, key: &str) -> Result<(), DualCacheError> {
        self.in_memory_cache.delete_cache(key)?;
        if let Some(redis) = self.redis() {
            let _deleted = redis.admin.async_delete_cache(key).await?;
        }
        Ok(())
    }

    // Redis takes the whole key list as one DEL, so a large batch is split into bounded commands
    pub async fn async_delete_cache_keys(&self, keys: &[String]) -> Result<(), DualCacheError> {
        for key in keys {
            self.in_memory_cache.delete_cache(key)?;
        }
        if let Some(redis) = self.redis() {
            for chunk in keys.chunks(self.redis_delete_chunk_size) {
                redis.admin.delete_cache_keys(chunk).await?;
            }
        }
        Ok(())
    }

    pub async fn async_get_ttl(&self, key: &str) -> Result<Option<Ttl>, DualCacheError> {
        let entries = self.in_memory_cache.ttl_entries()?;
        if let Some((_name, expiry)) = entries.into_iter().find(|(name, _expiry)| name == key) {
            return Ok(Some(Ttl::Expiry(expiry)));
        }
        let Some(redis) = self.redis() else {
            return Ok(None);
        };
        let seconds = redis.async_get_ttl(key).await?;
        Ok(seconds.map(Ttl::Seconds))
    }

    pub fn increment_cache(
        &self,
        key: &str,
        value: f64,
        options: CacheOptions,
    ) -> Result<f64, DualCacheError> {
        let memory = self
            .in_memory_cache
            .increment_cache(key, value, options.ttl)?;
        let Some(redis) = self.redis().filter(|_| !options.local_only) else {
            return Ok(memory);
        };
        increments::redis_increment(&redis, key, value, options.ttl)
    }

    pub async fn async_increment_cache(
        &self,
        key: &str,
        value: f64,
        options: CacheOptions,
    ) -> Option<f64> {
        let counted = self
            .in_memory_cache
            .increment_cache(key, value, options.ttl);
        let memory = logged(counted, "in-memory increment failed")?;
        let Some(redis) = self.redis().filter(|_| !options.local_only) else {
            return Some(memory);
        };
        Some(increments::async_redis_increment(&redis, key, value, options, memory).await)
    }

    pub async fn async_increment_cache_pipeline(
        &self,
        operations: &[RedisPipelineIncrement],
        local_only: bool,
    ) -> Option<Vec<f64>> {
        let counted = increments::memory_increments(self, operations);
        let memory = logged(counted, "in-memory increment failed")?;
        let Some(redis) = self.redis().filter(|_| !local_only) else {
            return Some(memory);
        };
        increments::async_redis_pipeline(&redis, operations, memory).await
    }

    pub async fn increment_cache_post_call(&self, key: &str, value: f64, ttl: Option<i64>) {
        let operation = RedisPipelineIncrement {
            key: key.to_owned(),
            increment_value: value,
            ttl,
        };
        self.pipeline_increment_post_call(&[operation]).await;
    }

    pub async fn pipeline_increment_post_call(&self, operations: &[RedisPipelineIncrement]) {
        let Some(batch) = self
            .redis()
            .and_then(|redis| active_post_call_redis_batch(&redis))
        else {
            let _results = self.async_increment_cache_pipeline(operations, false).await;
            return;
        };
        let memory = increments::memory_increments(self, operations);
        logged(memory, "in-memory increment failed");
        increments::queue_post_call(&batch, operations);
    }
}
