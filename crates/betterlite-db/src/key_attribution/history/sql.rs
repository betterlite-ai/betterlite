pub const ROWS_PER_PROBE: u32 = 100;
pub const NO_BITMAP_SCAN_SQL: &str = "SET LOCAL enable_bitmapscan = off";

fn named_edge_row_sql(direction: &str, since: &str) -> String {
    format!(
        r#"
    SELECT "startTime", key_alias, team_id, user_id
    FROM (
        SELECT "startTime",
            NULLIF(metadata->>'user_api_key_alias', '') AS key_alias,
            COALESCE(NULLIF(team_id, ''), NULLIF(metadata->>'user_api_key_team_id', '')) AS team_id,
            COALESCE(NULLIF("user", ''), NULLIF(metadata->>'user_api_key_user_id', '')) AS user_id
        FROM (
            SELECT "startTime", metadata, team_id, "user"
            FROM "LiteLLM_SpendLogs"
            WHERE api_key = keys.digest
              AND "startTime" >= {since}
              AND "startTime" < $3::timestamp
            ORDER BY "startTime" {direction}
            LIMIT {ROWS_PER_PROBE}
        ) edge
    ) named
    WHERE COALESCE(key_alias, user_id, team_id) IS NOT NULL
    ORDER BY "startTime" {direction}
    LIMIT 1
    "#
    )
}

#[must_use]
pub fn spend_log_alias_sql() -> String {
    let first_row = named_edge_row_sql("ASC", "$2::timestamp");
    let last_row = named_edge_row_sql("DESC", "oldest_probe.stopped_at");
    let stopping_offset = ROWS_PER_PROBE.saturating_sub(1);
    format!(
        r#"
SELECT keys.digest,
    first_row.key_alias AS first_alias,
    last_row.key_alias AS last_alias,
    first_row.team_id AS first_team,
    last_row.team_id AS last_team,
    first_row.user_id AS first_owner,
    last_row.user_id AS last_owner
FROM unnest($1::text[]) AS keys(digest)
LEFT JOIN LATERAL ({first_row}) first_row ON true
LEFT JOIN LATERAL (
    SELECT COALESCE(first_row."startTime", (
        SELECT "startTime"
        FROM "LiteLLM_SpendLogs"
        WHERE api_key = keys.digest
          AND "startTime" >= $2::timestamp
          AND "startTime" < $3::timestamp
        ORDER BY "startTime" ASC
        OFFSET {stopping_offset}
        LIMIT 1
    )) AS stopped_at
) oldest_probe ON true
LEFT JOIN LATERAL ({last_row}) last_row ON true
"#
    )
}
