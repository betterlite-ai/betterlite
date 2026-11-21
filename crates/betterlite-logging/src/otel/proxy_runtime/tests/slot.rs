#![cfg(test)]

use std::sync::Arc;

use opentelemetry::Context;
use tokio::task::JoinSet;
use tokio::task::yield_now;

use crate::otel::config::destination::OtelDestination;
use crate::otel::logger_runtime::tests::support::destination;
use crate::otel::plumbing::context::context_slots::AnchoredSpan;
use crate::otel::proxy_runtime::slot::{
    ACTIVE_REQUEST_SLOT, RequestSlot, anchor_request_root, enter_request_slot,
    enter_request_slot_sync, request_slot_scope, set_destinations,
};

use super::support::anchored;

fn active_slot() -> Option<Arc<RequestSlot>> {
    ACTIVE_REQUEST_SLOT.try_with(Arc::clone).ok()
}

async fn request_that_anchors(
    trace: u8,
    span: u8,
    destination_name: &'static str,
) -> (Option<AnchoredSpan>, Vec<OtelDestination>) {
    request_slot_scope(async move {
        anchor_request_root(anchored(trace, span), Context::new());
        set_destinations(vec![destination(destination_name)]);
        yield_now().await;
        let slot = active_slot().expect("the request task holds its slot");
        let held = slot.trace();
        (held.root.clone(), held.destinations.clone())
    })
    .await
    .1
}

#[tokio::test]
async fn concurrent_requests_never_see_each_others_anchor_or_destinations() {
    let mut tasks = JoinSet::new();
    tasks.spawn(request_that_anchors(1, 1, "first"));
    tasks.spawn(request_that_anchors(2, 2, "second"));
    let finished = tasks.join_all().await;
    let first = finished
        .iter()
        .find(|seen| seen.1 == vec![destination("first")])
        .expect("the first request recorded its own destinations");
    assert_eq!(first.0, Some(anchored(1, 1)));
    let second = finished
        .iter()
        .find(|seen| seen.1 == vec![destination("second")])
        .expect("the second request recorded its own destinations");
    assert_eq!(second.0, Some(anchored(2, 2)));
}

#[tokio::test]
async fn a_nested_scope_joins_the_outer_slot() {
    request_slot_scope(async {
        let outer = active_slot().expect("the request task holds its slot");
        anchor_request_root(anchored(3, 3), Context::new());
        let (inner, ()) = request_slot_scope(async {}).await;
        assert!(Arc::ptr_eq(&inner, &outer));
        assert_eq!(inner.trace().root, Some(anchored(3, 3)));
    })
    .await;
}

#[tokio::test]
async fn a_future_entered_with_the_captured_slot_sees_it() {
    let (slot, ()) = request_slot_scope(async {}).await;
    let mut tasks = JoinSet::new();
    let captured = Arc::clone(&slot);
    tasks.spawn(async move {
        let seen = enter_request_slot(Some(captured), async { active_slot() }).await;
        ("entered", seen)
    });
    tasks.spawn(async { ("spawned", active_slot()) });
    let finished = tasks.join_all().await;
    let entered = finished
        .iter()
        .find(|seen| seen.0 == "entered")
        .and_then(|seen| seen.1.as_ref().map(Arc::clone));
    let inherited = finished
        .iter()
        .find(|seen| seen.0 == "spawned")
        .and_then(|seen| seen.1.as_ref().map(Arc::clone));
    assert!(entered.is_some_and(|held| Arc::ptr_eq(&held, &slot)));
    assert!(
        inherited.is_none(),
        "a spawned task does not inherit the task_local slot"
    );
    let sync = enter_request_slot_sync(Some(Arc::clone(&slot)), active_slot);
    assert!(sync.is_some_and(|held| Arc::ptr_eq(&held, &slot)));
    assert!(
        enter_request_slot(None, async { active_slot() })
            .await
            .is_none()
    );
    assert!(enter_request_slot_sync(None, active_slot).is_none());
}

#[tokio::test]
async fn code_outside_any_slot_reads_the_empty_context() {
    assert!(active_slot().is_none());
    set_destinations(vec![destination("nowhere")]);
    anchor_request_root(anchored(4, 4), Context::new());
    assert!(active_slot().is_none());
    request_slot_scope(async {
        let slot = active_slot().expect("a fresh scope holds its slot");
        let held = slot.trace();
        let root = held.root.clone();
        let destinations = held.destinations.clone();
        drop(held);
        assert!(root.is_none());
        assert!(destinations.is_empty());
    })
    .await;
}
