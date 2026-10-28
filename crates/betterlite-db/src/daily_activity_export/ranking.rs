use super::DailyActivityExportError;
use crate::daily_activity_contracts::{DailyActivityScope, SqlParameter, SqlQuery};
use crate::daily_activity_sql::PTU_SENTINEL_API_KEY;
use crate::daily_activity_sql::filters::build_where_clause;
use crate::daily_activity_sql::metrics::KEY_SPEND_SELECT;
use chrono::{DateTime, Utc};

#[derive(Clone, Copy)]
pub(super) enum Filter<'input> {
    Search(&'input str),
    Model(&'static str, &'input str),
}

pub(super) fn build<Clock>(
    scope: &DailyActivityScope,
    filter: Filter<'_>,
    limit: i64,
    clock: &Clock,
) -> Result<SqlQuery, DailyActivityExportError>
where
    Clock: Fn() -> DateTime<Utc>,
{
    let SqlQuery {
        sql: where_sql,
        mut params,
    } = build_where_clause(scope, 1, clock)?;
    let filter_index = params
        .len()
        .checked_add(1)
        .ok_or(DailyActivityExportError::BindOverflow)?;
    let sentinel_index = filter_index
        .checked_add(1)
        .ok_or(DailyActivityExportError::BindOverflow)?;
    let (predicate, filter_value) = filter.predicate([filter_index, sentinel_index]);
    let limit_index = sentinel_index
        .checked_add(1)
        .ok_or(DailyActivityExportError::BindOverflow)?;
    params.extend([
        SqlParameter::Text(filter_value),
        SqlParameter::Text(PTU_SENTINEL_API_KEY.to_owned()),
        SqlParameter::Integer(limit),
    ]);
    let table = scope.as_ref().table.metadata().relation;
    let sql = format!(
        "SELECT api_key, {KEY_SPEND_SELECT} FROM \"{table}\" WHERE {where_sql} {predicate} GROUP BY api_key ORDER BY SUM(spend::numeric) DESC, api_key LIMIT ${limit_index}"
    );
    Ok(SqlQuery { sql, params })
}

impl Filter<'_> {
    fn predicate(&self, [filter_index, sentinel_index]: [usize; 2]) -> (String, String) {
        match self {
            Self::Search(search) => {
                let escaped = search
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_");
                let candidates = [
                    ("v", "LiteLLM_VerificationToken"),
                    ("d", "LiteLLM_DeletedVerificationToken"),
                ]
                .map(|(alias, relation)| {
                    format!(
                        "SELECT {alias}.token FROM \"{relation}\" {alias} LEFT JOIN \"LiteLLM_UserTable\" u ON u.user_id = {alias}.user_id WHERE {alias}.key_alias ILIKE ${filter_index} ESCAPE '\\' OR {alias}.user_id ILIKE ${filter_index} ESCAPE '\\' OR u.user_email ILIKE ${filter_index} ESCAPE '\\'"
                    )
                })
                .join(" UNION ");
                (
                    format!(
                        "AND api_key <> ${sentinel_index} AND ( api_key ILIKE ${filter_index} ESCAPE '\\' OR api_key IN ( {candidates} ) )"
                    ),
                    format!("%{escaped}%"),
                )
            }
            Self::Model(expression, value) => (
                format!("AND {expression} = ${filter_index} AND api_key <> ${sentinel_index}"),
                (*value).to_owned(),
            ),
        }
    }
}
