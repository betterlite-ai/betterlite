use super::filters::build_where_clause_at;
use super::metrics::{key_spend_select, rollup_metric_select};
use super::{PTU_SENTINEL_API_KEY, USAGE_KEY_PAGE_MAX, USAGE_TOP_API_KEYS_MAX};
use crate::daily_activity_contracts::{DailyActivityScope, SqlParameter, SqlQuery};
use chrono::DateTime;
use chrono::Utc;
use thiserror::Error;

const MODEL_GROUP: &str = "COALESCE(NULLIF(model_group, ''), model)";
const API_KEY_ROLLED_UP_BIT: &str = "32";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DailyActivitySqlError {
    #[error("api_key_limit must be between 1 and 1000")]
    ApiKeyLimit,
    #[error("limit must be between 1 and 100")]
    PageLimit,
    #[error("offset must be non-negative")]
    Offset,
}

fn top_keys(table: &str, where_clause: &str, sentinel: i64, limit: i64) -> String {
    format!(
        "SELECT api_key, COUNT(*) OVER () AS distinct_api_keys FROM \"{table}\" WHERE {where_clause} AND api_key <> ${sentinel} GROUP BY api_key ORDER BY SUM(spend::numeric) DESC, api_key LIMIT ${limit}"
    )
}

fn validate_api_key_limit(limit: i64) -> Result<(), DailyActivitySqlError> {
    if (1..=USAGE_TOP_API_KEYS_MAX).contains(&limit) {
        Ok(())
    } else {
        Err(DailyActivitySqlError::ApiKeyLimit)
    }
}

fn next_index(value: usize) -> i64 {
    i64::try_from(value).map_or(i64::MAX, |index| index.saturating_add(1))
}

pub fn build_aggregated_sql(
    scope: &DailyActivityScope,
    api_key_limit: i64,
) -> Result<SqlQuery, DailyActivitySqlError> {
    build_aggregated_sql_at(scope, api_key_limit, None)
}

pub fn build_aggregated_sql_at(
    scope: &DailyActivityScope,
    api_key_limit: i64,
    utc_now: Option<DateTime<Utc>>,
) -> Result<SqlQuery, DailyActivitySqlError> {
    let input = scope.as_ref();
    let (where_clause, where_params) = build_where_clause_at(scope, 1, utc_now);
    validate_api_key_limit(api_key_limit)?;
    let sentinel = next_index(where_params.len());
    let limit = sentinel.saturating_add(1);
    let metrics = rollup_metric_select(input.table);
    let table = input.table.metadata().relation;
    let top = top_keys(table, &where_clause, sentinel, limit);
    let sql = format!(
        "(SELECT date, NULL::text AS api_key, model, {MODEL_GROUP} AS model_group, custom_llm_provider, mcp_namespaced_tool_name, endpoint, (GROUPING(date) << 6) | {API_KEY_ROLLED_UP_BIT} | GROUPING(model, {MODEL_GROUP}, custom_llm_provider, mcp_namespaced_tool_name, endpoint) AS group_level, NULL::bigint AS distinct_api_keys, {metrics} FROM \"{table}\" WHERE {where_clause} GROUP BY GROUPING SETS ((date), (date, model), (date, {MODEL_GROUP}), (date, custom_llm_provider), (date, mcp_namespaced_tool_name), (date, endpoint), ())) UNION ALL (WITH top_api_keys AS ({top}) SELECT date, api_key, model, {MODEL_GROUP} AS model_group, custom_llm_provider, mcp_namespaced_tool_name, endpoint, GROUPING(date, api_key, model, {MODEL_GROUP}, custom_llm_provider, mcp_namespaced_tool_name, endpoint) AS group_level, MAX(top_api_keys.distinct_api_keys) AS distinct_api_keys, {metrics} FROM \"{table}\" JOIN top_api_keys USING (api_key) WHERE {where_clause} GROUP BY GROUPING SETS ((date, api_key), (date, model, api_key), (date, {MODEL_GROUP}, api_key), (date, custom_llm_provider, api_key), (date, mcp_namespaced_tool_name, api_key), (date, endpoint, api_key)))"
    );
    let mut params = where_params;
    params.push(SqlParameter::Text(PTU_SENTINEL_API_KEY.to_owned()));
    params.push(SqlParameter::Integer(api_key_limit));
    Ok(SqlQuery { sql, params })
}

pub fn build_entity_rollup_sql(
    scope: &DailyActivityScope,
    api_key_limit: i64,
) -> Result<SqlQuery, DailyActivitySqlError> {
    build_entity_rollup_sql_at(scope, api_key_limit, None)
}

pub fn build_entity_rollup_sql_at(
    scope: &DailyActivityScope,
    api_key_limit: i64,
    utc_now: Option<DateTime<Utc>>,
) -> Result<SqlQuery, DailyActivitySqlError> {
    let input = scope.as_ref();
    let (where_clause, where_params) = build_where_clause_at(scope, 1, utc_now);
    validate_api_key_limit(api_key_limit)?;
    let sentinel = next_index(where_params.len());
    let limit = sentinel.saturating_add(1);
    let table = input.table.metadata().relation;
    let entity = &input.entity_id_field;
    let metrics = rollup_metric_select(input.table);
    let top = top_keys(table, &where_clause, sentinel, limit);
    let sql = format!(
        "WITH top_api_keys AS ({top}), entity_api_keys AS (SELECT COALESCE(\"{entity}\", '') AS entity_id, COUNT(DISTINCT api_key)::bigint AS distinct_api_keys FROM \"{table}\" WHERE {where_clause} AND api_key <> ${sentinel} GROUP BY COALESCE(\"{entity}\", '')) (SELECT e.*, COALESCE(k.distinct_api_keys, 0)::bigint AS distinct_api_keys FROM (SELECT COALESCE(\"{entity}\", '') AS entity_id, date, NULL::text AS api_key, 1 AS api_key_rolled, {metrics} FROM \"{table}\" WHERE {where_clause} GROUP BY date, COALESCE(\"{entity}\", '')) e LEFT JOIN entity_api_keys k ON k.entity_id = e.entity_id) UNION ALL (SELECT COALESCE(\"{entity}\", '') AS entity_id, date, api_key, 0 AS api_key_rolled, {metrics}, NULL::bigint AS distinct_api_keys FROM \"{table}\" JOIN top_api_keys USING (api_key) WHERE {where_clause} GROUP BY date, COALESCE(\"{entity}\", ''), api_key)"
    );
    let mut params = where_params;
    params.push(SqlParameter::Text(PTU_SENTINEL_API_KEY.to_owned()));
    params.push(SqlParameter::Integer(api_key_limit));
    Ok(SqlQuery { sql, params })
}

pub fn build_key_page_sql(
    scope: &DailyActivityScope,
    offset: i64,
    limit: i64,
) -> Result<SqlQuery, DailyActivitySqlError> {
    build_key_page_sql_at(scope, offset, limit, None)
}

pub fn build_key_page_sql_at(
    scope: &DailyActivityScope,
    offset: i64,
    limit: i64,
    utc_now: Option<DateTime<Utc>>,
) -> Result<SqlQuery, DailyActivitySqlError> {
    if !(1..=USAGE_KEY_PAGE_MAX).contains(&limit) {
        return Err(DailyActivitySqlError::PageLimit);
    }
    if offset < 0 {
        return Err(DailyActivitySqlError::Offset);
    }
    let input = scope.as_ref();
    let (where_clause, where_params) = build_where_clause_at(scope, 1, utc_now);
    let sentinel = next_index(where_params.len());
    let limit_param = sentinel.saturating_add(1);
    let offset_param = limit_param.saturating_add(1);
    let table = input.table.metadata().relation;
    let sql = format!(
        "WITH ranked AS (SELECT api_key, {metrics}, SUM(spend::numeric) AS rank_spend FROM \"{table}\" WHERE {where_clause} AND api_key <> ${sentinel} GROUP BY api_key) SELECT (SELECT COUNT(*) FROM ranked)::bigint AS total_api_keys, page.* FROM (SELECT 1) AS one LEFT JOIN LATERAL (SELECT * FROM ranked ORDER BY rank_spend DESC, api_key LIMIT ${limit_param} OFFSET ${offset_param}) AS page ON TRUE",
        metrics = key_spend_select()
    );
    let mut params = where_params;
    params.extend([
        SqlParameter::Text(PTU_SENTINEL_API_KEY.to_owned()),
        SqlParameter::Integer(limit),
        SqlParameter::Integer(offset),
    ]);
    Ok(SqlQuery { sql, params })
}
