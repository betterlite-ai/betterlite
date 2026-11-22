use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::auth_cache::AuthCache;
use crate::auth_cache::backend::AuthCacheTier;

// user_api_key_cache.async_get_cache(key=...), the auth cache a counter falls back to when its row is missing
#[async_trait]
pub trait SourceSpendCache: Send + Sync {
    async fn cached(&self, cache_key: &str) -> Option<Json>;
}

#[async_trait]
impl<T> SourceSpendCache for AuthCache<T>
where
    T: AuthCacheTier + 'static,
{
    async fn cached(&self, cache_key: &str) -> Option<Json> {
        self.async_get(cache_key, false).await
    }
}
