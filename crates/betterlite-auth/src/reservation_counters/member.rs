use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::team_rows::membership::{LiteLLMTeamMembership, MembershipBudget};
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;

use crate::auth_cache::keys::CacheKey;
use crate::budget_counter::{BudgetCounter, CounterEntityType};

use super::CounterInput;
use super::CounterServices;
use super::cache::fail_open;

pub(crate) async fn member_counter(
    input: &CounterInput<'_>,
    services: &CounterServices<'_>,
) -> Option<BudgetCounter> {
    let team = input.team?;
    // LiteLLM skips the member counter when the request has no user row
    input.user?;
    let user_id = input.token.view.token.user_id.as_deref()?;
    let team_id = team.fields.base.team_id.as_str();
    let cache_key = CacheKey::TeamMembershipReservation { user_id, team_id }.render();
    let membership = fail_open(
        services.cache.read_membership(&cache_key).await,
        "team membership",
    );
    let member_budget = membership.as_ref().and_then(membership_budget);
    let budget = match member_budget.filter(|row| row.max_budget.is_some()) {
        Some(row) => row.effective_max_budget(services.now).ok().flatten(),
        None => default_member_budget(services, team, member_budget).await,
    };
    let max_budget = budget.filter(|value| *value > Decimal::ZERO)?;
    let fallback_spend = membership
        .as_ref()
        .and_then(|row| row.spend.0)
        .unwrap_or_default();
    Some(BudgetCounter {
        counter_key: format!("spend:team_member:{user_id}:{team_id}"),
        max_budget,
        fallback_spend,
        entity_type: CounterEntityType::TeamMember,
        entity_id: format!("{user_id}:{team_id}"),
        source_cache_key: Some(cache_key),
        spend_log_entity_id: None,
        window_duration: None,
        window_start: None,
    })
}

async fn default_member_budget(
    services: &CounterServices<'_>,
    team: &LiteLLMTeamTable,
    member_budget: Option<&LiteLLMBudgetTable>,
) -> Option<Decimal> {
    let default_id = match team
        .fields
        .base
        .metadata
        .as_ref()?
        .get("team_member_budget_id")?
    {
        JsonData::String(text) => text,
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => return None,
    };
    let key = format!("team_member_default_budget:{default_id}");
    let default_budget = fail_open(
        services.cache.read_default_budget(&key).await,
        "team member default budget",
    )?;
    let cap = default_budget
        .max_budget
        .filter(|value| *value > Decimal::ZERO)?;
    let increase = member_budget.map_or(Decimal::ZERO, |row| {
        row.active_temp_budget_increase(services.now)
    });
    cap.checked_add(increase)
}

fn membership_budget(membership: &LiteLLMTeamMembership) -> Option<&LiteLLMBudgetTable> {
    match membership.litellm_budget_table.as_ref()? {
        MembershipBudget::Full(row) => Some(&row.budget),
        MembershipBudget::Base(row) => Some(row),
    }
}
