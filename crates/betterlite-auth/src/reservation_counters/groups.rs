use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::auth_tag_budgets::cache::TagBudgetCache;
use crate::auth_tag_budgets::groups::group_budgets_batch;
use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::CounterServices;
use super::error::ReservationCounterError;
use super::names::dedupe_names;

pub(crate) async fn access_group_counters(
    token: &UserAPIKeyAuth,
    services: &CounterServices<'_>,
) -> Result<Vec<BudgetCounter>, ReservationCounterError> {
    let matched = token
        .extra
        .matched_model_access_groups
        .as_deref()
        .unwrap_or_default();
    let names = dedupe_names(matched);
    let cache: &dyn TagBudgetCache = services.cache;
    let budgets = group_budgets_batch(&names, services.database, cache, services.groups).await?;
    let mut counters = Vec::new();
    for name in &names {
        let Some(budget) = budgets.get(name) else {
            continue;
        };
        let Some(max_budget) = budget.max_budget.filter(|value| *value > Decimal::ZERO) else {
            continue;
        };
        counters.push(BudgetCounter {
            counter_key: CacheKey::ModelAccessGroupSpend(name).render(),
            max_budget,
            fallback_spend: budget.spend,
            entity_type: CounterEntityType::ModelAccessGroup,
            entity_id: name.clone(),
            source_cache_key: Some(CacheKey::ModelAccessGroup(name).render()),
            spend_log_entity_id: None,
            window_duration: None,
            window_start: None,
        });
    }
    Ok(counters)
}
