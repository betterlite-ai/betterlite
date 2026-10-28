use super::DailyActivityExportError;
use super::grouping::{Grouping, cursor_clause as export_cursor_clause, table_parts};
use crate::daily_activity_contracts::{DailyActivityScope, ExportType};
use crate::daily_activity_sql::metrics::ptu_flat_cost_select;

pub(super) struct ExportPlan<'scope> {
    pub(super) scope: &'scope DailyActivityScope,
    pub(super) export_type: ExportType,
    pub(super) grouping: &'scope Grouping,
    pub(super) where_sql: &'scope str,
    pub(super) indexes: ExportIndexes,
}
pub(super) struct ExportIndexes {
    pub(super) sentinel: Option<usize>,
    cursor: Option<[usize; 3]>,
    limit: usize,
}

pub(super) fn export_bind_indexes(
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

pub(super) fn render_export_sql(plan: &ExportPlan<'_>) -> String {
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
    let (entity_alias, entity_join) = table_parts(scope.as_ref().table);
    let model = if plan.export_type == ExportType::DailyWithModels {
        "NULLIF(COALESCE(scoped.model, ''), '')"
    } else {
        "NULL::text"
    };
    let flat_cost = ptu_flat_cost_select(scope.as_ref().table, false);
    let cursor_sql = export_cursor_clause(scope, plan.indexes.cursor, grouping.key);
    format!(
        "WITH scoped AS ( SELECT * FROM \"{table}\" WHERE {}{} ) SELECT scoped.date, COALESCE(scoped.\"{entity}\", '') AS entity_id, {entity_alias} AS entity_alias, {} AS api_key, {}, {model} AS model, COALESCE(SUM(scoped.spend), 0)::float AS spend, {flat_cost} AS flat_cost, COALESCE(SUM(scoped.prompt_tokens), 0)::bigint AS prompt_tokens, COALESCE(SUM(scoped.completion_tokens), 0)::bigint AS completion_tokens, COALESCE(SUM(scoped.api_requests), 0)::bigint AS api_requests, COALESCE(SUM(scoped.successful_requests), 0)::bigint AS successful_requests, COALESCE(SUM(scoped.failed_requests), 0)::bigint AS failed_requests, COALESCE(SUM(scoped.cache_read_input_tokens), 0)::bigint AS cache_read_input_tokens, COALESCE(SUM(scoped.cache_creation_input_tokens), 0)::bigint AS cache_creation_input_tokens FROM scoped{}{entity_join} WHERE TRUE{} GROUP BY scoped.date, COALESCE(scoped.\"{entity}\", ''){} ORDER BY scoped.date, COALESCE(scoped.\"{entity}\", ''){} LIMIT ${}",
        plan.where_sql,
        sentinel_sql,
        grouping.output,
        grouping.user_fields,
        grouping.joins,
        cursor_sql,
        group_suffix,
        group_suffix,
        plan.indexes.limit
    )
}
