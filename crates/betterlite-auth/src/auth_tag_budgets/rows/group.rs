use super::{
    DriverError, FindMany, Including, IncludingQuery, Infallible, Key, KeyAction, KeyColumn, Query,
    Row, RowCodec, RowError, Table,
};

budget_table! {
    ModelAccessGroupBudgetTable, super::ModelAccessGroupBudgetRow,
    one: "SELECT to_jsonb(entry)::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry WHERE entry.\"access_group_name\" = $1",
    delete: "DELETE FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry WHERE entry.\"access_group_name\" = $1 RETURNING to_jsonb(entry)::text",
    update: "UPDATE \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry SET \"access_group_name\" = \"access_group_name\" WHERE entry.\"access_group_name\" = $1 RETURNING to_jsonb(entry)::text",
    insert: "INSERT INTO \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry DEFAULT VALUES RETURNING to_jsonb(entry)::text",
    count: "SELECT count(*) FROM \"LiteLLM_ModelAccessGroupBudgetTable\"",
    membership: "SELECT to_jsonb(entry)::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry WHERE entry.\"access_group_name\" = ANY($1::text[])",
    list: [
        "SELECT (to_jsonb(entry) || jsonb_build_object('litellm_budget_table', CASE WHEN budget.\"budget_id\" IS NULL THEN NULL ELSE to_jsonb(budget) END))::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry LEFT JOIN \"LiteLLM_BudgetTable\" AS budget ON budget.\"budget_id\" = entry.\"budget_id\" LIMIT $1",
        "SELECT to_jsonb(entry)::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry LIMIT $1"
    ],
    matching: [
        "SELECT (to_jsonb(entry) || jsonb_build_object('litellm_budget_table', CASE WHEN budget.\"budget_id\" IS NULL THEN NULL ELSE to_jsonb(budget) END))::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry LEFT JOIN \"LiteLLM_BudgetTable\" AS budget ON budget.\"budget_id\" = entry.\"budget_id\" WHERE entry.\"access_group_name\" = ANY($1::text[]) LIMIT $2",
        "SELECT to_jsonb(entry)::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry WHERE entry.\"access_group_name\" = ANY($1::text[]) LIMIT $2"
    ]
}
