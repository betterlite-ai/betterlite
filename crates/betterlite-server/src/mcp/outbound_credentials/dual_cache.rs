mod tests;

use std::error::Error;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;

use super::cache_codec::OAuthTokenCacheCodec;
use super::{OAuthToken, TokenCacheBackend, TokenKey};

#[async_trait]
pub trait AsyncCache: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, Self::Error>;
    async fn async_set_cache(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), Self::Error>;
    async fn async_delete_cache(&self, key: &str) -> Result<(), Self::Error>;
}

pub struct DualCacheTokenCacheBackend<Cache> {
    pub cache: Cache,
    pub codec: OAuthTokenCacheCodec,
    pub key_prefix: &'static str,
}

#[async_trait]
impl<Cache: AsyncCache> TokenCacheBackend for DualCacheTokenCacheBackend<Cache> {
    async fn get(&self, key: &TokenKey) -> Option<OAuthToken> {
        let cache_key = format!("{}{}:{}", self.key_prefix, key.user_id, key.server_id);
        match self.cache.async_get_cache(&cache_key).await {
            Ok(Some(CacheValue::Text(blob))) => self.codec.decode(&blob),
            Ok(_) => None,
            Err(error) => {
                tracing::debug!(%error, "MCP per-user token cache get failed (miss)");
                None
            }
        }
    }

    async fn set(&self, key: &TokenKey, token: &OAuthToken, ttl: Duration) {
        if ttl.is_zero() {
            return;
        }
        let cache_key = format!("{}{}:{}", self.key_prefix, key.user_id, key.server_id);
        let encoded = match self.codec.encode(token) {
            Ok(encoded) => encoded,
            Err(error) => {
                tracing::debug!(%error, "MCP per-user token cache set failed (ignored)");
                return;
            }
        };
        if let Err(error) = self
            .cache
            .async_set_cache(&cache_key, &encoded, Some(ttl))
            .await
        {
            tracing::debug!(%error, "MCP per-user token cache set failed (ignored)");
        }
    }

    async fn delete(&self, key: &TokenKey) {
        let cache_key = format!("{}{}:{}", self.key_prefix, key.user_id, key.server_id);
        if let Err(error) = self.cache.async_delete_cache(&cache_key).await {
            tracing::debug!(%error, "MCP per-user token cache delete failed (ignored)");
        }
    }
}
