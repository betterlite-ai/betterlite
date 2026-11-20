use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::CounterServices;
use super::cache::fail_open;

// LiteLLM appends the organization counter and then the project counter
pub(crate) async fn organization_project_counters(
    token: &UserAPIKeyAuth,
    team: Option<&LiteLLMTeamTable>,
    services: &CounterServices<'_>,
) -> Vec<BudgetCounter> {
    let mut counters = Vec::new();
    let resolved_org = match token.view.token.org_id.as_deref() {
        Some(org_id) => Some(org_id),
        None => team.and_then(|row| row.fields.base.organization_id.as_deref()),
    };
    if let Some(org_id) = resolved_org {
        let cache_key = format!("org_id:{org_id}:with_budget");
        let table = fail_open(
            services.cache.read_organization(&cache_key).await,
            "organization",
        );
        let cap = table
            .as_ref()
            .and_then(|row| row.litellm_budget_table.as_ref())
            .and_then(|budget| budget.max_budget)
            .filter(|value| *value > Decimal::ZERO);
        if let Some(max_budget) = cap {
            counters.push(BudgetCounter {
                counter_key: format!("spend:org:{org_id}"),
                max_budget,
                fallback_spend: table.as_ref().map_or(Decimal::ZERO, |row| row.spend),
                entity_type: CounterEntityType::Organization,
                entity_id: org_id.to_owned(),
                source_cache_key: Some(cache_key),
                spend_log_entity_id: None,
                window_duration: None,
                window_start: None,
            });
        }
    }
    if let Some(project_id) = token.view.token.project_id.as_deref() {
        let cache_key = CacheKey::ProjectId(project_id).render();
        let project = fail_open(services.cache.read_project(&cache_key).await, "project");
        let cap = project
            .as_ref()
            .and_then(|row| row.project.litellm_budget_table.as_ref())
            .and_then(|budget| budget.max_budget)
            .filter(|value| *value > Decimal::ZERO);
        if let Some(max_budget) = cap {
            counters.push(BudgetCounter {
                counter_key: CacheKey::ProjectSpend(project_id).render(),
                max_budget,
                fallback_spend: project
                    .as_ref()
                    .map_or(Decimal::ZERO, |row| row.project.spend),
                entity_type: CounterEntityType::Project,
                entity_id: project_id.to_owned(),
                source_cache_key: Some(cache_key),
                spend_log_entity_id: None,
                window_duration: None,
                window_start: None,
            });
        }
    }
    counters
}
