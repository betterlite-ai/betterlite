use super::scope::ScopeColumn;

// _SPEND_REPORT_SCOPE_COLUMNS: the column name is interpolated into the SQL, so it only ever comes from this closed set
pub(super) const fn scope_column_sql(column: ScopeColumn) -> &'static str {
    match column {
        ScopeColumn::ApiKey => "api_key",
        ScopeColumn::User => "user",
        ScopeColumn::TeamId => "team_id",
    }
}

// _scoped_spend_report_sql: spend grouped by api_key with a per-model breakdown, cut to one scope column
pub(super) const SCOPED_REPORT_SQL_TEMPLATE: &str = r#"
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
            AND sl.{scope_column} = $3
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

// _ORG_SPEND_REPORT_SQL: rows stamped with this organization plus unstamped rows on its current teams
pub(super) const ORGANIZATION_SPEND_REPORT_SQL: &str = r#"
    WITH SpendByModelApiKey AS (
        SELECT
            sl.api_key,
            sl.team_id,
            sl.model,
            SUM(sl.spend) AS model_cost,
            SUM(sl.prompt_tokens) AS model_input_tokens,
            SUM(sl.completion_tokens) AS model_output_tokens
        FROM
            "LiteLLM_SpendLogs" sl
        WHERE
            sl."startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
            AND sl."startTime" <  (($2::timestamptz + INTERVAL '1 day') AT TIME ZONE 'UTC')
            AND (
                sl.organization_id = $3
                OR (
                    (sl.organization_id IS NULL OR sl.organization_id = '')
                    AND sl.team_id = ANY($4::text[])
                )
            )
        GROUP BY
            sl.api_key,
            sl.team_id,
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
                'team_id', team_id,
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

// get_spend_by_team: one row per day, each day's teams grouped with their per-model, per-key spend
pub(super) const SPEND_BY_TEAM_SQL: &str = r#"
    WITH SpendByModelApiKey AS (
        SELECT
            date_trunc('day', sl."startTime") AS group_by_day,
            COALESCE(tt.team_alias, 'Unassigned Team') AS team_name,
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
            AND ($3::text IS NULL OR sl.team_id = $3)
        GROUP BY
            date_trunc('day', sl."startTime"),
            tt.team_alias,
            sl.model,
            sl.api_key
    )
        SELECT row_to_json(report_row)::text
        FROM (
            SELECT
                to_char(group_by_day, 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS group_by_day,
                jsonb_agg(jsonb_build_object(
                    'team_name', team_name,
                    'total_spend', total_spend,
                    'metadata', metadata
                )) AS teams
            FROM (
                SELECT
                    group_by_day,
                    team_name,
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
                    team_name
            ) AS aggregated
            GROUP BY
                group_by_day
        ) AS report_row
        ORDER BY
            report_row.group_by_day;
"#;
