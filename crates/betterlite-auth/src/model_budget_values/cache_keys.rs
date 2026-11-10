mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetScope {
    Key,
    User,
    EndUser,
    Team,
}

#[derive(Clone, Copy, Debug)]
pub struct BudgetSlot<'slot> {
    pub scope: BudgetScope,
    pub entity_id: &'slot str,
    pub budget_model: &'slot str,
    pub budget_duration: &'slot str,
}

const fn scope_prefix(scope: BudgetScope, names: [&'static str; 4]) -> &'static str {
    let [key, user, end_user, team] = names;
    match scope {
        BudgetScope::Key => key,
        BudgetScope::User => user,
        BudgetScope::EndUser => end_user,
        BudgetScope::Team => team,
    }
}

#[must_use]
pub fn model_budget_spend_cache_key(
    scope: BudgetScope,
    entity_id: &str,
    budget_model: &str,
    budget_duration: &str,
) -> String {
    let prefix = match scope {
        BudgetScope::Key => "virtual_key_spend",
        BudgetScope::User => "user_model_spend",
        BudgetScope::EndUser => "end_user_model_spend",
        BudgetScope::Team => "team_model_spend",
    };
    format!("{prefix}:{entity_id}:{budget_model}:{budget_duration}")
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn model_budget_start_time_cache_key(slot: &BudgetSlot<'_>) -> String {
    let prefix = scope_prefix(
        slot.scope,
        [
            "virtual_key_budget_start_time",
            "user_model_budget_start_time",
            "end_user_budget_start_time",
            "team_model_budget_start_time",
        ],
    );
    [
        prefix,
        slot.entity_id,
        slot.budget_model,
        slot.budget_duration,
    ]
    .join(":")
}

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn batch_charged_once_marker_key(spend_key: &str, batch_id: &str) -> String {
    format!("{spend_key}:batch:{batch_id}")
}
