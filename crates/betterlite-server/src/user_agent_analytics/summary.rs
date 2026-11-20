use axum::Json;
use axum::response::IntoResponse as _;
use axum::response::Response;
use chrono::NaiveDate;

use super::error::db_not_connected;
use super::error::invalid_date;
use super::error::query_failed;
use super::query::AnalyticsQuery;
use super::responses::TagSummaryMetrics;
use super::responses::TagSummaryResponse;
use super::state::UserAgentAnalyticsState;
use super::window::DATE_FORMAT;

const FAILURE: &str = "Failed to fetch tag summary analytics";

pub(super) async fn tag_summary(
    state: &UserAgentAnalyticsState,
    query: &AnalyticsQuery,
) -> Response {
    let (start_date, end_date) = match query.date_range() {
        Ok(range) => range,
        Err(issues) => return issues.into_response(),
    };
    let Some(store) = state.store.as_deref() else {
        return db_not_connected();
    };
    if let Err(source) = NaiveDate::parse_from_str(start_date, DATE_FORMAT) {
        return invalid_date(&source);
    }
    if let Err(source) = NaiveDate::parse_from_str(end_date, DATE_FORMAT) {
        return invalid_date(&source);
    }
    let filter = query.filter();
    match store
        .tag_summary(start_date, end_date, filter.as_ref())
        .await
    {
        Ok(rows) => Json(TagSummaryResponse {
            results: rows.into_iter().map(TagSummaryMetrics::from).collect(),
        })
        .into_response(),
        Err(source) => query_failed(FAILURE, &source),
    }
}
