use super::grouping::{Grouping, cursor_clause as export_cursor_clause, export_grouping};
use super::{DailyActivityExportError, bounded_limit};
use crate::daily_activity_contracts::{
    DailyActivityScope, ExportCursor, ExportType, SqlParameter, SqlQuery,
};
use crate::daily_activity_sql::PTU_SENTINEL_API_KEY;
use crate::daily_activity_sql::filters::build_where_clause;
use crate::daily_activity_sql::metrics::ptu_flat_cost_select;
use chrono::{DateTime, Utc};

pub(super) fn build_export_sql<Clock>(
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
    let grouping = export_grouping(export_type, scope.as_ref().table);
    let indexes = export_bind_indexes(params.len(), export_type, after.is_some())?;
    let plan = ExportPlan {
        scope,
        export_type,
        grouping: &grouping,
        where_sql: &where_sql,
        indexes,
    };
    let sql = render_export_sql(&plan);
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

struct ExportPlan<'scope> {
    scope: &'scope DailyActivityScope,
    export_type: ExportType,
    grouping: &'scope Grouping,
    where_sql: &'scope str,
    indexes: ExportIndexes,
}
struct ExportIndexes {
    sentinel: Option<usize>,
    cursor: Option<[usize; 3]>,
    limit: usize,
}

fn export_bind_indexes(
    parameter_count: usize,
    export_type: ExportType,
    has_cursor: bool,
) -> Result<ExportIndexes, DailyActivityExportError> {
    let mut next = parameter_count
        .checked_add(1)
        .ok_or(DailyActivityExportError::BindOverflow)?;
    let sentinel = (export_type != ExportType::Daily).then_some(next);
    if sentinel.is_some() {
        next = next
            .checked_add(1)
            .ok_or(DailyActivityExportError::BindOverflow)?;
    }
    let cursor = if has_cursor {
        let result = [
            next,
            next.checked_add(1)
                .ok_or(DailyActivityExportError::BindOverflow)?,
            next.checked_add(2)
                .ok_or(DailyActivityExportError::BindOverflow)?,
        ];
        next = next
            .checked_add(3)
            .ok_or(DailyActivityExportError::BindOverflow)?;
        Some(result)
    } else {
        None
    };
    Ok(ExportIndexes {
        sentinel,
        cursor,
        limit: next,
    })
}

fn render_export_sql(plan: &ExportPlan<'_>) -> String {
    let scope = plan.scope;
    let grouping = plan.grouping;
    let sentinel_sql = plan
        .indexes
        .sentinel
        .map_or_else(String::new, |index| format!(" AND api_key <> ${index}"));
    let table = scope.as_ref().table.metadata().relation;
    let entity = scope.as_ref().entity_id_field.as_str();
    let group_suffix = if plan.export_type == ExportType::Daily {
        String::new()
    } else {
        format!(", {}", grouping.key)
    };
    let flat_cost = ptu_flat_cost_select(scope.as_ref().table, false);
    let cursor_sql = export_cursor_clause(scope, plan.indexes.cursor, &grouping.key);
    format!(
        "WITH scoped AS ( SELECT * FROM \"{table}\" WHERE {}{} ) SELECT scoped.date, COALESCE(scoped.\"{entity}\", '') AS entity_id, {} AS entity_alias, {} AS api_key, {}, {}, {}, {} AS model, COALESCE(SUM(scoped.spend), 0)::float AS spend, {flat_cost} AS flat_cost, COALESCE(SUM(scoped.prompt_tokens), 0)::bigint AS prompt_tokens, COALESCE(SUM(scoped.completion_tokens), 0)::bigint AS completion_tokens, COALESCE(SUM(scoped.api_requests), 0)::bigint AS api_requests, COALESCE(SUM(scoped.successful_requests), 0)::bigint AS successful_requests, COALESCE(SUM(scoped.failed_requests), 0)::bigint AS failed_requests, COALESCE(SUM(scoped.cache_read_input_tokens), 0)::bigint AS cache_read_input_tokens, COALESCE(SUM(scoped.cache_creation_input_tokens), 0)::bigint AS cache_creation_input_tokens FROM scoped{} WHERE TRUE{} GROUP BY scoped.date, COALESCE(scoped.\"{entity}\", ''){} ORDER BY scoped.date, COALESCE(scoped.\"{entity}\", ''){} LIMIT ${}",
        plan.where_sql,
        sentinel_sql,
        grouping.alias,
        grouping.output,
        grouping.key_alias,
        grouping.user_id,
        grouping.user_email,
        grouping.model,
        grouping.joins,
        cursor_sql,
        group_suffix,
        group_suffix,
        plan.indexes.limit
    )
}
