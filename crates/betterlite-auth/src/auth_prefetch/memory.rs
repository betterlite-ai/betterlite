use std::sync::{Arc, PoisonError};

use betterlite_cache::backend::BackendError;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::{DualCache, MemoryMaxSize};
use betterlite_context::local_ttl_cache::CacheValue;

use super::redis_rows::{PrefetchRedis, RedisRows};
use crate::auth_cache::AuthCache;

// litellm's InMemoryCache: get_cache answers None on a backend failure, set_cache raises
pub trait PrefetchMemory: Send + Sync {
    fn get(&self, key: &str) -> Option<CacheValue>;
    fn set(&self, key: &str, value: CacheValue, ttl: Option<f64>) -> Result<(), MemoryWriteError>;
}
#[derive(Debug, thiserror::Error)]
#[error("the in-memory cache write failed")]
pub struct MemoryWriteError(#[source] pub(crate) BackendError);
impl<Memory> PrefetchMemory for Memory
where
    Memory: MemoryCacheOps + ?Sized,
{
    fn get(&self, key: &str) -> Option<CacheValue> {
        self.get_cache(key).unwrap_or_else(|error| {
            tracing::error!(%error, "the in-memory cache read failed");
            None
        })
    }
    fn set(&self, key: &str, value: CacheValue, ttl: Option<f64>) -> Result<(), MemoryWriteError> {
        self.set_cache(key, value, ttl).map_err(MemoryWriteError)
    }
}
pub trait PrefetchCache: Send + Sync {
    fn ordinary_memory(&self) -> &dyn PrefetchMemory;
    fn memory_for(&self, key: &str) -> &dyn PrefetchMemory;
    fn management_ttl(&self) -> f64;
    fn default_memory_ttl(&self) -> Option<f64>;
    fn redis(&self) -> Option<Arc<dyn PrefetchRedis>>;
}
impl<Memory> PrefetchCache for AuthCache<DualCache<Memory>>
where
    Memory: MemoryCacheOps + MemoryMaxSize + Send + Sync + 'static,
{
    fn ordinary_memory(&self) -> &dyn PrefetchMemory {
        self.ordinary.in_memory_cache.as_ref()
    }
    fn memory_for(&self, key: &str) -> &dyn PrefetchMemory {
        self.in_memory_cache_for(key).as_ref()
    }
    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }
    fn default_memory_ttl(&self) -> Option<f64> {
        let ttl = &self.ordinary.default_in_memory_ttl;
        *ttl.read().unwrap_or_else(PoisonError::into_inner)
    }
    fn redis(&self) -> Option<Arc<dyn PrefetchRedis>> {
        let client = self
            .ordinary
            .redis_cache
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        Some(Arc::new(RedisRows { client }))
    }
}
