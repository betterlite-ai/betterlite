use async_trait::async_trait;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::team_rows::membership::{LiteLLMTeamMembership, MembershipBudget};
use rust_decimal::Decimal;

use crate::auth_end_users::cache::EndUserCache;
use crate::auth_end_users::defaults::{DefaultBudgetLookup, MemberBudgetError};
use crate::auth_end_users::rows::BudgetRows;
use crate::auth_key_budgets::spend::{SpendReadError, SpendReader};
use crate::current_spend::{SpendQuery, WindowDescriptors};
use crate::membership_load::MembershipLookup;
use crate::membership_load::error::MembershipLoadError;

use super::error::TeamBudgetError;

// what the caller already knows about the membership; the row is boxed so the enum stays small, and a loaded None answers without a query
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Membership {
    Unloaded,
    Loaded(Option<Box<LiteLLMTeamMembership>>),
}

// B2318's get_team_membership, which the server wires to the shared load
#[async_trait]
pub trait MembershipSource: Send + Sync {
    async fn team_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError>;
}

#[async_trait]
impl MembershipSource for MembershipLookup {
    async fn team_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        self.get_team_membership(Some(user_id), Some(team_id)).await
    }
}

// B739's get_team_member_default_budget, the team-level default a member without an override falls back to
#[async_trait]
pub trait DefaultBudgetSource: Send + Sync {
    async fn team_member_default(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError>;
}

#[async_trait]
impl<Rows, Cache> DefaultBudgetSource for DefaultBudgetLookup<'_, Rows, Cache>
where
    Rows: BudgetRows + ?Sized,
    Cache: EndUserCache + Send + Sync + ?Sized,
{
    async fn team_member_default(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError> {
        self.team_member_default_budget(budget_id).await
    }
}

pub(crate) async fn resolve_membership(
    source: &dyn MembershipSource,
    membership: &Membership,
    user_id: &str,
    team_id: &str,
) -> Result<Option<LiteLLMTeamMembership>, TeamBudgetError> {
    match membership {
        Membership::Loaded(loaded) => Ok(loaded.as_deref().cloned()),
        Membership::Unloaded => Ok(source.team_membership(user_id, team_id).await?),
    }
}

// the member and team hard reads name only the counter, its fallback and the maximum
pub(crate) async fn read_counter(
    reader: &dyn SpendReader,
    counter_key: &str,
    fallback_spend: Decimal,
    max_budget: Option<Decimal>,
) -> Result<Decimal, SpendReadError> {
    let query = SpendQuery {
        counter_key,
        fallback_spend,
        max_budget,
        window: WindowDescriptors {
            entity_type: None,
            entity_id: None,
            duration: None,
            start: None,
        },
        fallback_authoritative: false,
    };
    reader.read_spend(&query).await
}

// litellm_budget_table holds the base or the server row; both carry the same budget fields
#[must_use]
pub(crate) const fn member_budget_row(
    membership: &LiteLLMTeamMembership,
) -> Option<&LiteLLMBudgetTable> {
    match &membership.litellm_budget_table {
        Some(MembershipBudget::Full(row)) => Some(&row.budget),
        Some(MembershipBudget::Base(row)) => Some(row),
        None => None,
    }
}
