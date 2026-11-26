use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use betterlite_prometheus::collector_factory::collector::{Collector as _, MetricValue};
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::PrometheusMetric;
use rust_decimal::Decimal;

use super::body::GuardedBody;
use super::metrics::Registration;

#[derive(Debug, Clone, Copy)]
pub struct InFlightStats {
    pub current: u64,
}

#[derive(Debug)]
pub struct InFlightRequests {
    current: AtomicU64,
    registry: Arc<CollectorRegistry>,
    gauge: Mutex<Registration<PrometheusMetric>>,
}

impl InFlightRequests {
    #[must_use]
    pub const fn new(registry: Arc<CollectorRegistry>) -> Self {
        Self {
            current: AtomicU64::new(0),
            registry,
            gauge: Mutex::new(Registration::empty()),
        }
    }

    #[must_use]
    pub fn stats(&self) -> InFlightStats {
        InFlightStats {
            current: self.current.load(Ordering::Acquire),
        }
    }

    // LiteLLM builds the gauge on the first HTTP request and never retries a refused build
    fn gauge(&self) -> Option<PrometheusMetric> {
        self.gauge
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .once(|| super::metrics::in_flight_gauge(&self.registry))
    }
}

struct InFlightGuard {
    requests: Arc<InFlightRequests>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        super::decrement(&self.requests.current);
        if let Some(gauge) = self.requests.gauge() {
            gauge.inc(MetricValue::Finite(Decimal::NEGATIVE_ONE));
        }
    }
}

pub(super) async fn track_in_flight(
    State(requests): State<Arc<InFlightRequests>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if super::is_websocket_upgrade(&request) {
        return next.run(request).await;
    }
    requests.current.fetch_add(1, Ordering::AcqRel);
    if let Some(gauge) = requests.gauge() {
        gauge.inc(MetricValue::Finite(Decimal::ONE));
    }
    let guard = InFlightGuard {
        requests: Arc::clone(&requests),
    };
    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        Body::new(GuardedBody {
            inner: body,
            guard: Some(guard),
        }),
    )
}
