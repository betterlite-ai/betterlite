use super::cutoff::Cutoff;

// the six tables retention prunes; only these constants reach SQL, so no identifier comes from a request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupTable {
    pub name: &'static str,
    pub key_columns: &'static [&'static str],
    pub time_column: &'static str,
}

impl CleanupTable {
    pub const SPEND_LOGS: Self = Self {
        name: "LiteLLM_SpendLogs",
        key_columns: &["request_id", "startTime"],
        time_column: "startTime",
    };
    pub const TOOL_INDEX: Self = Self {
        name: "LiteLLM_SpendLogToolIndex",
        key_columns: &["request_id", "tool_name"],
        time_column: "start_time",
    };
    pub const AUTO_ROUTER_SESSIONS: Self = Self {
        name: "LiteLLM_AutoRouterSession",
        key_columns: &["api_key", "session_id", "router_name"],
        time_column: "last_turn_at",
    };
    pub const AUTO_ROUTER_USER_SESSIONS: Self = Self {
        name: "LiteLLM_AutoRouterUserSession",
        key_columns: &["user_id", "api_key", "session_id", "router_name"],
        time_column: "last_turn_at",
    };
    pub const HEALTH_CHECKS: Self = Self {
        name: "LiteLLM_HealthCheckTable",
        key_columns: &["health_check_id"],
        time_column: "checked_at",
    };
    pub const DAILY_TAG_SPEND: Self = Self {
        name: "LiteLLM_DailyTagSpend",
        key_columns: &["id"],
        time_column: "date",
    };
}

// one subquery picks at most $2 expired keys and the outer DELETE removes them, so no row travels to Rust
#[must_use]
pub fn delete_statement(table: CleanupTable, cutoff: Cutoff) -> String {
    let keys = table
        .key_columns
        .iter()
        .map(|column| format!("\"{column}\""))
        .collect::<Vec<String>>()
        .join(", ");
    format!(
        "DELETE FROM \"{name}\" WHERE ({keys}) IN (SELECT {keys} FROM \"{name}\" WHERE \"{time}\" < $1::{cast} LIMIT $2)",
        name = table.name,
        time = table.time_column,
        cast = cutoff.cast(),
    )
}

// a result equal to the cap means at least that many rows are still expired
#[must_use]
pub fn remaining_statement(table: CleanupTable, cutoff: Cutoff) -> String {
    format!(
        "SELECT count(*)::int AS remaining FROM (SELECT 1 FROM \"{name}\" WHERE \"{time}\" < $1::{cast} LIMIT $2) capped",
        name = table.name,
        time = table.time_column,
        cast = cutoff.cast(),
    )
}
