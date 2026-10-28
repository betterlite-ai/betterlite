use crate::daily_activity_contracts::{
    DailyActivityScope, ExportCursor, ExportType, SqlParameter, SqlQuery,
};
use crate::daily_activity_sql::filters::build_where_clause;
use crate::daily_activity_sql::metrics::KEY_SPEND_SELECT;
use crate::daily_activity_sql::{DailyActivitySqlError, PTU_SENTINEL_API_KEY};
use chrono::{DateTime, Utc};
pub mod export;
mod grouping;
mod ranking;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DailyActivityExportError {
    #[error("limit must be at least 1")]
    Limit,
    #[error("SQL bind index overflow")]
    BindOverflow,
    #[error(transparent)]
    Sql(#[from] DailyActivitySqlError),
}

mod tests;

pub(super) const fn bounded_limit(limit: i64) -> Result<(), DailyActivityExportError> {
    if limit < 1 {
        return Err(DailyActivityExportError::Limit);
    }
    Ok(())
}

pub fn build_export_sql<Clock>(
    scope: &DailyActivityScope,
    export_type: ExportType,
    after: Option<&ExportCursor>,
    batch_size: i64,
    clock: &Clock,
) -> Result<SqlQuery, DailyActivityExportError>
where
    Clock: Fn() -> DateTime<Utc>,
{
    bounded_limit(batch_size)?;
    let SqlQuery {
        sql: where_sql,
        mut params,
    } = build_where_clause(scope, 1, clock)?;
    let grouping = grouping::group_fields(export_type);
    let indexes = export::export_bind_indexes(params.len(), export_type, after.is_some())?;
    let plan = export::ExportPlan {
        scope,
        export_type,
        grouping: &grouping,
        where_sql: &where_sql,
        indexes,
    };
    let sql = export::render_export_sql(&plan);
    if plan.indexes.sentinel.is_some() {
        params.push(SqlParameter::Text(PTU_SENTINEL_API_KEY.to_owned()));
    }
    if let Some(cursor_value) = after {
        params.extend([
            SqlParameter::Text(cursor_value.date.clone()),
            SqlParameter::Text(cursor_value.entity_id.clone()),
            SqlParameter::Text(cursor_value.group_key.clone()),
        ]);
    }
    params.push(SqlParameter::Integer(batch_size));
    Ok(SqlQuery { sql, params })
}

pub fn build_key_search_sql<Clock>(
    scope: &DailyActivityScope,
    search: &str,
    limit: i64,
    clock: &Clock,
) -> Result<SqlQuery, DailyActivityExportError>
where
    Clock: Fn() -> DateTime<Utc>,
{
    bounded_limit(limit)?;
    ranking::build(scope, ranking::Filter::Search(search), limit, clock)
}

pub fn build_model_top_keys_sql<Clock>(
    scope: &DailyActivityScope,
    model_group: &str,
    by_model_group: bool,
    limit: i64,
    clock: &Clock,
) -> Result<SqlQuery, DailyActivityExportError>
where
    Clock: Fn() -> DateTime<Utc>,
{
    bounded_limit(limit)?;
    let expression = if by_model_group {
        "COALESCE(NULLIF(model_group, ''), model)"
    } else {
        "model"
    };
    ranking::build(
        scope,
        ranking::Filter::Model(expression, model_group),
        limit,
        clock,
    )
}

pub fn build_cache_leakage_keys_sql<Clock>(
    scope: &DailyActivityScope,
    limit: i64,
    clock: &Clock,
) -> Result<SqlQuery, DailyActivityExportError>
where
    Clock: Fn() -> DateTime<Utc>,
{
    bounded_limit(limit)?;
    let SqlQuery {
        sql: where_sql,
        mut params,
    } = build_where_clause(scope, 1, clock)?;
    let sentinel_index = params
        .len()
        .checked_add(1)
        .ok_or(DailyActivityExportError::BindOverflow)?;
    let limit_index = sentinel_index
        .checked_add(1)
        .ok_or(DailyActivityExportError::BindOverflow)?;
    let table = scope.as_ref().table.metadata().relation;
    let sql = format!(
        "SELECT api_key, {KEY_SPEND_SELECT} FROM \"{table}\" WHERE {where_sql} AND api_key <> ${sentinel_index} GROUP BY api_key HAVING SUM(prompt_tokens) - SUM(cache_read_input_tokens) > 0 ORDER BY SUM(prompt_tokens) - SUM(cache_read_input_tokens) DESC, api_key LIMIT ${limit_index}"
    );
    params.extend([
        SqlParameter::Text(PTU_SENTINEL_API_KEY.to_owned()),
        SqlParameter::Integer(limit),
    ]);
    Ok(SqlQuery { sql, params })
}
