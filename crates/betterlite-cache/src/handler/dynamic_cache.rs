use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;

use crate::backend::memory::MemoryCacheOps;
use crate::backend::{BackendError, CacheResult, Ttl};
use crate::dual_cache::{CacheOptions, DualCache};
use crate::envelope::backend::EnvelopeBackend;
use crate::envelope::lookup::CacheLookup;

use super::value::{to_cache_value, to_json};

// LiteLLM passes the handler's dual cache as dynamic_cache_object, so the envelope facade reads and writes through it
pub struct DualCacheEnvelope<'cache, Memory: MemoryCacheOps> {
    pub cache: &'cache DualCache<Memory>,
}

#[async_trait]
impl<Memory: MemoryCacheOps> EnvelopeBackend for DualCacheEnvelope<'_, Memory> {
    fn get_cache(&self, key: &str, _lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        Ok(self
            .cache
            .get_cache(key, false)
            .and_then(|json| to_cache_value(&json)))
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        let json = to_json(&value).ok_or(BackendError::Unsupported("set_cache"))?;
        self.cache.set_cache(
            key,
            &json,
            CacheOptions {
                ttl,
                local_only: false,
                refresh_ttl: false,
            },
        );
        Ok(())
    }

    async fn async_get_cache(
        &self,
        key: &str,
        _lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        let read = self.cache.async_get_cache(key, false).await;
        Ok(read.and_then(|json| to_cache_value(&json)))
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        let json = to_json(&value).ok_or(BackendError::Unsupported("async_set_cache"))?;
        self.cache
            .async_set_cache(
                key,
                &json,
                CacheOptions {
                    ttl,
                    local_only: false,
                    refresh_ttl: false,
                },
            )
            .await;
        Ok(())
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        let json = to_json(&value).ok_or(BackendError::Unsupported("batch_cache_write"))?;
        let entries = [(key.to_owned(), json)];
        self.cache
            .async_set_cache_pipeline(
                &entries,
                CacheOptions {
                    ttl,
                    local_only: false,
                    refresh_ttl: false,
                },
            )
            .await;
        Ok(())
    }
}
