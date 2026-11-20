use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::auth_tag_budgets::cache::TagBudgetCache;
use crate::auth_tag_budgets::tags::get_tag_objects_batch;
use crate::budget_counter::{BudgetCounter, CounterEntityType};

use super::CounterServices;
use super::error::ReservationCounterError;
use super::names::dedupe_names;

pub(crate) async fn tag_counters(
    tag_names: &[String],
    services: &CounterServices<'_>,
) -> Result<Vec<BudgetCounter>, ReservationCounterError> {
    let names = dedupe_names(tag_names);
    let cache: &dyn TagBudgetCache = services.cache;
    let objects = get_tag_objects_batch(&names, services.database, cache, services.tags).await?;
    let mut counters = Vec::new();
    for name in &names {
        let Some(object) = objects.get(name) else {
            continue;
        };
        let cap = object
            .litellm_budget_table
            .as_ref()
            .and_then(|budget| budget.max_budget);
        let Some(max_budget) = cap.filter(|value| *value > Decimal::ZERO) else {
            continue;
        };
        counters.push(BudgetCounter {
            counter_key: format!("spend:tag:{name}"),
            max_budget,
            fallback_spend: object.spend,
            entity_type: CounterEntityType::Tag,
            entity_id: name.clone(),
            source_cache_key: Some(CacheKey::Tag(name).render()),
            spend_log_entity_id: None,
            window_duration: None,
            window_start: None,
        });
    }
    Ok(counters)
}
