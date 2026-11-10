mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetScope {
    Key,
    User,
    EndUser,
    Team,
}

impl BudgetScope {
    const fn spend_prefix(self) -> &'static str {
        match self {
            Self::Key => "virtual_key_spend",
            Self::User => "user_model_spend",
            Self::EndUser => "end_user_model_spend",
            Self::Team => "team_model_spend",
        }
    }
}

#[must_use]
pub fn model_budget_spend_cache_key(
    scope: BudgetScope,
    entity_id: &str,
    budget_model: &str,
    budget_duration: &str,
) -> String {
    format!(
        "{}:{entity_id}:{budget_model}:{budget_duration}",
        scope.spend_prefix()
    )
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn model_budget_start_time_cache_key(
    scope: BudgetScope,
    entity_id: &str,
    budget_model: &str,
    budget_duration: &str,
) -> String {
    let prefix = match scope {
        BudgetScope::Key => "virtual_key_budget_start_time",
        BudgetScope::User => "user_model_budget_start_time",
        BudgetScope::EndUser => "end_user_budget_start_time",
        BudgetScope::Team => "team_model_budget_start_time",
    };
    [prefix, entity_id, budget_model, budget_duration].join(":")
}

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn batch_charged_once_marker_key(spend_key: &str, batch_id: &str) -> String {
    format!("{spend_key}:batch:{batch_id}")
}
