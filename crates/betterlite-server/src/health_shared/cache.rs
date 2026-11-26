use std::sync::Arc;

use async_trait::async_trait;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::RedisCache;
use num_traits::ToPrimitive as _;

use super::error::SharedHealthCacheError;

// The three calls the manager makes, as async_get_cache, async_set_cache and async_delete_cache serve them.
#[async_trait]
pub trait SharedHealthCache: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<CachedValue>, SharedHealthCacheError>;

    async fn set(
        &self,
        key: &str,
        value: &str,
        ttl_seconds: i64,
        only_if_absent: bool,
    ) -> Result<bool, SharedHealthCacheError>;

    async fn delete(&self, key: &str) -> Result<i64, SharedHealthCacheError>;
}

// B126's RedisCache as the port sees it: nx is SET NX and the TTL rides on EX.
pub struct RedisHealthCache {
    pub cache: Arc<RedisCache>,
}

#[async_trait]
impl SharedHealthCache for RedisHealthCache {
    // A live cache needs a running Redis, which the server tests fake at the SharedHealthCache seam.
    #[cfg_attr(test, mutants::skip)]
    async fn get(&self, key: &str) -> Result<Option<CachedValue>, SharedHealthCacheError> {
        self.cache
            .async_get_cache(key)
            .await
            .map_err(|source| SharedHealthCacheError::Read {
                operation: "async_get_cache",
                source,
            })
    }

    // A live cache needs a running Redis, which the server tests fake at the SharedHealthCache seam.
    #[cfg_attr(test, mutants::skip)]
    async fn set(
        &self,
        key: &str,
        value: &str,
        ttl_seconds: i64,
        only_if_absent: bool,
    ) -> Result<bool, SharedHealthCacheError> {
        let ttl = ttl_seconds.to_f64().unwrap_or_default();
        let stored = self
            .cache
            .values
            .async_set_cache(Some(key), value, Some(ttl), only_if_absent)
            .await
            .map_err(|source| SharedHealthCacheError::Write {
                operation: "async_set_cache",
                source,
            })?;
        Ok(stored == Some(true))
    }

    // A live cache needs a running Redis, which the server tests fake at the SharedHealthCache seam.
    #[cfg_attr(test, mutants::skip)]
    async fn delete(&self, key: &str) -> Result<i64, SharedHealthCacheError> {
        self.cache
            .admin
            .async_delete_cache(key)
            .await
            .map_err(|source| SharedHealthCacheError::Delete {
                operation: "async_delete_cache",
                source,
            })
    }
}
