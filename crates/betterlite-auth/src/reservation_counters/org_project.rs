use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::CounterServices;
use super::cache::fail_open;
use super::entities::{CounterEntity, entity_counter};

fn table_cap(table: Option<&LiteLLMBudgetTable>) -> Option<Decimal> {
    table
        .and_then(|budget| budget.max_budget)
        .filter(|value| *value > Decimal::ZERO)
}

// LiteLLM resolves the organization id from the token first and the team second
pub(crate) async fn org_counter(
    token: &UserAPIKeyAuth,
    team: Option<&LiteLLMTeamTable>,
    services: &CounterServices<'_>,
) -> Option<BudgetCounter> {
    let token_org = token.view.token.org_id.as_deref();
    let team_org = team.and_then(|row| row.fields.base.organization_id.as_deref());
    let org_id = token_org.or(team_org)?;
    let cache_key = format!("org_id:{org_id}:with_budget");
    let table = fail_open(
        services.cache.read_organization(&cache_key).await,
        "organization",
    );
    let table_ref = table.as_ref();
    let max_budget = table_cap(table_ref.and_then(|row| row.litellm_budget_table.as_ref()))?;
    let entity = CounterEntity {
        prefix: format!("spend:org:{org_id}"),
        entity_type: CounterEntityType::Organization,
        entity_id: org_id.to_owned(),
        source_cache_key: cache_key,
    };
    Some(entity_counter(
        &entity,
        max_budget,
        table_ref.map_or(Decimal::ZERO, |row| row.spend),
    ))
}

// LiteLLM appends the project counter after the organization counter
pub(crate) async fn project_counter(
    token: &UserAPIKeyAuth,
    services: &CounterServices<'_>,
) -> Option<BudgetCounter> {
    let project_id = token.view.token.project_id.as_deref()?;
    let cache_key = CacheKey::ProjectId(project_id).render();
    let project = fail_open(services.cache.read_project(&cache_key).await, "project");
    let project_ref = project.as_ref();
    let max_budget =
        table_cap(project_ref.and_then(|row| row.project.litellm_budget_table.as_ref()))?;
    let entity = CounterEntity {
        prefix: CacheKey::ProjectSpend(project_id).render(),
        entity_type: CounterEntityType::Project,
        entity_id: project_id.to_owned(),
        source_cache_key: cache_key,
    };
    Some(entity_counter(
        &entity,
        max_budget,
        project_ref.map_or(Decimal::ZERO, |row| row.project.spend),
    ))
}
