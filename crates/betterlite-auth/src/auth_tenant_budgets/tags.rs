use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::auth_key_budgets::money::Money;
use crate::auth_tag_budgets::groups::{ModelAccessGroupBudget, group_budgets_batch};
use crate::auth_tag_budgets::tags::get_tag_objects_batch;
use crate::current_spend::SpendQuery;

use super::denial::{Entity, denial};
use super::error::TenantBudgetError;
use super::project::NO_WINDOW;
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
        let counter_key = format!("spend:tag:{tag_name}");
        let query = SpendQuery {
            counter_key: &counter_key,
            fallback_spend: tag.spend,
            max_budget: Some(ceiling),
            window: NO_WINDOW,
            fallback_authoritative: true,
        };
        let spend = admission.reader.read_spend(&query).await?;
        if !tag_denies(spend, Money::Finite(ceiling)) {
            continue;
        }
        return Err(TenantBudgetError::Denied(denial(
            Entity::Tag(tag_name),
            spend,
            ceiling,
        )));
    }
    Ok(())
}

// Python's `tag_spend <= max_budget` gate; NaN and negative infinity deny a finite spend, positive infinity allows it
pub(crate) fn tag_denies(spend: Decimal, ceiling: Money) -> bool {
    match ceiling {
        Money::Finite(limit) => spend > limit,
        Money::Nan | Money::NegativeInfinity => true,
        Money::PositiveInfinity => false,
    }
}

// _model_access_group_max_budget_check: only matched groups are priced, a nonpositive ceiling skips, and equality denies
pub async fn model_access_group_max_budget_check(
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
    for group_name in group_names {
        check_group(group_name, budgets.get(group_name), admission).await?;
    }
    Ok(())
}

// one matched group: an absent row or a ceiling that doesn't apply skips, and the authoritative counter decides
async fn check_group(
    group_name: &str,
    budget: Option<&ModelAccessGroupBudget>,
    admission: &TenantAdmission<'_>,
) -> Result<(), TenantBudgetError> {
    let Some(budget) = budget else {
        return Ok(());
    };
    let Some(row_ceiling) = budget.max_budget else {
        return Ok(());
    };
    let ceiling = Money::Finite(row_ceiling);
    if !group_ceiling_applies(ceiling) {
        return Ok(());
    }
    let Some(max_budget) = ceiling.finite_value() else {
        return Ok(());
    };
    let counter_key = CacheKey::ModelAccessGroupSpend(group_name).render();
    let query = SpendQuery {
        counter_key: &counter_key,
        fallback_spend: budget.spend,
        max_budget: Some(max_budget),
        window: NO_WINDOW,
        fallback_authoritative: true,
    };
    let spend = admission.reader.read_spend(&query).await?;
    if !group_denies(spend, ceiling) {
        return Ok(());
    }
    Err(TenantBudgetError::Denied(denial(
        Entity::ModelAccessGroup(group_name),
        spend,
        max_budget,
    )))
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
