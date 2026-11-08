use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_config::loader::ConfigLoader;
use rstest::rstest;

use super::harness::{
    Behavior, Requests, application, await_queued, await_started, gates, harness, in_flight,
    release,
};
use crate::admission_control::install;
use crate::routing::admission_control::hot_routes_first;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    QueueFullAtCapacity,
    QueueTimeoutAtTheLimit,
    CancelQueuedDecrements,
    HugeLimitDisables,
    HotRoutesFirst,
    InstallWiresTheStack,
}

#[rstest]
#[case::queue_full_at_capacity(Boundary::QueueFullAtCapacity)]
#[case::queue_timeout_at_the_limit(Boundary::QueueTimeoutAtTheLimit)]
#[case::cancel_queued_decrements(Boundary::CancelQueuedDecrements)]
#[case::huge_limit_disables(Boundary::HugeLimitDisables)]
#[case::hot_routes_first(Boundary::HotRoutesFirst)]
#[case::install_wires_the_stack(Boundary::InstallWiresTheStack)]
#[tokio::test]
async fn admission_control_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::QueueFullAtCapacity => queue_full_at_capacity().await,
        Boundary::QueueTimeoutAtTheLimit => queue_timeout_at_the_limit().await,
        Boundary::CancelQueuedDecrements => cancel_queued_decrements().await,
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
    release(&harness.gates, "/a");
    await_started(&harness.gates, "/b").await;
    assert_eq!(requests.status("/a").await, StatusCode::OK);
    assert_eq!(requests.status("/b").await, StatusCode::OK);
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
    release(&harness.gates, "/a");
    assert_eq!(requests.status("/a").await, StatusCode::OK);
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
    assert_eq!(in_flight(&harness.stack), 1);
    release(&harness.gates, "/a");
    assert_eq!(requests.status("/a").await, StatusCode::OK);
    assert_eq!(harness.stack.admission.stats().admitted, 0);
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
    assert_eq!(in_flight(&harness.stack), 2);
    release(&harness.gates, "/a");
    release(&harness.gates, "/b");
    assert_eq!(requests.status("/a").await, StatusCode::OK);
    assert_eq!(requests.status("/b").await, StatusCode::OK);
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
    let (router, gates) = application(
        &config.sections.request_root_paths,
        "",
        gates(&[("/a", Behavior::Hold)]),
        |inner| install(inner, &settings),
    );
    let mut requests = Requests::default();
    requests.start(&router, "/a");
    await_started(&gates, "/a").await;
    requests.start(&router, "/b");
    let (status, _headers, _body) = requests.complete("/b").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    release(&gates, "/a");
    assert_eq!(requests.status("/a").await, StatusCode::OK);
}
