use super::RedisCache;
use super::error::RedisCacheError;
use crate::cache_primitives::protocol::AsyncRedisCommands as _;

#[must_use]
pub const fn remaining_ttl(reply: i64) -> Option<i64> {
    if reply <= -1 { None } else { Some(reply) }
}

pub(crate) async fn get(cache: &RedisCache, key: &str) -> Result<Option<i64>, RedisCacheError> {
    cache
        .transport
        .async_client()
        .await?
        .ttl(key)
        .await
        .map(remaining_ttl)
        .map_err(RedisCacheError::from)
}

pub(crate) async fn expire(
    cache: &RedisCache,
    key: &str,
    ttl: i64,
) -> Result<bool, RedisCacheError> {
    cache
        .transport
        .async_client()
        .await?
        .expire(key, ttl)
        .await
        .map_err(RedisCacheError::from)
}
