use axum::Json;
use axum::response::IntoResponse as _;
use axum::response::Response;

use super::error::db_not_connected;
use super::error::query_failed;
use super::query::AnalyticsQuery;
use super::query::Pagination;
use super::rows::TagSpendRecord;
use super::state::UserAgentAnalyticsState;
use super::store::RangeQuery;
use super::store::UserAgentAnalyticsStore;
use super::window::PER_USER_SPAN_DAYS;
use super::window::Window;

mod metrics;

const FAILURE: &str = "Failed to fetch per-user analytics";

pub(super) async fn per_user_analytics(
    state: &UserAgentAnalyticsState,
    query: &AnalyticsQuery,
) -> Response {
    let pagination = match query.pagination() {
        Ok(pagination) => pagination,
        Err(issues) => return issues.into_response(),
    };
    let Some(store) = state.store.as_deref() else {
        return db_not_connected();
    };
    let window = match Window::ending_tomorrow(state.clock.as_ref(), PER_USER_SPAN_DAYS) {
        Ok(window) => window,
        Err(source) => return query_failed(FAILURE, &source),
    };
    let filter = query.filter();
    let range = RangeQuery {
        start_date: &window.start_date,
        end_date: &window.end_date,
        filter: filter.as_ref(),
    };
    let records = match store.tag_records(range).await {
        Ok(records) => records,
        Err(source) => return query_failed(FAILURE, &source),
    };
    page_records(store, records, pagination).await
}

async fn page_records(
    store: &dyn UserAgentAnalyticsStore,
    records: Vec<TagSpendRecord>,
    pagination: Pagination,
) -> Response {
    let tokens = metrics::api_keys(&records);
    if tokens.is_empty() {
        return Json(metrics::page(Vec::new(), pagination)).into_response();
    }
    let owners = match store.token_owners(&tokens).await {
        Ok(owners) => owners,
        Err(source) => return query_failed(FAILURE, &source),
    };
    let user_ids = metrics::user_ids(&owners);
    let emails = match store.user_emails(&user_ids).await {
        Ok(emails) => emails,
        Err(source) => return query_failed(FAILURE, &source),
    };
    match metrics::aggregate(&records, &owners, &emails) {
        Ok(users) => Json(metrics::page(users, pagination)).into_response(),
        Err(source) => query_failed(FAILURE, &source),
    }
}
