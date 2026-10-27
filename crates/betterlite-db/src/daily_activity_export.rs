use crate::daily_activity_contracts::{
    DailyActivityScope, ExportCursor, ExportType, SqlParameter, SqlQuery,
};
use crate::daily_activity_sql::filters::build_where_clause;
use crate::daily_activity_sql::metrics::KEY_SPEND_SELECT;
use crate::daily_activity_sql::{DailyActivitySqlError, PTU_SENTINEL_API_KEY};
use chrono::{DateTime, Utc};
pub mod export;
mod grouping;

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
    let grouping = grouping::export_grouping(export_type, scope.as_ref().table);
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
    let escaped = search
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let wildcard = format!("%{escaped}%");
    ranked_query(
        scope,
        limit,
        clock,
        move |[search_index, sentinel_index, _]: [usize; 3]| {
            format!(
                "AND api_key <> ${sentinel_index} AND ( api_key ILIKE ${search_index} ESCAPE '\\\\' OR api_key IN ( SELECT v.token FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_UserTable\" u ON u.user_id = v.user_id WHERE v.key_alias ILIKE ${search_index} ESCAPE '\\\\' OR v.user_id ILIKE ${search_index} ESCAPE '\\\\' OR u.user_email ILIKE ${search_index} ESCAPE '\\\\' UNION SELECT d.token FROM \"LiteLLM_DeletedVerificationToken\" d LEFT JOIN \"LiteLLM_UserTable\" u ON u.user_id = d.user_id WHERE d.key_alias ILIKE ${search_index} ESCAPE '\\\\' OR d.user_id ILIKE ${search_index} ESCAPE '\\\\' OR u.user_email ILIKE ${search_index} ESCAPE '\\\\' ) )"
            )
            .replace("\\\\", "\\")
        },
        wildcard,
    )
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
    let model_expression = if by_model_group {
        "COALESCE(NULLIF(model_group, ''), model)"
    } else {
        "model"
    };
    let model_value = model_group.to_owned();
    ranked_query(
        scope,
        limit,
        clock,
        move |[model_index, sentinel_index, _]: [usize; 3]| {
            format!("AND {model_expression} = ${model_index} AND api_key <> ${sentinel_index}")
        },
        model_value,
    )
}

fn ranked_query<Clock, Predicate>(
    scope: &DailyActivityScope,
    limit: i64,
    clock: &Clock,
    predicate: Predicate,
    extra: String,
) -> Result<SqlQuery, DailyActivityExportError>
where
    Clock: Fn() -> DateTime<Utc>,
    Predicate: FnOnce([usize; 3]) -> String,
{
    let (where_sql, params, indexes @ [_, _, limit_index]) = ranked_indexes(scope, limit, clock)?;
    let table = scope.as_ref().table.metadata().relation;
    let predicate_sql = predicate(indexes);
    Ok(finish_ranked(
        RankedSql {
            table,
            where_sql,
            predicate: predicate_sql,
            limit_index,
            params,
        },
        extra,
        limit,
    ))
}

fn ranked_indexes<Clock>(
    scope: &DailyActivityScope,
    limit: i64,
    clock: &Clock,
) -> Result<(String, Vec<SqlParameter>, [usize; 3]), DailyActivityExportError>
where
    Clock: Fn() -> DateTime<Utc>,
{
    bounded_limit(limit)?;
    let SqlQuery { sql, params } = build_where_clause(scope, 1, clock)?;
    let first = params
        .len()
        .checked_add(1)
        .ok_or(DailyActivityExportError::BindOverflow)?;
    let second = first
        .checked_add(1)
        .ok_or(DailyActivityExportError::BindOverflow)?;
    let third = second
        .checked_add(1)
        .ok_or(DailyActivityExportError::BindOverflow)?;
    Ok((sql, params, [first, second, third]))
}

struct RankedSql {
    table: &'static str,
    where_sql: String,
    predicate: String,
    limit_index: usize,
    params: Vec<SqlParameter>,
}

fn finish_ranked(plan: RankedSql, extra: String, limit: i64) -> SqlQuery {
    let RankedSql {
        table,
        where_sql,
        predicate,
        limit_index,
        mut params,
    } = plan;
    let sql = format!(
        "SELECT api_key, {KEY_SPEND_SELECT} FROM \"{table}\" WHERE {where_sql} {predicate} GROUP BY api_key ORDER BY SUM(spend::numeric) DESC, api_key LIMIT ${limit_index}"
    );
    params.extend([
        SqlParameter::Text(extra),
        SqlParameter::Text(PTU_SENTINEL_API_KEY.to_owned()),
        SqlParameter::Integer(limit),
    ]);
    SqlQuery { sql, params }
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
