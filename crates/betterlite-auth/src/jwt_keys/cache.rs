use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};

// The values JWTHandler stores are raw JSON: a keys list, a jwks_uri string and a numeric epoch, none of which
// is an object the typed cache codec would accept.
#[async_trait]
pub trait KeyCache: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<Json>, AuthCacheError>;
    async fn set(&self, key: &str, value: &Json, ttl_seconds: f64) -> Result<(), AuthCacheError>;
}

#[async_trait]
impl<Tier: AuthCacheTier> KeyCache for AuthCache<Tier> {
    async fn get(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(self.async_get(key, false).await)
    }

    async fn set(&self, key: &str, value: &Json, ttl_seconds: f64) -> Result<(), AuthCacheError> {
        self.async_set(
            Some(key),
            value,
            WriteOptions {
                ttl: Some(WriteTtl {
                    seconds: ttl_seconds,
                }),
                local_only: false,
                refresh_ttl: false,
            },
        )
        .await
    }
}
