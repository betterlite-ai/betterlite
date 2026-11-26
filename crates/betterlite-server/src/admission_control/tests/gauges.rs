use std::fs;
use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::gauge_mode::GaugeMode;
use betterlite_prometheus::multiprocess::files::decode_db_file;
use betterlite_prometheus::multiprocess::writer::MultiProcessWriter;
use rstest::rstest;
use rust_decimal::Decimal;
use tempfile::{NamedTempFile, TempDir};
use tracing::subscriber::set_default;

use super::harness::{
    Behavior, Harness, Requests, await_queued, await_started, gates, harness, harness_on,
};

const IN_FLIGHT: &str = "litellm_in_flight_requests";
const ADMITTED: &str = "litellm_admission_admitted_requests";
const QUEUED: &str = "litellm_admission_queued_requests";
const REJECTED: &str = "litellm_admission_rejected_requests";
const IN_FLIGHT_HELP: &str = "Number of HTTP requests currently in-flight on this uvicorn worker";

const ONE_SLOT: &str = "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 0";
const ONE_QUEUED: &str = "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 1";
const QUEUE_TIMEOUT: &str = "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 1\n  admission_queue_timeout_seconds: 0.01";

fn one() -> MetricValue {
    MetricValue::Finite(Decimal::ONE)
}

fn zero() -> MetricValue {
    MetricValue::Finite(Decimal::ZERO)
}

fn gauge(registry: &CollectorRegistry, name: &str) -> MetricValue {
    registry
        .get(name)
        .expect("a registered gauge")
        .series(&[])
        .expect("the label-less series")
        .total
}

fn rejection_counter(registry: &CollectorRegistry, reason: &str) -> MetricValue {
    registry
        .get(REJECTED)
        .expect("the rejection counter")
        .series(&[Some(reason.to_owned())])
        .expect("the rejection reason's child")
        .total
}

fn child_is_absent(registry: &CollectorRegistry, reason: &str) -> bool {
    registry
        .get(REJECTED)
        .expect("the rejection counter")
        .series(&[Some(reason.to_owned())])
        .is_none()
}

async fn harness_after_one_request() -> Harness {
    let harness = harness(ONE_SLOT, "", gates(&[("/only", Behavior::Immediate)]));
    let mut requests = Requests::default();
    requests.start(&harness.router, "/only");
    assert_eq!(requests.complete("/only").await.0, StatusCode::OK);
    harness
}

#[tokio::test]
async fn no_metric_exists_before_the_first_request() {
    let harness = harness(ONE_SLOT, "", gates(&[("/only", Behavior::Immediate)]));
    assert!(harness.registry.get(IN_FLIGHT).is_none());
    assert!(harness.registry.get(ADMITTED).is_none());
    assert!(harness.registry.get(QUEUED).is_none());
    assert!(harness.registry.get(REJECTED).is_none());
}

#[rstest]
#[case::in_flight(IN_FLIGHT, IN_FLIGHT_HELP, 0)]
#[case::admitted(ADMITTED, "Number of requests admitted by this worker", 0)]
#[case::queued(QUEUED, "Number of requests queued by this worker", 0)]
#[case::rejected(REJECTED, "Number of requests rejected by this worker", 1)]
#[tokio::test]
async fn the_metrics_match_the_litellm_definitions(
    #[case] name: &str,
    #[case] help: &str,
    #[case] labels: usize,
) {
    let harness = harness_after_one_request().await;
    let metric = harness.registry.get(name).expect("a registered metric");
    assert_eq!(metric.description.help, help);
    assert_eq!(metric.description.label_names.len(), labels);
}

#[rstest]
#[case::in_flight(IN_FLIGHT)]
#[case::admitted(ADMITTED)]
#[case::queued(QUEUED)]
#[tokio::test]
async fn the_gauges_are_live_sum(#[case] name: &str) {
    let harness = harness_after_one_request().await;
    let metric = harness.registry.get(name).expect("a registered gauge");
    assert_eq!(metric.description.multiprocess_mode, GaugeMode::LiveSum);
}

#[tokio::test]
async fn the_in_flight_gauge_reads_one_while_a_request_is_held() {
    let harness = harness(
        "general_settings: {}",
        "",
        gates(&[("/hold", Behavior::Hold)]),
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/hold");
    await_started(&harness.gates, "/hold").await;
    assert_eq!(gauge(&harness.registry, IN_FLIGHT), one());
    harness.gates.get("/hold").unwrap().release.notify_one();
    assert_eq!(requests.complete("/hold").await.0, StatusCode::OK);
    assert_eq!(gauge(&harness.registry, IN_FLIGHT), zero());
}

#[tokio::test]
async fn the_in_flight_gauge_falls_when_the_client_drops_the_body() {
    let harness = harness(
        "general_settings: {}",
        "",
        gates(&[("/stream", Behavior::Stream)]),
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/stream");
    await_started(&harness.gates, "/stream").await;
    assert_eq!(gauge(&harness.registry, IN_FLIGHT), one());
    requests.cancel("/stream").await;
    assert_eq!(gauge(&harness.registry, IN_FLIGHT), zero());
}

#[tokio::test]
async fn a_websocket_upgrade_touches_no_metric() {
    let harness = harness(ONE_SLOT, "", gates(&[("/ws", Behavior::Immediate)]));
    let mut requests = Requests::default();
    requests.start_upgrade(&harness.router, "/ws");
    assert_eq!(requests.complete("/ws").await.0, StatusCode::OK);
    assert!(harness.registry.get(IN_FLIGHT).is_none());
    assert!(harness.registry.get(ADMITTED).is_none());
    assert!(harness.registry.get(QUEUED).is_none());
    assert!(harness.registry.get(REJECTED).is_none());
}

#[tokio::test]
async fn admission_moves_the_admitted_gauge_and_releases_it() {
    let harness = harness(ONE_SLOT, "", gates(&[("/hold", Behavior::Hold)]));
    let mut requests = Requests::default();
    requests.start(&harness.router, "/hold");
    await_started(&harness.gates, "/hold").await;
    assert_eq!(gauge(&harness.registry, ADMITTED), one());
    assert_eq!(gauge(&harness.registry, QUEUED), zero());
    harness.gates.get("/hold").unwrap().release.notify_one();
    assert_eq!(requests.complete("/hold").await.0, StatusCode::OK);
    assert_eq!(gauge(&harness.registry, ADMITTED), zero());
}

#[tokio::test]
async fn the_queued_gauge_follows_entry_and_dequeue() {
    let harness = harness(
        ONE_QUEUED,
        "",
        gates(&[("/a", Behavior::Hold), ("/b", Behavior::Immediate)]),
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    assert_eq!(gauge(&harness.registry, ADMITTED), one());
    requests.start(&harness.router, "/b");
    await_queued(&harness.stack, 1).await;
    assert_eq!(gauge(&harness.registry, QUEUED), one());
    harness.gates.get("/a").unwrap().release.notify_one();
    await_started(&harness.gates, "/b").await;
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
    assert_eq!(gauge(&harness.registry, QUEUED), zero());
    assert_eq!(gauge(&harness.registry, ADMITTED), zero());
}

#[tokio::test]
async fn a_queue_timeout_counts_the_reason_and_clears_the_queue() {
    let harness = harness(
        QUEUE_TIMEOUT,
        "",
        gates(&[("/a", Behavior::Hold), ("/b", Behavior::Immediate)]),
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    let (status, _headers, _body) = requests.complete("/b").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(rejection_counter(&harness.registry, "queue_timeout"), one());
    assert!(child_is_absent(&harness.registry, "queue_full"));
    assert_eq!(gauge(&harness.registry, QUEUED), zero());
    assert_eq!(gauge(&harness.registry, ADMITTED), one());
    harness.gates.get("/a").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(gauge(&harness.registry, ADMITTED), zero());
}

#[tokio::test]
async fn a_queue_full_rejection_counts_the_reason_without_queueing() {
    let harness = harness(
        ONE_SLOT,
        "",
        gates(&[("/a", Behavior::Hold), ("/b", Behavior::Immediate)]),
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start(&harness.router, "/b");
    let (status, _headers, _body) = requests.complete("/b").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(rejection_counter(&harness.registry, "queue_full"), one());
    assert!(child_is_absent(&harness.registry, "queue_timeout"));
    assert_eq!(gauge(&harness.registry, QUEUED), zero());
    harness.gates.get("/a").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    assert_eq!(gauge(&harness.registry, ADMITTED), zero());
}

#[tokio::test]
async fn disabled_admission_touches_no_admission_metric() {
    let harness = harness(
        "general_settings: {}",
        "",
        gates(&[("/only", Behavior::Immediate)]),
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/only");
    assert_eq!(requests.complete("/only").await.0, StatusCode::OK);
    assert_eq!(gauge(&harness.registry, IN_FLIGHT), zero());
    assert!(harness.registry.get(ADMITTED).is_none());
    assert!(harness.registry.get(QUEUED).is_none());
    assert!(harness.registry.get(REJECTED).is_none());
}

#[tokio::test]
async fn an_exempt_path_touches_no_admission_metric() {
    let harness = harness(ONE_SLOT, "", gates(&[("/metrics", Behavior::Immediate)]));
    let mut requests = Requests::default();
    requests.start(&harness.router, "/metrics");
    assert_eq!(requests.complete("/metrics").await.0, StatusCode::OK);
    assert_eq!(gauge(&harness.registry, IN_FLIGHT), zero());
    assert!(harness.registry.get(ADMITTED).is_none());
    assert!(harness.registry.get(QUEUED).is_none());
    assert!(harness.registry.get(REJECTED).is_none());
}

#[tokio::test]
async fn a_second_layer_logs_the_refused_registration_and_keeps_counting() {
    let first = harness(ONE_SLOT, "", gates(&[("/one", Behavior::Immediate)]));
    let mut requests = Requests::default();
    requests.start(&first.router, "/one");
    assert_eq!(requests.complete("/one").await.0, StatusCode::OK);
    let second = harness_on(
        Arc::clone(&first.registry),
        ONE_SLOT,
        "",
        gates(&[("/two", Behavior::Hold)]),
    );
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    requests.start(&second.router, "/two");
    await_started(&second.gates, "/two").await;
    assert_eq!(second.stack.in_flight.stats().current, 1);
    assert_eq!(second.stack.admission.stats().admitted, 1);
    drop(guard);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(
        written.contains("could not register the in-flight requests gauge"),
        "{written}"
    );
    assert!(
        written.contains("could not register the admission control metrics"),
        "{written}"
    );
    second.gates.get("/two").unwrap().release.notify_one();
    assert_eq!(requests.complete("/two").await.0, StatusCode::OK);
    assert_eq!(second.stack.in_flight.stats().current, 0);
    assert_eq!(second.stack.admission.stats().admitted, 0);
    assert_eq!(gauge(&first.registry, IN_FLIGHT), zero());
}

#[tokio::test]
async fn the_in_flight_gauge_reaches_the_livesum_file() {
    let directory = TempDir::new().unwrap();
    let writer = MultiProcessWriter::new(directory.path().to_path_buf(), 4242).unwrap();
    let registry = Arc::new(CollectorRegistry::multiprocess(writer));
    let harness = harness_on(
        Arc::clone(&registry),
        "general_settings: {}",
        "",
        gates(&[("/only", Behavior::Immediate)]),
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/only");
    assert_eq!(requests.complete("/only").await.0, StatusCode::OK);
    let path = directory.path().join("gauge_livesum_4242.db");
    let entries = decode_db_file(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].key.contains(IN_FLIGHT));
    assert_eq!(entries[0].value.to_bits(), 0.0_f64.to_bits());
}
