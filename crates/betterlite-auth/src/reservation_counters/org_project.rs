use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::CounterServices;
use super::cache::fail_open;

pub(crate) async fn org_counter(
    token: &UserAPIKeyAuth,
    team: Option<&LiteLLMTeamTable>,
    services: &CounterServices<'_>,
) -> Option<BudgetCounter> {
    let org_id = match token.view.token.org_id.as_deref() {
        Some(org_id) => org_id,
        None => team?.fields.base.organization_id.as_deref()?,
    };
    let cache_key = format!("org_id:{org_id}:with_budget");
    let table = fail_open(
        services.cache.read_organization(&cache_key).await,
        "organization",
    )?;
    let max_budget = table
        .litellm_budget_table
        .as_ref()
        .and_then(|budget| budget.max_budget)
        .filter(|value| *value > Decimal::ZERO)?;
    Some(BudgetCounter {
        counter_key: format!("spend:org:{org_id}"),
        max_budget,
        fallback_spend: table.spend,
        entity_type: CounterEntityType::Organization,
        entity_id: org_id.to_owned(),
        source_cache_key: Some(cache_key),
        spend_log_entity_id: None,
        window_duration: None,
        window_start: None,
    })
}

pub(crate) async fn project_counter(
    token: &UserAPIKeyAuth,
    services: &CounterServices<'_>,
) -> Option<BudgetCounter> {
    let project_id = token.view.token.project_id.as_deref()?;
    let cache_key = CacheKey::ProjectId(project_id).render();
    let project = fail_open(services.cache.read_project(&cache_key).await, "project")?;
    let max_budget = project
        .project
        .litellm_budget_table
        .as_ref()
        .and_then(|budget| budget.max_budget)
        .filter(|value| *value > Decimal::ZERO)?;
    Some(BudgetCounter {
        counter_key: CacheKey::ProjectSpend(project_id).render(),
        max_budget,
        fallback_spend: project.project.spend,
        entity_type: CounterEntityType::Project,
        entity_id: project_id.to_owned(),
        source_cache_key: Some(cache_key),
        spend_log_entity_id: None,
        window_duration: None,
        window_start: None,
    })
}
