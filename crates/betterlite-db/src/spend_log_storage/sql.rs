use crate::redis_spend::log_row::SpendLogRow;

use super::batch_claim::{ARETRIEVE_BATCH, SUCCESS};

// LiteLLM_SpendLogs after the pinned migrations, with the type jsonb_to_recordset reads each column as; created_at and updated_at keep their database defaults.
pub const COLUMNS: [(&str, &str); 34] = [
    ("request_id", "text"),
    ("call_type", "text"),
    ("api_key", "text"),
    ("spend", "float8"),
    ("total_tokens", "int4"),
    ("prompt_tokens", "int4"),
    ("completion_tokens", "int4"),
    ("startTime", "timestamp(3)"),
    ("endTime", "timestamp(3)"),
    ("completionStartTime", "timestamp(3)"),
    ("model", "text"),
    ("model_id", "text"),
    ("model_group", "text"),
    ("custom_llm_provider", "text"),
    ("api_base", "text"),
    ("user", "text"),
    ("metadata", "jsonb"),
    ("cache_hit", "text"),
    ("cache_key", "text"),
    ("request_tags", "jsonb"),
    ("team_id", "text"),
    ("organization_id", "text"),
    ("end_user", "text"),
    ("requester_ip_address", "text"),
    ("messages", "jsonb"),
    ("response", "jsonb"),
    ("agent_id", "text"),
    ("request_duration_ms", "int4"),
    ("status", "text"),
    ("mcp_namespaced_tool_name", "text"),
    ("proxy_server_request", "jsonb"),
    ("session_id", "text"),
    ("litellm_call_id", "text"),
    ("billing_agent_id", "text"),
];

// Prisma fills these when the payload leaves the key out, so the insert takes the column default rather than NULL.
const DEFAULTED: [(&str, &str); 11] = [
    ("api_key", "''"),
    ("spend", "0.0"),
    ("total_tokens", "0"),
    ("prompt_tokens", "0"),
    ("completion_tokens", "0"),
    ("model", "''"),
    ("metadata", "'{}'::jsonb"),
    ("request_tags", "'[]'::jsonb"),
    ("messages", "'{}'::jsonb"),
    ("response", "'{}'::jsonb"),
    ("proxy_server_request", "'{}'::jsonb"),
];

pub const SELECT_BATCH_ROW: &str = r#"SELECT "call_type", "status", "spend"::numeric AS "spend" FROM "LiteLLM_SpendLogs" WHERE "request_id" = $1"#;

// create_many(skip_duplicates=True): ON CONFLICT takes no target so a partitioned deployment keeps working
#[must_use]
pub fn insert_statement() -> String {
    let columns: Vec<String> = COLUMNS
        .iter()
        .map(|(name, _)| format!("\"{name}\""))
        .collect();
    let values: Vec<String> = COLUMNS
        .iter()
        .map(|(name, _)| {
            let column = format!("\"{name}\"");
            match default_of(name) {
                Some(default) => format!("COALESCE(src.{column}, {default})"),
                None => format!("src.{column}"),
            }
        })
        .collect();
    let column_list = columns.join(", ");
    let value_list = values.join(", ");
    let source_columns = recordset_columns();
    format!(
        r#"INSERT INTO "LiteLLM_SpendLogs" ({column_list}) SELECT {value_list} FROM jsonb_to_recordset($1::jsonb) AS src({source_columns}) ON CONFLICT DO NOTHING"#
    )
}

// _take_over_uncharged_batch_cost_row's update_many: every field the row carries is replaced on the zero row that still matches
#[must_use]
pub fn takeover_statement(data: &SpendLogRow) -> String {
    let mut assignments: Vec<String> = COLUMNS
        .iter()
        .filter(|(name, _)| *name != "request_id" && data.contains_key(*name))
        .map(|(name, _)| {
            let column = format!("\"{name}\"");
            format!("{column} = src.{column}")
        })
        .collect();
    // Prisma refreshes the @updatedAt column on every update, and the payload never carries it
    if !data.contains_key("updated_at") {
        assignments.push(r#""updated_at" = CURRENT_TIMESTAMP"#.to_owned());
    }
    let set_clause = assignments.join(", ");
    let source_columns = recordset_columns();
    format!(
        r#"UPDATE "LiteLLM_SpendLogs" AS logs SET {set_clause} FROM (SELECT * FROM jsonb_to_recordset($1::jsonb) AS src({source_columns})) AS src WHERE logs."request_id" = $2 AND logs."call_type" = '{ARETRIEVE_BATCH}' AND logs."status" = '{SUCCESS}' AND logs."spend" = 0.0"#
    )
}

fn recordset_columns() -> String {
    COLUMNS
        .iter()
        .map(|(name, kind)| {
            let column = format!("\"{name}\"");
            format!("{column} {kind}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn default_of(name: &str) -> Option<&'static str> {
    DEFAULTED
        .iter()
        .find(|(column, _)| *column == name)
        .map(|(_, default)| *default)
}
