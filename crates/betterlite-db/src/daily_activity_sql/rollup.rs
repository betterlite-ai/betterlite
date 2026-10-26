use super::filters::build_where_clause;
use super::metrics::rollup_metric_select;
use super::{DailyActivitySqlError, PTU_SENTINEL_API_KEY, USAGE_TOP_API_KEYS_MAX};
use crate::daily_activity_contracts::{DailyActivityScope, SqlParameter, SqlQuery};
use chrono::{DateTime, Utc};

const MODEL_GROUP: &str = "COALESCE(NULLIF(model_group, ''), model)";
const API_KEY_ROLLED_UP_BIT: &str = "32";

#[derive(Clone, Copy)]
pub enum Rollup {
    Aggregate,
    Entity,
}

pub fn build_rollup_sql<Clock>(
    scope: &DailyActivityScope,
    api_key_limit: i64,
    kind: Rollup,
    clock: &Clock,
) -> Result<SqlQuery, DailyActivitySqlError>
where
    Clock: Fn() -> DateTime<Utc>,
{
    let SqlQuery {
        sql: where_clause,
        mut params,
    } = build_where_clause(scope, 1, clock)?;
    if !(1..=USAGE_TOP_API_KEYS_MAX).contains(&api_key_limit) {
        return Err(DailyActivitySqlError::ApiKeyLimit);
    }
    let sentinel = params.len().saturating_add(1);
    let limit = sentinel.saturating_add(1);
    let input = scope.as_ref();
    let table = input.table.metadata().relation;
    let metrics = rollup_metric_select(input.table);
    let entity = &input.entity_id_field;
    let top = format!(
        "SELECT api_key, COUNT(*) OVER () AS distinct_api_keys FROM \"{table}\" WHERE {where_clause} AND api_key <> ${sentinel} GROUP BY api_key ORDER BY SUM(spend::numeric) DESC, api_key LIMIT ${limit}"
    );
    let sql = match kind {
        Rollup::Aggregate => format!(
            "(SELECT date, NULL::text AS api_key, model, {MODEL_GROUP} AS model_group, custom_llm_provider, mcp_namespaced_tool_name, endpoint, (GROUPING(date) << 6) | {API_KEY_ROLLED_UP_BIT} | GROUPING(model, {MODEL_GROUP}, custom_llm_provider, mcp_namespaced_tool_name, endpoint) AS group_level, NULL::bigint AS distinct_api_keys, {metrics} FROM \"{table}\" WHERE {where_clause} GROUP BY GROUPING SETS ( (date), (date, model), (date, {MODEL_GROUP}), (date, custom_llm_provider), (date, mcp_namespaced_tool_name), (date, endpoint), () )) UNION ALL (WITH top_api_keys AS ( {top} ) SELECT date, api_key, model, {MODEL_GROUP} AS model_group, custom_llm_provider, mcp_namespaced_tool_name, endpoint, GROUPING(date, api_key, model, {MODEL_GROUP}, custom_llm_provider, mcp_namespaced_tool_name, endpoint) AS group_level, MAX(top_api_keys.distinct_api_keys) AS distinct_api_keys, {metrics} FROM \"{table}\" JOIN top_api_keys USING (api_key) WHERE {where_clause} GROUP BY GROUPING SETS ( (date, api_key), (date, model, api_key), (date, {MODEL_GROUP}, api_key), (date, custom_llm_provider, api_key), (date, mcp_namespaced_tool_name, api_key), (date, endpoint, api_key) ))"
        ),
        Rollup::Entity => format!(
            "WITH top_api_keys AS ( {top} ), entity_api_keys AS ( SELECT COALESCE(\"{entity}\", '') AS entity_id, COUNT(DISTINCT api_key)::bigint AS distinct_api_keys FROM \"{table}\" WHERE {where_clause} AND api_key <> ${sentinel} GROUP BY COALESCE(\"{entity}\", '') ) (SELECT e.*, COALESCE(k.distinct_api_keys, 0)::bigint AS distinct_api_keys FROM ( SELECT COALESCE(\"{entity}\", '') AS entity_id, date, NULL::text AS api_key, 1 AS api_key_rolled, {metrics} FROM \"{table}\" WHERE {where_clause} GROUP BY date, COALESCE(\"{entity}\", '') ) e LEFT JOIN entity_api_keys k ON k.entity_id = e.entity_id) UNION ALL (SELECT COALESCE(\"{entity}\", '') AS entity_id, date, api_key, 0 AS api_key_rolled, {metrics}, NULL::bigint AS distinct_api_keys FROM \"{table}\" JOIN top_api_keys USING (api_key) WHERE {where_clause} GROUP BY date, COALESCE(\"{entity}\", ''), api_key)"
        ),
    };
    params.extend([
        SqlParameter::Text(PTU_SENTINEL_API_KEY.to_owned()),
        SqlParameter::Integer(api_key_limit),
    ]);
    Ok(SqlQuery { sql, params })
}
