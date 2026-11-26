use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::auth_key_budgets::money::Money;
use crate::auth_tag_budgets::groups::group_budgets_batch;

use super::denial::{Entity, PricedRow, read_and_deny};
use super::error::TenantBudgetError;
use super::{TenantAdmission, TenantSources};

// only matched groups are priced, a nonpositive ceiling skips, and equality denies
pub async fn access_group_max_budget_check(
    group_names: &[String],
    sources: &TenantSources<'_>,
    admission: &TenantAdmission<'_>,
) -> Result<(), TenantBudgetError> {
    if !sources.database || group_names.is_empty() {
        return Ok(());
    }
    let (Some(cache), Some(rows)) = (sources.cache, sources.groups) else {
        return Ok(());
    };
    let budgets = group_budgets_batch(group_names, sources.database, cache, rows).await?;
    let priced = group_names
        .iter()
        .filter_map(|group_name| {
            let budget = budgets.get(group_name)?;
            let limit = budget.max_budget?;
            let ceiling = Money::Finite(limit);
            if !group_ceiling_applies(ceiling) {
                return None;
            }
            Some(PricedRow {
                entity: Entity::ModelAccessGroup(group_name),
                counter_key: CacheKey::ModelAccessGroupSpend(group_name).render(),
                fallback_spend: budget.spend,
                ceiling: limit,
                denies: group_denies,
            })
        })
        .collect();
    read_and_deny(priced, admission).await
}

// Python's `budget.max_budget is None or budget.max_budget <= 0`; a NaN ceiling compares false and so applies
pub(crate) fn group_ceiling_applies(ceiling: Money) -> bool {
    match ceiling {
        Money::Finite(limit) => limit > Decimal::ZERO,
        Money::Nan | Money::PositiveInfinity => true,
        Money::NegativeInfinity => false,
    }
}

// Python's `group_spend < budget.max_budget` gate, exclusive unlike the tag check
pub(crate) fn group_denies(spend: Decimal, ceiling: Money) -> bool {
    match ceiling {
        Money::Finite(limit) => spend >= limit,
        Money::Nan | Money::NegativeInfinity => true,
        Money::PositiveInfinity => false,
    }
}
