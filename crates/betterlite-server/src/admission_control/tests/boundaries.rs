use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::admission_control::{Limits, Settings};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tower::ServiceExt as _;

use super::harness::{Behavior, Requests, WAIT, await_queued, await_started, gates, harness};
use crate::admission_control::AdmissionControl;
use crate::routing::admission_control::hot_routes_first;
use crate::server::chat_setup::chat_executor;
use crate::server::{application, configure_connection_builder, serve_connection};

const RESERVATION_THREADS: usize = 8;
const RESERVATION_ROUNDS: u32 = 200;
const UPGRADE: (&str, &str) = ("upgrade", "websocket");
const CONNECTION: (&str, &str) = ("connection", "upgrade");
const KEY: (&str, &str) = ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==");
const VERSION: (&str, &str) = ("sec-websocket-version", "13");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    UpgradeLookalike(&'static str, &'static [(&'static str, &'static str)]),
    QueueFullAtCapacity,
    QueueTimeoutAtTheLimit,
    CancelQueuedDecrements,
    QueueReservationThreadContention,
    HugeLimitDisables,
    HotRoutesFirst,
    InstallWiresTheStack,
    UpgradeBypassesSaturation,
    ContentLengthSurvivesTheStack,
    MatchedRootPathSkipsTheScalarRoot,
    ScalarRootExemption(&'static str, &'static str, bool),
}

#[rstest]
#[case::no_connection(Boundary::UpgradeLookalike("GET", &[UPGRADE, KEY, VERSION]))]
#[case::post(Boundary::UpgradeLookalike("POST", &[UPGRADE, CONNECTION, KEY, VERSION]))]
#[case::no_key(Boundary::UpgradeLookalike("GET", &[UPGRADE, CONNECTION, VERSION]))]
#[case::no_version(Boundary::UpgradeLookalike("GET", &[UPGRADE, CONNECTION, KEY]))]
#[case::junk_key(Boundary::UpgradeLookalike("GET", &[UPGRADE, CONNECTION, ("sec-websocket-key", "k"), VERSION]))]
#[case::version_12(Boundary::UpgradeLookalike("GET", &[UPGRADE, CONNECTION, KEY, ("sec-websocket-version", "12")]))]
#[case::last_connection_wins(Boundary::UpgradeLookalike("GET", &[UPGRADE, CONNECTION, ("connection", "close"), KEY, VERSION]))]
#[case::short_key(Boundary::UpgradeLookalike("GET", &[UPGRADE, CONNECTION, ("sec-websocket-key", "a2V5"), VERSION]))]
#[case::two_keys(Boundary::UpgradeLookalike("GET", &[UPGRADE, CONNECTION, KEY, KEY, VERSION]))]
#[case::two_versions(Boundary::UpgradeLookalike("GET", &[UPGRADE, CONNECTION, KEY, VERSION, VERSION]))]
#[case::websocket_then_h2c(Boundary::UpgradeLookalike("GET", &[UPGRADE, ("upgrade", "h2c"), CONNECTION, KEY, VERSION]))]
#[case::not_websocket(Boundary::UpgradeLookalike("GET", &[("upgrade", "h2c"), CONNECTION, KEY, VERSION]))]
#[case::h2c_then_websocket(Boundary::UpgradeLookalike("GET", &[("upgrade", "h2c"), UPGRADE, CONNECTION, KEY, VERSION]))]
#[case::queue_full_at_capacity(Boundary::QueueFullAtCapacity)]
#[case::queue_timeout_at_the_limit(Boundary::QueueTimeoutAtTheLimit)]
#[case::cancel_queued_decrements(Boundary::CancelQueuedDecrements)]
#[case::queue_reservation_thread_contention(Boundary::QueueReservationThreadContention)]
#[case::huge_limit_disables(Boundary::HugeLimitDisables)]
#[case::hot_routes_first(Boundary::HotRoutesFirst)]
#[case::install_wires_the_stack(Boundary::InstallWiresTheStack)]
#[case::upgrade_bypasses_saturation(Boundary::UpgradeBypassesSaturation)]
#[case::content_length_survives_the_stack(Boundary::ContentLengthSurvivesTheStack)]
#[case::matched_root_path_skips_the_scalar_root(Boundary::MatchedRootPathSkipsTheScalarRoot)]
#[case::slash_root_keeps_metrics_exempt(Boundary::ScalarRootExemption("/", "/metrics", true))]
#[case::slash_root_keeps_health_exempt(Boundary::ScalarRootExemption(
    "/",
    "/health/liveliness",
    true
))]
#[case::slash_root_still_limits_other_paths(Boundary::ScalarRootExemption("/", "/other", false))]
#[case::exact_root_is_not_exempt(Boundary::ScalarRootExemption("/metrics", "/metrics", false))]
#[case::root_then_slash_is_exempt(Boundary::ScalarRootExemption("/proxy", "/proxy/metrics", true))]
#[case::partial_segment_is_not_exempt(Boundary::ScalarRootExemption(
    "/proxy",
    "/proxyhealth/liveliness",
    false
))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admission_control_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::UpgradeLookalike(method, headers) => {
            upgrade_lookalike_is_http(method, headers).await;
        }
        Boundary::QueueFullAtCapacity => queue_full_at_capacity().await,
        Boundary::QueueTimeoutAtTheLimit => queue_timeout_at_the_limit().await,
        Boundary::CancelQueuedDecrements => cancel_queued_decrements().await,
        Boundary::QueueReservationThreadContention => {
            queue_reservation_holds_under_thread_contention();
        }
        Boundary::HugeLimitDisables => huge_limit_disables().await,
        Boundary::HotRoutesFirst => hot_routes_first_audit(),
        Boundary::InstallWiresTheStack => install_wires_the_stack().await,
        Boundary::UpgradeBypassesSaturation => upgrade_bypasses_saturation().await,
        Boundary::ContentLengthSurvivesTheStack => content_length_survives_the_stack().await,
        Boundary::MatchedRootPathSkipsTheScalarRoot => {
            matched_root_path_skips_the_scalar_root().await;
        }
        Boundary::ScalarRootExemption(root, path, exempt) => {
            scalar_root_exemption(root, path, exempt).await;
        }
    }
}

async fn scalar_root_exemption(root: &str, path: &str, exempt: bool) {
    let source = format!(
        "general_settings:\n  SERVER_ROOT_PATH: {root}\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 0"
    );
    let harness = harness(&source, "", gates(&[("/hold", Behavior::Hold)]));
    let mut requests = Requests::default();
    requests.start(&harness.router, "/hold");
    await_started(&harness.gates, "/hold").await;
    requests.start(&harness.router, path);
    let expected = if exempt {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    assert_eq!(requests.complete(path).await.0, expected);
    harness.gates.get("/hold").unwrap().release.notify_one();
    assert_eq!(requests.complete("/hold").await.0, StatusCode::OK);
}

async fn matched_root_path_skips_the_scalar_root() {
    let gates = gates(&[("/other/chat", Behavior::Hold)]);
    let harness = harness(
        "general_settings:\n  SERVER_ROOT_PATH: /proxy\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 0",
        "/other",
        gates,
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/other/chat");
    await_started(&harness.gates, "/other/chat").await;
    requests.start(&harness.router, "/other/health/liveliness");
    assert_eq!(
        requests.complete("/other/health/liveliness").await.0,
        StatusCode::OK
    );
    requests.start(&harness.router, "/other/proxy/health/liveliness");
    assert_eq!(
        requests.complete("/other/proxy/health/liveliness").await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    harness
        .gates
        .get("/other/chat")
        .unwrap()
        .release
        .notify_one();
    assert_eq!(requests.complete("/other/chat").await.0, StatusCode::OK);
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

fn queue_reservation_holds_under_thread_contention() {
    let control = AdmissionControl::from_settings(&Settings {
        limits: Some(Limits {
            max_in_flight_requests: 1,
            max_queued_requests: 1,
            queue_timeout: Duration::from_secs(1),
        }),
    });
    for round in 0..RESERVATION_ROUNDS {
        control.queued.store(0, Ordering::Release);
        let successes = reservation_round(&control);
        assert_eq!(successes, 1, "round {round}");
    }
}

fn reservation_round(control: &AdmissionControl) -> u64 {
    let barrier = Barrier::new(RESERVATION_THREADS);
    let successes = AtomicU64::new(0);
    thread::scope(|scope| {
        for _ in 0..RESERVATION_THREADS {
            scope.spawn(|| attempt_reservation(control, &barrier, &successes));
        }
    });
    successes.load(Ordering::Acquire)
}

fn attempt_reservation(control: &AdmissionControl, barrier: &Barrier, successes: &AtomicU64) {
    barrier.wait();
    if control.reserve_queue_slot(1) {
        successes.fetch_add(1, Ordering::AcqRel);
    }
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
        "/cold-one",
        "/health/liveliness",
        "/cold-two",
        "/health/liveness",
        "/cold-three",
        "/v1/chat/completions",
        "/cold-four",
        "/chat/completions",
        "/cold-five",
        "/v1/messages",
        "/cold-six",
    ]);
    assert_eq!(
        ordered,
        vec![
            "/health/liveliness",
            "/health/liveness",
            "/v1/chat/completions",
            "/chat/completions",
            "/v1/messages",
            "/cold-one",
            "/cold-two",
            "/cold-three",
            "/cold-four",
            "/cold-five",
            "/cold-six",
        ]
    );
}

const SATURATED: &str = "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 0";

fn server_application() -> Router {
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(SATURATED)
    .unwrap();
    let executor = chat_executor(&config, &environment, StdRng::seed_from_u64(1)).unwrap();
    application(&config, executor, Arc::new(AtomicBool::new(false))).unwrap()
}

async fn content_length_survives_the_stack() {
    for path in ["/health/liveliness", "/models"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = server_application();
        let mut serving = JoinSet::new();
        serving.spawn(async move {
            let (stream, remote) = listener.accept().await.unwrap();
            serve_connection(configure_connection_builder(), router, stream, remote).await;
        });
        let mut client = TcpStream::connect(address).await.unwrap();
        let request =
            format!("GET {path} HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n");
        client.write_all(request.as_bytes()).await.unwrap();
        let mut raw = String::new();
        timeout(WAIT, client.read_to_string(&mut raw))
            .await
            .unwrap()
            .unwrap();
        serving.join_next().await.unwrap().unwrap();
        let head = raw.split("\r\n\r\n").next().unwrap().to_ascii_lowercase();
        assert!(head.contains("\r\ncontent-length: "), "{path}: {head}");
        assert!(!head.contains("transfer-encoding"), "{path}: {head}");
    }
}

async fn upgrade_lookalike_is_http(method: &str, headers: &'static [(&'static str, &'static str)]) {
    let gates = gates(&[("/a", Behavior::Hold), ("/ws", Behavior::Immediate)]);
    let harness = harness(SATURATED, "", gates);
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    let mut request = Request::builder()
        .method(method.parse::<Method>().unwrap())
        .uri("/ws")
        .body(Body::empty())
        .unwrap();
    *request.headers_mut() = headers
        .iter()
        .map(|(name, value)| {
            (
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            )
        })
        .collect::<HeaderMap>();
    let response = harness.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(harness.stack.admission.stats().rejected_total, 1);
    harness.gates.get("/a").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
}

async fn install_wires_the_stack() {
    let router = server_application();
    let get = |path: &str| Request::builder().uri(path).body(Body::empty()).unwrap();
    let held = router.clone().oneshot(get("/models")).await.unwrap();
    let saturated = router.clone().oneshot(get("/models")).await.unwrap();
    assert_eq!(saturated.status(), StatusCode::SERVICE_UNAVAILABLE);
    let probe = router.clone().oneshot(get("/health/liveliness")).await;
    assert_eq!(probe.unwrap().status(), StatusCode::OK);
    drop(held);
    let released = router.oneshot(get("/models")).await.unwrap();
    assert_ne!(released.status(), StatusCode::SERVICE_UNAVAILABLE);
}

async fn upgrade_bypasses_saturation() {
    let gates = gates(&[
        ("/a", Behavior::Hold),
        ("/ws", Behavior::Immediate),
        ("/b", Behavior::Immediate),
    ]);
    let harness = harness(
        "general_settings:\n  max_in_flight_requests_per_worker: 1\n  max_queued_requests_per_worker: 0",
        "",
        gates,
    );
    let mut requests = Requests::default();
    requests.start(&harness.router, "/a");
    await_started(&harness.gates, "/a").await;
    requests.start_upgrade(&harness.router, "/ws");
    let (status, _headers, _body) = requests.complete("/ws").await;
    assert_eq!(status, StatusCode::OK);
    let snapshot = harness.stack.admission.stats();
    assert_eq!(
        (snapshot.admitted, snapshot.queued, snapshot.rejected_total),
        (1, 0, 0)
    );
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 1);
    harness.gates.get("/a").unwrap().release.notify_one();
    assert_eq!(requests.complete("/a").await.0, StatusCode::OK);
    requests.start(&harness.router, "/b");
    await_started(&harness.gates, "/b").await;
    assert_eq!(requests.complete("/b").await.0, StatusCode::OK);
    assert_eq!(harness.stack.admission.stats().admitted, 0);
    assert_eq!(harness.stack.in_flight.current.load(Ordering::Acquire), 0);
}
