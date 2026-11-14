#![cfg(test)]
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde::Deserialize;
use std::env;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::time::sleep;

const BINARY: &str = env!("CARGO_BIN_EXE_betterlite-backend");
const EMPTY_CONFIG: &str = "model_list: []\n";

#[derive(Deserialize)]
struct LogLine {
    timestamp: String,
    level: String,
    message: String,
    category: Option<String>,
}

fn config_file(source: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.yaml");
    fs::write(&path, source).unwrap();
    (directory, path)
}

fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

fn command(config: &Path, port: u16) -> Command {
    let mut command = Command::new(BINARY);
    command.args([
        "--config",
        config.to_str().unwrap(),
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ]);
    command
}

fn terminate(mut child: Child) -> Option<i32> {
    kill(
        Pid::from_raw(i32::try_from(child.id()).unwrap()),
        Signal::SIGTERM,
    )
    .unwrap();
    child.wait().unwrap().code()
}

fn stderr_lines(stderr: Vec<u8>) -> Vec<LogLine> {
    let text = String::from_utf8(stderr).unwrap();
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

async fn wait_until_listening(port: u16) {
    for _ in 0_u32..200_u32 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("the server never started listening on {port}");
}

async fn raw_response(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

async fn status_and_body(port: u16, path: &str) -> (u16, String) {
    let response = raw_response(port, path).await;
    let status = response.split(' ').nth(1).unwrap().parse().unwrap();
    let body = response.split_once("\r\n\r\n").unwrap().1.to_owned();
    (status, body)
}

#[tokio::test]
async fn shutdown_exits_zero_on_sigterm() {
    let (_directory, config) = config_file(EMPTY_CONFIG);
    let port = free_port();
    let child = command(&config, port)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until_listening(port).await;
    assert_eq!(terminate(child), Some(0_i32));
}

#[tokio::test]
async fn shutdown_exits_zero_with_a_config_that_reads_the_process_environment() {
    let (_directory, config) =
        config_file("general_settings:\n  master_key: os.environ/B4471_MASTER\n");
    let port = free_port();
    let child = command(&config, port)
        .env("B4471_MASTER", "environment-master")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until_listening(port).await;
    assert_eq!(terminate(child), Some(0_i32));
}

#[tokio::test]
async fn startup_failure_exits_two_on_a_taken_port() {
    let (_directory, config) = config_file(EMPTY_CONFIG);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let output = command(&config, port).output().unwrap();
    assert_eq!(output.status.code(), Some(2_i32));
}

#[tokio::test]
async fn stderr_json_lines_on_a_bad_config() {
    let missing = env::temp_dir().join("betterlite-backend-no-such-config.yaml");
    let output = Command::new(BINARY)
        .args(["--config", missing.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1_i32));
    let lines = stderr_lines(output.stderr);
    assert!(!lines.is_empty());
    assert!(lines.iter().all(|line| {
        !line.timestamp.is_empty() && !line.level.is_empty() && !line.message.is_empty()
    }));
}

#[tokio::test]
async fn stderr_json_names_the_category_of_a_config_error() {
    let (_directory, config) =
        config_file("general_settings:\n  master_key: os.environ/B4471_UNSET\n");
    let output = command(&config, free_port()).output().unwrap();
    assert_eq!(output.status.code(), Some(1_i32));
    let lines = stderr_lines(output.stderr);
    assert!(
        lines
            .iter()
            .any(|line| line.category.as_deref() == Some("missing_environment"))
    );
}

#[tokio::test]
async fn shutdown_exits_zero_after_filtering_requests_by_path() {
    let (_directory, config) = config_file(EMPTY_CONFIG);
    let port = free_port();
    let child = command(&config, port)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until_listening(port).await;
    let (blocked_status, blocked_body) = status_and_body(port, "/not-a-route").await;
    assert_eq!(blocked_status, 404);
    assert!(blocked_body.contains("Not Found"));
    let (allowed_status, _) = status_and_body(port, "/health/liveness").await;
    assert_eq!(allowed_status, 200);
    let (chat_status, chat_body) = status_and_body(port, "/v1/chat/completions").await;
    assert_eq!(chat_status, 404);
    assert!(chat_body.contains("Not Found"));
    assert_eq!(terminate(child), Some(0_i32));
}

#[tokio::test]
async fn blocked_route_404_carries_the_security_headers() {
    let (_directory, config) = config_file(EMPTY_CONFIG);
    let port = free_port();
    let child = command(&config, port)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until_listening(port).await;
    let blocked = raw_response(port, "/v1/chat/completions").await;
    let missing = raw_response(port, "/not-a-route").await;
    assert!(blocked.starts_with("HTTP/1.1 404"));
    assert!(
        blocked
            .to_lowercase()
            .contains("x-content-type-options: nosniff")
    );
    assert!(
        missing
            .to_lowercase()
            .contains("x-content-type-options: nosniff")
    );
    assert_eq!(terminate(child), Some(0_i32));
}
