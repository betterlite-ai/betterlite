use std::collections::BTreeMap;
use std::fs;
use std::future;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse as _;
use axum::routing::get;
use betterlite_prometheus::multiprocess::files::{DbEntry, encode_db_file};
use betterlite_test_ports::refused_port::RefusedPort;
use reqwest::{Client, ClientBuilder, Method};
use rstest::rstest;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{
    METRICS_PATH, MetricsServerError, PID_HEADER, STARTUP_POLLING, StartupPolling, StartupProbe,
    answered_by, build_metrics_app, metrics_url, wait_until_serving,
};

const PROBED_PID: u32 = 4242;

const QUICK: StartupPolling = StartupPolling {
    deadline: Duration::from_millis(350),
    interval: Duration::from_millis(100),
    probe_timeout: Duration::from_secs(1),
};

const GRACE: Duration = Duration::from_millis(100);

fn client() -> Client {
    ClientBuilder::new().build().unwrap()
}

async fn serve(tasks: &TaskTracker, app: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let _serving = tasks.spawn(async move { axum::serve(listener, app).await });
    address
}

async fn served_app(folder: &Path, tasks: &TaskTracker) -> SocketAddr {
    let app = build_metrics_app(folder, process::id()).unwrap();
    serve(tasks, app).await
}

fn answering(status: StatusCode, pid: Option<&'static str>) -> Router {
    let reply = move || async move {
        let mut response = status.into_response();
        if let Some(value) = pid {
            let _previous = response
                .headers_mut()
                .insert(PID_HEADER, HeaderValue::from_static(value));
        }
        response
    };
    Router::new().route(METRICS_PATH, get(reply))
}

fn counting(hits: &Arc<AtomicUsize>) -> Router {
    let counter = Arc::clone(hits);
    let reply = move || {
        let _before = counter.fetch_add(1, Ordering::SeqCst);
        async { StatusCode::OK }
    };
    Router::new().route(METRICS_PATH, get(reply))
}

fn probe(address: SocketAddr, polling: StartupPolling) -> StartupProbe {
    StartupProbe::new("127.0.0.1", address.port(), PROBED_PID, polling).unwrap()
}

fn worker_file(folder: &Path, name: &str, value: f64) {
    let labels: BTreeMap<&str, &str> = BTreeMap::new();
    let key = serde_json::to_string(&("requests", "requests_total", labels, "requests")).unwrap();
    let entry = DbEntry {
        key,
        value,
        timestamp: 0.0,
    };
    fs::write(folder.join(name), encode_db_file(&[entry])).unwrap();
}

#[test]
fn startup_waits_30_seconds_polling_every_100_ms_with_1_second_probes() {
    assert_eq!(STARTUP_POLLING.deadline, Duration::from_secs(30));
    assert_eq!(STARTUP_POLLING.interval, Duration::from_millis(100));
    assert_eq!(STARTUP_POLLING.probe_timeout, Duration::from_secs(1));
}

#[rstest]
#[case::ipv4_wildcard("0.0.0.0", "http://127.0.0.1:4001/metrics")]
#[case::ipv6_wildcard("::", "http://[::1]:4001/metrics")]
#[case::ipv6_address("fe80::1", "http://[fe80::1]:4001/metrics")]
#[case::ipv6_loopback("::1", "http://[::1]:4001/metrics")]
#[case::ipv4_address("10.0.0.5", "http://10.0.0.5:4001/metrics")]
#[case::hostname("localhost", "http://localhost:4001/metrics")]
fn metrics_url_dials_loopback_for_wildcards_and_brackets_ipv6(
    #[case] host: &str,
    #[case] expected: &str,
) {
    assert_eq!(metrics_url(host, 4001), expected);
}

#[rstest]
#[case::ok_with_the_pid(StatusCode::OK, Some("4242"), true)]
#[case::ok_with_another_pid(StatusCode::OK, Some("4243"), false)]
#[case::ok_without_a_pid(StatusCode::OK, None, false)]
#[case::error_with_the_pid(StatusCode::INTERNAL_SERVER_ERROR, Some("4242"), false)]
#[case::not_found_with_the_pid(StatusCode::NOT_FOUND, Some("4242"), false)]
#[tokio::test]
async fn answered_by_needs_a_200_that_carries_the_pid(
    #[case] status: StatusCode,
    #[case] pid: Option<&'static str>,
    #[case] expected: bool,
) {
    let tasks = TaskTracker::new();
    let address = serve(&tasks, answering(status, pid)).await;
    assert_eq!(answered_by(&probe(address, QUICK)).await, expected);
}

#[tokio::test]
async fn answered_by_is_false_when_nothing_listens() {
    let refused = RefusedPort::bind().unwrap();
    assert!(!answered_by(&probe(refused.address, QUICK)).await);
}

#[tokio::test]
async fn wait_until_serving_returns_once_the_pid_answers() {
    let tasks = TaskTracker::new();
    let address = serve(&tasks, answering(StatusCode::OK, Some("4242"))).await;
    let mut serving = tasks.spawn(future::pending::<io::Result<()>>());
    let stop = CancellationToken::new();
    wait_until_serving(&mut serving, &probe(address, QUICK), &stop, GRACE)
        .await
        .unwrap();
    assert!(!stop.is_cancelled());
    assert!(!serving.is_finished());
}

#[tokio::test]
async fn wait_until_serving_fails_with_the_serve_error_when_the_listener_stops_first() {
    let tasks = TaskTracker::new();
    let refused = RefusedPort::bind().unwrap();
    let mut serving = tasks.spawn(async { Err(io::Error::other("accept loop failed")) });
    time::sleep(Duration::from_millis(20)).await;
    let stop = CancellationToken::new();
    let error = wait_until_serving(&mut serving, &probe(refused.address, QUICK), &stop, GRACE)
        .await
        .unwrap_err();
    let MetricsServerError::Stopped { url, source } = error else {
        panic!("expected Stopped, got {error:?}");
    };
    assert_eq!(url, metrics_url("127.0.0.1", refused.address.port()));
    assert_eq!(source.unwrap().to_string(), "accept loop failed");
    assert!(!stop.is_cancelled());
}

#[tokio::test]
async fn wait_until_serving_fails_without_a_cause_when_the_listener_ends_cleanly() {
    let tasks = TaskTracker::new();
    let refused = RefusedPort::bind().unwrap();
    let mut serving = tasks.spawn(async { Ok(()) });
    time::sleep(Duration::from_millis(20)).await;
    let stop = CancellationToken::new();
    let error = wait_until_serving(&mut serving, &probe(refused.address, QUICK), &stop, GRACE)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        MetricsServerError::Stopped { source: None, .. }
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "Prometheus metrics server stopped before serving {}",
            metrics_url("127.0.0.1", refused.address.port())
        )
    );
}

#[tokio::test]
async fn wait_until_serving_reports_a_listener_task_that_panicked() {
    let tasks = TaskTracker::new();
    let refused = RefusedPort::bind().unwrap();
    let mut serving = tasks.spawn(async { panic!("listener crashed") });
    time::sleep(Duration::from_millis(20)).await;
    let stop = CancellationToken::new();
    let error = wait_until_serving(&mut serving, &probe(refused.address, QUICK), &stop, GRACE)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        MetricsServerError::Stopped {
            source: Some(_),
            ..
        }
    ));
}

#[tokio::test]
async fn wait_until_serving_polls_every_interval_and_stops_the_listener_at_the_deadline() {
    let tasks = TaskTracker::new();
    let hits = Arc::new(AtomicUsize::new(0));
    let address = serve(&tasks, counting(&hits)).await;
    let mut serving = tasks.spawn(future::pending::<io::Result<()>>());
    let stop = CancellationToken::new();
    let started = Instant::now();
    let error = wait_until_serving(&mut serving, &probe(address, QUICK), &stop, GRACE)
        .await
        .unwrap_err();
    assert!(started.elapsed() >= QUICK.deadline);
    let probes = hits.load(Ordering::SeqCst);
    assert!((2..=5).contains(&probes), "{probes} probes");
    assert!(stop.is_cancelled());
    assert!(serving.await.unwrap_err().is_cancelled());
    assert_eq!(
        error.to_string(),
        format!(
            "Prometheus metrics server did not answer {} within 0s",
            metrics_url("127.0.0.1", address.port())
        )
    );
}

#[rstest]
#[case::mount("/metrics")]
#[case::mount_with_slash("/metrics/")]
#[case::below_the_mount("/metrics/anything")]
#[tokio::test]
async fn metrics_answer_without_auth_with_every_worker_summed_and_the_pid(#[case] path: &str) {
    let folder = TempDir::new().unwrap();
    worker_file(folder.path(), "counter_1001.db", 3.0);
    worker_file(folder.path(), "counter_1002.db", 4.0);
    let tasks = TaskTracker::new();
    let address = served_app(folder.path(), &tasks).await;
    let url = format!("http://{address}{path}");
    let response = client().get(url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let pid = response.headers().get(PID_HEADER).unwrap();
    assert_eq!(pid.to_str().unwrap(), process::id().to_string());
    let body = response.text().await.unwrap();
    assert!(body.contains("requests_total 7.0"), "{body}");
}

#[tokio::test]
async fn health_answers_with_its_status_and_folder() {
    let folder = TempDir::new().unwrap();
    let tasks = TaskTracker::new();
    let address = served_app(folder.path(), &tasks).await;
    let url = format!("http://{address}/health");
    let response = client().get(url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert!(response.headers().get(PID_HEADER).is_none());
    let folder_json = serde_json::to_string(folder.path()).unwrap();
    let expected = format!(r#"{{"status":"healthy","multiproc_dir":{folder_json}}}"#);
    assert_eq!(response.text().await.unwrap(), expected);
}

#[rstest]
#[case::post(Method::POST)]
#[case::head(Method::HEAD)]
#[case::delete(Method::DELETE)]
#[tokio::test]
async fn health_answers_only_get(#[case] method: Method) {
    let folder = TempDir::new().unwrap();
    let tasks = TaskTracker::new();
    let address = served_app(folder.path(), &tasks).await;
    let url = format!("http://{address}/health");
    let response = client().request(method.clone(), url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers().get(header::ALLOW).unwrap(), "GET");
    if method != Method::HEAD {
        let body = response.text().await.unwrap();
        assert_eq!(body, r#"{"detail":"Method Not Allowed"}"#);
    }
}

#[tokio::test]
async fn unknown_paths_are_a_json_404() {
    let folder = TempDir::new().unwrap();
    let tasks = TaskTracker::new();
    let address = served_app(folder.path(), &tasks).await;
    let url = format!("http://{address}/metricsx");
    let response = client().get(url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.text().await.unwrap(), r#"{"detail":"Not Found"}"#);
}
