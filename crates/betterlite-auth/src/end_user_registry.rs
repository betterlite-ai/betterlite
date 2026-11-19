pub mod cache;
mod end_user_rows;
pub mod errors;
pub mod loader;
pub mod predicate;
mod tests;

use std::collections::BTreeSet;

use betterlite_config::sections::end_user_registry::Settings;
use betterlite_db::postgres_store::session::PostgresStore;
use tokio::sync::Mutex;

use crate::auth_cache::keys::{CacheKey, END_USER_RESTRICTED_OVERFLOW};

use self::cache::RegistryCache;
use self::end_user_rows::restricted_end_user_ids;
use self::errors::RegistryError;
use self::loader::{RegistrySpec, load_bounded_registry};

// LiteLLM's fixed bound on one cached registry: past it the registry is unusable and every id is read
pub const RESTRICTED_REGISTRY_MAX_SIZE: usize = 5000;
// the query takes one row past the bound, so a truncated answer is distinguishable from a complete one
pub const RESTRICTED_REGISTRY_TAKE: i64 = 5001;
// a failed load suppresses the registry query for this many seconds and fails open to per-id lookups
pub const REGISTRY_ERROR_TTL: f64 = 30.0;

// the per-call reasons an end-user row's mere existence matters, so the registry cannot answer for it
#[derive(Clone, Copy)]
pub struct RegistryGuards<'call> {
    // ast-grep-ignore: no-float-money
    pub token_end_user_max_budget: Option<f64>,
    pub key_end_user_budget_id: Option<&'call str>,
}

pub struct EndUserRegistry {
    load_lock: Mutex<()>,
    settings: Settings,
}

impl EndUserRegistry {
    #[must_use]
    pub fn new(settings: Settings) -> Self {
        Self {
            load_lock: Mutex::new(()),
            settings,
        }
    }

    // LiteLLM's _end_user_is_known_unrestricted: true only when a usable registry lacks the id
    pub async fn is_known_unrestricted<Cache>(
        &self,
        store: &PostgresStore,
        cache: &Cache,
        end_user_id: &str,
        guards: RegistryGuards<'_>,
    ) -> Result<bool, RegistryError>
    where
        Cache: RegistryCache + ?Sized,
    {
        if self.settings.validate_end_user_id_in_db
            || self.settings.max_end_user_budget_id.is_some()
            || guards.key_end_user_budget_id.is_some()
            || guards.token_end_user_max_budget.is_some()
        {
            return Ok(false);
        }
        let registry = self.restricted_registry(store, cache).await?;
        Ok(registry.is_some_and(|registry| !registry.contains(end_user_id)))
    }

    // LiteLLM's _load_end_user_restricted_registry: one cache key and one load lock for every caller
    async fn restricted_registry<Cache>(
        &self,
        store: &PostgresStore,
        cache: &Cache,
    ) -> Result<Option<BTreeSet<String>>, RegistryError>
    where
        Cache: RegistryCache + ?Sized,
    {
        let cache_key = CacheKey::EndUserRestrictedRegistry.render();
        let spec = RegistrySpec {
            cache_key: &cache_key,
            overflow_sentinel: END_USER_RESTRICTED_OVERFLOW,
            max_size: RESTRICTED_REGISTRY_MAX_SIZE,
        };
        load_bounded_registry(cache, spec, &self.load_lock, || {
            restricted_end_user_ids(store, RESTRICTED_REGISTRY_TAKE)
        })
        .await
    }
}
