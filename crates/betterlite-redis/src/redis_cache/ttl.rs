use redis::cmd;

use super::RedisCache;
use super::error::RedisCacheError;
use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::redis_client_reuse::breaker::BreakerAdmission;

#[must_use]
pub const fn remaining_ttl(reply: i64) -> Option<i64> {
    if reply <= -1 { None } else { Some(reply) }
}

pub(crate) async fn expire(
    cache: &RedisCache,
    key: &str,
    ttl: i64,
) -> Result<bool, RedisCacheError> {
    let mut command = cmd("EXPIRE");
    command.arg(key).arg(ttl);
    let client = cache.transport.async_client().await?;
    let reply: i64 = client.query(&command).await?;
    Ok(reply == 1)
}

// a TTL read reports no hook, so its failure only reaches the breaker and a debug line
pub(crate) async fn read(
    cache: &RedisCache,
    key: &str,
    admission: BreakerAdmission,
) -> Option<i64> {
    let outcome = match cache.transport.async_client().await {
        Ok(client) => client
            .ttl(key)
            .await
            .map(remaining_ttl)
            .map_err(RedisCacheError::from),
        Err(source) => Err(RedisCacheError::from(source)),
    };
    match outcome {
        Ok(remaining) => {
            cache.gate.exit(admission);
            remaining
        }
        Err(error) => {
            tracing::debug!("Redis TTL Error: {error}");
            cache.gate.swallowed(&admission, &error);
            cache.gate.exit(admission);
            None
        }
    }
}
