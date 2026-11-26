use rust_decimal::Decimal;

use crate::auth_key_budgets::money::Money;
use crate::auth_tag_budgets::tags::get_tag_objects_batch;

use super::denial::{Entity, PricedRow, read_and_deny};
use super::error::TenantBudgetError;
use super::{TenantAdmission, TenantSources};

// _tag_max_budget_check for tags B1102 already extracted: the first tag over its ceiling denies and queues no alert
pub async fn max_budget_check(
    tag_names: &[String],
    sources: &TenantSources<'_>,
    admission: &TenantAdmission<'_>,
) -> Result<(), TenantBudgetError> {
    if !sources.database || tag_names.is_empty() {
        return Ok(());
    }
    let (Some(cache), Some(rows)) = (sources.cache, sources.tags) else {
        return Ok(());
    };
    let tag_objects = get_tag_objects_batch(tag_names, sources.database, cache, rows).await?;
    let mut priced = Vec::new();
    for tag_name in tag_names {
        let Some(tag) = tag_objects.get(tag_name) else {
            continue;
        };
        let Some(ceiling) = tag
            .litellm_budget_table
            .as_ref()
            .and_then(|budget| budget.max_budget)
        else {
            continue;
        };
        priced.push(PricedRow {
            entity: Entity::Tag(tag_name),
            counter_key: format!("spend:tag:{tag_name}"),
            fallback_spend: tag.spend,
            ceiling,
            denies: tag_denies,
        });
    }
    read_and_deny(priced, admission).await
}

// Python's `tag_spend <= max_budget` gate; NaN and negative infinity deny a finite spend, positive infinity allows it
pub(crate) fn tag_denies(spend: Decimal, ceiling: Money) -> bool {
    match ceiling {
        Money::Finite(limit) => spend > limit,
        Money::Nan | Money::NegativeInfinity => true,
        Money::PositiveInfinity => false,
    }
}
