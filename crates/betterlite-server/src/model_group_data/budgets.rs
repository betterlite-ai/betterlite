use std::collections::BTreeMap;

use betterlite_auth::auth_cache::keys::CacheKey;
use rust_decimal::Decimal;

use super::errors::GroupDataError;
use super::ports::{BudgetInclude, GroupDatabase, ModelAccessGroupCache};
use super::types::{AccessGroupBudget, AccessGroupBudgetResponse, AccessGroupInfo, GroupBudgetRow};

// _model_access_group_budget_rows: every link row in one read, keyed by group.
pub async fn model_access_group_budget_rows<Database>(
    database: &Database,
) -> Result<BTreeMap<String, GroupBudgetRow>, GroupDataError>
where
    Database: GroupDatabase + ?Sized,
{
    let rows = database
        .budget_find_many(BudgetInclude {
            litellm_budget_table: true,
        })
        .await?;
    let mut mapped = BTreeMap::new();
    for row in rows {
        mapped.insert(row.access_group_name.clone(), row);
    }
    Ok(mapped)
}

// _budget_or_none: a budget_id without a joined row reads as no budget, not a partial one.
#[must_use]
pub fn budget_or_none(row: Option<&GroupBudgetRow>) -> Option<AccessGroupBudget> {
    let budget = row.and_then(|linked| linked.litellm_budget_table.as_ref())?;
    Some(AccessGroupBudget {
        budget_id: budget.budget_id.clone(),
        max_budget: budget.max_budget,
        soft_budget: budget.soft_budget,
        budget_duration: budget.budget_duration.clone(),
        budget_reset_at: budget.budget_reset_at,
    })
}

// _with_budget: a group without its own row has spent nothing.
#[must_use]
pub fn with_budget(info: &AccessGroupInfo, row: Option<&GroupBudgetRow>) -> AccessGroupInfo {
    let spend = row.map_or(Decimal::ZERO, |linked| linked.spend.unwrap_or_default());
    AccessGroupInfo {
        access_group: info.access_group.clone(),
        model_names: info.model_names.clone(),
        deployment_count: info.deployment_count,
        spend: Some(spend),
        budget: budget_or_none(row),
    }
}

// _budget_response: the same spend rule for the single-group read.
#[must_use]
pub fn budget_response(
    access_group: &str,
    row: Option<&GroupBudgetRow>,
) -> AccessGroupBudgetResponse {
    let spend = row.map_or(Decimal::ZERO, |linked| linked.spend.unwrap_or_default());
    AccessGroupBudgetResponse {
        access_group: access_group.to_owned(),
        spend,
        budget: budget_or_none(row),
    }
}

// _evict_model_access_group_cache_keys: both keys every budget write invalidates.
pub async fn evict_group_cache_keys<Cache>(access_group: &str, cache: &Cache)
where
    Cache: ModelAccessGroupCache + ?Sized,
{
    let cache_keys = [
        CacheKey::ModelAccessGroup(access_group).render(),
        CacheKey::ModelAccessGroupRegistry.render(),
    ];
    cache.evict_group_cache_keys(&cache_keys).await;
}

// _delete_model_access_group_budget_row: the delete lands first, then the eviction runs even when no row was deleted, since the cached registry may still list the group.
pub async fn delete_group_budget_row<Database, Cache>(
    access_group: &str,
    database: &Database,
    cache: &Cache,
) -> Result<bool, GroupDataError>
where
    Database: GroupDatabase + ?Sized,
    Cache: ModelAccessGroupCache + ?Sized,
{
    let row = database.budget_delete(access_group).await?;
    evict_group_cache_keys(access_group, cache).await;
    Ok(row.is_some())
}
