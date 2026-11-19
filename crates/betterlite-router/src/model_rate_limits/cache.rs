use async_trait::async_trait;

// what the remote tier answered a tpm read with: a value (or an absent key), an open breaker, or any other failure
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteRead {
    Value(Option<i64>),
    BreakerOpen,
    Failed,
}

// LiteLLM's DualCache as the check reads it: the in-memory tier answers synchronously and the Redis tier asynchronously
#[async_trait]
pub trait RateLimitCache: Send + Sync {
    // dual_cache.get_cache(local_only=True)
    fn local(&self, key: &str) -> Option<i64>;

    // dual_cache.redis_cache is not None
    fn has_remote(&self) -> bool;

    // redis_cache.get_cache / async_get_cache called directly, so RedisCircuitBreakerOpenError stays distinguishable
    async fn remote(&self, key: &str) -> RemoteRead;

    // dual_cache.increment_cache against the in-memory tier alone, for the blocking facade
    fn increment_local(&self, key: &str, by: i64, ttl_seconds: u64) -> Option<i64>;

    // dual_cache.increment_cache / async_increment_cache: the in-memory tier counts first, then Redis wins, and an async Redis failure keeps the memory count
    async fn increment(&self, key: &str, by: i64, ttl_seconds: u64) -> Option<i64>;
}
