use betterlite_models::key_digest::hashing::hash_token;
use chrono::{DateTime, Utc};
use serde::Serialize;

pub const SPEND_LOGS_COUNT_CAP: i64 = 10_000;

pub const SESSION_KEY_EXPR: &str = "COALESCE(NULLIF(session_id, ''), request_id)";
pub const SESSION_GROUP_KEY_SQL: &str = "COALESCE(NULLIF(session_id, ''), request_id), api_key";
pub const MCP_CALL_TYPES_SQL: &str = "('call_mcp_tool', 'list_mcp_tools')";
pub const AGENT_CALL_TYPE_SQL: &str = "'asend_message'";
pub const BATCH_CALL_TYPES_SQL: &str =
    "('acreate_batch', 'create_batch', 'aretrieve_batch', 'retrieve_batch')";
pub const REPRESENTATIVE_ORDER_SQL: &str = concat!(
    "(call_type = 'asend_message') DESC, ",
    "CASE WHEN call_type = 'asend_message' THEN \"endTime\" END DESC NULLS LAST, ",
    "call_type IN ('call_mcp_tool', 'list_mcp_tools'), \"startTime\" DESC, request_id",
);
pub const SPEND_LOG_LIST_COLUMNS: &str = concat!(
    "request_id, call_type, api_key, spend, total_tokens, ",
    "prompt_tokens, completion_tokens, \"startTime\", \"endTime\", ",
    "\"completionStartTime\", model, model_id, model_group, ",
    "custom_llm_provider, api_base, \"user\", metadata, ",
    "cache_hit, cache_key, request_tags, team_id, ",
    "organization_id, end_user, requester_ip_address, ",
    "session_id, status, mcp_namespaced_tool_name, agent_id, ",
    "litellm_call_id, ",
    "COALESCE(request_duration_ms, ",
    "(EXTRACT(EPOCH FROM (\"endTime\" - \"startTime\")) * 1000)::INTEGER) AS request_duration_ms",
);

pub const HEALTH_CHECK_SERVICE_ACCOUNT: &str = "litellm-internal-health-check";

const LLM_SPAN_SQL: &str = concat!(
    "(call_type NOT IN ('call_mcp_tool', 'list_mcp_tools') ",
    "AND call_type != 'asend_message' ",
    "AND call_type NOT IN ('acreate_batch', 'create_batch', 'aretrieve_batch', 'retrieve_batch'))",
);
const AGENT_SPAN_SQL: &str = "call_type = 'asend_message'";
const MCP_SPAN_SQL: &str = "call_type IN ('call_mcp_tool', 'list_mcp_tools')";
const BATCH_SPAN_SQL: &str =
    "call_type IN ('acreate_batch', 'create_batch', 'aretrieve_batch', 'retrieve_batch')";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusFilterCondition {
    #[serde(rename = "OR", skip_serializing_if = "Option::is_none")]
    pub or: Option<[StatusField; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusEquals>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusField {
    pub status: Option<StatusEquals>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusEquals {
    pub equals: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendLogSearchCondition {
    pub sql: String,
    pub params: (String, DateTime<Utc>, DateTime<Utc>),
}

#[must_use]
pub fn status_filter_condition(status_filter: Option<&str>) -> Option<StatusFilterCondition> {
    match status_filter {
        None => None,
        Some("success") => Some(StatusFilterCondition {
            or: Some([
                StatusField {
                    status: Some(StatusEquals {
                        equals: "success".to_owned(),
                    }),
                },
                StatusField { status: None },
            ]),
            status: None,
        }),
        Some(status) => Some(StatusFilterCondition {
            or: None,
            status: Some(StatusEquals {
                equals: status.to_owned(),
            }),
        }),
    }
}

#[must_use]
pub fn span_type_sql_condition(span_type: Option<&str>) -> Option<&'static str> {
    match span_type {
        Some("llm") => Some(LLM_SPAN_SQL),
        Some("agent") => Some(AGENT_SPAN_SQL),
        Some("mcp") => Some(MCP_SPAN_SQL),
        Some("batch") => Some(BATCH_SPAN_SQL),
        Some(_) | None => None,
    }
}

#[must_use]
pub fn spend_log_search_condition(
    search: &str,
    start_inclusive: DateTime<Utc>,
    end_inclusive: DateTime<Utc>,
    next_param_index: usize,
) -> SpendLogSearchCondition {
    let raw = next_param_index;
    let window_start = next_param_index.saturating_add(1);
    let window_end = next_param_index.saturating_add(2);
    let sql = format!(
        "(request_id = ${raw} OR (\"startTime\" >= (${window_start}::timestamptz AT TIME ZONE 'UTC') AND \"startTime\" <= (${window_end}::timestamptz AT TIME ZONE 'UTC') AND (litellm_call_id = ${raw} OR api_key = ${raw} OR team_id = ${raw} OR \"user\" = ${raw} OR end_user = ${raw} OR session_id = ${raw} OR model_id = ${raw})))"
    );
    SpendLogSearchCondition {
        sql,
        params: (search.to_owned(), start_inclusive, end_inclusive),
    }
}

#[must_use]
pub fn internal_health_check_api_keys() -> [String; 2] {
    let name = HEALTH_CHECK_SERVICE_ACCOUNT;
    let hashed = hash_token(name);
    [name.to_owned(), hashed]
}
