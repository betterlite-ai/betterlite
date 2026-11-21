#![expect(
    clippy::disallowed_types,
    reason = "tokio's task_local macro keeps its slot in a thread-local RefCell; that is the runtime's storage, not a borrow-checked value of ours"
)]

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

use opentelemetry::Context;
use tokio::task_local;

use crate::otel::config::destination::OtelDestination;
use crate::otel::plumbing::context::context_slots::{AnchoredSpan, RequestTraceContext};

task_local! {
    pub(crate) static ACTIVE_REQUEST_SLOT: Arc<RequestSlot>;
}

// litellm.integrations.otel.plumbing.context: the root span and tenant destinations a request task carries into the tasks it hands work to
#[derive(Default)]
pub struct RequestSlot {
    trace: Mutex<RequestTraceContext>,
    live_root: Mutex<Context>,
}

impl RequestSlot {
    pub(crate) fn trace(&self) -> MutexGuard<'_, RequestTraceContext> {
        match self.trace.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(crate) fn live_root(&self) -> MutexGuard<'_, Context> {
        match self.live_root.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

// litellm.integrations.otel.plumbing.context.set_request_root_span: an invalid anchor never replaces a good one, and the live Context that carries the span follows only when the anchor took
pub fn anchor_request_root(span: AnchoredSpan, live: Context) {
    if !span.context.is_valid() {
        return;
    }
    if let Some(slot) = ACTIVE_REQUEST_SLOT.try_with(Arc::clone).ok() {
        slot.trace().anchor_request_root(span);
        *slot.live_root() = live;
    }
}

// litellm.integrations.otel.plumbing.context.set_request_destinations
pub fn set_destinations(destinations: Vec<OtelDestination>) {
    if let Some(slot) = ACTIVE_REQUEST_SLOT.try_with(Arc::clone).ok() {
        slot.trace().destinations = destinations;
    }
}

// litellm.integrations.otel.plumbing.context's ContextVars ride the request task; a nested scope joins the outer slot the way betterlite-redis's batch scope does
pub async fn request_slot_scope<F>(future: F) -> (Arc<RequestSlot>, F::Output)
where
    F: Future,
{
    if let Ok(outer) = ACTIVE_REQUEST_SLOT.try_with(Arc::clone) {
        return (outer, future.await);
    }
    let slot = Arc::new(RequestSlot::default());
    let output = enter_request_slot(Some(Arc::clone(&slot)), future).await;
    (slot, output)
}

// a task_local does not follow tokio::spawn, so the handlers re-enter the slot LoggingState captured; no slot runs the future exactly as an SDK call would
pub async fn enter_request_slot<F>(slot: Option<Arc<RequestSlot>>, future: F) -> F::Output
where
    F: Future,
{
    match slot {
        Some(active) => ACTIVE_REQUEST_SLOT.scope(active, future).await,
        None => future.await,
    }
}

// the sync handlers' half of enter_request_slot, through the task_local's sync_scope
pub fn enter_request_slot_sync<T, F>(slot: Option<Arc<RequestSlot>>, body: F) -> T
where
    F: FnOnce() -> T,
{
    match slot {
        Some(active) => ACTIVE_REQUEST_SLOT.sync_scope(active, body),
        None => body(),
    }
}
