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
use self::store::ExceptionWindow;
use self::store::ModelMetricsStore;
use self::store::SlowRequest;
use self::store::SpendFilters;
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

#[derive(Clone, Copy)]
enum ModelMetricsReport {
    Streaming,
    Latency,
    Slow,
    Exceptions,
}

enum ModelMetricsRequest {
    Streaming(StreamingWindow),
    Latency(SpendFilters),
    Slow(SlowRequest),
    Exceptions(ExceptionWindow),
}

impl ModelMetricsReport {
    const fn lookback_days(self) -> u64 {
        match self {
            Self::Streaming => STREAMING_LOOKBACK_DAYS,
            Self::Latency | Self::Slow | Self::Exceptions => METRICS_LOOKBACK_DAYS,
        }
    }

    const fn missing_store(self) -> ModelMetricsError {
        match self {
            Self::Streaming => ModelMetricsError::StreamingDatabaseMissing,
            Self::Latency | Self::Slow | Self::Exceptions => ModelMetricsError::DatabaseMissing,
        }
    }

    fn read_request(
        self,
        requested: &MetricsQuery,
        window: Window,
        alerting_threshold_seconds: f64,
    ) -> Result<ModelMetricsRequest, ModelMetricsError> {
        Ok(match self {
            Self::Streaming => ModelMetricsRequest::Streaming(StreamingWindow {
                model_group: requested.model_group.clone(),
                start: window.start,
                end: window.end,
            }),
            Self::Latency => ModelMetricsRequest::Latency(SpendFilters {
                model_group: requested.model_group_or(DEFAULT_MODEL_GROUP),
                start: window.start,
                end: window.end,
                api_key: normalized(requested.api_key.as_deref()),
                customer: normalized(requested.customer.as_deref()),
            }),
            Self::Slow => ModelMetricsRequest::Slow(SlowRequest {
                filters: SpendFilters {
                    model_group: requested.model_group_or(DEFAULT_MODEL_GROUP),
                    start: window.start,
                    end: window.end,
                    api_key: normalized(requested.api_key.as_deref()),
                    customer: normalized(requested.customer.as_deref()),
                },
                threshold_seconds: alerting_threshold_seconds
                    .to_i64()
                    .ok_or(ModelMetricsError::AlertingThreshold)?,
            }),
            Self::Exceptions => ModelMetricsRequest::Exceptions(ExceptionWindow {
                model_group: requested.model_group.clone(),
                start: window.start,
                end: window.end,
            }),
        })
    }
}

pub fn routes(state: ModelMetricsState) -> Router {
    Router::new()
        .route(
            STREAMING_METRICS_PATH,
            get(respond).layer(Extension(ModelMetricsReport::Streaming)),
        )
        .route(
            METRICS_PATH,
            get(respond).layer(Extension(ModelMetricsReport::Latency)),
        )
        .route(
            SLOW_RESPONSES_PATH,
            get(respond).layer(Extension(ModelMetricsReport::Slow)),
        )
        .route(
            EXCEPTIONS_PATH,
            get(respond).layer(Extension(ModelMetricsReport::Exceptions)),
        )
        .with_state(state)
}

async fn respond(
    State(state): State<ModelMetricsState>,
    Extension(report): Extension<ModelMetricsReport>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let requested = MetricsQuery::decode(raw_query.as_deref());
    let window = match requested.window(state.clock.as_ref(), report.lookback_days()) {
        Ok(window) => window,
        Err(error) => return error.into_response(),
    };
    let Some(store) = state.store.as_deref() else {
        return report.missing_store().into_response();
    };
    let request = match report.read_request(&requested, window, state.alerting_threshold_seconds) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    match read_response(store, &request).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn read_response(
    store: &dyn ModelMetricsStore,
    request: &ModelMetricsRequest,
) -> Result<Response, ModelMetricsError> {
    match request {
        ModelMetricsRequest::Streaming(streaming) => {
            let rows = if streaming.start.date_naive() == streaming.end.date_naive() {
                store.streaming_same_day(streaming).await
            } else {
                store.streaming_daily(streaming).await
            }
            .map_err(ModelMetricsError::Store)?;
            Ok(Json(series::response(&rows)).into_response())
        }
        ModelMetricsRequest::Latency(latency) => {
            let rows = store
                .latency(latency)
                .await
                .map_err(ModelMetricsError::Store)?;
            Ok(Json(series::response(&rows)).into_response())
        }
        ModelMetricsRequest::Slow(slow) => {
            let rows = store
                .slow_responses(slow)
                .await
                .map_err(ModelMetricsError::Store)?;
            Ok(Json(series::slow(&rows)).into_response())
        }
        ModelMetricsRequest::Exceptions(exceptions) => {
            let rows = store
                .exceptions(exceptions)
                .await
                .map_err(ModelMetricsError::Store)?;
            Ok(Json(series::exceptions(&rows)).into_response())
        }
    }
}
