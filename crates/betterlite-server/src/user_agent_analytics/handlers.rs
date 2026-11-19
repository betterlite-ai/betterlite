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
use super::rows::ActiveUsersReport;
use super::state::UserAgentAnalyticsState;
use super::store::UserAgentAnalyticsStore;
use super::window::MAX_DAYS;
use super::window::WAU_SPAN_DAYS;
use super::window::Window;
use super::window::active_users_result;

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
        AnalyticsReport::Daily => {
            active_users(&state, store, raw_query, ActiveUsersReport::Daily).await
        }
        AnalyticsReport::Weekly => {
            active_users(&state, store, raw_query, ActiveUsersReport::Weekly).await
        }
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

async fn active_users(
    state: &UserAgentAnalyticsState,
    store: &dyn UserAgentAnalyticsStore,
    raw_query: &str,
    report: ActiveUsersReport,
) -> Response {
    let (span_days, failure) = match report {
        ActiveUsersReport::Daily => (MAX_DAYS, "Failed to fetch DAU analytics"),
        ActiveUsersReport::Weekly => (WAU_SPAN_DAYS, "Failed to fetch WAU analytics"),
    };
    let request = match active_users_request(state.clock.as_ref(), raw_query, span_days) {
        Ok(request) => request,
        Err(source) => return query_failed(failure, &source),
    };
    let rows = match store
        .active_users(
            report,
            &request.window.start_date,
            &request.window.end_date,
            request.filter.as_ref(),
        )
        .await
    {
        Ok(rows) => rows,
        Err(source) => return query_failed(failure, &source),
    };
    let results = match rows
        .into_iter()
        .map(|row| active_users_result(&request.window, report, row))
        .collect::<Result<Vec<TagActiveUsersResponse>, AnalyticsError>>()
    {
        Ok(results) => results,
        Err(source) => return query_failed(failure, &source),
    };
    Json(ActiveUsersAnalyticsResponse { results }).into_response()
}
