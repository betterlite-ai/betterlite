use axum::Json;
use axum::extract::Extension;
use axum::extract::RawQuery;
use axum::extract::State;
use axum::response::IntoResponse as _;
use axum::response::Response;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;

use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

use super::error::AnalyticsError;
use super::error::db_not_connected;
use super::error::query_failed;
use super::query::ActiveUsersQuery;
use super::query::TagFilter;
use super::responses::ActiveUsersAnalyticsResponse;
use super::responses::DistinctTagResponse;
use super::responses::DistinctTagsResponse;
use super::responses::TagActiveUsersResponse;
use super::state::UserAgentAnalyticsState;
use super::store::UserAgentAnalyticsStore;
use super::window::MAX_DAYS;
use super::window::WAU_SPAN_DAYS;
use super::window::Window;
use super::window::weekly_result;

#[derive(Clone, Copy)]
pub(super) enum AnalyticsReport {
    Distinct,
    Daily,
    Weekly,
}

struct ActiveUsersRequest {
    window: Window,
    filter: Option<TagFilter>,
}

pub(super) async fn respond(
    State(state): State<UserAgentAnalyticsState>,
    Extension(report): Extension<AnalyticsReport>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let Some(store) = state.store.as_deref() else {
        return db_not_connected();
    };
    let raw_query = raw_query.as_deref().unwrap_or_default();
    match report {
        AnalyticsReport::Distinct => distinct(store).await,
        AnalyticsReport::Daily => daily(&state, store, raw_query).await,
        AnalyticsReport::Weekly => weekly(&state, store, raw_query).await,
    }
}

fn active_users_request(
    clock: &dyn GatewayRequestClock,
    raw_query: &str,
    span_days: u64,
) -> Result<ActiveUsersRequest, AnalyticsError> {
    let window = Window::ending_tomorrow(clock, span_days)?;
    let filter = ActiveUsersQuery::decode(raw_query).filter();
    Ok(ActiveUsersRequest { window, filter })
}

async fn distinct(store: &dyn UserAgentAnalyticsStore) -> Response {
    match store.distinct_tags().await {
        Ok(rows) => Json(DistinctTagsResponse {
            results: rows
                .into_iter()
                .map(|row| DistinctTagResponse { tag: row.tag })
                .collect(),
        })
        .into_response(),
        Err(source) => query_failed("Failed to fetch distinct user agent tags", &source),
    }
}

async fn daily(
    state: &UserAgentAnalyticsState,
    store: &dyn UserAgentAnalyticsStore,
    raw_query: &str,
) -> Response {
    let request = match active_users_request(state.clock.as_ref(), raw_query, MAX_DAYS) {
        Ok(request) => request,
        Err(source) => return query_failed("Failed to fetch DAU analytics", &source),
    };
    let rows = match store
        .daily_active_users(
            &request.window.start_date,
            &request.window.end_date,
            request.filter.as_ref(),
        )
        .await
    {
        Ok(rows) => rows,
        Err(source) => return query_failed("Failed to fetch DAU analytics", &source),
    };
    let results = rows
        .into_iter()
        .map(|row| TagActiveUsersResponse {
            tag: row.tag,
            active_users: row.active_users,
            date: row.date,
            period_start: None,
            period_end: None,
        })
        .collect();
    Json(ActiveUsersAnalyticsResponse { results }).into_response()
}

async fn weekly(
    state: &UserAgentAnalyticsState,
    store: &dyn UserAgentAnalyticsStore,
    raw_query: &str,
) -> Response {
    let request = match active_users_request(state.clock.as_ref(), raw_query, WAU_SPAN_DAYS) {
        Ok(request) => request,
        Err(source) => return query_failed("Failed to fetch WAU analytics", &source),
    };
    let rows = match store
        .weekly_active_users(
            &request.window.start_date,
            &request.window.end_date,
            request.filter.as_ref(),
        )
        .await
    {
        Ok(rows) => rows,
        Err(source) => return query_failed("Failed to fetch WAU analytics", &source),
    };
    match rows
        .into_iter()
        .map(|row| weekly_result(&request.window, row))
        .collect::<Result<Vec<TagActiveUsersResponse>, AnalyticsError>>()
    {
        Ok(results) => Json(ActiveUsersAnalyticsResponse { results }).into_response(),
        Err(source) => query_failed("Failed to fetch WAU analytics", &source),
    }
}
