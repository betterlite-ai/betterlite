use async_trait::async_trait;
use betterlite_redis::redis_cache::RedisCache;

use super::{CacheWriteError, LoginCode, LoginCodeCache};

#[async_trait]
impl LoginCodeCache for RedisCache {
    async fn store_login_code(
        &self,
        cache_key: &str,
        code: &LoginCode<'_>,
        ttl_seconds: f64,
    ) -> Result<(), CacheWriteError> {
        let _stored = self
            .values
            .async_set_cache(Some(cache_key), code, Some(ttl_seconds), false)
            .await
            .map_err(CacheWriteError::RedisUsage)?;
        Ok(())
    }
}
