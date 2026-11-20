use axum::Json;
use axum::extract::Extension;
use axum::extract::RawQuery;
use axum::extract::State;
use axum::response::IntoResponse as _;
use axum::response::Response;

use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

use super::error::AnalyticsError;
use super::error::db_not_connected;
use super::error::query_failed;
use super::per_user;
use super::query::AnalyticsQuery;
use super::responses::ActiveUsersAnalyticsResponse;
use super::responses::DistinctTagResponse;
use super::responses::DistinctTagsResponse;
use super::responses::TagActiveUsersResponse;
use super::rows::ActiveUsersReport;
use super::state::UserAgentAnalyticsState;
use super::store::RangeQuery;
use super::summary;
use super::window::MAU_SPAN_DAYS;
use super::window::MAX_DAYS;
use super::window::WAU_SPAN_DAYS;
use super::window::Window;
use super::window::active_users_result;

const DISTINCT_FAILURE: &str = "Failed to fetch distinct user agent tags";
const DAU_FAILURE: &str = "Failed to fetch DAU analytics";
const WAU_FAILURE: &str = "Failed to fetch WAU analytics";
const MAU_FAILURE: &str = "Failed to fetch MAU analytics";

#[derive(Clone, Copy)]
pub(super) enum AnalyticsReport {
    Distinct,
    Daily,
    Weekly,
    Monthly,
    Summary,
    PerUser,
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
    let query = AnalyticsQuery::decode(raw_query.as_deref().unwrap_or_default());
    match report {
        AnalyticsReport::Distinct => distinct(&state).await,
        AnalyticsReport::Daily => active_users(&state, &query, ActiveUsersReport::Daily).await,
        AnalyticsReport::Weekly => active_users(&state, &query, ActiveUsersReport::Weekly).await,
        AnalyticsReport::Monthly => active_users(&state, &query, ActiveUsersReport::Monthly).await,
        AnalyticsReport::Summary => summary::tag_summary(&state, &query).await,
        AnalyticsReport::PerUser => per_user::per_user_analytics(&state, &query).await,
    }
}

async fn distinct(state: &UserAgentAnalyticsState) -> Response {
    let Some(store) = state.store.as_deref() else {
        return db_not_connected();
    };
    match store.distinct_tags().await {
        Ok(rows) => Json(DistinctTagsResponse {
            results: rows
                .into_iter()
                .map(|row| DistinctTagResponse { tag: row.tag })
                .collect(),
        })
        .into_response(),
        Err(source) => query_failed(DISTINCT_FAILURE, &source),
    }
}

async fn active_users(
    state: &UserAgentAnalyticsState,
    query: &AnalyticsQuery,
    report: ActiveUsersReport,
) -> Response {
    let (span_days, failure) = match report {
        ActiveUsersReport::Daily => (MAX_DAYS, DAU_FAILURE),
        ActiveUsersReport::Weekly => (WAU_SPAN_DAYS, WAU_FAILURE),
        ActiveUsersReport::Monthly => (MAU_SPAN_DAYS, MAU_FAILURE),
    };
    let Some(store) = state.store.as_deref() else {
        return db_not_connected();
    };
    let window = match Window::ending_tomorrow(state.clock.as_ref(), span_days) {
        Ok(window) => window,
        Err(source) => return query_failed(failure, &source),
    };
    let filter = query.filter();
    let range = RangeQuery {
        start_date: &window.start_date,
        end_date: &window.end_date,
        filter: filter.as_ref(),
    };
    let rows = match store.active_users(report, range).await {
        Ok(rows) => rows,
        Err(source) => return query_failed(failure, &source),
    };
    let results = match rows
        .into_iter()
        .map(|row| active_users_result(&window, report, row))
        .collect::<Result<Vec<TagActiveUsersResponse>, AnalyticsError>>()
    {
        Ok(results) => results,
        Err(source) => return query_failed(failure, &source),
    };
    Json(ActiveUsersAnalyticsResponse { results }).into_response()
}
