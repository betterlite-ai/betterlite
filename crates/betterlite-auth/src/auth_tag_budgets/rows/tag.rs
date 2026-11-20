use super::{
    DriverError, FindMany, Including, IncludingQuery, Infallible, Key, KeyAction, KeyColumn, Query,
    Row, RowCodec, RowError, Table, TagTable,
};

budget_table! {
    TagBudgetTable, TagTable,
    one: "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0')))::text FROM \"LiteLLM_TagTable\" AS entry WHERE entry.\"tag_name\" = $1",
    delete: "DELETE FROM \"LiteLLM_TagTable\" AS entry WHERE entry.\"tag_name\" = $1 RETURNING to_jsonb(entry)::text",
    update: "UPDATE \"LiteLLM_TagTable\" AS entry SET \"tag_name\" = \"tag_name\" WHERE entry.\"tag_name\" = $1 RETURNING to_jsonb(entry)::text",
    insert: "INSERT INTO \"LiteLLM_TagTable\" AS entry DEFAULT VALUES RETURNING to_jsonb(entry)::text",
    count: "SELECT count(*) FROM \"LiteLLM_TagTable\"",
    membership: "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0')))::text FROM \"LiteLLM_TagTable\" AS entry WHERE entry.\"tag_name\" = ANY($1::text[])",
    list: [
        "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0'), 'litellm_budget_table', CASE WHEN budget.\"budget_id\" IS NULL THEN NULL ELSE to_jsonb(budget) END))::text FROM \"LiteLLM_TagTable\" AS entry LEFT JOIN \"LiteLLM_BudgetTable\" AS budget ON budget.\"budget_id\" = entry.\"budget_id\" LIMIT $1",
        "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0')))::text FROM \"LiteLLM_TagTable\" AS entry LIMIT $1"
    ],
    matching: [
        "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0'), 'litellm_budget_table', CASE WHEN budget.\"budget_id\" IS NULL THEN NULL ELSE to_jsonb(budget) END))::text FROM \"LiteLLM_TagTable\" AS entry LEFT JOIN \"LiteLLM_BudgetTable\" AS budget ON budget.\"budget_id\" = entry.\"budget_id\" WHERE entry.\"tag_name\" = ANY($1::text[]) LIMIT $2",
        "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0')))::text FROM \"LiteLLM_TagTable\" AS entry WHERE entry.\"tag_name\" = ANY($1::text[]) LIMIT $2"
    ]
}
