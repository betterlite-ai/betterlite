mod classify_a2a;
mod classify_billable;
mod classify_llm;
mod classify_mcp;
mod extract_model;
mod failure_handling;
mod lazy_resolution;
mod pass_through;
pub mod recorder;
pub mod sink;
mod status_code;
mod tests;

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use self::classify_billable::classify_billable_request;
use self::extract_model::extract_model_id;
use self::lazy_resolution::LazyResolve;
use self::recorder::BillingRecorder;
use self::sink::GatewayRequestSink;

pub type SinkFactory = Box<dyn Fn() -> Option<Arc<dyn GatewayRequestSink>> + Send + Sync>;
pub type RecorderFactory = Box<dyn Fn() -> Option<Arc<dyn BillingRecorder>> + Send + Sync>;

// No sink or recorder is a transparent pass-through.
pub struct BillableRequestMetrics {
    sink: LazyResolve<dyn GatewayRequestSink>,
    recorder: LazyResolve<dyn BillingRecorder>,
}

impl BillableRequestMetrics {
    #[must_use]
    pub fn new(
        sink_factory: Option<SinkFactory>,
        recorder_factory: Option<RecorderFactory>,
    ) -> Self {
        Self {
            sink: LazyResolve::new(sink_factory),
            recorder: LazyResolve::new(recorder_factory),
        }
    }
}

pub async fn record_billable_request(
    State(state): State<Arc<BillableRequestMetrics>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let resolved_sink = state.sink.get();
    let resolved_recorder = state.recorder.get();
    if resolved_sink.is_none() && resolved_recorder.is_none() {
        return next.run(request).await;
    }

    let Some((category, route)) = classify_billable_request(request.uri().path(), request.method())
    else {
        return next.run(request).await;
    };

    let response = next.run(request).await;
    let status_code = response.status();

    if let Some(sink) = resolved_sink
        && let Err(source) = sink.record(category, route, status_code)
    {
        tracing::warn!(%source, route, "gateway request metering failed");
    }

    if status_code.is_success()
        && let Some(recorder) = resolved_recorder
    {
        let model_id = extract_model_id(response.headers());
        if let Err(source) = recorder.record(category, route, status_code, model_id.as_deref()) {
            tracing::warn!(%source, route, "billable request metering failed");
        }
    }

    response
}
