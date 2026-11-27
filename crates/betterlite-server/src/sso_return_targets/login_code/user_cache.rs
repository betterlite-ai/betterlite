use async_trait::async_trait;
use betterlite_auth::auth_cache::backend::AuthCacheTier;
use betterlite_auth::auth_cache::{AuthCache, WriteOptions, WriteTtl};

use super::{CacheWriteError, LoginCode, LoginCodeCache};

#[async_trait]
impl<Tier: AuthCacheTier> LoginCodeCache for AuthCache<Tier> {
    async fn store_login_code(
        &self,
        cache_key: &str,
        code: &LoginCode<'_>,
        ttl_seconds: f64,
    ) -> Result<(), CacheWriteError> {
        let options = WriteOptions {
            ttl: Some(WriteTtl {
                seconds: ttl_seconds,
            }),
            local_only: false,
            refresh_ttl: false,
        };
        self.async_set(Some(cache_key), code, options)
            .await
            .map_err(CacheWriteError::UserCache)
    }
}
