use std::error::Error;
use std::future::Future;
use std::pin::Pin;

// GLOBAL_PROXY_SPEND_CACHE_KEY: the cached global-proxy accumulator that is dropped after the proxy budget tier resets.
pub const GLOBAL_PROXY_SPEND_CACHE_KEY: &str = "default_user_id:spend";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidatedCache {
    SpendCounter,
    UserApiKeyCache,
}

impl InvalidatedCache {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SpendCounter => "spend counter",
            Self::UserApiKeyCache => "user_api_key_cache",
        }
    }
}

pub type CacheDeleteFuture<'cache, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'cache>>;

// The cache side of the reset job: spend counters and user_api_key_cache are separate sinks so one failing still leaves the other invalidated.
pub trait ResetCache: Send + Sync {
    type Failure: Error + Send + Sync + 'static;

    fn delete_spend_counters<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, Self::Failure>;

    fn delete_user_api_keys<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, Self::Failure>;
}

pub async fn invalidate_caches<Cache>(cache: &Cache, counter_keys: &[String], cache_keys: &[String])
where
    Cache: ResetCache,
{
    invalidate_cache(cache, InvalidatedCache::SpendCounter, counter_keys).await;
    invalidate_cache(cache, InvalidatedCache::UserApiKeyCache, cache_keys).await;
}

async fn invalidate_cache<Cache>(cache: &Cache, sink: InvalidatedCache, keys: &[String])
where
    Cache: ResetCache,
{
    if keys.is_empty() {
        return;
    }
    let outcome = match sink {
        InvalidatedCache::SpendCounter => cache.delete_spend_counters(keys).await,
        InvalidatedCache::UserApiKeyCache => cache.delete_user_api_keys(keys).await,
    };
    let Err(error) = outcome else {
        return;
    };
    // Warning only: a stale cache entry expires naturally, while raising here would fail a reset whose DB row already committed.
    tracing::warn!(
        cache = sink.as_str(),
        entries = keys.len(),
        error = %error,
        "Failed to invalidate cache entries; budgets may be over-enforced until they expire"
    );
}

mod tests;
