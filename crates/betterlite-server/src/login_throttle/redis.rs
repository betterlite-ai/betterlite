use std::sync::Arc;

use async_trait::async_trait;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_counters::scripts::RegisteredScript;
use redis::{Value, from_redis_value};
use thiserror::Error;

use super::keys::LoginKeys;

// KEYS are pair counter, pair block, source counter and source block, one cluster slot via the source hash tag; both scripts return {pair block TTL, source block TTL}, 0 or below meaning not blocked
pub const BLOCK_TTLS_LUA: &str = "return {redis.call('TTL', KEYS[2]), redis.call('TTL', KEYS[4])}";
// ARGV is pair limit, source limit (0 = source scope off), window seconds and block seconds
pub const RECORD_FAILURE_LUA: &str = concat!(
    "local function bump(count_key, block_key, limit) ",
    "local blocked = redis.call('TTL', block_key) ",
    "if blocked > 0 then return blocked end ",
    "local count = redis.call('INCR', count_key) ",
    "if redis.call('TTL', count_key) < 0 then redis.call('EXPIRE', count_key, ARGV[3]) end ",
    "if count > limit then redis.call('SET', block_key, '1', 'EX', ARGV[4]) return tonumber(ARGV[4]) end ",
    "return 0 end ",
    "local user_block = bump(KEYS[1], KEYS[2], tonumber(ARGV[1])) ",
    "local source_block = 0 ",
    "if tonumber(ARGV[2]) > 0 and user_block == 0 then ",
    "source_block = bump(KEYS[3], KEYS[4], tonumber(ARGV[2])) end ",
    "return {user_block, source_block}",
);

// the block TTLs for both scopes; a scope at or below zero is not blocked
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BlockTtls {
    pub user: i64,
    pub source: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FailureLimits {
    pub user_limit: i64,
    pub source_limit: i64,
    pub window_seconds: i64,
    pub block_seconds: i64,
}

// LiteLLM catches RedisError, RedisCircuitBreakerOpenError, OSError and asyncio.TimeoutError, so every shared failure is one recoverable class; a reply that is not two whole numbers is a validation error and propagates instead
#[derive(Debug, Error)]
pub enum ThrottleRedisError {
    #[error("redis failed while counting Admin UI sign-in attempts: {reason}")]
    Unavailable { reason: String },
    #[error("the login block TTL reply is not two whole numbers")]
    MalformedBlockTtls,
}

impl From<RedisCacheError> for ThrottleRedisError {
    fn from(error: RedisCacheError) -> Self {
        Self::Unavailable {
            reason: error.to_string(),
        }
    }
}

// the three shared operations login_throttle.py runs, as typed collaborators a test can replace
#[async_trait]
pub trait LoginRedis: Send + Sync {
    async fn block_ttls(&self, keys: &LoginKeys) -> Result<BlockTtls, ThrottleRedisError>;
    async fn record_failure(
        &self,
        keys: &LoginKeys,
        limits: FailureLimits,
    ) -> Result<BlockTtls, ThrottleRedisError>;
    async fn delete_counter(&self, key: &str) -> Result<(), ThrottleRedisError>;
}

// B126's registered scripts over one shared cache, as LiteLLM's async_register_script results are
pub struct RedisLogin {
    cache: Arc<RedisCache>,
    block_ttls: RegisteredScript,
    record_failure: RegisteredScript,
}

impl RedisLogin {
    #[must_use]
    pub fn new(cache: Arc<RedisCache>) -> Self {
        let block_ttls = cache.counters.async_register_script(BLOCK_TTLS_LUA);
        let record_failure = cache.counters.async_register_script(RECORD_FAILURE_LUA);
        Self {
            cache,
            block_ttls,
            record_failure,
        }
    }
}

#[async_trait]
impl LoginRedis for RedisLogin {
    async fn block_ttls(&self, keys: &LoginKeys) -> Result<BlockTtls, ThrottleRedisError> {
        let reply = self
            .cache
            .run_script(&self.block_ttls, &keys.all(), &[])
            .await?;
        parse_block_ttls(&reply)
    }

    async fn record_failure(
        &self,
        keys: &LoginKeys,
        limits: FailureLimits,
    ) -> Result<BlockTtls, ThrottleRedisError> {
        let args = [
            RedisScalar::Text(limits.user_limit.to_string()),
            RedisScalar::Text(limits.source_limit.to_string()),
            RedisScalar::Text(limits.window_seconds.to_string()),
            RedisScalar::Text(limits.block_seconds.to_string()),
        ];
        let reply = self
            .cache
            .run_script(&self.record_failure, &keys.all(), &args)
            .await?;
        parse_block_ttls(&reply)
    }

    async fn delete_counter(&self, key: &str) -> Result<(), ThrottleRedisError> {
        self.cache
            .admin
            .async_delete_cache(key)
            .await
            .map(|_count| ())
            .map_err(|error| ThrottleRedisError::Unavailable {
                reason: error.to_string(),
            })
    }
}

// _LUA_BLOCK_TTLS validates a two-element sequence of ints; anything else is a malformed reply
fn parse_block_ttls(reply: &Value) -> Result<BlockTtls, ThrottleRedisError> {
    let values: Vec<i64> =
        from_redis_value(reply).map_err(|_error| ThrottleRedisError::MalformedBlockTtls)?;
    let [user, source] = values.as_slice() else {
        return Err(ThrottleRedisError::MalformedBlockTtls);
    };
    Ok(BlockTtls {
        user: *user,
        source: *source,
    })
}
