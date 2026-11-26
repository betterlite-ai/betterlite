#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::io;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::path::{Path, PathBuf};
use std::process::{self, ExitCode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use betterlite_http::metrics_server::{
    METRICS_PATH, MetricsServerError, STARTUP_POLLING, StartupPolling, StartupProbe, answered_by,
    build_metrics_app, wait_until_serving,
};
use betterlite_http::proxy::base_builder;
use clap::Error;
use clap::error::ErrorKind;
use rstest::rstest;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::Notify;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{
    MULTIPROC_DIR, MetricsServerArgs, PARENT_POLL_INTERVAL, SHUTDOWN_GRACE, metrics_server_args,
    parent_exited, report, run_metrics_server, run_metrics_server_watched, serve, serve_bounded,
    serve_until_parent_exits, start_metrics_listener,
};

const PROBED_PID: u32 = 4242;

const LARGE_BODY: usize = 32 * 1024 * 1024;

const QUICK: StartupPolling = StartupPolling {
    deadline: Duration::from_millis(350),
    interval: Duration::from_millis(100),
    probe_timeout: Duration::from_secs(1),
};

const QUICK_GRACE: Duration = Duration::from_millis(100);

fn settings(folder: &Path, port: u16) -> MetricsServerArgs {
    MetricsServerArgs {
        host: "127.0.0.1".to_owned(),
        port,
        multiproc_dir: folder.to_path_buf(),
    }
}

fn probe_for(host: &str, port: u16, pid: u32, polling: StartupPolling) -> StartupProbe {
    let client = base_builder().no_proxy().build().unwrap();
    StartupProbe::new(client, host, port, pid, polling)
}

fn free_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// the packaged sidecar, built beside the test binary as tests/tests.rs finds betterlite
fn sidecar_binary() -> PathBuf {
    let mut test_binary = env::current_exe().unwrap();
    test_binary.pop();
    test_binary.set_file_name("betterlite-metrics-server");
    test_binary
}

fn large_app() -> Router {
    Router::new().route(METRICS_PATH, get(|| async { "x".repeat(LARGE_BODY) }))
}

async fn stalled_client(address: SocketAddr) -> TcpStream {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!("GET {METRICS_PATH} HTTP/1.1\r\nHost: {address}\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    time::sleep(Duration::from_millis(50)).await;
    stream
}

async fn expect_closed(mut stream: TcpStream) {
    let mut received = Vec::new();
    let closed = time::timeout(Duration::from_secs(5), stream.read_to_end(&mut received)).await;
    let _read = closed.expect("the stalled connection stayed open");
}

fn parsed(arguments: &[&str], env_folder: Option<&str>) -> Result<MetricsServerArgs, Error> {
    let mut environment = BTreeMap::new();
    if let Some(folder) = env_folder {
        let _previous = environment.insert(MULTIPROC_DIR.to_owned(), folder.to_owned());
    }
    metrics_server_args(
        arguments.iter().copied().map(OsString::from).collect(),
        &environment,
    )
}

#[test]
fn the_listener_drains_for_5_seconds_and_polls_the_parent_every_second() {
    assert_eq!(SHUTDOWN_GRACE, Duration::from_secs(5));
    assert_eq!(PARENT_POLL_INTERVAL, Duration::from_secs(1));
}

#[tokio::test]
async fn start_fails_when_the_port_is_already_taken() {
    let folder = TempDir::new().unwrap();
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = reserved.local_addr().unwrap().port();
    let tasks = TaskTracker::new();
    let error = start_metrics_listener(
        &settings(folder.path(), port),
        &tasks,
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, MetricsServerError::Bind { port: failed, .. } if failed == port));
    assert_eq!(
        error.to_string(),
        format!(
            "Prometheus metrics server cannot listen on 127.0.0.1:{port}; is the port already in use?"
        )
    );
}

#[tokio::test]
async fn start_fails_when_the_folder_is_missing() {
    let folder = TempDir::new().unwrap();
    let missing = folder.path().join("missing");
    let tasks = TaskTracker::new();
    let error = start_metrics_listener(&settings(&missing, 0), &tasks, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(&error, MetricsServerError::Folder { path, .. } if *path == missing));
}

#[tokio::test]
async fn the_listener_stops_with_the_shutdown_token() {
    let folder = TempDir::new().unwrap();
    let tasks = TaskTracker::new();
    let token = CancellationToken::new();
    let listener = start_metrics_listener(&settings(folder.path(), 0), &tasks, token.clone())
        .await
        .unwrap();
    token.cancel();
    tasks.close();
    let stopped = time::timeout(SHUTDOWN_GRACE, tasks.wait()).await;
    stopped.expect("the listener did not stop within its shutdown grace");
    assert!(listener.serving.is_finished());
    let refused = TcpStream::connect(listener.address).await;
    refused.expect_err("the listener still accepts after it stopped");
}

#[tokio::test]
async fn the_sidecar_serves_the_folder_on_its_port() {
    let folder = TempDir::new().unwrap();
    let port = free_port();
    let sidecar = settings(folder.path(), port);
    let tasks = TaskTracker::new();
    let mut serving =
        tasks.spawn(async move { run_metrics_server(&sidecar).await.map_err(io::Error::other) });
    let ready = probe_for("127.0.0.1", port, process::id(), STARTUP_POLLING);
    wait_until_serving(
        &mut serving,
        &ready,
        &CancellationToken::new(),
        SHUTDOWN_GRACE,
    )
    .await
    .unwrap();
    serving.abort();
}

#[tokio::test]
async fn the_sidecar_fails_when_the_folder_is_missing() {
    let folder = TempDir::new().unwrap();
    let sidecar = settings(&folder.path().join("missing"), 0);
    let error = run_metrics_server(&sidecar).await.unwrap_err();
    assert!(matches!(error, MetricsServerError::Folder { .. }));
}

#[test]
fn serve_fails_when_the_folder_is_missing() {
    let folder = TempDir::new().unwrap();
    let error = serve(&settings(&folder.path().join("missing"), 0)).unwrap_err();
    assert!(matches!(error, MetricsServerError::Folder { .. }));
}

#[test]
fn the_cli_exit_code_follows_the_server_outcome() {
    assert_eq!(report(Ok(())), ExitCode::SUCCESS);
    let folder = TempDir::new().unwrap();
    let error = serve(&settings(&folder.path().join("missing"), 0)).unwrap_err();
    assert_eq!(report(Err(error)), ExitCode::FAILURE);
}

// the packaged entry point: main parses the CLI and serves the folder on the port it is given
#[tokio::test]
async fn the_binary_serves_the_folder_on_the_port_it_is_given() {
    let folder = TempDir::new().unwrap();
    let port = free_port();
    let mut sidecar = Command::new(sidecar_binary())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--multiproc_dir")
        .arg(folder.path())
        .env_clear()
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let probe = probe_for("127.0.0.1", port, sidecar.id().unwrap(), STARTUP_POLLING);
    let started = Instant::now();
    let mut answered = answered_by(&probe).await;
    while !answered && started.elapsed() < Duration::from_secs(5) {
        if sidecar.try_wait().unwrap().is_some() {
            break;
        }
        time::sleep(Duration::from_millis(50)).await;
        answered = answered_by(&probe).await;
    }
    let _killed = sidecar.start_kill();
    let _waited = sidecar.wait().await;
    assert!(
        answered,
        "the sidecar binary never answered on its port with its own pid"
    );
}

// main exits with clap's usage code for a missing required argument, before any listener starts
#[tokio::test]
async fn the_binary_exits_with_2_when_the_folder_argument_is_missing() {
    let status = Command::new(sidecar_binary())
        .arg("--port")
        .arg("4001")
        .env_clear()
        .status()
        .await
        .unwrap();
    assert_eq!(status.code(), Some(2_i32));
}

#[tokio::test(start_paused = true)]
async fn parent_exited_waits_a_poll_interval_until_the_parent_changes() {
    let mut parents = [77, 77, 77, 1].into_iter();
    let started = Instant::now();
    parent_exited(77, || parents.next().unwrap(), PARENT_POLL_INTERVAL).await;
    assert_eq!(started.elapsed(), Duration::from_secs(3));
}

#[tokio::test(start_paused = true)]
async fn parent_exited_returns_at_once_when_already_reparented() {
    let started = Instant::now();
    parent_exited(77, || 1, PARENT_POLL_INTERVAL).await;
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test]
async fn the_startup_timeout_stops_the_listener_and_its_connections() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let mut serving = tasks.spawn(serve_bounded(
        listener,
        large_app(),
        stop.clone(),
        QUICK_GRACE,
    ));
    let stalled = stalled_client(address).await;
    let unanswered = probe_for("127.0.0.1", address.port(), PROBED_PID, QUICK);
    let error = wait_until_serving(&mut serving, &unanswered, &stop, QUICK_GRACE)
        .await
        .unwrap_err();
    assert!(matches!(error, MetricsServerError::Timeout { .. }));
    assert!(stop.is_cancelled());
    let ended = time::timeout(SHUTDOWN_GRACE, &mut serving).await;
    let _ended = ended.expect("the timed-out listener did not finish");
    expect_closed(stalled).await;
}

#[tokio::test]
async fn serve_bounded_closes_a_stalled_scrape_when_it_stops() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let mut serving = tasks.spawn(serve_bounded(
        listener,
        large_app(),
        stop.clone(),
        QUICK_GRACE,
    ));
    let stalled = stalled_client(address).await;
    stop.cancel();
    let outcome = time::timeout(SHUTDOWN_GRACE, &mut serving).await;
    outcome
        .expect("the listener drained past its grace")
        .expect("the listener task failed")
        .expect("serving failed");
    expect_closed(stalled).await;
}

#[tokio::test]
async fn serve_bounded_finishes_the_scrape_in_flight_before_it_closes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let signal = Arc::new(Notify::new());
    let handler = Arc::clone(&signal);
    let app = Router::new().route(
        METRICS_PATH,
        get(move || {
            let reached = Arc::clone(&handler);
            async move {
                reached.notify_one();
                time::sleep(Duration::from_millis(100)).await;
                "done"
            }
        }),
    );
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let mut serving = tasks.spawn(serve_bounded(listener, app, stop.clone(), SHUTDOWN_GRACE));
    let mut scrape = TcpStream::connect(address).await.unwrap();
    let request = format!("GET {METRICS_PATH} HTTP/1.1\r\nHost: {address}\r\n\r\n");
    scrape.write_all(request.as_bytes()).await.unwrap();
    let arrived = time::timeout(Duration::from_secs(5), signal.notified()).await;
    arrived.expect("the scrape never reached the handler");
    stop.cancel();
    let mut response = String::new();
    let read = time::timeout(Duration::from_secs(5), scrape.read_to_string(&mut response)).await;
    let _read = read.expect("the drained connection never closed");
    assert!(response.ends_with("done"), "{response}");
    let outcome = time::timeout(SHUTDOWN_GRACE, &mut serving).await;
    outcome
        .expect("the listener did not finish after draining")
        .expect("the listener task failed")
        .expect("serving failed");
}

// the parent is read before the app build, so an exit during startup is not missed as a reparenting
#[tokio::test]
async fn a_parent_that_exits_while_the_app_builds_still_stops_the_sidecar() {
    let folder = TempDir::new().unwrap();
    let sidecar = settings(folder.path(), 0);
    let parent = Arc::new(AtomicU32::new(77));
    let reparenting = Arc::clone(&parent);
    let built = Arc::new(AtomicBool::new(false));
    let built_flag = Arc::clone(&built);
    let make_app = move |directory: PathBuf, worker: u32| {
        built_flag.store(true, Ordering::SeqCst);
        reparenting.store(1, Ordering::SeqCst);
        build_metrics_app(&directory, worker)
    };
    let current = Arc::clone(&parent);
    let served =
        run_metrics_server_watched(&sidecar, make_app, move || current.load(Ordering::SeqCst));
    let outcome = time::timeout(Duration::from_secs(5), served).await;
    outcome
        .expect("the sidecar outlived the parent that exited while the app was built")
        .expect("the sidecar failed");
    assert!(built.load(Ordering::SeqCst));
}

#[tokio::test]
async fn serve_until_parent_exits_closes_a_stalled_scrape() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let parent = Arc::new(AtomicU32::new(77));
    let current = Arc::clone(&parent);
    let tasks = TaskTracker::new();
    let mut serving = tasks.spawn(serve_until_parent_exits(
        listener,
        large_app(),
        77,
        move || current.load(Ordering::SeqCst),
        QUICK_GRACE,
    ));
    let stalled = stalled_client(address).await;
    parent.store(1, Ordering::SeqCst);
    let outcome = time::timeout(Duration::from_secs(5), &mut serving).await;
    outcome
        .expect("the sidecar kept serving after its parent changed")
        .expect("the sidecar task failed")
        .expect("serving failed");
    expect_closed(stalled).await;
}

// uvicorn serves HTTP/1.1, so an HTTP/2 preface is refused instead of spawning request tasks the listener never owns
#[tokio::test]
async fn serve_bounded_refuses_an_http2_preface() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let tasks = TaskTracker::new();
    let mut serving = tasks.spawn(serve_bounded(
        listener,
        large_app(),
        stop.clone(),
        QUICK_GRACE,
    ));
    let mut stream = TcpStream::connect(address).await.unwrap();
    let preface = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
    stream.write_all(preface).await.unwrap();
    let mut received = Vec::new();
    let closed = time::timeout(Duration::from_secs(5), stream.read_to_end(&mut received)).await;
    let _read = closed.expect("the listener kept an HTTP/2 connection open");
    stop.cancel();
    let outcome = time::timeout(SHUTDOWN_GRACE, &mut serving).await;
    outcome
        .expect("the listener did not stop")
        .expect("the listener task failed")
        .expect("serving failed");
}

#[test]
fn cli_defaults_the_host_and_takes_the_port_and_folder() {
    let server = parsed(
        &["metrics", "--port", "4001", "--multiproc_dir", "/samples"],
        None,
    )
    .unwrap();
    assert_eq!(
        server,
        MetricsServerArgs {
            host: "0.0.0.0".to_owned(),
            port: 4001,
            multiproc_dir: PathBuf::from("/samples"),
        }
    );
}

#[test]
fn cli_takes_the_host_flag() {
    let arguments = [
        "metrics",
        "--host",
        "::",
        "--port",
        "9090",
        "--multiproc_dir",
        "/samples",
    ];
    let server = parsed(&arguments, None).unwrap();
    assert_eq!(server.host, "::");
    assert_eq!(server.port, 9090);
}

#[test]
fn cli_reads_the_folder_from_the_environment() {
    let server = parsed(&["metrics", "--port", "4001"], Some("/from/env")).unwrap();
    assert_eq!(server.multiproc_dir, Path::new("/from/env"));
}

#[test]
fn cli_prefers_the_folder_flag_over_the_environment() {
    let arguments = ["metrics", "--port", "4001", "--multiproc_dir", "/flag"];
    let server = parsed(&arguments, Some("/from/env")).unwrap();
    assert_eq!(server.multiproc_dir, Path::new("/flag"));
}

#[rstest]
#[case::no_folder(&["metrics", "--port", "4001"], None, ErrorKind::MissingRequiredArgument)]
#[case::empty_folder_flag(&["metrics", "--port", "4001", "--multiproc_dir", ""], None, ErrorKind::InvalidValue)]
#[case::empty_folder_env(&["metrics", "--port", "4001"], Some(""), ErrorKind::MissingRequiredArgument)]
#[case::no_port(&["metrics", "--multiproc_dir", "/samples"], None, ErrorKind::MissingRequiredArgument)]
#[case::port_not_a_number(&["metrics", "--port", "x", "--multiproc_dir", "/samples"], None, ErrorKind::ValueValidation)]
#[case::port_out_of_range(&["metrics", "--port", "70000", "--multiproc_dir", "/samples"], None, ErrorKind::ValueValidation)]
fn cli_refuses_a_missing_or_empty_value(
    #[case] arguments: &[&str],
    #[case] env_folder: Option<&str>,
    #[case] expected: ErrorKind,
) {
    let error = parsed(arguments, env_folder).unwrap_err();
    assert_eq!(error.kind(), expected);
    assert_eq!(error.exit_code(), 2_i32);
}
