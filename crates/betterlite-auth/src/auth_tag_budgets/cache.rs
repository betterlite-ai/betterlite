use async_trait::async_trait;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_models::decisions::Json;

use super::groups::ModelAccessGroupBudget;
use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::payload::PayloadError;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};

// The cache surface both batch reads and both registries use: typed tag and group values, raw registry answers, one ttl.
#[async_trait]
pub trait TagBudgetCache: Send + Sync {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError>;
    async fn write_tag(&self, key: &str, tag: &TagTable, ttl: f64) -> Result<(), AuthCacheError>;
    async fn read_group(&self, key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError>;
    async fn write_group(
        &self,
        key: &str,
        budget: &ModelAccessGroupBudget,
        ttl: f64,
    ) -> Result<(), AuthCacheError>;
    async fn read_registry(&self, key: &str) -> Result<Option<Json>, AuthCacheError>;
    async fn write_registry(
        &self,
        key: &str,
        answer: &Json,
        ttl: f64,
    ) -> Result<(), AuthCacheError>;
    fn management_ttl(&self) -> f64;
}

// LiteLLM writes every tag and group row with get_management_object_ttl and never refreshes it
const fn management_options(ttl: f64) -> WriteOptions {
    WriteOptions {
        ttl: Some(WriteTtl { seconds: ttl }),
        local_only: false,
        refresh_ttl: false,
    }
}

#[async_trait]
impl<T: AuthCacheTier + 'static> TagBudgetCache for AuthCache<T> {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError> {
        self.async_get_typed(key, false).await
    }

    async fn write_tag(&self, key: &str, tag: &TagTable, ttl: f64) -> Result<(), AuthCacheError> {
        self.async_set(Some(key), tag, management_options(ttl))
            .await
    }

    async fn read_group(&self, key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        self.async_get_typed(key, false).await
    }

    async fn write_group(
        &self,
        key: &str,
        budget: &ModelAccessGroupBudget,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.async_set(Some(key), budget, management_options(ttl))
            .await
    }

    async fn read_registry(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(self.async_get(key, false).await)
    }

    async fn write_registry(
        &self,
        key: &str,
        answer: &Json,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.async_set(Some(key), answer, management_options(ttl))
            .await
    }

    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }
}
