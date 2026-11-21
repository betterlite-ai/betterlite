use super::cutoff::Cutoff;

// the six tables retention prunes; the enum keeps every identifier that reaches SQL a constant
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupTable {
    SpendLogs,
    ToolIndex,
    AutoRouterSessions,
    AutoRouterUserSessions,
    HealthChecks,
    DailyTagSpend,
}

// one row per table, so a name, a key or a time column is written once next to the table it belongs to
#[derive(Debug, Clone, Copy)]
pub struct TableSpec {
    pub name: &'static str,
    pub key_columns: &'static [&'static str],
    pub time_column: &'static str,
}

impl CleanupTable {
    #[must_use]
    pub const fn spec(self) -> TableSpec {
        match self {
            Self::SpendLogs => TableSpec {
                name: "LiteLLM_SpendLogs",
                key_columns: &["request_id", "startTime"],
                time_column: "startTime",
            },
            Self::ToolIndex => TableSpec {
                name: "LiteLLM_SpendLogToolIndex",
                key_columns: &["request_id", "tool_name"],
                time_column: "start_time",
            },
            Self::AutoRouterSessions => TableSpec {
                name: "LiteLLM_AutoRouterSession",
                key_columns: &["api_key", "session_id", "router_name"],
                time_column: "last_turn_at",
            },
            Self::AutoRouterUserSessions => TableSpec {
                name: "LiteLLM_AutoRouterUserSession",
                key_columns: &["user_id", "api_key", "session_id", "router_name"],
                time_column: "last_turn_at",
            },
            Self::HealthChecks => TableSpec {
                name: "LiteLLM_HealthCheckTable",
                key_columns: &["health_check_id"],
                time_column: "checked_at",
            },
            Self::DailyTagSpend => TableSpec {
                name: "LiteLLM_DailyTagSpend",
                key_columns: &["id"],
                time_column: "date",
            },
        }
    }
}

// one subquery picks at most $2 expired keys and the outer DELETE removes them, so no row travels to Rust
#[must_use]
pub fn delete_statement(table: CleanupTable, cutoff: Cutoff) -> String {
    let keys = table
        .spec()
        .key_columns
        .iter()
        .map(|column| format!("\"{column}\""))
        .collect::<Vec<String>>()
        .join(", ");
    format!(
        "DELETE FROM \"{name}\" WHERE ({keys}) IN (SELECT {keys} FROM \"{name}\" WHERE \"{time}\" < $1::{cast} LIMIT $2)",
        name = table.spec().name,
        time = table.spec().time_column,
        cast = cutoff.cast(),
    )
}

// a result equal to the cap means at least that many rows are still expired
#[must_use]
pub fn remaining_statement(table: CleanupTable, cutoff: Cutoff) -> String {
    format!(
        "SELECT count(*)::int AS remaining FROM (SELECT 1 FROM \"{name}\" WHERE \"{time}\" < $1::{cast} LIMIT $2) capped",
        name = table.spec().name,
        time = table.spec().time_column,
        cast = cutoff.cast(),
    )
}
