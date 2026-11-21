pub(super) const STREAMING_SAME_DAY_SQL: &str = r#"
SELECT
    api_base,
    model,
    request_id,
    EXTRACT(epoch FROM ("completionStartTime" - "startTime"))::double precision AS time_to_first_token
FROM
    "LiteLLM_SpendLogs"
WHERE
    "model_group" = $1 AND "cache_hit" != 'True'
    AND "completionStartTime" IS NOT NULL
    AND "completionStartTime" != "endTime"
    AND DATE("startTime") = DATE($2::timestamptz AT TIME ZONE 'UTC')
GROUP BY
    api_base,
    model_group,
    model,
    request_id
ORDER BY
    time_to_first_token DESC
"#;

pub(super) const STREAMING_DAILY_SQL: &str = r#"
SELECT
    api_base,
    model,
    DATE_TRUNC('day', "startTime")::DATE AS day,
    AVG(EXTRACT(epoch FROM ("completionStartTime" - "startTime")))::double precision AS time_to_first_token
FROM
    "LiteLLM_SpendLogs"
WHERE
    "startTime" BETWEEN ($2::timestamptz AT TIME ZONE 'UTC') AND ($3::timestamptz AT TIME ZONE 'UTC')
    AND "model_group" = $1 AND "cache_hit" != 'True'
    AND "completionStartTime" IS NOT NULL
    AND "completionStartTime" != "endTime"
GROUP BY
    api_base,
    model_group,
    model,
    day
ORDER BY
    time_to_first_token DESC
"#;

pub(super) const LATENCY_SQL: &str = r#"
SELECT
    api_base,
    model,
    DATE_TRUNC('day', "startTime")::DATE AS day,
    AVG(EXTRACT(epoch FROM ("endTime" - "startTime")) / NULLIF("completion_tokens", 0))::double precision AS avg_latency_per_token
FROM
    "LiteLLM_SpendLogs"
WHERE
    "startTime" >= ($2::timestamptz AT TIME ZONE 'UTC') AND "startTime" <= ($3::timestamptz AT TIME ZONE 'UTC')
    AND "model_group" = $1 AND "cache_hit" != 'True'
    AND (
        CASE
            WHEN $4 != 'null' THEN "api_key" = $4
            ELSE TRUE
        END
    )
    AND (
        CASE
            WHEN $5 != 'null' THEN "end_user" = $5
            ELSE TRUE
        END
    )
GROUP BY
    api_base,
    model_group,
    model,
    day
HAVING
    SUM(completion_tokens) > 0
ORDER BY
    avg_latency_per_token DESC
"#;

pub(super) const SLOW_RESPONSES_SQL: &str = r#"
SELECT
    api_base,
    COUNT(*)::bigint AS total_count,
    SUM(CASE
        WHEN ("endTime" - "startTime") >= ($1::bigint * INTERVAL '1 SECOND') THEN 1
        ELSE 0
    END)::bigint AS slow_count
FROM
    "LiteLLM_SpendLogs"
WHERE
    "model_group" = $2
    AND "cache_hit" != 'True'
    AND "startTime" >= ($3::timestamptz AT TIME ZONE 'UTC')
    AND "startTime" <= ($4::timestamptz AT TIME ZONE 'UTC')
    AND (
        CASE
            WHEN $5 != 'null' THEN "api_key" = $5
            ELSE TRUE
        END
    )
    AND (
        CASE
            WHEN $6 != 'null' THEN "end_user" = $6
            ELSE TRUE
        END
    )
GROUP BY
    api_base
ORDER BY
    slow_count DESC
"#;

pub(super) const EXCEPTIONS_SQL: &str = r#"
WITH cte AS (
    SELECT
        CASE WHEN api_base = '' THEN litellm_model_name ELSE CONCAT(litellm_model_name, '-', api_base) END AS combined_model_api_base,
        exception_type,
        COUNT(*) AS num_rate_limit_exceptions
    FROM "LiteLLM_ErrorLogs"
    WHERE
        "startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
        AND "endTime" <= ($2::timestamptz AT TIME ZONE 'UTC')
        AND model_group = $3
    GROUP BY combined_model_api_base, exception_type
),
ranked AS (
    SELECT
        combined_model_api_base,
        COUNT(*) AS total_exceptions
    FROM cte
    GROUP BY combined_model_api_base
    ORDER BY total_exceptions DESC, combined_model_api_base
    LIMIT 200
)
SELECT
    c.combined_model_api_base,
    c.exception_type,
    c.num_rate_limit_exceptions
FROM cte c
JOIN ranked r ON r.combined_model_api_base = c.combined_model_api_base
ORDER BY r.total_exceptions DESC, c.combined_model_api_base, c.exception_type
"#;
