use async_trait::async_trait;
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;

use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct PermissionCacheError(#[from] AuthCacheError);

// async_get_cache(key, model_type) and async_set_cache(key, value, model_type, ttl) over B733
#[async_trait]
pub trait ObjectPermissionCache: Send + Sync {
    async fn get(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionCacheError>;

    async fn set(
        &self,
        key: &str,
        permission: &LiteLLMObjectPermissionTable,
        ttl: f64,
    ) -> Result<(), PermissionCacheError>;

    fn management_ttl(&self) -> f64;
}

#[async_trait]
impl<Tier> ObjectPermissionCache for AuthCache<Tier>
where
    Tier: AuthCacheTier,
{
    async fn get(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionCacheError> {
        self.async_get_typed::<LiteLLMObjectPermissionTable>(key, false)
            .await
            .map_err(|source| PermissionCacheError::from(AuthCacheError::from(source)))
    }

    async fn set(
        &self,
        key: &str,
        permission: &LiteLLMObjectPermissionTable,
        ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        let options = WriteOptions {
            ttl: Some(WriteTtl { seconds: ttl }),
            local_only: false,
            refresh_ttl: false,
        };
        self.async_set(Some(key), permission, options)
            .await
            .map_err(PermissionCacheError::from)
    }

    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }
}
