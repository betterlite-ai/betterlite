pub mod cache;
pub mod error;

mod end_user;
mod entities;
mod groups;
mod member;
mod names;
mod org_project;
mod tags;
mod windows;

mod tests;

use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, Utc};

use crate::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow};
use crate::budget_counter::BudgetCounter;
use crate::budget_primitives::window_start::DurationParser;
use crate::resolved_key::auth::UserAPIKeyAuth;

use self::cache::ReservationCounterCache;
use self::end_user::end_user_counter;
use self::entities::{key_counters, team_counters, user_counter};
use self::error::ReservationCounterError;
use self::groups::access_group_counters;
use self::member::member_counter;
use self::org_project::{org_counter, project_counter};
use self::tags::tag_counters;

// the authenticated request facts a counter list is built from
pub struct CounterInput<'input> {
    pub token: &'input UserAPIKeyAuth,
    pub team: Option<&'input LiteLLMTeamTable>,
    pub user: Option<&'input LiteLLMUserTable>,
    pub end_user_id: Option<&'input str>,
    pub end_user: Option<&'input LiteLLMEndUserTable>,
    pub tag_names: &'input [String],
    pub apply_user_budget_to_team_keys: bool,
}

// the services every counter reads its entity row through
pub struct CounterServices<'services> {
    pub database: bool,
    pub cache: &'services dyn ReservationCounterCache,
    pub tags: &'services dyn BudgetRows<TagTable>,
    pub groups: &'services dyn BudgetRows<ModelAccessGroupBudgetRow>,
    pub durations: &'services dyn DurationParser,
    pub now: DateTime<Utc>,
}

pub async fn budget_counters(
    input: &CounterInput<'_>,
    services: &CounterServices<'_>,
) -> Result<Vec<BudgetCounter>, ReservationCounterError> {
    let mut counters = key_counters(input.token, services.durations, services.now);
    counters.extend(team_counters(input.team, services.durations, services.now));
    counters.extend(user_counter(input));
    counters.extend(end_user_counter(
        input.token,
        input.end_user_id,
        input.end_user,
    ));
    counters.extend(tag_counters(input.tag_names, services).await?);
    counters.extend(access_group_counters(input.token, services).await?);
    counters.extend(member_counter(input, services).await);
    counters.extend(org_counter(input.token, input.team, services).await);
    counters.extend(project_counter(input.token, services).await);
    Ok(counters)
}
