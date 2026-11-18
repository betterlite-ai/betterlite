use std::error::Error;
use std::time::Duration;

use async_trait::async_trait;

// the two DualCache tiers the handler reaches, one method per LiteLLM call it stands for
#[async_trait]
pub trait SensitiveRoutingCache: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    // dual_cache.redis_cache is not None
    fn redis_configured(&self) -> bool;

    // redis_cache.async_get_cache; Err is the exception that sends the read to the in-memory tier
    async fn redis_get(&self, key: &str) -> Result<Option<String>, Self::Error>;

    // redis_cache.async_get_ttl; None inherits the configured ttl, and Err abandons the hit like the GET's
    async fn redis_ttl(&self, key: &str) -> Result<Option<Duration>, Self::Error>;

    // redis_cache.async_set_cache; Err only logs a warning, because the local set always follows
    async fn redis_set(&self, key: &str, value: &str, ttl: Duration) -> Result<(), Self::Error>;

    // async_get_cache(key, local_only=True)
    async fn memory_get(&self, key: &str) -> Option<String>;

    // async_set_cache(key, value, ttl, local_only=True)
    async fn memory_set(&self, key: &str, value: &str, ttl: Duration);
}
