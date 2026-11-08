mod body;
pub mod in_flight;
mod response;
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::Response;
use betterlite_config::sections::admission_control::{Limits, Settings};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::timeout;

use self::body::GuardedBody;
use self::in_flight::{InFlightRequests, track_in_flight};
use self::response::overloaded;
use crate::middleware::request_root_paths::RequestRootPath;

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

pub struct AdmissionControl {
    limits: Option<Limits>,
    semaphore: Arc<Semaphore>,
    admitted: AtomicU64,
    queued: AtomicU64,
    rejected_total: AtomicU64,
    rejected_queue_full: AtomicU64,
    rejected_queue_timeout: AtomicU64,
}

impl AdmissionControl {
    fn with_limits(limits: Option<Limits>) -> Self {
        Self {
            limits,
            semaphore: Arc::new(Semaphore::new(
                limits.map_or(0, |set| set.max_in_flight_requests),
            )),
            admitted: AtomicU64::new(0),
            queued: AtomicU64::new(0),
            rejected_total: AtomicU64::new(0),
            rejected_queue_full: AtomicU64::new(0),
            rejected_queue_timeout: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        let Some(limits) = settings.limits else {
            return Self::with_limits(None);
        };
        if limits.max_in_flight_requests > Semaphore::MAX_PERMITS {
            tracing::warn!(
                max_in_flight_requests = limits.max_in_flight_requests,
                "admission control limit exceeds the semaphore maximum; per-worker admission control is disabled"
            );
            return Self::with_limits(None);
        }
        Self::with_limits(Some(limits))
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

    fn reserve_queue_slot(&self, max_queued: u64) -> bool {
        self.queued
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                (queued < max_queued).then_some(queued.saturating_add(1))
            })
            .is_ok()
    }

    async fn admit(&self, limits: &Limits) -> Option<OwnedSemaphorePermit> {
        if let Ok(permit) = Arc::clone(&self.semaphore).try_acquire_owned() {
            return Some(permit);
        }
        let max_queued = u64::try_from(limits.max_queued_requests).unwrap_or(u64::MAX);
        if !self.reserve_queue_slot(max_queued) {
            self.rejected_queue_full.fetch_add(1, Ordering::AcqRel);
            self.rejected_total.fetch_add(1, Ordering::AcqRel);
            return None;
        }
        let slot = QueueSlot { control: self };
        let acquired = timeout(
            limits.queue_timeout,
            Arc::clone(&self.semaphore).acquire_owned(),
        )
        .await;
        drop(slot);
        if let Ok(Ok(permit)) = acquired {
            return Some(permit);
        }
        self.rejected_queue_timeout.fetch_add(1, Ordering::AcqRel);
        self.rejected_total.fetch_add(1, Ordering::AcqRel);
        None
    }
}

fn decrement(counter: &AtomicU64) {
    let _previous = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
        Some(value.saturating_sub(1))
    });
}

struct QueueSlot<'control> {
    control: &'control AdmissionControl,
}

impl Drop for QueueSlot<'_> {
    fn drop(&mut self) {
        decrement(&self.control.queued);
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
    pub scalar_root: String,
}

struct AdmissionGate {
    control: Arc<AdmissionControl>,
    scalar_root: String,
}

impl AdmissionStack {
    pub fn wrap(&self, application: Router) -> Router {
        let gate = Arc::new(AdmissionGate {
            control: Arc::clone(&self.admission),
            scalar_root: self.scalar_root.clone(),
        });
        // the root path layer must wrap this stack so an exemption check sees the route path
        application
            .layer(from_fn_with_state(
                Arc::clone(&self.in_flight),
                track_in_flight,
            ))
            .layer(from_fn_with_state(gate, admit_request))
    }
}

// uvicorn reads the last Upgrade and Connection header, and its handshake takes only a GET with a key and a version
fn is_websocket_upgrade(request: &Request<Body>) -> bool {
    let headers = request.headers();
    let upgrade = headers.get_all(header::UPGRADE).iter().next_back();
    let connection = headers.get_all(header::CONNECTION).iter().next_back();
    request.method() == Method::GET
        && upgrade.is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
        && connection.is_some_and(|value| {
            let mut tokens = value.as_bytes().split(|byte| *byte == b',');
            tokens.any(|token| token.trim_ascii().eq_ignore_ascii_case(b"upgrade"))
        })
        && headers.contains_key(header::SEC_WEBSOCKET_KEY)
        && headers.contains_key(header::SEC_WEBSOCKET_VERSION)
}

// LiteLLM strips the scope root_path itself, and a scalar SERVER_ROOT_PATH has no middleware to do it here
fn scalar_route_path<'path>(request: &'path Request<Body>, scalar_root: &str) -> &'path str {
    let path = request.uri().path();
    if scalar_root.is_empty() || request.extensions().get::<RequestRootPath>().is_some() {
        return path;
    }
    match path.strip_prefix(scalar_root) {
        Some("") => "/",
        Some(rest) if rest.starts_with('/') => rest,
        _ => path,
    }
}

async fn admit_request(
    State(gate): State<Arc<AdmissionGate>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if is_websocket_upgrade(&request) {
        return next.run(request).await;
    }
    let control = &gate.control;
    let Some(limits) = control.limits else {
        return next.run(request).await;
    };
    if EXEMPT_PATHS.contains(&scalar_route_path(&request, &gate.scalar_root)) {
        return next.run(request).await;
    }
    let Some(permit) = control.admit(&limits).await else {
        return overloaded(&control.stats());
    };
    control.admitted.fetch_add(1, Ordering::AcqRel);
    let slot = AdmissionSlot {
        control: Arc::clone(control),
        permit: Some(permit),
    };
    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        Body::new(GuardedBody {
            inner: body,
            guard: Some(slot),
        }),
    )
}
