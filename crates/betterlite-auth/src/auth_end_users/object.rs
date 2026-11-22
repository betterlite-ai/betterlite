use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::postgres_store::session::PostgresStore;
use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::auth_cache::payload::PayloadError;
use crate::end_user_registry::errors::RegistryError;
use crate::end_user_registry::{EndUserRegistry, RegistryGuards};

use super::apply::apply_default_budget;
use super::cache::EndUserCache;
use super::defaults::DefaultBudgetLookup;
use super::rows::{BudgetRows, EndUserRows};

// The per-request identity get_end_user_object resolves: the id plus what the key asks to apply to it
#[derive(Clone, Copy, Debug, Default)]
pub struct EndUserRequest<'req> {
    pub end_user_id: Option<&'req str>,
    pub key_end_user_budget_id: Option<&'req str>,
    pub token_end_user_max_budget: Option<Decimal>,
}

#[derive(Debug, thiserror::Error)]
pub enum EndUserObjectError {
    #[error("No db connected")]
    NoDatabase,
    #[error("the cached end-user row could not be read")]
    CacheRead(#[source] PayloadError),
    #[error("the end-user restricted registry could not be read")]
    Registry(#[source] RegistryError),
    #[error("the end-user default budget could not be resolved")]
    Defaults(#[source] PayloadError),
}

// The end-user lookup over the row table, B1218's registry and the budget rows its defaults read
pub struct EndUserObjects<
    'deps,
    Rows: EndUserRows + ?Sized,
    Budgets: BudgetRows + ?Sized,
    Cache: EndUserCache + ?Sized,
> {
    pub rows: Option<&'deps Rows>,
    pub budgets: Option<&'deps Budgets>,
    pub cache: &'deps Cache,
    pub registry: &'deps EndUserRegistry,
    pub store: &'deps PostgresStore,
    pub max_end_user_budget_id: Option<&'deps str>,
}

impl<Rows, Budgets, Cache> EndUserObjects<'_, Rows, Budgets, Cache>
where
    Rows: EndUserRows + ?Sized,
    Budgets: BudgetRows + ?Sized,
    Cache: EndUserCache + ?Sized,
{
    // LiteLLM's get_end_user_object: the cache, the registry fast path, then the caught database block
    pub async fn get_end_user_object(
        &self,
        request: &EndUserRequest<'_>,
    ) -> Result<Option<LiteLLMEndUserTable>, EndUserObjectError> {
        let Some(rows) = self.rows else {
            return Err(EndUserObjectError::NoDatabase);
        };
        let Some(end_user_id) = request.end_user_id else {
            return Ok(None);
        };
        let cache_key = CacheKey::EndUserId(end_user_id).render();
        if let Some(cached) = self
            .cache
            .read_row::<LiteLLMEndUserTable>(&cache_key)
            .await
            .map_err(EndUserObjectError::CacheRead)?
        {
            return apply_default_budget(&cached, &self.defaults(), request.key_end_user_budget_id)
                .await
                .map(Some)
                .map_err(EndUserObjectError::Defaults);
        }
        let guards = RegistryGuards {
            token_end_user_max_budget: request.token_end_user_max_budget,
            key_end_user_budget_id: request.key_end_user_budget_id,
        };
        if self
            .registry
            .is_known_unrestricted(self.store, self.cache, end_user_id, guards)
            .await
            .map_err(EndUserObjectError::Registry)?
        {
            return Ok(None);
        }
        Ok(self
            .store_object(rows, end_user_id, request.key_end_user_budget_id)
            .await)
    }

    fn defaults(&self) -> DefaultBudgetLookup<'_, Budgets, Cache> {
        DefaultBudgetLookup {
            rows: self.budgets,
            cache: self.cache,
            max_end_user_budget_id: self.max_end_user_budget_id,
        }
    }

    // the block LiteLLM wraps in one catch: a row, default and cache write failure all answer None
    async fn store_object(
        &self,
        rows: &Rows,
        end_user_id: &str,
        key_end_user_budget_id: Option<&str>,
    ) -> Option<LiteLLMEndUserTable> {
        let cache_key = CacheKey::EndUserId(end_user_id).render();
        let fetched = rows.find_end_user(end_user_id).await.ok()??;
        let stored = apply_default_budget(&fetched, &self.defaults(), None)
            .await
            .ok()?;
        self.cache
            .write_row(&cache_key, &stored, self.cache.management_ttl())
            .await
            .ok()?;
        if key_end_user_budget_id.is_none() {
            return Some(stored);
        }
        apply_default_budget(&stored, &self.defaults(), key_end_user_budget_id)
            .await
            .ok()
    }
}
