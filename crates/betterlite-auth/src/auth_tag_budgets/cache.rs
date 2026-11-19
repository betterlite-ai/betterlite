use async_trait::async_trait;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_models::decisions::Json;

use super::groups::ModelAccessGroupBudget;
use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::payload::PayloadError;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};

// The three management objects a batch read writes back under the ttl LiteLLM gives them.
pub enum CachedValue<'value> {
    Tag(&'value TagTable),
    Group(&'value ModelAccessGroupBudget),
    Registry(&'value Json),
}

// The cache surface both batch reads and both registries use: typed reads, one typed write, one ttl.
#[async_trait]
pub trait TagBudgetCache: Send + Sync {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError>;
    async fn read_group(&self, key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError>;
    async fn read_registry(&self, key: &str) -> Result<Option<Json>, AuthCacheError>;
    async fn write(
        &self,
        key: &str,
        value: CachedValue<'_>,
        ttl: f64,
    ) -> Result<(), AuthCacheError>;
    fn management_ttl(&self) -> f64;
}

#[async_trait]
impl<T: AuthCacheTier + 'static> TagBudgetCache for AuthCache<T> {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError> {
        self.async_get_typed(key, false).await
    }

    async fn read_group(&self, key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        self.async_get_typed(key, false).await
    }

    async fn read_registry(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(self.async_get(key, false).await)
    }

    async fn write(
        &self,
        key: &str,
        value: CachedValue<'_>,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        // LiteLLM writes every tag and group row with get_management_object_ttl and never refreshes it
        let options = WriteOptions {
            ttl: Some(WriteTtl { seconds: ttl }),
            local_only: false,
            refresh_ttl: false,
        };
        match value {
            CachedValue::Tag(tag) => self.async_set(Some(key), tag, options).await,
            CachedValue::Group(group) => self.async_set(Some(key), group, options).await,
            CachedValue::Registry(answer) => self.async_set(Some(key), answer, options).await,
        }
    }

    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }
}
