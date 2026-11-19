use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::TagBudgetError;
use super::cache::{CachedValue, TagBudgetCache};
use super::merged;
use super::rows::{BudgetRows, ModelAccessGroupBudgetRow};
use crate::auth_cache::keys::CacheKey;

// LiteLLM's ModelAccessGroupBudget: the flattened row both enforcement paths read from the cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAccessGroupBudget {
    pub access_group_name: String,
    #[serde(with = "rust_decimal::serde::float")]
    pub spend: Decimal,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub max_budget: Option<Decimal>,
}

// LiteLLM's _model_access_group_budget: spend or zero, and the ceiling only when the budget row exists.
#[must_use]
pub fn model_access_group_budget(row: &ModelAccessGroupBudgetRow) -> ModelAccessGroupBudget {
    ModelAccessGroupBudget {
        access_group_name: row.access_group_name.clone(),
        spend: row.spend.unwrap_or_default(),
        max_budget: row
            .litellm_budget_table
            .as_ref()
            .and_then(|budget| budget.max_budget),
    }
}

// LiteLLM's get_model_access_group_budgets_batch: probe every name, then one read for the misses.
pub async fn group_budgets_batch(
    access_group_names: &[String],
    database: bool,
    cache: &dyn TagBudgetCache,
    rows: &dyn BudgetRows<ModelAccessGroupBudgetRow>,
) -> Result<BTreeMap<String, ModelAccessGroupBudget>, TagBudgetError> {
    if !database || access_group_names.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut probed = Vec::with_capacity(access_group_names.len());
    let mut misses = Vec::new();
    for name in access_group_names {
        let cached = cache
            .read_group(&CacheKey::ModelAccessGroup(name).render())
            .await
            .map_err(|error| TagBudgetError::GroupPayload(Box::new(error)))?;
        if cached.is_none() {
            misses.push(name.clone());
        }
        probed.push((name.clone(), cached));
    }
    let fetched = fetch_missing_groups(&misses, cache, rows).await?;
    Ok(merged(probed, fetched))
}

// LiteLLM's _fetch_uncached_model_access_group_budgets: no registry gate, since the caller intersected already.
async fn fetch_missing_groups(
    uncached_groups: &[String],
    cache: &dyn TagBudgetCache,
    rows: &dyn BudgetRows<ModelAccessGroupBudgetRow>,
) -> Result<Vec<(String, ModelAccessGroupBudget)>, TagBudgetError> {
    if uncached_groups.is_empty() {
        return Ok(Vec::new());
    }
    let db_rows = match rows.by_names(uncached_groups.to_vec()).await {
        Ok(found) => found,
        Err(failure) => {
            tracing::debug!(
                error = %failure,
                "Error batch fetching model access group budgets from database"
            );
            return Ok(Vec::new());
        }
    };
    let ttl = cache.management_ttl();
    let mut fetched = Vec::with_capacity(db_rows.len());
    for row in db_rows {
        let budget = model_access_group_budget(&row);
        let key = CacheKey::ModelAccessGroup(&budget.access_group_name).render();
        if let Err(error) = cache.write(&key, CachedValue::Group(&budget), ttl).await {
            tracing::debug!(
                error = %error,
                "Error batch fetching model access group budgets from database"
            );
            return Ok(Vec::new());
        }
        fetched.push((budget.access_group_name.clone(), budget));
    }
    Ok(fetched)
}
