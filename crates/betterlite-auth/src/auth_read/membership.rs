use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::team_rows::membership::{LiteLLMTeamMembership, MembershipBudget};

#[must_use]
pub fn member_allowed_models(membership: &LiteLLMTeamMembership) -> &[String] {
    let Some(budget) = membership.litellm_budget_table.as_ref() else {
        return &[];
    };
    let allowed = match budget {
        MembershipBudget::Full(row) => row.budget.allowed_models.as_deref(),
        MembershipBudget::Base(row) => row.allowed_models.as_deref(),
    };
    allowed.unwrap_or_default()
}

pub enum SharedMembershipLoad {
    Membership(Box<LiteLLMTeamMembership>),
    // boxed to stay within 3x of Membership, the limit variant_size_differences enforces
    Foreign(Box<CacheValue>),
}

#[must_use]
pub fn membership_from_shared_load(load: SharedMembershipLoad) -> Option<LiteLLMTeamMembership> {
    match load {
        SharedMembershipLoad::Membership(membership) => Some(*membership),
        SharedMembershipLoad::Foreign(_) => None,
    }
}
