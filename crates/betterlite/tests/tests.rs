#![cfg(test)]
use betterlite_server::server::test_support::port_in_line;
use std::env;
use std::mem::take;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::fs;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, ChildStderr, Command};
use tokio::time::{sleep, timeout};
const STANDARD_CONFIG: &str = "general_settings:\n  master_key: test-master\n";
#[derive(serde::Deserialize)]
struct LogEvent {
    timestamp: String,
    level: String,
    message: String,
}
const CHAT_BODY: &str = r#"{"model":"held","messages":[{"role":"user","content":"hi"}]}"#;
const CHAT_REPLY: &str = r#"{"id":"chatcmpl-held","object":"chat.completion","created":1,"model":"held","choices":[{"index":0,"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
#[tokio::test]
async fn sigterm_finishes_in_flight_request_and_writes_flat_json_logs() {
    let upstream = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let config = format!(
        "{STANDARD_CONFIG}model_list:\n  - model_name: held\n    litellm_params:\n      model: openai/held\n      api_key: sk-held\n      api_base: http://127.0.0.1:{upstream_port}/v1\n"
    );
    let (_directory, config_path) = fixture(&config).await;
    let (mut server, port) = start_server("--config", &config_path, &[]).await;
    let mut request = start_chat(port).await;
    // the upstream holds the call, so SIGTERM lands while the request is in flight however slow the runner is
    let (mut held, _address) = upstream.accept().await.unwrap();
    read_request(&mut held).await;
    server.terminate().await;
    answer_chat(&mut held).await;
    let mut bytes = Vec::new();
    request.read_to_end(&mut bytes).await.unwrap();
    let response = String::from_utf8(bytes).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains(r#""content":"done""#), "{response}");
    assert_json_logs(&server.finish().await);
}
async fn start_chat(port: u16) -> TcpStream {
    let mut request = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    request
        .write_all(
            format!(
                "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer test-master\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{CHAT_BODY}",
                CHAT_BODY.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    request
}
async fn answer_chat(held: &mut TcpStream) {
    held.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{CHAT_REPLY}",
            CHAT_REPLY.len()
        )
        .as_bytes(),
    )
    .await
    .unwrap();
}
// closing a socket with unread bytes resets it, so the whole call is read before the reply
async fn read_request(upstream: &mut TcpStream) {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = upstream.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "the server closed its upstream call");
        received.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8(received.clone()).unwrap();
        let Some((head, body)) = text.split_once("\r\n\r\n") else {
            continue;
        };
        let length = head
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _value)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_name, value)| value.trim().parse::<usize>().unwrap())
            .unwrap();
        if body.len() >= length {
            return;
        }
    }
}
#[tokio::test]
async fn short_config_alias_starts_server() {
    let (_directory, config_path) = fixture(STANDARD_CONFIG).await;
    let (server, _port) = start_server("-c", &config_path, &[]).await;
    drop(server);
}
#[tokio::test]
async fn dropping_server_guard_kills_server() {
    let (_directory, config_path) = fixture(STANDARD_CONFIG).await;
    let (server, port) = start_server("--config", &config_path, &[]).await;
    let connection = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    timeout(Duration::from_secs(30), connection.readable())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(connection.try_read(&mut [0]).unwrap(), 0);
    drop(server);
    wait_until_closed(port).await;
}
#[tokio::test]
async fn malformed_config_does_not_log_master_key() {
    let secret = "config-secret-must-not-be-logged";
    let (_directory, config_path) = fixture(&format!(
        "general_settings:\n  master_key: {secret}\n  broken: [\n"
    ))
    .await;
    let output = Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--"])
        .arg(binary_path())
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains(secret));
}
#[tokio::test]
async fn unsupported_config_logs_paths() {
    let (_directory, config_path) =
        fixture("general_settings:\n  master_key: test-master\n  unsupported: true\n").await;
    let output = Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--"])
        .arg(binary_path())
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unsupported_key"));
    assert!(stderr.contains("general_settings.unsupported"));
}
#[tokio::test]
async fn missing_environment_logs_variable_and_path() {
    let (_directory, config_path) =
        fixture("general_settings:\n  master_key: os.environ/MISSING_MASTER_KEY\n").await;
    let output = Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--"])
        .arg(binary_path())
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("missing_environment"));
    assert!(stderr.contains("MISSING_MASTER_KEY"));
    assert!(stderr.contains("general_settings.master_key"));
}
#[tokio::test]
async fn reads_config_references_from_the_process_environment() {
    let (_directory, config_path) =
        fixture("general_settings:\n  master_key: os.environ/B004_MASTER\n").await;
    let (server, port) = start_server(
        "--config",
        &config_path,
        &[("B004_MASTER", "environment-master")],
    )
    .await;
    let mut request = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    request
        .write_all(b"GET /v1/models HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer environment-master\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    request.read_to_end(&mut response).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200"));
    drop(server);
}
#[tokio::test]
async fn unusable_chat_deployment_stops_startup() {
    let source = "model_list:\n  - {model_name: g, litellm_params: {model: openai/m, api_base: 'not a url'}}\n";
    let (_directory, config_path) = fixture(source).await;
    let output = Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--"])
        .arg(binary_path())
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            "0",
        ])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("failed to set up chat completions"));
    assert!(stderr.contains("model_list entry g"));
}
#[tokio::test]
async fn partial_headers_are_closed_after_the_read_deadline() {
    let (_directory, config_path) = fixture(STANDARD_CONFIG).await;
    let (server, port) = start_server("--config", &config_path, &[]).await;
    let mut connection = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    connection
        .write_all(b"GET /health/liveliness HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    timeout(
        Duration::from_secs(30),
        connection.read_to_end(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    drop(server);
}
fn assert_json_logs(lines: &[String]) {
    let events = lines
        .iter()
        .map(|line| serde_json::from_str::<LogEvent>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| {
        !event.timestamp.is_empty() && !event.level.is_empty() && !event.message.is_empty()
    }));
}
async fn fixture(source: &str) -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("config.yaml");
    fs::write(&config_path, source).await.unwrap();
    (directory, config_path)
}
fn binary_path() -> PathBuf {
    let mut test_binary = env::current_exe().unwrap();
    test_binary.pop();
    test_binary.set_file_name("betterlite");
    test_binary
}
// port 0 lets the kernel pick, and the server logs the port it got on its listening line
async fn start_server(
    config_flag: &str,
    config_path: &Path,
    environment: &[(&str, &str)],
) -> (ServerGuard, u16) {
    let mut child = Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--"])
        .arg(binary_path())
        .args([
            config_flag,
            config_path.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            "0",
        ])
        .envs(environment.iter().copied())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let mut server = ServerGuard {
        child: Some(child),
        lines,
        logs: Vec::new(),
    };
    let port = server.read_listening_port().await;
    (server, port)
}
async fn send_sigterm(process_id: u32) {
    let status = Command::new("kill")
        .args(["-TERM", &process_id.to_string()])
        .status()
        .await
        .unwrap();
    assert!(status.success());
}
async fn wait_until_closed(port: u16) {
    let started = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "port {port} did not close"
        );
        sleep(Duration::from_millis(10)).await;
    }
}
struct ServerGuard {
    child: Option<Child>,
    lines: Lines<BufReader<ChildStderr>>,
    logs: Vec<String>,
}
impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _result = child.start_kill();
        }
    }
}
impl ServerGuard {
    async fn next_log_line(&mut self) -> Option<String> {
        let line = timeout(Duration::from_secs(60), self.lines.next_line())
            .await
            .unwrap()
            .unwrap()?;
        self.logs.push(line.clone());
        Some(line)
    }
    async fn read_listening_port(&mut self) -> u16 {
        loop {
            let line = self
                .next_log_line()
                .await
                .expect("the server closed stderr before its listening line");
            if let Some(port) = port_in_line(&line) {
                return port;
            }
        }
    }
    // requests in flight must finish after the server has seen SIGTERM, or they race its shutdown flag
    async fn terminate(&mut self) {
        send_sigterm(self.child.as_ref().unwrap().id().unwrap()).await;
        while !self
            .logs
            .last()
            .is_some_and(|line| line.contains("betterlite server shutting down"))
        {
            self.next_log_line().await.unwrap();
        }
    }
    async fn finish(mut self) -> Vec<String> {
        while self.next_log_line().await.is_some() {}
        let status = self.child.take().unwrap().wait().await.unwrap();
        assert!(status.success());
        take(&mut self.logs)
    }
}
