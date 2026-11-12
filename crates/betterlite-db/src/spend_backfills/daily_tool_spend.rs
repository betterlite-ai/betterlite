pub const SET_TIME_ZONE_SQL: &str = "SET TIME ZONE 'UTC'";

pub const INSERT_SQL: &str = r#"INSERT INTO "LiteLLM_DailyToolSpend" (date, tool_name, spend, total_tokens, request_count, created_at, updated_at)
SELECT to_char(ti.start_time, 'YYYY-MM-DD') AS date,
       ti.tool_name,
       COALESCE(SUM(sl.spend), 0) AS spend,
       COALESCE(SUM(sl.total_tokens), 0) AS total_tokens,
       COUNT(*) AS request_count,
       now() AS created_at,
       now() AS updated_at
FROM "LiteLLM_SpendLogToolIndex" ti
JOIN "LiteLLM_SpendLogs" sl ON sl.request_id = ti.request_id
WHERE ti.start_time < $1::timestamptz
GROUP BY 1, 2
ON CONFLICT (date, tool_name) DO NOTHING"#;

pub(crate) const SESSION_TIME_ZONE_FAILURE: &str =
    "the daily tool spend backfill session time zone update";

pub(crate) const INSERT_FAILURE: &str = "the daily tool spend backfill insert";

pub(crate) const fn statements(
    cutover: chrono::DateTime<chrono::Utc>,
) -> [(super::BackfillStatement, &'static str); 2] {
    [
        (
            super::BackfillStatement::SessionTimeZoneUtc,
            SESSION_TIME_ZONE_FAILURE,
        ),
        (
            super::BackfillStatement::DailyToolSpend { cutover },
            INSERT_FAILURE,
        ),
    ]
}
