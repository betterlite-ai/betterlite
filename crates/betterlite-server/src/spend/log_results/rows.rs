use super::filters::LegacyFilters;
use crate::spend::log_query::sql::SPEND_LOGS_COUNT_CAP;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendLogRowsQuery {
    pub sql: String,
    pub params: Vec<String>,
}

// the stored timestamps become UTC instants, the way Prisma hands an aware datetime to the export
const ROW_EXPR: &str = concat!(
    "to_jsonb(t) || jsonb_build_object(",
    "'startTime', t.\"startTime\" AT TIME ZONE 'UTC', ",
    "'endTime', t.\"endTime\" AT TIME ZONE 'UTC', ",
    "'completionStartTime', t.\"completionStartTime\" AT TIME ZONE 'UTC', ",
    "'created_at', t.\"created_at\" AT TIME ZONE 'UTC', ",
    "'updated_at', t.\"updated_at\" AT TIME ZONE 'UTC')",
);

// _find_spend_logs: the whole stored row, newest first, capped at SPEND_LOGS_PAGINATION_COUNT_CAP rows
#[must_use]
pub fn spend_log_rows_query(filters: &LegacyFilters) -> SpendLogRowsQuery {
    let mut params: Vec<String> = Vec::new();
    let mut conditions: Vec<String> = Vec::new();
    if let Some(window) = &filters.window {
        params.push(window.start_iso.clone());
        conditions.push(format!(
            "t.\"startTime\" >= (${}::timestamptz AT TIME ZONE 'UTC')",
            params.len()
        ));
        params.push(window.end_iso.clone());
        conditions.push(format!(
            "t.\"startTime\" <= (${}::timestamptz AT TIME ZONE 'UTC')",
            params.len()
        ));
    }
    if let Some(api_key) = &filters.api_key {
        params.push(api_key.clone());
        conditions.push(format!("t.api_key = ${}", params.len()));
    }
    if let Some(request_id) = &filters.request_id {
        params.push(request_id.clone());
        conditions.push(format!(
            "(t.request_id = ${0} OR t.litellm_call_id = ${0})",
            params.len()
        ));
    }
    if let Some(user_id) = &filters.user_id {
        params.push(user_id.clone());
        conditions.push(format!("t.\"user\" = ${}", params.len()));
    }
    let where_sql = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };
    let sql = format!(
        "SELECT ({ROW_EXPR})::text FROM \"LiteLLM_SpendLogs\" t{where_sql} ORDER BY t.\"startTime\" DESC LIMIT {SPEND_LOGS_COUNT_CAP}"
    );
    SpendLogRowsQuery { sql, params }
}
