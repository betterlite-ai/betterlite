// The key find_all with expires and reset_at: an expired key is skipped, and only a row with its own budget_duration is due.
pub(crate) const KEY_DUE_SQL: &str = "SELECT token, spend, max_budget, budget_duration, budget_reset_at FROM \"LiteLLM_VerificationToken\" WHERE (expires IS NULL OR expires > $1) AND budget_reset_at < $1 AND budget_duration IS NOT NULL LIMIT $2";

pub(crate) const KEY_ID_COLUMN: &str = "token";

mod tests;
