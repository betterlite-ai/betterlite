use std::fs::read_to_string;
use std::str::from_utf8;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::sections::admission_control::{Limits, Settings};
use rstest::rstest;
use tempfile::NamedTempFile;
use tokio::sync::Semaphore;
use tracing::subscriber::set_default;

use super::harness::{Behavior, Harness, Requests, await_queued, await_started, gates, harness};
use crate::admission_control::AdmissionControl;

const ENABLED: &str = "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 0";
const ONE_QUEUED: &str = "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    Disabled,
    Immediate,
    BoundedQueue,
    QueueFull,
    QueueTimeout,
    HealthExempt,
    RootPathBoundary,
    ScalarRootExempt,
    StreamHoldsSlot,
    DisconnectReleases,
    InvalidSettings,
    OmittedQueue,
    DefaultTimeout,
}

#[rstest]
#[case::disabled(Scenario::Disabled)]
#[case::immediate(Scenario::Immediate)]
#[case::bounded_queue(Scenario::BoundedQueue)]
#[case::queue_full(Scenario::QueueFull)]
#[case::queue_timeout(Scenario::QueueTimeout)]
#[case::health_exempt(Scenario::HealthExempt)]
#[case::root_path_boundary(Scenario::RootPathBoundary)]
#[case::scalar_root_exempt(Scenario::ScalarRootExempt)]
#[case::stream_holds_slot(Scenario::StreamHoldsSlot)]
#[case::disconnect_releases(Scenario::DisconnectReleases)]
#[case::invalid_settings(Scenario::InvalidSettings)]
#[case::omitted_queue(Scenario::OmittedQueue)]
#[case::default_timeout(Scenario::DefaultTimeout)]
#[tokio::test]
async fn admission_control_behaviors(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Disabled => disabled_admission().await,
        Scenario::Immediate => immediate_admission().await,
        Scenario::BoundedQueue => bounded_queue().await,
        Scenario::QueueFull => queue_full().await,
        Scenario::QueueTimeout => queue_timeout().await,
        Scenario::HealthExempt => health_exempt().await,
        Scenario::RootPathBoundary => root_path_boundary().await,
        Scenario::ScalarRootExempt => scalar_root_exempt().await,
        Scenario::StreamHoldsSlot => stream_holds_slot().await,
        Scenario::DisconnectReleases => disconnect_releases().await,
        Scenario::InvalidSettings => invalid_settings().await,
        Scenario::OmittedQueue => omitted_queue().await,
        Scenario::DefaultTimeout => default_timeout().await,
    }
}

async fn start_and_wait(harness: &Harness, requests: &mut Requests, path: &str) -> StatusCode {
    requests.start(&harness.router, path);
    requests.complete(path).await.0
}

#[test]
fn semaphore_maximum_is_the_last_usable_limit() {
    assert!(!exceeds_semaphore_maximum(Semaphore::MAX_PERMITS));
    assert!(exceeds_semaphore_maximum(
        Semaphore::MAX_PERMITS.saturating_add(1)
    ));
}

fn exceeds_semaphore_maximum(max_in_flight_requests: usize) -> bool {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    let _control = AdmissionControl::from_settings(&Settings {
        limits: Some(Limits {
            max_in_flight_requests,
            max_queued_requests: 0,
            queue_timeout: Duration::from_secs(1),
        }),
    });
    drop(guard);
    read_to_string(log.path())
        .unwrap()
        .contains("exceeds the semaphore maximum")
}

#[test]
fn absent_settings_do_not_warn() {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    let _harness = harness("general_settings: {}", "", gates(&[]));
    drop(guard);
    assert!(
        !read_to_string(log.path())
            .unwrap()
            .contains("invalid admission control setting")
    );
}

async fn disabled_admission() {
    let gates = gates(&[("/a", Behavior::Hold), ("/b", Behavior::Hold)]);
    let harness = harness("general_settings: {}", "", gates);
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    await_started(&harness.gates, "/b").await;
    let snapshot = harness.stack.admission.stats();
    assert_eq!(
        (snapshot.admitted, snapshot.queued, snapshot.rejected_total),
        (0, 0, 0)
    );
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 2);
    harness.gates.get("/a").unwrap().release.notify_one();
    harness.gates.get("/b").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 0);
}

async fn immediate_admission() {
    let gates = gates(&[("/only", Behavior::Immediate)]);
    let harness = harness(ENABLED, "", gates);
    let mut requests = Requests::default();
    requests.start(&harness.router, "/only");
    await_started(&harness.gates, "/only").await;
    let (status, _, body) = requests.complete("/only").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), b"ok");
    let snapshot = harness.stack.admission.stats();
    assert_eq!(
        (snapshot.admitted, snapshot.queued, snapshot.rejected_total),
        (0, 0, 0)
    );
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 0);
}

async fn bounded_queue() {
    let gates = gates(&[("/a", Behavior::Hold), ("/b", Behavior::Immediate)]);
    let harness = harness(ONE_QUEUED, "", gates);
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    await_queued(&harness.stack, 1).await;
    let snapshot = harness.stack.admission.stats();
    assert_eq!(
        (snapshot.admitted, snapshot.queued, snapshot.rejected_total),
        (1, 1, 0)
    );
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 1);
    harness.gates.get("/a").unwrap().release.notify_one();
    await_started(&harness.gates, "/b").await;
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
    let final_snapshot = harness.stack.admission.stats();
    assert_eq!(
        (
            final_snapshot.admitted,
            final_snapshot.queued,
            final_snapshot.rejected_total
        ),
        (0, 0, 0)
    );
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 0);
}

async fn queue_full() {
    let gates = gates(&[("/a", Behavior::Hold), ("/b", Behavior::Immediate)]);
    let harness = harness(ENABLED, "", gates);
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    let (status, headers, body) = requests.complete("/b").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers.get("retry-after").unwrap(), "1");
    assert_eq!(headers.get("content-type").unwrap(), "application/json");
    assert_eq!(
        from_utf8(&body).unwrap(),
        r#"{"error":{"message":"Worker at capacity: 1 in-flight, 0 queued requests. Retry later.","type":"overloaded_error","code":"503"}}"#
    );
    let snapshot = harness.stack.admission.stats();
    assert_eq!(
        (snapshot.admitted, snapshot.queued, snapshot.rejected_total),
        (1, 0, 1)
    );
    assert_eq!(snapshot.rejected_queue_full, 1);
    assert_eq!(snapshot.rejected_queue_timeout, 0);
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 1);
    harness.gates.get("/a").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(harness.stack.admission.stats().admitted, 0);
}

async fn queue_timeout() {
    let gates = gates(&[("/a", Behavior::Hold), ("/b", Behavior::Immediate)]);
    let harness = harness(
        "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 1\n  admission_queue_timeout_seconds: 0.01",
        "",
        gates,
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    let (status, headers, body) = requests.complete("/b").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers.get("retry-after").unwrap(), "1");
    assert!(
        from_utf8(&body)
            .unwrap()
            .contains(r#""type":"overloaded_error""#)
    );
    let snapshot = harness.stack.admission.stats();
    assert_eq!(
        (snapshot.admitted, snapshot.queued, snapshot.rejected_total),
        (1, 0, 1)
    );
    assert_eq!(snapshot.rejected_queue_timeout, 1);
    assert_eq!(snapshot.rejected_queue_full, 0);
    harness.gates.get("/a").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
}

async fn health_exempt() {
    let gates = gates(&[("/a", Behavior::Hold)]);
    let harness = harness(ENABLED, "", gates);
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    let exempt = [
        "/health/liveliness",
        "/health/liveness",
        "/health/readiness",
        "/health/readiness/details",
        "/health/backlog",
        "/health/drain",
        "/metrics",
        "/metrics/",
    ];
    let mut statuses = Vec::new();
    for path in exempt {
        statuses.push(start_and_wait(&harness, &mut requests, path).await);
    }
    assert_eq!(statuses, vec![StatusCode::OK; 8]);
    assert_eq!(
        start_and_wait(&harness, &mut requests, "/health/liveliness/").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        start_and_wait(&harness, &mut requests, "/Metrics").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(harness.stack.admission.stats().rejected_total, 2);
    harness.gates.get("/a").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
}

async fn root_path_boundary() {
    let gates = gates(&[("/proxy/chat", Behavior::Hold)]);
    let harness = harness(ENABLED, "/proxy", gates);
    let mut requests = Requests::default();
    requests.start(&harness.router, "/proxy/chat");
    await_started(&harness.gates, "/proxy/chat").await;
    assert_eq!(
        start_and_wait(&harness, &mut requests, "/proxy/health/liveliness").await,
        StatusCode::OK
    );
    assert_eq!(
        start_and_wait(&harness, &mut requests, "/proxy/metrics").await,
        StatusCode::OK
    );
    assert_eq!(
        start_and_wait(&harness, &mut requests, "/proxyhealth/liveliness").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        start_and_wait(&harness, &mut requests, "/proxy").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    harness
        .gates
        .get("/proxy/chat")
        .unwrap()
        .release
        .notify_one();
    assert_eq!(requests.complete("/proxy/chat").await.0, StatusCode::OK);
}

async fn stream_holds_slot() {
    let gates = gates(&[("/a", Behavior::Stream), ("/b", Behavior::Immediate)]);
    let harness = harness(ONE_QUEUED, "", gates);
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    await_queued(&harness.stack, 1).await;
    assert_eq!(harness.stack.admission.stats().admitted, 1);
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 1);
    harness.gates.get("/a").unwrap().release.notify_one();
    await_started(&harness.gates, "/b").await;
    let (status, _, body) = requests.complete("/a").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), b"firstlast");
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
    assert_eq!(harness.stack.admission.stats().admitted, 0);
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 0);
}

async fn disconnect_releases() {
    let gates = gates(&[("/a", Behavior::Stream), ("/b", Behavior::Immediate)]);
    let harness = harness(ONE_QUEUED, "", gates);
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.cancel("/a").await;
    assert_eq!(harness.stack.admission.stats().admitted, 0);
    assert_eq!(harness.stack.admission.stats().queued, 0);
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 0);
    requests.start(&harness.router, "/b");
    await_started(&harness.gates, "/b").await;
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
    assert_eq!(harness.stack.admission.stats().rejected_total, 0);
}

async fn scalar_root_exempt() {
    let gates = gates(&[("/proxy/chat", Behavior::Hold)]);
    let harness = harness(
        "general_settings:\n  SERVER_ROOT_PATH: /proxy\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 0",
        "",
        gates,
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/proxy/chat");
    await_started(&harness.gates, "/proxy/chat").await;
    assert_eq!(
        start_and_wait(&harness, &mut requests, "/proxy/health/liveliness").await,
        StatusCode::OK
    );
    assert_eq!(
        start_and_wait(&harness, &mut requests, "/proxy/metrics").await,
        StatusCode::OK
    );
    assert_eq!(
        start_and_wait(&harness, &mut requests, "/proxyhealth/liveliness").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    harness
        .gates
        .get("/proxy/chat")
        .unwrap()
        .release
        .notify_one();
    assert_eq!(requests.complete("/proxy/chat").await.0, StatusCode::OK);
}

async fn invalid_settings() {
    let gates = gates(&[("/a", Behavior::Hold), ("/b", Behavior::Hold)]);
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    let harness = harness(
        "general_settings:\n  max_in_flight_requests_per_worker: 0",
        "",
        gates,
    );
    drop(guard);
    let written = read_to_string(log.path()).unwrap();
    assert!(
        written.contains("invalid admission control setting"),
        "{written}"
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    await_started(&harness.gates, "/b").await;
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 2);
    assert_eq!(harness.stack.admission.stats().rejected_total, 0);
    harness.gates.get("/a").unwrap().release.notify_one();
    harness.gates.get("/b").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
}

async fn omitted_queue() {
    let gates = gates(&[
        ("/a", Behavior::Hold),
        ("/b", Behavior::Hold),
        ("/c", Behavior::Immediate),
    ]);
    let harness = harness(
        "general_settings:\n  max_in_flight_requests_per_worker: 2",
        "",
        gates,
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    await_started(&harness.gates, "/b").await;
    requests.start(&harness.router, "/c");
    await_queued(&harness.stack, 1).await;
    let snapshot = harness.stack.admission.stats();
    assert_eq!(
        (snapshot.admitted, snapshot.queued, snapshot.rejected_total),
        (2, 1, 0)
    );
    harness.gates.get("/a").unwrap().release.notify_one();
    await_started(&harness.gates, "/c").await;
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(requests.complete("/c").await.0, StatusCode::OK);
    harness.gates.get("/b").unwrap().release.notify_one();
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
    assert_eq!(harness.stack.admission.stats().queued, 0);
}

async fn default_timeout() {
    let gates = gates(&[("/a", Behavior::Hold), ("/b", Behavior::Immediate)]);
    let harness = harness(ONE_QUEUED, "", gates);
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    await_queued(&harness.stack, 1).await;
    harness.gates.get("/a").unwrap().release.notify_one();
    await_started(&harness.gates, "/b").await;
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
    assert_eq!(harness.stack.admission.stats().rejected_total, 0);
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 0);
}
