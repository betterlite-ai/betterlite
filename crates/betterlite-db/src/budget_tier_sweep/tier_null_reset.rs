pub(crate) const DUE_TIERS_SQL: &str = "SELECT row_to_json(b)::text FROM \"LiteLLM_BudgetTable\" b WHERE b.\"budget_duration\" IS NOT NULL AND (b.\"budget_reset_at\" IS NULL OR b.\"budget_reset_at\" < $1) LIMIT $2";

mod tests;
