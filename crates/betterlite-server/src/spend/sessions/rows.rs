use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::params::{SessionParam, SessionQuery};

// ui_view_session_spend_logs' raw projection: no messages, response, proxy_server_request, request_duration_ms or litellm_call_id, and the two JSON columns as their text, the way Prisma's query_raw serialises them
pub const EXPANSION_COLUMNS: &str = concat!(
    "request_id, call_type, api_key, spend, total_tokens, ",
    "prompt_tokens, completion_tokens, \"startTime\", \"endTime\", ",
    "\"completionStartTime\", model, model_id, model_group, ",
    "custom_llm_provider, api_base, \"user\", metadata::text AS metadata, ",
    "cache_hit, cache_key, request_tags::text AS request_tags, team_id, ",
    "organization_id, end_user, requester_ip_address, ",
    "session_id, status, mcp_namespaced_tool_name, agent_id"
);

// LiteLLM_SpendLogs as the expansion route returns it; no Debug, api_key holds a key
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSpendRow {
    pub request_id: String,
    pub call_type: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::float::serialize",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub spend: Decimal,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(rename = "startTime")]
    pub start_time: Timestamp,
    #[serde(rename = "endTime")]
    pub end_time: Timestamp,
    #[serde(rename = "completionStartTime", default)]
    pub completion_start_time: Option<Timestamp>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(rename = "user", default)]
    pub user: Option<String>,
    #[serde(default)]
    pub metadata: Option<JsonData>,
    #[serde(default)]
    pub cache_hit: Option<String>,
    #[serde(default)]
    pub cache_key: Option<String>,
    #[serde(default)]
    pub request_tags: Option<JsonData>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub end_user: Option<String>,
    #[serde(default)]
    pub requester_ip_address: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub mcp_namespaced_tool_name: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
}

#[must_use]
pub fn session_rows_query(
    where_clause: &str,
    params: Vec<SessionParam>,
    page_size: i64,
    offset: i64,
) -> SessionQuery {
    let limit_index = params.len().saturating_add(1);
    let offset_index = limit_index.saturating_add(1);
    let sql = format!(
        "SELECT {EXPANSION_COLUMNS} FROM \"LiteLLM_SpendLogs\" WHERE {where_clause} ORDER BY \"startTime\" DESC LIMIT ${limit_index} OFFSET ${offset_index}"
    );
    let mut values = params;
    values.push(SessionParam::Integer(page_size));
    values.push(SessionParam::Integer(offset));
    SessionQuery {
        sql,
        params: values,
    }
}

#[must_use]
pub fn session_row_count_query(where_clause: &str, params: Vec<SessionParam>) -> SessionQuery {
    let sql =
        format!("SELECT COUNT(*) AS total_count FROM \"LiteLLM_SpendLogs\" WHERE {where_clause}");
    SessionQuery { sql, params }
}
