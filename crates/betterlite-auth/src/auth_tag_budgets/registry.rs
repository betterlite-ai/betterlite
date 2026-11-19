use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use tokio::sync::Mutex;

use super::cache::TagBudgetCache;
use super::rows::{BudgetRows, RowName, TagBudgetRowsFailure};
use crate::auth_cache::AuthCacheError;
use crate::auth_cache::keys::{CacheKey, GROUP_REGISTRY_OVERFLOW, TAG_REGISTRY_OVERFLOW};
use crate::end_user_registry::cache::RegistryCache;
use crate::end_user_registry::errors::RegistryError;
use crate::end_user_registry::loader::{RegistrySpec, load_bounded_registry};

// LiteLLM's fixed bound on one cached registry, and the row past it that marks an overflow.
pub const TAG_REGISTRY_MAX_SIZE: usize = 5000;
pub const TAG_REGISTRY_TAKE: i64 = 5001;
pub const GROUP_REGISTRY_TAKE: i64 = 5001;

// One load lock per registry, so a tag load never queues behind a model access group load.
static TAG_REGISTRY_LOAD_LOCK: Mutex<()> = Mutex::const_new(());
static GROUP_REGISTRY_LOAD_LOCK: Mutex<()> = Mutex::const_new(());

// Which registry a load reads: each has its own cache key, sentinel, row bound and load lock.
#[derive(Clone, Copy)]
pub enum RegistryKind {
    Tag,
    Group,
}

// B1218's loader reads through RegistryCache; this view is the same cache seen through that surface.
struct RegistryView<'cache>(&'cache dyn TagBudgetCache);

#[async_trait]
impl RegistryCache for RegistryView<'_> {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.0.read_registry(cache_key).await
    }

    async fn write(&self, cache_key: &str, answer: &Json, ttl: f64) -> Result<(), AuthCacheError> {
        self.0
            .write(cache_key, super::cache::CachedValue::Registry(answer), ttl)
            .await
    }

    fn management_ttl(&self) -> f64 {
        self.0.management_ttl()
    }
}

// A failed row lookup reaches the loader as the typed cause rather than its text.
#[derive(Debug, thiserror::Error)]
#[error("the registry row lookup failed")]
struct RegistryRows(#[from] TagBudgetRowsFailure);

// LiteLLM's _load_tag_registry and _load_model_access_group_registry: the names one row read can prove exist.
pub async fn load_registry<Row: RowName>(
    kind: RegistryKind,
    cache: &dyn TagBudgetCache,
    rows: &dyn BudgetRows<Row>,
) -> Result<Option<BTreeSet<String>>, RegistryError> {
    let (key, sentinel, take, lock) = match kind {
        RegistryKind::Tag => (
            CacheKey::TagRegistry,
            TAG_REGISTRY_OVERFLOW,
            TAG_REGISTRY_TAKE,
            &TAG_REGISTRY_LOAD_LOCK,
        ),
        RegistryKind::Group => (
            CacheKey::ModelAccessGroupRegistry,
            GROUP_REGISTRY_OVERFLOW,
            GROUP_REGISTRY_TAKE,
            &GROUP_REGISTRY_LOAD_LOCK,
        ),
    };
    let cache_key = key.render();
    let spec = RegistrySpec {
        cache_key: &cache_key,
        overflow_sentinel: sentinel,
        max_size: TAG_REGISTRY_MAX_SIZE,
    };
    load_bounded_registry(&RegistryView(cache), spec, lock, || async move {
        let fetched = rows.registry_rows(take).await.map_err(RegistryRows::from)?;
        Ok(fetched
            .iter()
            .map(|row| row.row_name().to_owned())
            .collect())
    })
    .await
}
