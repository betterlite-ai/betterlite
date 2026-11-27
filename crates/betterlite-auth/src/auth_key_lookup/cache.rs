use std::sync::Arc;

use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::auth_cache::AuthCache;
use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::decode::decode_auth_payload;
use crate::auth_cache::payload::ReadOperation;
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
        let mut cached = self.async_get(hashed_token, false).await?;
        let Json::Object(entries) = &mut cached else {
            return None;
        };
        // model_validate keeps a null optional field as None, and serde's flatten rejects the null before the field reads it
        entries.retain(|_, value| !matches!(value, Json::Null));
        decode_auth_payload(&cached, ReadOperation::AsyncGet)
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
