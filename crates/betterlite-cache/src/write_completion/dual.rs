use std::sync::Arc;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;

use crate::backend::memory::MemoryCacheOps;
use crate::backend::{BackendError, CacheBackend, CacheResult, Ttl};
use crate::dual_cache::{CacheOptions, DualCache, DualCacheSettings, lock_read};
use crate::envelope::backend::EnvelopeBackend;
use crate::envelope::lookup::CacheLookup;
use crate::handler::dynamic_cache::DualCacheEnvelope;
use crate::handler::value::to_json;

// A cache write outlives the handler that started it, so it shares the same in-memory and Redis tiers.
pub struct SharedDualCache<Memory: MemoryCacheOps> {
    cache: Arc<DualCache<Memory>>,
}

impl<Memory: MemoryCacheOps> SharedDualCache<Memory> {
    #[must_use]
    pub fn shared(cache: &DualCache<Memory>) -> Self {
        let max_redis_batch_cache_size = match cache.last_redis_batch_access_time.lock() {
            Ok(guard) => guard.max_size,
            Err(poisoned) => poisoned.into_inner().max_size,
        };
        let settings = DualCacheSettings {
            default_in_memory_ttl: *lock_read(&cache.default_in_memory_ttl),
            default_redis_ttl: *lock_read(&cache.default_redis_ttl),
            batch_cache_expiry: Some(cache.redis_batch_cache_expiry),
            max_redis_batch_cache_size,
            redis_delete_chunk_size: cache.redis_delete_chunk_size,
        };
        let shared = DualCache::new(Arc::clone(&cache.in_memory_cache), cache.redis(), settings);
        Self {
            cache: Arc::new(shared),
        }
    }
}

impl<Memory: MemoryCacheOps> Clone for SharedDualCache<Memory> {
    fn clone(&self) -> Self {
        Self {
            cache: Arc::clone(&self.cache),
        }
    }
}

#[async_trait]
impl<Memory: MemoryCacheOps> EnvelopeBackend for SharedDualCache<Memory> {
    fn get_cache(&self, key: &str, lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        DualCacheEnvelope {
            cache: self.cache.as_ref(),
        }
        .get_cache(key, lookup)
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        DualCacheEnvelope {
            cache: self.cache.as_ref(),
        }
        .set_cache(key, value, ttl)
    }

    async fn async_get_cache(
        &self,
        key: &str,
        lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        DualCacheEnvelope {
            cache: self.cache.as_ref(),
        }
        .async_get_cache(key, lookup)
        .await
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        DualCacheEnvelope {
            cache: self.cache.as_ref(),
        }
        .async_set_cache(key, value, ttl)
        .await
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        DualCacheEnvelope {
            cache: self.cache.as_ref(),
        }
        .batch_cache_write(key, value, ttl)
        .await
    }
}

// The embedding pipeline's dynamic store writes the same dual cache in bulk.
#[async_trait]
impl<Memory: MemoryCacheOps> CacheBackend for SharedDualCache<Memory> {
    fn default_ttl(&self) -> i64 {
        self.cache.in_memory_cache.default_ttl()
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        let json = to_json(&value).ok_or(BackendError::Unsupported("async_set_cache"))?;
        let options = CacheOptions {
            ttl,
            local_only: false,
            refresh_ttl: false,
        };
        self.cache.async_set_cache(key, &json, options).await;
        Ok(())
    }

    async fn async_set_cache_pipeline(
        &self,
        entries: Vec<(String, CacheValue)>,
        ttl: Ttl,
    ) -> CacheResult<()> {
        let mut converted = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let json =
                to_json(&value).ok_or(BackendError::Unsupported("async_set_cache_pipeline"))?;
            converted.push((key, json));
        }
        let options = CacheOptions {
            ttl,
            local_only: false,
            refresh_ttl: false,
        };
        self.cache
            .async_set_cache_pipeline(&converted, options)
            .await;
        Ok(())
    }
}
