pub(crate) const GROUPS_SQL: &str = r#"
    SELECT
        CASE WHEN sl."call_type" = '' THEN 'Unknown' ELSE sl."call_type" END AS call_type,
        (COUNT(*)
            - SUM(CASE WHEN COALESCE(sl."cache_hit", '') = 'True' THEN 1 ELSE 0 END)
            - SUM(CASE WHEN sl."status" = 'failure' THEN 1 ELSE 0 END))::int AS api_requests,
        SUM(CASE WHEN COALESCE(sl."cache_hit", '') = 'True' THEN 1 ELSE 0 END)::int AS cache_hits,
        SUM(CASE WHEN sl."status" = 'failure' THEN 1 ELSE 0 END)::int AS failed_requests,
        SUM(CASE WHEN COALESCE(sl."cache_hit", '') = 'True' THEN sl."completion_tokens" ELSE 0 END)::int
            AS cached_completion_tokens,
        SUM(CASE WHEN COALESCE(sl."cache_hit", '') != 'True' THEN sl."completion_tokens" ELSE 0 END)::int
            AS generated_completion_tokens
    FROM "LiteLLM_SpendLogs" sl
    LEFT JOIN "LiteLLM_VerificationToken" vt ON sl."api_key" = vt."token"
    WHERE
        sl."startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
        AND sl."startTime" <  (($2::timestamptz + INTERVAL '1 day') AT TIME ZONE 'UTC')
        AND ($3::jsonb = '[]'::jsonb
            OR COALESCE(vt."key_alias", 'Unnamed Key') IN (SELECT jsonb_array_elements_text($3::jsonb)))
        AND ($4::jsonb = '[]'::jsonb
            OR sl."model" IN (SELECT jsonb_array_elements_text($4::jsonb)))
        AND sl."call_type" NOT IN (SELECT jsonb_array_elements_text($5::jsonb))
    GROUP BY 1
    ORDER BY (COUNT(*)) DESC
"#;

pub(crate) const ERROR_BREAKDOWN_SQL: &str = r#"
    SELECT
        CASE WHEN sl."call_type" = '' THEN 'Unknown' ELSE sl."call_type" END AS call_type,
        COALESCE(NULLIF(sl."metadata"->'error_information'->>'error_code', ''), 'Unknown') AS error_code,
        COALESCE(NULLIF(sl."metadata"->'error_information'->>'error_class', ''), 'Unknown') AS error_class,
        COUNT(*)::int AS count
    FROM "LiteLLM_SpendLogs" sl
    LEFT JOIN "LiteLLM_VerificationToken" vt ON sl."api_key" = vt."token"
    WHERE
        sl."status" = 'failure'
        AND sl."startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
        AND sl."startTime" <  (($2::timestamptz + INTERVAL '1 day') AT TIME ZONE 'UTC')
        AND ($3::jsonb = '[]'::jsonb
            OR COALESCE(vt."key_alias", 'Unnamed Key') IN (SELECT jsonb_array_elements_text($3::jsonb)))
        AND ($4::jsonb = '[]'::jsonb
            OR sl."model" IN (SELECT jsonb_array_elements_text($4::jsonb)))
        AND sl."call_type" NOT IN (SELECT jsonb_array_elements_text($5::jsonb))
    GROUP BY 1, 2, 3
    ORDER BY (COUNT(*)) DESC
"#;

pub(crate) const KEY_ALIAS_OPTIONS_SQL: &str = r#"
    SELECT DISTINCT COALESCE(vt."key_alias", 'Unnamed Key') AS key_alias
    FROM "LiteLLM_SpendLogs" sl
    LEFT JOIN "LiteLLM_VerificationToken" vt ON sl."api_key" = vt."token"
    WHERE
        sl."startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
        AND sl."startTime" <  (($2::timestamptz + INTERVAL '1 day') AT TIME ZONE 'UTC')
        AND sl."call_type" NOT IN (SELECT jsonb_array_elements_text($3::jsonb))
    ORDER BY 1
"#;

pub(crate) const MODEL_OPTIONS_SQL: &str = r#"
    SELECT DISTINCT sl."model" AS model
    FROM "LiteLLM_SpendLogs" sl
    WHERE
        sl."startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
        AND sl."startTime" <  (($2::timestamptz + INTERVAL '1 day') AT TIME ZONE 'UTC')
        AND sl."model" != ''
        AND sl."call_type" NOT IN (SELECT jsonb_array_elements_text($3::jsonb))
    ORDER BY 1
"#;
