mod error;
mod query;
mod series;
mod sql;
pub mod store;
mod tests;
mod wire;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::extract::RawQuery;
use axum::extract::State;
use axum::response::IntoResponse as _;
use axum::response::Response;
use axum::routing::get;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use num_traits::ToPrimitive as _;

use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

use self::error::ModelMetricsError;
use self::query::MetricsQuery;
use self::query::Window;
use self::query::normalized;
use self::series::SeriesResponse;
use self::store::ExceptionWindow;
use self::store::LatencyRequest;
use self::store::ModelMetricsStore;
use self::store::SlowRequest;
use self::store::StreamingWindow;

pub const STREAMING_METRICS_PATH: &str = "/model/streaming_metrics";
pub const METRICS_PATH: &str = "/model/metrics";
pub const SLOW_RESPONSES_PATH: &str = "/model/metrics/slow_responses";
pub const EXCEPTIONS_PATH: &str = "/model/metrics/exceptions";

const STREAMING_LOOKBACK_DAYS: u64 = 7;
const METRICS_LOOKBACK_DAYS: u64 = 28;
const DEFAULT_MODEL_GROUP: &str = "gpt-4-32k";

#[derive(Clone)]
pub struct ModelMetricsState {
    pub store: Option<Arc<dyn ModelMetricsStore>>,
    pub clock: Arc<dyn GatewayRequestClock>,
    pub alerting_threshold_seconds: f64,
}

pub fn routes(state: ModelMetricsState) -> Router {
    Router::new()
        .route(STREAMING_METRICS_PATH, get(streaming_metrics))
        .route(METRICS_PATH, get(model_metrics))
        .route(SLOW_RESPONSES_PATH, get(slow_responses))
        .route(EXCEPTIONS_PATH, get(model_metrics_exceptions))
        .with_state(state)
}

async fn streaming_metrics(
    State(state): State<ModelMetricsState>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let requested = MetricsQuery::decode(raw_query.as_deref());
    let window = match requested.window(state.clock.as_ref(), STREAMING_LOOKBACK_DAYS) {
        Ok(window) => window,
        Err(error) => return error.into_response(),
    };
    let Some(store) = state.store.as_deref() else {
        return ModelMetricsError::StreamingDatabaseMissing.into_response();
    };
    match streaming_series(store, requested.model_group.clone(), window).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn streaming_series(
    store: &dyn ModelMetricsStore,
    model_group: Option<String>,
    window: Window,
) -> Result<SeriesResponse, ModelMetricsError> {
    let request = StreamingWindow {
        model_group,
        start: window.start,
        end: window.end,
    };
    if window.start.date_naive() == window.end.date_naive() {
        let rows = store
            .streaming_same_day(&request)
            .await
            .map_err(ModelMetricsError::Store)?;
        return Ok(series::same_day(&rows));
    }
    let rows = store
        .streaming_daily(&request)
        .await
        .map_err(ModelMetricsError::Store)?;
    Ok(series::daily(&rows))
}

async fn model_metrics(
    State(state): State<ModelMetricsState>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let requested = MetricsQuery::decode(raw_query.as_deref());
    let window = match requested.window(state.clock.as_ref(), METRICS_LOOKBACK_DAYS) {
        Ok(window) => window,
        Err(error) => return error.into_response(),
    };
    let Some(store) = state.store.as_deref() else {
        return ModelMetricsError::DatabaseMissing.into_response();
    };
    let request = LatencyRequest {
        model_group: requested.model_group_or(DEFAULT_MODEL_GROUP),
        start: window.start,
        end: window.end,
        api_key: normalized(requested.api_key.as_deref()),
        customer: normalized(requested.customer.as_deref()),
    };
    match store.latency(&request).await {
        Ok(rows) => Json(series::latency(&rows)).into_response(),
        Err(source) => ModelMetricsError::Store(source).into_response(),
    }
}

async fn slow_responses(
    State(state): State<ModelMetricsState>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let requested = MetricsQuery::decode(raw_query.as_deref());
    let window = match requested.window(state.clock.as_ref(), METRICS_LOOKBACK_DAYS) {
        Ok(window) => window,
        Err(error) => return error.into_response(),
    };
    let Some(store) = state.store.as_deref() else {
        return ModelMetricsError::DatabaseMissing.into_response();
    };
    let Some(alerting_threshold_seconds) = state.alerting_threshold_seconds.to_i64() else {
        return ModelMetricsError::AlertingThreshold.into_response();
    };
    let request = SlowRequest {
        model_group: requested.model_group_or(DEFAULT_MODEL_GROUP),
        start: window.start,
        end: window.end,
        api_key: normalized(requested.api_key.as_deref()),
        customer: normalized(requested.customer.as_deref()),
        alerting_threshold_seconds,
    };
    match store.slow_responses(&request).await {
        Ok(rows) => Json(series::slow(&rows)).into_response(),
        Err(source) => ModelMetricsError::Store(source).into_response(),
    }
}

async fn model_metrics_exceptions(
    State(state): State<ModelMetricsState>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let requested = MetricsQuery::decode(raw_query.as_deref());
    let window = match requested.window(state.clock.as_ref(), METRICS_LOOKBACK_DAYS) {
        Ok(window) => window,
        Err(error) => return error.into_response(),
    };
    let Some(store) = state.store.as_deref() else {
        return ModelMetricsError::DatabaseMissing.into_response();
    };
    let request = ExceptionWindow {
        model_group: requested.model_group.clone(),
        start: window.start,
        end: window.end,
    };
    match store.exceptions(&request).await {
        Ok(rows) => Json(series::exceptions(&rows)).into_response(),
        Err(source) => ModelMetricsError::Store(source).into_response(),
    }
}
