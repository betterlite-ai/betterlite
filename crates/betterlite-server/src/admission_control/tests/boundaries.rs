use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::loader::ConfigLoader;
use rstest::rstest;
use tokio::time::{sleep, timeout};

use super::harness::{
    Behavior, Requests, WAIT, application, await_queued, await_started, gates, harness,
};
use crate::admission_control::{AdmissionControl, AdmissionStack, InFlightRequests};
use crate::routing::admission_control::hot_routes_first;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    QueueFullAtCapacity,
    QueueTimeoutAtTheLimit,
    CancelQueuedDecrements,
    QueueBoundUnderConcurrency,
    HugeLimitDisables,
    HotRoutesFirst,
    InstallWiresTheStack,
}

#[rstest]
#[case::queue_full_at_capacity(Boundary::QueueFullAtCapacity)]
#[case::queue_timeout_at_the_limit(Boundary::QueueTimeoutAtTheLimit)]
#[case::cancel_queued_decrements(Boundary::CancelQueuedDecrements)]
#[case::queue_bound_under_concurrency(Boundary::QueueBoundUnderConcurrency)]
#[case::huge_limit_disables(Boundary::HugeLimitDisables)]
#[case::hot_routes_first(Boundary::HotRoutesFirst)]
#[case::install_wires_the_stack(Boundary::InstallWiresTheStack)]
#[tokio::test]
async fn admission_control_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::QueueFullAtCapacity => queue_full_at_capacity().await,
        Boundary::QueueTimeoutAtTheLimit => queue_timeout_at_the_limit().await,
        Boundary::CancelQueuedDecrements => cancel_queued_decrements().await,
        Boundary::QueueBoundUnderConcurrency => queue_bound_holds_under_concurrency().await,
        Boundary::HugeLimitDisables => huge_limit_disables().await,
        Boundary::HotRoutesFirst => hot_routes_first_audit(),
        Boundary::InstallWiresTheStack => install_wires_the_stack().await,
    }
}

async fn queue_full_at_capacity() {
    let gates = gates(&[
        ("/a", Behavior::Hold),
        ("/b", Behavior::Immediate),
        ("/c", Behavior::Immediate),
    ]);
    let harness = harness(
        "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 1",
        "",
        gates,
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    await_queued(&harness.stack, 1).await;
    requests.start(&harness.router, "/c");
    let (status, headers, _body) = requests.complete("/c").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers.get("retry-after").unwrap(), "1");
    let snapshot = harness.stack.admission.stats();
    assert_eq!(snapshot.queued, 1);
    assert_eq!(snapshot.rejected_queue_full, 1);
    assert_eq!(snapshot.rejected_queue_timeout, 0);
    harness.gates.get("/a").unwrap().release.notify_one();
    await_started(&harness.gates, "/b").await;
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
    assert_eq!(harness.stack.admission.stats().queued, 0);
}

async fn queue_timeout_at_the_limit() {
    let gates = gates(&[("/a", Behavior::Hold), ("/b", Behavior::Immediate)]);
    let harness = harness(
        "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 1\n  admission_queue_timeout_seconds: 0.000001",
        "",
        gates,
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    let (status, _headers, _body) = requests.complete("/b").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(harness.stack.admission.stats().rejected_queue_timeout, 1);
    assert_eq!(harness.stack.admission.stats().queued, 0);
    harness.gates.get("/a").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
}

async fn cancel_queued_decrements() {
    let gates = gates(&[("/a", Behavior::Hold), ("/b", Behavior::Immediate)]);
    let harness = harness(
        "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 1",
        "",
        gates,
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    await_queued(&harness.stack, 1).await;
    requests.cancel("/b").await;
    let snapshot = harness.stack.admission.stats();
    assert_eq!(
        (snapshot.admitted, snapshot.queued, snapshot.rejected_total),
        (1, 0, 0)
    );
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 1);
    harness.gates.get("/a").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(harness.stack.admission.stats().admitted, 0);
}

async fn queue_bound_holds_under_concurrency() {
    let gates = gates(&[
        ("/a", Behavior::Hold),
        ("/b", Behavior::Immediate),
        ("/c", Behavior::Immediate),
        ("/d", Behavior::Immediate),
    ]);
    let harness = harness(
        "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 1\n  admission_queue_timeout_seconds: 2.0",
        "",
        gates,
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    requests.start(&harness.router, "/c");
    requests.start(&harness.router, "/d");
    await_one_queued_and_two_rejected(&harness.stack).await;
    harness.gates.get("/a").unwrap().release.notify_one();
    let statuses = [
        requests.complete("/b").await.0,
        requests.complete("/c").await.0,
        requests.complete("/d").await.0,
    ];
    let admitted = statuses
        .iter()
        .filter(|status| **status == StatusCode::OK)
        .count();
    let rejected = statuses
        .iter()
        .filter(|status| **status == StatusCode::SERVICE_UNAVAILABLE)
        .count();
    assert_eq!((admitted, rejected), (1, 2), "{statuses:?}");
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    let snapshot = harness.stack.admission.stats();
    assert_eq!(snapshot.rejected_queue_full, 2);
    assert_eq!(snapshot.rejected_queue_timeout, 0);
    assert_eq!(snapshot.queued, 0);
}

async fn await_one_queued_and_two_rejected(stack: &AdmissionStack) {
    timeout(WAIT, async {
        loop {
            let snapshot = stack.admission.stats();
            if snapshot.queued == 1 && snapshot.rejected_queue_full == 2 {
                break;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}

async fn huge_limit_disables() {
    let gates = gates(&[("/a", Behavior::Hold), ("/b", Behavior::Hold)]);
    let harness = harness(
        "general_settings:\n  max_in_flight_requests_per_worker: 9223372036854775807",
        "",
        gates,
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    await_started(&harness.gates, "/b").await;
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 2);
    harness.gates.get("/a").unwrap().release.notify_one();
    harness.gates.get("/b").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
}

fn hot_routes_first_audit() {
    let ordered = hot_routes_first([
        "/cold-b",
        "/v1/chat/completions",
        "/cold-a",
        "/metrics",
        "/health/liveliness",
        "/v1/messages",
    ]);
    assert_eq!(
        ordered,
        vec![
            "/v1/chat/completions",
            "/health/liveliness",
            "/v1/messages",
            "/cold-b",
            "/cold-a",
            "/metrics",
        ]
    );
}

async fn install_wires_the_stack() {
    let source = "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 0";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap();
    let settings = config.sections.admission_control;
    let stack = AdmissionStack {
        admission: Arc::new(AdmissionControl::from_settings(&settings)),
        in_flight: Arc::new(InFlightRequests::default()),
    };
    let (router, gates) = application(
        &config.sections.request_root_paths,
        "",
        gates(&[("/a", Behavior::Hold)]),
        |inner| stack.wrap(inner),
    );
    let mut requests = Requests::default();
    requests.start(&router, "/a");
    await_started(&gates, "/a").await;
    requests.start(&router, "/b");
    let (status, _headers, _body) = requests.complete("/b").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    gates.get("/a").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
}
