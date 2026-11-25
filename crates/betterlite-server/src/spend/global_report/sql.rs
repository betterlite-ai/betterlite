pub(super) const TEAM_CUSTOMER_SQL: &str = r#"
    WITH SpendByModelApiKey AS (
        SELECT
            date_trunc('day', sl."startTime") AS group_by_day,
            COALESCE(tt.team_alias, 'Unassigned Team') AS team_name,
            sl.end_user AS customer,
            sl.model,
            sl.api_key,
            SUM(sl.spend) AS model_api_spend,
            SUM(sl.total_tokens) AS model_api_tokens
        FROM
            "LiteLLM_SpendLogs" sl
        LEFT JOIN
            "LiteLLM_TeamTable" tt
        ON
            sl.team_id = tt.team_id
        WHERE
            sl."startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
            AND sl."startTime" <  (($2::timestamptz + INTERVAL '1 day') AT TIME ZONE 'UTC')
            AND sl.team_id = $3
            AND sl.end_user = $4
        GROUP BY
            date_trunc('day', sl."startTime"),
            tt.team_alias,
            sl.end_user,
            sl.model,
            sl.api_key
    )
        SELECT row_to_json(report_row)::text
        FROM (
            SELECT
                to_char(group_by_day, 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS group_by_day,
                jsonb_agg(jsonb_build_object(
                    'team_name', team_name,
                    'customer', customer,
                    'total_spend', total_spend,
                    'metadata', metadata
                )) AS teams_customers
            FROM (
                SELECT
                    group_by_day,
                    team_name,
                    customer,
                    SUM(model_api_spend) AS total_spend,
                    jsonb_agg(jsonb_build_object(
                        'model', model,
                        'api_key', api_key,
                        'spend', model_api_spend,
                        'total_tokens', model_api_tokens
                    )) AS metadata
                FROM
                    SpendByModelApiKey
                GROUP BY
                    group_by_day,
                    team_name,
                    customer
            ) AS aggregated
            GROUP BY
                group_by_day
        ) AS report_row
        ORDER BY
            report_row.group_by_day;
"#;

// group_by=customer reads every customer of the window; a customer_id or team_id alone never narrows it
pub(super) const CUSTOMER_SQL: &str = r#"
    WITH SpendByModelApiKey AS (
        SELECT
            date_trunc('day', sl."startTime") AS group_by_day,
            sl.end_user AS customer,
            sl.model,
            sl.api_key,
            SUM(sl.spend) AS model_api_spend,
            SUM(sl.total_tokens) AS model_api_tokens
        FROM
            "LiteLLM_SpendLogs" sl
        WHERE
            sl."startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
            AND sl."startTime" <  (($2::timestamptz + INTERVAL '1 day') AT TIME ZONE 'UTC')
        GROUP BY
            date_trunc('day', sl."startTime"),
            customer,
            sl.model,
            sl.api_key
    )
    SELECT row_to_json(report_row)::text
    FROM (
        SELECT
            to_char(group_by_day, 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS group_by_day,
            jsonb_agg(jsonb_build_object(
                'customer', customer,
                'total_spend', total_spend,
                'metadata', metadata
            )) AS customers
        FROM (
            SELECT
                group_by_day,
                customer,
                SUM(model_api_spend) AS total_spend,
                jsonb_agg(jsonb_build_object(
                    'model', model,
                    'api_key', api_key,
                    'spend', model_api_spend,
                    'total_tokens', model_api_tokens
                )) AS metadata
            FROM
                SpendByModelApiKey
            GROUP BY
                group_by_day,
                customer
        ) AS aggregated
        GROUP BY
            group_by_day
    ) AS report_row
    ORDER BY
        report_row.group_by_day;
"#;

// group_by=api_key ranks every key of the window, with no team or customer filter
pub(super) const ALL_KEYS_SQL: &str = r#"
    WITH SpendByModelApiKey AS (
        SELECT
            sl.api_key,
            sl.model,
            SUM(sl.spend) AS model_cost,
            SUM(sl.prompt_tokens) AS model_input_tokens,
            SUM(sl.completion_tokens) AS model_output_tokens
        FROM
            "LiteLLM_SpendLogs" sl
        WHERE
            sl."startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
            AND sl."startTime" <  (($2::timestamptz + INTERVAL '1 day') AT TIME ZONE 'UTC')
        GROUP BY
            sl.api_key,
            sl.model
    )
    SELECT row_to_json(report_row)::text
    FROM (
        SELECT
            api_key,
            SUM(model_cost) AS total_cost,
            SUM(model_input_tokens) AS total_input_tokens,
            SUM(model_output_tokens) AS total_output_tokens,
            jsonb_agg(jsonb_build_object(
                'model', model,
                'total_cost', model_cost,
                'total_input_tokens', model_input_tokens,
                'total_output_tokens', model_output_tokens
            )) AS model_details
        FROM
            SpendByModelApiKey
        GROUP BY
            api_key
    ) AS report_row
    ORDER BY
        report_row.total_cost DESC;
"#;
