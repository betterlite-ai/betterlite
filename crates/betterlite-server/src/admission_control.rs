mod body;
pub mod in_flight;
mod response;
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::Response;
use betterlite_config::sections::admission_control::{Limits, Settings};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::timeout;

use self::body::GuardedBody;
use self::in_flight::{InFlightRequests, track_in_flight};
use self::response::overloaded;

// LiteLLM lets health probes and metric scrapes through so a saturated worker stays observable
const EXEMPT_PATHS: [&str; 8] = [
    "/health/liveliness",
    "/health/liveness",
    "/health/readiness",
    "/health/readiness/details",
    "/health/backlog",
    "/health/drain",
    "/metrics",
    "/metrics/",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionStats {
    pub admitted: u64,
    pub queued: u64,
    pub rejected_total: u64,
    pub rejected_queue_full: u64,
    pub rejected_queue_timeout: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    QueueFull,
    QueueTimeout,
}

pub struct AdmissionControl {
    limits: Option<Limits>,
    semaphore: Arc<Semaphore>,
    admitted: AtomicU64,
    queued: AtomicU64,
    rejected_total: AtomicU64,
    rejected_queue_full: AtomicU64,
    rejected_queue_timeout: AtomicU64,
}

impl Default for AdmissionControl {
    fn default() -> Self {
        Self {
            limits: None,
            semaphore: Arc::new(Semaphore::new(0)),
            admitted: AtomicU64::new(0),
            queued: AtomicU64::new(0),
            rejected_total: AtomicU64::new(0),
            rejected_queue_full: AtomicU64::new(0),
            rejected_queue_timeout: AtomicU64::new(0),
        }
    }
}

impl AdmissionControl {
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        let Some(limits) = settings.limits else {
            return Self::default();
        };
        if limits.max_in_flight_requests > Semaphore::MAX_PERMITS {
            tracing::warn!(
                max_in_flight_requests = limits.max_in_flight_requests,
                "admission control limit exceeds the semaphore maximum; per-worker admission control is disabled"
            );
            return Self::default();
        }
        Self {
            limits: Some(limits),
            semaphore: Arc::new(Semaphore::new(limits.max_in_flight_requests)),
            admitted: AtomicU64::new(0),
            queued: AtomicU64::new(0),
            rejected_total: AtomicU64::new(0),
            rejected_queue_full: AtomicU64::new(0),
            rejected_queue_timeout: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn stats(&self) -> AdmissionStats {
        AdmissionStats {
            admitted: self.admitted.load(Ordering::Acquire),
            queued: self.queued.load(Ordering::Acquire),
            rejected_total: self.rejected_total.load(Ordering::Acquire),
            rejected_queue_full: self.rejected_queue_full.load(Ordering::Acquire),
            rejected_queue_timeout: self.rejected_queue_timeout.load(Ordering::Acquire),
        }
    }

    async fn admit(&self, limits: &Limits) -> Result<OwnedSemaphorePermit, Rejection> {
        if let Ok(permit) = Arc::clone(&self.semaphore).try_acquire_owned() {
            return Ok(permit);
        }
        let max_queued = u64::try_from(limits.max_queued_requests).unwrap_or(u64::MAX);
        if self.queued.load(Ordering::Acquire) >= max_queued {
            self.rejected_queue_full.fetch_add(1, Ordering::AcqRel);
            self.rejected_total.fetch_add(1, Ordering::AcqRel);
            return Err(Rejection::QueueFull);
        }
        self.queued.fetch_add(1, Ordering::AcqRel);
        let mut slot = QueueSlot {
            control: Some(self),
        };
        let acquired = timeout(
            limits.queue_timeout,
            Arc::clone(&self.semaphore).acquire_owned(),
        )
        .await;
        slot.finish();
        if let Ok(Ok(permit)) = acquired {
            return Ok(permit);
        }
        self.rejected_queue_timeout.fetch_add(1, Ordering::AcqRel);
        self.rejected_total.fetch_add(1, Ordering::AcqRel);
        Err(Rejection::QueueTimeout)
    }
}

fn decrement(counter: &AtomicU64) {
    let _previous = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
        Some(value.saturating_sub(1))
    });
}

struct QueueSlot<'control> {
    control: Option<&'control AdmissionControl>,
}

impl QueueSlot<'_> {
    fn finish(&mut self) {
        if let Some(control) = self.control.take() {
            decrement(&control.queued);
        }
    }
}

impl Drop for QueueSlot<'_> {
    fn drop(&mut self) {
        self.finish();
    }
}

struct AdmissionSlot {
    control: Arc<AdmissionControl>,
    permit: Option<OwnedSemaphorePermit>,
}

impl Drop for AdmissionSlot {
    fn drop(&mut self) {
        drop(self.permit.take());
        decrement(&self.control.admitted);
    }
}

pub struct AdmissionStack {
    pub admission: Arc<AdmissionControl>,
    pub in_flight: Arc<InFlightRequests>,
}

impl AdmissionStack {
    pub fn wrap(&self, application: Router) -> Router {
        // the root path layer must wrap this stack so an exemption check sees the route path
        application
            .layer(from_fn_with_state(
                Arc::clone(&self.in_flight),
                track_in_flight,
            ))
            .layer(from_fn_with_state(
                Arc::clone(&self.admission),
                admit_request,
            ))
    }
}

async fn admit_request(
    State(control): State<Arc<AdmissionControl>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(limits) = control.limits else {
        return next.run(request).await;
    };
    if EXEMPT_PATHS.contains(&request.uri().path()) {
        return next.run(request).await;
    }
    let permit = match control.admit(&limits).await {
        Ok(permit) => permit,
        Err(_rejection) => return overloaded(&control.stats()),
    };
    control.admitted.fetch_add(1, Ordering::AcqRel);
    let slot = AdmissionSlot {
        control: Arc::clone(&control),
        permit: Some(permit),
    };
    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        Body::from_stream(GuardedBody {
            inner: body.into_data_stream(),
            guard: Some(slot),
        }),
    )
}
