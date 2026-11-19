use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use tokio::sync::Mutex;

use super::cache::TagBudgetCache;
use super::rows::{GroupRows, TagRows};
use crate::auth_cache::AuthCacheError;
use crate::auth_cache::keys::{CacheKey, GROUP_REGISTRY_OVERFLOW, TAG_REGISTRY_OVERFLOW};
use crate::end_user_registry::cache::RegistryCache;
use crate::end_user_registry::errors::RegistryError;
use crate::end_user_registry::loader::{RegistrySpec, load_bounded_registry};

// LiteLLM's fixed bound on one cached registry, and the row past it that marks an overflow.
pub const TAG_REGISTRY_MAX_SIZE: usize = 5000;
pub const TAG_REGISTRY_TAKE: i64 = 5001;
pub const GROUP_REGISTRY_MAX_SIZE: usize = 5000;
pub const GROUP_REGISTRY_TAKE: i64 = 5001;

// One load lock per registry, so a tag load never queues behind a model access group load.
static TAG_REGISTRY_LOAD_LOCK: Mutex<()> = Mutex::const_new(());
static GROUP_REGISTRY_LOAD_LOCK: Mutex<()> = Mutex::const_new(());

// B1218's loader reads through RegistryCache; this view is the same cache seen through that surface.
struct RegistryView<'cache>(&'cache dyn TagBudgetCache);

#[async_trait]
impl RegistryCache for RegistryView<'_> {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.0.read_registry(cache_key).await
    }

    async fn write(&self, cache_key: &str, answer: &Json, ttl: f64) -> Result<(), AuthCacheError> {
        self.0.write_registry(cache_key, answer, ttl).await
    }

    fn management_ttl(&self) -> f64 {
        self.0.management_ttl()
    }
}

// LiteLLM's _load_tag_registry: the names one LiteLLM_TagTable read can prove exist.
pub async fn load_tag_registry(
    cache: &dyn TagBudgetCache,
    rows: &dyn TagRows,
) -> Result<Option<BTreeSet<String>>, RegistryError> {
    let cache_key = CacheKey::TagRegistry.render();
    let spec = RegistrySpec {
        cache_key: &cache_key,
        overflow_sentinel: TAG_REGISTRY_OVERFLOW,
        max_size: TAG_REGISTRY_MAX_SIZE,
    };
    load_bounded_registry(
        &RegistryView(cache),
        spec,
        &TAG_REGISTRY_LOAD_LOCK,
        || async {
            let fetched = rows
                .registry_rows(TAG_REGISTRY_TAKE)
                .await
                .map_err(|failure| failure.to_string())?;
            Ok(fetched.into_iter().map(|row| row.tag_name).collect())
        },
    )
    .await
}

// LiteLLM's _load_model_access_group_registry: the group names that have a budget row.
pub async fn load_model_access_group_registry(
    cache: &dyn TagBudgetCache,
    rows: &dyn GroupRows,
) -> Result<Option<BTreeSet<String>>, RegistryError> {
    let cache_key = CacheKey::ModelAccessGroupRegistry.render();
    let spec = RegistrySpec {
        cache_key: &cache_key,
        overflow_sentinel: GROUP_REGISTRY_OVERFLOW,
        max_size: GROUP_REGISTRY_MAX_SIZE,
    };
    load_bounded_registry(
        &RegistryView(cache),
        spec,
        &GROUP_REGISTRY_LOAD_LOCK,
        || async {
            let fetched = rows
                .registry_rows(GROUP_REGISTRY_TAKE)
                .await
                .map_err(|failure| failure.to_string())?;
            Ok(fetched
                .into_iter()
                .map(|row| row.access_group_name)
                .collect())
        },
    )
    .await
}
