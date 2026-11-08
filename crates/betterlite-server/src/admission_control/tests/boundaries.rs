use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::loader::ConfigLoader;
use rstest::rstest;
use tokio::sync::Barrier;
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
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
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
    let waiters = 64_u16;
    let harness = harness(
        "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 1\n  admission_queue_timeout_seconds: 2.0",
        "",
        gates(&[("/a", Behavior::Hold)]),
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    let paths = waiter_paths(waiters);
    let barrier = Arc::new(Barrier::new(usize::from(waiters)));
    for path in &paths {
        requests.start_at(&harness.router, path, Arc::clone(&barrier));
    }
    await_admission_settled(&harness.stack, u64::from(waiters)).await;
    let snapshot = harness.stack.admission.stats();
    assert_eq!(snapshot.queued, 1, "{snapshot:?}");
    assert_eq!(
        snapshot.rejected_queue_full,
        u64::from(waiters).saturating_sub(1)
    );
    harness.gates.get("/a").unwrap().release.notify_one();
    let (admitted, rejected) = complete_waiters(&mut requests, &paths).await;
    assert_eq!((admitted, rejected), (1_u16, waiters.saturating_sub(1)));
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    let final_snapshot = harness.stack.admission.stats();
    assert_eq!(
        final_snapshot.rejected_queue_full,
        u64::from(waiters).saturating_sub(1)
    );
    assert_eq!(final_snapshot.rejected_queue_timeout, 0);
    assert_eq!(final_snapshot.queued, 0);
}

fn waiter_paths(waiters: u16) -> Vec<String> {
    (0..waiters)
        .map(|index| format!("/waiter-{index}"))
        .collect()
}

async fn complete_waiters(requests: &mut Requests, paths: &[String]) -> (u16, u16) {
    let mut admitted = 0_u16;
    let mut rejected = 0_u16;
    for path in paths {
        match requests.complete(path).await.0 {
            StatusCode::OK => admitted = admitted.saturating_add(1),
            StatusCode::SERVICE_UNAVAILABLE => rejected = rejected.saturating_add(1),
            other => panic!("unexpected status {other} for {path}"),
        }
    }
    (admitted, rejected)
}

async fn await_admission_settled(stack: &AdmissionStack, waiters: u64) {
    timeout(WAIT, async {
        loop {
            let snapshot = stack.admission.stats();
            if snapshot.queued.saturating_add(snapshot.rejected_queue_full) == waiters {
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
