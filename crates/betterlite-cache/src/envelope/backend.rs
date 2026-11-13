use std::sync::Arc;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;

use super::lookup::CacheLookup;
use crate::backend::{CacheBackend, CacheResult, Ttl};

// One storage call per LiteLLM BaseCache method, with the projected lookup the Python backends receive as kwargs.
#[async_trait]
pub trait EnvelopeBackend: Send + Sync {
    fn get_cache(&self, key: &str, lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>>;

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()>;

    async fn async_get_cache(
        &self,
        key: &str,
        lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>>;

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()>;

    async fn batch_cache_write(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()>;
}

// The storage contract takes no lookup, so this bridge drops the projection the plain backends never read.
pub struct StoredBackend {
    pub backend: Arc<dyn CacheBackend>,
}

#[async_trait]
impl EnvelopeBackend for StoredBackend {
    fn get_cache(&self, key: &str, _lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        self.backend.get_cache(key)
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.backend.set_cache(key, value, ttl)
    }

    async fn async_get_cache(
        &self,
        key: &str,
        _lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        self.backend.async_get_cache(key).await
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.backend.async_set_cache(key, value, ttl).await
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.backend.batch_cache_write(key, value, ttl).await
    }
}
