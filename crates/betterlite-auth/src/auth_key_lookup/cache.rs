use std::sync::Arc;

use async_trait::async_trait;

use crate::auth_cache::AuthCache;
use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_invalidation::AuthInvalidation;
use crate::auth_invalidation::error::InvalidationError;
use crate::resolved_key::auth::UserAPIKeyAuth;

// The key cache read: a payload that no longer validates is a miss, as UserApiKeyCache.async_get_cache answers None.
#[async_trait]
pub trait KeyCache: Send + Sync {
    async fn read_key(&self, hashed_token: &str) -> Option<UserAPIKeyAuth>;
}

#[async_trait]
impl<Tier> KeyCache for AuthCache<Tier>
where
    Tier: AuthCacheTier,
{
    async fn read_key(&self, hashed_token: &str) -> Option<UserAPIKeyAuth> {
        self.async_get_auth(hashed_token, false)
            .await
            .ok()
            .flatten()
    }
}

// B734's _cache_key_object: stamp the refresh and write the copy that drops request-local fields.
#[async_trait]
pub trait KeyCacheWriter: Send + Sync {
    async fn write_key(
        &self,
        hashed_token: &str,
        auth: &mut UserAPIKeyAuth,
    ) -> Result<(), InvalidationError>;
}

#[async_trait]
impl KeyCacheWriter for AuthInvalidation {
    async fn write_key(
        &self,
        hashed_token: &str,
        auth: &mut UserAPIKeyAuth,
    ) -> Result<(), InvalidationError> {
        self.cache_key_object(hashed_token, auth).await
    }
}

// The auth cache is held behind the trait above, so a test can answer a hit without a Redis tier.
pub type SharedKeyCache = Arc<dyn KeyCache>;
