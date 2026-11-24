use crate::spend::log_query::validate::{SortColumn, SortDirection, SpendLogChoices};

// ttft_ms is the completion-start delta; a null or end-time completion start has no ttft and sorts last either way
pub const TTFT_ORDER_SQL: &str = concat!(
    "CASE WHEN \"completionStartTime\" IS NULL ",
    "OR \"completionStartTime\" = \"endTime\" THEN NULL ",
    "ELSE (EXTRACT(EPOCH FROM (\"completionStartTime\" - \"startTime\")) * 1000) END"
);

#[must_use]
pub const fn order_expression(column: SortColumn) -> &'static str {
    match column {
        SortColumn::Spend => "spend",
        SortColumn::TotalTokens => "total_tokens",
        SortColumn::StartTime => "\"startTime\"",
        SortColumn::EndTime => "\"endTime\"",
        SortColumn::RequestDurationMs => "request_duration_ms",
        SortColumn::Model => "model",
        SortColumn::TtftMs => TTFT_ORDER_SQL,
    }
}

#[must_use]
pub const fn direction_sql(direction: SortDirection) -> &'static str {
    match direction {
        SortDirection::Ascending => "ASC",
        SortDirection::Descending => "DESC",
    }
}

#[must_use]
pub const fn nulls_sql(column: SortColumn) -> &'static str {
    if matches!(column, SortColumn::TtftMs) {
        " NULLS LAST"
    } else {
        ""
    }
}

// the ORDER BY of the page read: an exact primary-id match first, then the closed column and direction, and for grouped rows request_id as the last tie-breaker
#[must_use]
pub fn order_clause(
    choices: &SpendLogChoices,
    request_id_index: Option<usize>,
    grouped: bool,
) -> String {
    let exact = request_id_index.map_or_else(String::new, |index| {
        format!("(request_id = ${index}) DESC, ")
    });
    let column = order_expression(choices.sort_column);
    let direction = direction_sql(choices.sort_direction);
    let nulls = nulls_sql(choices.sort_column);
    let mut clause = format!("{exact}{column} {direction}{nulls}");
    if grouped {
        clause.push_str(", request_id");
    }
    clause
}
