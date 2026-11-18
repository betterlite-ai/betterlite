pub(crate) const USER_ID_COLUMN: &str = "user_id";

// The user find_all with reset_at treats a NULL budget_reset_at as due whenever budget_duration is set, so that row can reset.
pub(crate) const USER_DUE_SQL: &str = "SELECT user_id, spend, max_budget, budget_duration, budget_reset_at FROM \"LiteLLM_UserTable\" WHERE budget_duration IS NOT NULL AND (budget_reset_at IS NULL OR budget_reset_at < $1) LIMIT $2";

mod tests;
