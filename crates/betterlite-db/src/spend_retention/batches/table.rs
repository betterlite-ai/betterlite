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

impl CleanupTable {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SpendLogs => "LiteLLM_SpendLogs",
            Self::ToolIndex => "LiteLLM_SpendLogToolIndex",
            Self::AutoRouterSessions => "LiteLLM_AutoRouterSession",
            Self::AutoRouterUserSessions => "LiteLLM_AutoRouterUserSession",
            Self::HealthChecks => "LiteLLM_HealthCheckTable",
            Self::DailyTagSpend => "LiteLLM_DailyTagSpend",
        }
    }

    #[must_use]
    pub const fn key_columns(self) -> &'static [&'static str] {
        match self {
            Self::SpendLogs => &["request_id", "startTime"],
            Self::ToolIndex => &["request_id", "tool_name"],
            Self::AutoRouterSessions => &["api_key", "session_id", "router_name"],
            Self::AutoRouterUserSessions => &["user_id", "api_key", "session_id", "router_name"],
            Self::HealthChecks => &["health_check_id"],
            Self::DailyTagSpend => &["id"],
        }
    }

    #[must_use]
    pub const fn time_column(self) -> &'static str {
        match self {
            Self::SpendLogs => "startTime",
            Self::ToolIndex => "start_time",
            Self::AutoRouterSessions | Self::AutoRouterUserSessions => "last_turn_at",
            Self::HealthChecks => "checked_at",
            Self::DailyTagSpend => "date",
        }
    }
}

// one subquery picks at most $2 expired keys and the outer DELETE removes them, so no row travels to Rust
#[must_use]
pub fn delete_statement(table: CleanupTable, cutoff: Cutoff) -> String {
    let keys = table
        .key_columns()
        .iter()
        .map(|column| format!("\"{column}\""))
        .collect::<Vec<String>>()
        .join(", ");
    format!(
        "DELETE FROM \"{name}\" WHERE ({keys}) IN (SELECT {keys} FROM \"{name}\" WHERE \"{time}\" < $1::{cast} LIMIT $2)",
        name = table.name(),
        time = table.time_column(),
        cast = cutoff.cast(),
    )
}

// a result equal to the cap means at least that many rows are still expired
#[must_use]
pub fn remaining_statement(table: CleanupTable, cutoff: Cutoff) -> String {
    format!(
        "SELECT count(*)::int AS remaining FROM (SELECT 1 FROM \"{name}\" WHERE \"{time}\" < $1::{cast} LIMIT $2) capped",
        name = table.name(),
        time = table.time_column(),
        cast = cutoff.cast(),
    )
}
