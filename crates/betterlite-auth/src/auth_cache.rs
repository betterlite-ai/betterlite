pub mod backend;
pub mod copy;
pub mod decode;
pub mod keys;
pub mod payload;

mod operations;

mod tests;

use std::sync::Arc;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::{
    CacheOptions, DualCache, DualCacheError, DualCacheSettings, MemoryMaxSize,
};
use betterlite_redis::redis_cache::RedisCache;
use thiserror::Error;

use self::backend::AuthCacheTier;
use self::payload::PayloadError;

#[derive(Debug, Error)]
pub enum AuthCacheError {
    #[error("the cached payload could not be serialized")]
    Payload(#[source] Box<PayloadError>),
    #[error("the cache backend failed")]
    Backend(#[source] Box<DualCacheError>),
}

impl From<PayloadError> for AuthCacheError {
    fn from(error: PayloadError) -> Self {
        Self::Payload(Box::new(error))
    }
}

impl From<DualCacheError> for AuthCacheError {
    fn from(error: DualCacheError) -> Self {
        Self::Backend(Box::new(error))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WriteTtl {
    Seconds(f64),
    // Python's bool is an int, so LiteLLM's numeric ttl check accepts one
    Boolean(bool),
}

impl WriteTtl {
    #[must_use]
    pub const fn seconds(self) -> f64 {
        match self {
            Self::Seconds(seconds) => seconds,
            Self::Boolean(flag) => {
                if flag {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WriteOptions {
    pub ttl: Option<WriteTtl>,
    pub local_only: bool,
    pub refresh_ttl: bool,
}

impl WriteOptions {
    #[must_use]
    pub fn cache_options(self) -> CacheOptions {
        CacheOptions {
            ttl: self.ttl.map(WriteTtl::seconds),
            local_only: self.local_only,
            refresh_ttl: self.refresh_ttl,
        }
    }
}

pub struct AuthCache<T: AuthCacheTier> {
    pub(crate) ordinary: T,
    pub(crate) key_objects: T,
    pub(crate) management_ttl_fallback: f64,
}

// two independent memory partitions over the same optional Redis and the same supplied TTLs
pub fn gateway_cache<Memory: MemoryCacheOps + MemoryMaxSize>(
    in_memory: Arc<Memory>,
    key_object_memory: Arc<Memory>,
    redis: Option<Arc<RedisCache>>,
    settings: DualCacheSettings,
    management_ttl_fallback: f64,
) -> AuthCache<DualCache<Memory>> {
    AuthCache {
        ordinary: DualCache::new(in_memory, redis.clone(), settings),
        key_objects: DualCache::new(key_object_memory, redis, settings),
        management_ttl_fallback,
    }
}
