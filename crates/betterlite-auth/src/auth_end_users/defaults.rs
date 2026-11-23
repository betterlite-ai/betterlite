use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::identity_rows::Payload;
use rust_decimal::Decimal;

use crate::auth_cache::AuthCacheError;
use crate::auth_cache::payload::PayloadError;
use crate::auth_read::table::AuthReadError;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::cache::{EndUserCache, EndUserCacheKey};
use super::rows::{BudgetFetchError, BudgetRows};

// LiteLLM's END_USER_BUDGET_ID_FIELD, the key metadata a default budget is named in
pub const END_USER_BUDGET_ID_FIELD: &str = "end_user_budget_id";

// key_metadata.get(field), kept only when it is a nonempty string: whitespace is a real id
#[must_use]
pub fn key_end_user_budget_id(metadata: Option<&Payload>) -> Option<&str> {
    match metadata?.0.get(END_USER_BUDGET_ID_FIELD)? {
        JsonData::String(id) if !id.is_empty() => Some(id),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}

// What get_end_user_object reads off the requesting key: its default end-user budget and its budget ceiling
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyEndUserDefaults {
    pub key_end_user_budget_id: Option<String>,
    pub token_end_user_max_budget: Option<Decimal>,
}

impl KeyEndUserDefaults {
    #[must_use]
    pub fn of_key(key: &UserAPIKeyAuth) -> Self {
        Self {
            key_end_user_budget_id: key_end_user_budget_id(Some(&key.view.token.metadata))
                .map(str::to_owned),
            token_end_user_max_budget: key.view.end_user_max_budget,
        }
    }
}

// The team-member read lets a decode failure escape, so it has its own failure type
#[derive(Debug, thiserror::Error)]
pub enum MemberBudgetError {
    #[error("the stored budget row is not readable")]
    Decode(#[source] Box<AuthReadError<TableError>>),
    #[error("the budget cache failed")]
    Cache(#[from] AuthCacheError),
}

impl From<PayloadError> for MemberBudgetError {
    fn from(source: PayloadError) -> Self {
        Self::Cache(AuthCacheError::from(source))
    }
}

// The two default-budget reads, over the budget rows, the management cache and the configured proxy default
pub struct DefaultBudgetLookup<'deps, Rows: BudgetRows + ?Sized, Cache: EndUserCache + ?Sized> {
    pub rows: Option<&'deps Rows>,
    pub cache: &'deps Cache,
    pub max_end_user_budget_id: Option<&'deps str>,
}

impl<Rows, Cache> DefaultBudgetLookup<'_, Rows, Cache>
where
    Rows: BudgetRows + ?Sized,
    Cache: EndUserCache + ?Sized,
{
    // LiteLLM's get_default_end_user_budget: every failure inside the fetch block becomes None
    pub async fn default_end_user_budget(
        &self,
        budget_id: Option<&str>,
    ) -> Result<Option<LiteLLMBudgetTable>, PayloadError> {
        let (Some(rows), Some(id)) = (self.rows, budget_id.or(self.max_end_user_budget_id)) else {
            return Ok(None);
        };
        let cache_key = EndUserCacheKey::DefaultEndUserBudget(id);
        if let Some(cached) = self.cached_budget(cache_key).await? {
            return Ok(Some(cached));
        }
        let fetched = match rows.find_budget(id).await {
            Ok(Some(budget)) => budget,
            Ok(None) => {
                tracing::warn!(
                    "Default end user budget not found in database: {}",
                    id.replace(['\r', '\n'], "")
                );
                return Ok(None);
            }
            Err(error) => {
                tracing::error!(error = %error, "Error fetching default end user budget");
                return Ok(None);
            }
        };
        if let Err(error) = self.store_budget(cache_key, &fetched).await {
            tracing::error!(error = %error, "Error fetching default end user budget");
            return Ok(None);
        }
        Ok(Some(fetched))
    }

    // LiteLLM's get_team_member_default_budget: only the query is caught, the write escapes
    pub async fn team_member_default_budget(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError> {
        let Some(rows) = self.rows else {
            return Ok(None);
        };
        let cache_key = EndUserCacheKey::TeamMemberDefaultBudget(budget_id);
        if let Some(cached) = self.cached_budget(cache_key).await? {
            return Ok(Some(cached));
        }
        let fetched = match rows.find_budget(budget_id).await {
            Ok(fetched) => fetched,
            Err(BudgetFetchError::Decode(source)) => {
                return Err(MemberBudgetError::Decode(source));
            }
            Err(BudgetFetchError::Query(error)) => {
                tracing::error!(
                    error = %error,
                    "Error fetching team-default member budget {}", budget_id
                );
                return Ok(None);
            }
        };
        let Some(budget) = fetched else {
            tracing::warn!(
                "Team-default member budget not found in database: {}",
                budget_id
            );
            return Ok(None);
        };
        self.store_budget(cache_key, &budget).await?;
        Ok(Some(budget))
    }

    // both budget reads answer a cached row before touching the database, under their own key
    async fn cached_budget(
        &self,
        key: EndUserCacheKey<'_>,
    ) -> Result<Option<LiteLLMBudgetTable>, PayloadError> {
        self.cache
            .read_row::<LiteLLMBudgetTable>(&key.render())
            .await
    }

    // both budget reads store a fetched row under the management ttl, so the next read hits it
    async fn store_budget(
        &self,
        key: EndUserCacheKey<'_>,
        budget: &LiteLLMBudgetTable,
    ) -> Result<(), AuthCacheError> {
        self.cache
            .write_row(&key.render(), budget, self.cache.management_ttl())
            .await
    }

    // LiteLLM's resolve_default_budget: the key's budget first, the proxy default second
    pub async fn resolve_default_budget(
        &self,
        key_end_user_budget_id: Option<&str>,
    ) -> Result<Option<LiteLLMBudgetTable>, PayloadError> {
        if let Some(key_id) = key_end_user_budget_id
            && let Some(budget) = self.default_end_user_budget(Some(key_id)).await?
        {
            return Ok(Some(budget));
        }
        if self.max_end_user_budget_id.is_none() {
            return Ok(None);
        }
        self.default_end_user_budget(None).await
    }
}
