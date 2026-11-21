// _UPDATE_LOGS: drop the raw observation, then store the publication and its signed savings or JSON null.
pub const UPDATE_LOGS: &str = r#"
WITH changes AS (
    SELECT request_id, publication::jsonb AS publication
    FROM jsonb_to_recordset($1::jsonb) AS x(request_id text, publication jsonb)
)
UPDATE "LiteLLM_SpendLogs" AS logs
SET metadata = (COALESCE(logs.metadata::jsonb, '{}'::jsonb) - 'autorouter_baseline_observation') || jsonb_build_object(
    'autorouter_savings_estimate', changes.publication,
    'autorouter_savings', CASE WHEN changes.publication->>'status' = 'estimated' THEN
        (changes.publication->>'baseline_spend')::float8 - (changes.publication->>'actual_spend')::float8
        ELSE NULL END
)
FROM changes WHERE logs.request_id = changes.request_id
"#;

// _UPDATE_PUBLICATIONS: the observation rows keep the text every later replay decodes.
pub const UPDATE_PUBLICATIONS: &str = r#"
UPDATE "LiteLLM_AutoRouterBaselineObservation" AS observations
SET publication = x.publication::text
FROM jsonb_to_recordset($1::jsonb) AS x(request_id text, publication jsonb)
WHERE observations.request_id = x.request_id
"#;

// _session_correction_sql: add the deltas to one session scope and merge the per-baseline estimate counts.
#[must_use]
pub fn session_correction_sql(user_scoped: bool) -> String {
    let table = if user_scoped {
        "LiteLLM_AutoRouterUserSession"
    } else {
        "LiteLLM_AutoRouterSession"
    };
    let identity = if user_scoped {
        "user_id, api_key, session_id, router_name"
    } else {
        "api_key, session_id, router_name"
    };
    let user_filter = if user_scoped {
        "WHERE user_id <> ''"
    } else {
        ""
    };
    let user_match = if user_scoped {
        "session.user_id = totals.user_id AND "
    } else {
        ""
    };
    session_correction_statement(table, identity, user_filter, user_match)
}

fn session_correction_statement(
    table: &str,
    identity: &str,
    user_filter: &str,
    user_match: &str,
) -> String {
    format!(
        r#"
WITH changes AS (
    SELECT * FROM jsonb_to_recordset($1::jsonb) AS x(
        user_id text, api_key text, session_id text, router_name text, baseline_model text,
        covered_delta int, actual_delta float8, savings_delta float8
    )
    {user_filter}
), totals AS (
    SELECT {identity}, SUM(covered_delta)::int AS covered_delta,
        SUM(actual_delta) AS actual_delta, SUM(savings_delta) AS savings_delta
    FROM changes GROUP BY {identity}
), models AS (
    SELECT {identity}, jsonb_object_agg(baseline_model, delta) AS deltas
    FROM (
        SELECT {identity}, baseline_model, SUM(covered_delta)::int AS delta
        FROM changes GROUP BY {identity}, baseline_model
    ) grouped GROUP BY {identity}
)
UPDATE "{table}" AS session
SET saved_spend = session.saved_spend + totals.savings_delta,
    savings_estimated_turns = session.savings_estimated_turns + totals.covered_delta,
    savings_estimated_actual_spend = session.savings_estimated_actual_spend + totals.actual_delta,
    savings_estimated_saved_spend = session.savings_estimated_saved_spend + totals.savings_delta,
    savings_estimated_baseline_models = (
        SELECT COALESCE(jsonb_object_agg(key, value), '{{}}'::jsonb) FROM (
            SELECT key, SUM(value::int)::int AS value FROM (
                SELECT * FROM jsonb_each_text(session.savings_estimated_baseline_models)
                UNION ALL SELECT * FROM jsonb_each_text(models.deltas)
            ) combined GROUP BY key HAVING SUM(value::int) > 0
        ) counts
    )
FROM totals JOIN models USING ({identity})
WHERE {user_match}session.api_key = totals.api_key AND session.session_id = totals.session_id
    AND session.router_name = totals.router_name
"#
    )
}
