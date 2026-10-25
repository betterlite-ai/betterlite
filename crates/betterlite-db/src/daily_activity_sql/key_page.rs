use super::filters::build_where_clause;
use super::metrics::KEY_SPEND_SELECT;
use super::{DailyActivitySqlError, PTU_SENTINEL_API_KEY, USAGE_KEY_PAGE_MAX};
use crate::daily_activity_contracts::{DailyActivityScope, SqlParameter, SqlQuery};
use chrono::{DateTime, Utc};

pub fn build_key_page_sql<Clock>(
    scope: &DailyActivityScope,
    offset: i64,
    limit: i64,
    clock: &Clock,
) -> Result<SqlQuery, DailyActivitySqlError>
where
    Clock: Fn() -> DateTime<Utc>,
{
    if !(1..=USAGE_KEY_PAGE_MAX).contains(&limit) {
        return Err(DailyActivitySqlError::PageLimit);
    }
    if offset < 0 {
        return Err(DailyActivitySqlError::Offset);
    }
    let input = scope.as_ref();
    let SqlQuery {
        sql: where_clause,
        mut params,
    } = build_where_clause(scope, 1, clock)?;
    let sentinel = params.len().saturating_add(1);
    let limit_param = sentinel.saturating_add(1);
    let offset_param = limit_param.saturating_add(1);
    let table = input.table.metadata().relation;
    let sql = format!(
        "WITH ranked AS ( SELECT api_key, {KEY_SPEND_SELECT}, SUM(spend::numeric) AS rank_spend FROM \"{table}\" WHERE {where_clause} AND api_key <> ${sentinel} GROUP BY api_key ) SELECT (SELECT COUNT(*) FROM ranked)::bigint AS total_api_keys, page.* FROM (SELECT 1) AS one LEFT JOIN LATERAL ( SELECT * FROM ranked ORDER BY rank_spend DESC, api_key LIMIT ${limit_param} OFFSET ${offset_param} ) AS page ON TRUE"
    );
    params.extend([
        SqlParameter::Text(PTU_SENTINEL_API_KEY.to_owned()),
        SqlParameter::Integer(limit),
        SqlParameter::Integer(offset),
    ]);
    Ok(SqlQuery { sql, params })
}
