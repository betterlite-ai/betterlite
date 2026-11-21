use async_trait::async_trait;
use betterlite_db::identity_rows::user::LiteLLMUserTable;

use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::payload::PayloadError;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};

#[derive(Debug, thiserror::Error)]
pub enum UserCacheError {
    #[error("the cached user could not be read")]
    Read(#[source] PayloadError),
    #[error("the user could not be cached")]
    Write(#[source] AuthCacheError),
}

// async_get_cache / async_set_cache over B733, read and write under the caller's key
#[async_trait]
pub trait UserCache: Send + Sync {
    async fn read(&self, key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError>;
    async fn write(
        &self,
        key: &str,
        user: &LiteLLMUserTable,
        ttl: f64,
    ) -> Result<(), UserCacheError>;
    fn management_ttl(&self) -> f64;
}

#[async_trait]
impl<Tier> UserCache for AuthCache<Tier>
where
    Tier: AuthCacheTier,
{
    async fn read(&self, key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        self.async_get_typed::<LiteLLMUserTable>(key, false)
            .await
            .map_err(UserCacheError::Read)
    }

    async fn write(
        &self,
        key: &str,
        user: &LiteLLMUserTable,
        ttl: f64,
    ) -> Result<(), UserCacheError> {
        let options = WriteOptions {
            ttl: Some(WriteTtl { seconds: ttl }),
            local_only: false,
            refresh_ttl: false,
        };
        self.async_set(Some(key), user, options)
            .await
            .map_err(UserCacheError::Write)
    }

    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }
}
