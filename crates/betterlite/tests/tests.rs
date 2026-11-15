#![cfg(test)]
use std::env;
use std::path::{Path, PathBuf};
use std::process::{self, Stdio};
use std::time::{Duration, Instant};
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
    let port = free_port().await;
    let config = format!(
        "{STANDARD_CONFIG}model_list:\n  - model_name: held\n    litellm_params:\n      model: openai/held\n      api_key: sk-held\n      api_base: http://127.0.0.1:{upstream_port}/v1\n"
    );
    let config_path = fixture(port, &config).await;
    let mut server = spawn_server(&config_path, port);
    wait_for_port(port, true).await;
    let mut request = start_chat(port).await;
    // the upstream holds the call, so SIGTERM lands while the request is in flight however slow the runner is
    let (mut held, _address) = upstream.accept().await.unwrap();
    read_request(&mut held).await;
    let (mut logs, mut lines) = server.terminate().await;
    answer_chat(&mut held).await;
    let mut bytes = Vec::new();
    request.read_to_end(&mut bytes).await.unwrap();
    let response = String::from_utf8(bytes).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains(r#""content":"done""#), "{response}");
    while let Some(line) = lines.next_line().await.unwrap() {
        logs.push(line);
    }
    assert!(server.wait().await.is_empty());
    assert_json_logs(logs.join("\n").as_bytes());
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
    let port = free_port().await;
    let config_path = fixture(port, STANDARD_CONFIG).await;
    let child = Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--"])
        .arg(binary_path())
        .args([
            "-c",
            config_path.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let server = ServerGuard { child: Some(child) };
    wait_for_port(port, true).await;
    drop(server);
}
#[tokio::test]
async fn dropping_server_guard_kills_server() {
    let port = free_port().await;
    let config_path = fixture(port, STANDARD_CONFIG).await;
    let server = spawn_server(&config_path, port);
    wait_for_port(port, true).await;
    let connection = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    timeout(Duration::from_secs(30), connection.readable())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(connection.try_read(&mut [0]).unwrap(), 0);
    drop(server);
    wait_for_port(port, false).await;
}
#[tokio::test]
async fn malformed_config_does_not_log_master_key() {
    let port = free_port().await;
    let secret = "config-secret-must-not-be-logged";
    let config_path = fixture(
        port,
        &format!("general_settings:\n  master_key: {secret}\n  broken: [\n"),
    )
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
            &port.to_string(),
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
    let port = free_port().await;
    let config_path = fixture(
        port,
        "general_settings:\n  master_key: test-master\n  unsupported: true\n",
    )
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
            &port.to_string(),
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
    let port = free_port().await;
    let config_path = fixture(
        port,
        "general_settings:\n  master_key: os.environ/MISSING_MASTER_KEY\n",
    )
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
            &port.to_string(),
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
    let port = free_port().await;
    let config_path = fixture(
        port,
        "general_settings:\n  master_key: os.environ/B004_MASTER\n",
    )
    .await;
    let child = Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--"])
        .arg(binary_path())
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .env("B004_MASTER", "environment-master")
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let server = ServerGuard { child: Some(child) };
    wait_for_port(port, true).await;
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
    let port = free_port().await;
    let source = "model_list:\n  - {model_name: g, litellm_params: {model: openai/m, api_base: 'not a url'}}\n";
    let config_path = fixture(port, source).await;
    let output = Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--"])
        .arg(binary_path())
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
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
    let port = free_port().await;
    let config_path = fixture(port, STANDARD_CONFIG).await;
    let server = spawn_server(&config_path, port);
    wait_for_port(port, true).await;
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
fn assert_json_logs(stderr: &[u8]) {
    let events = String::from_utf8(stderr.to_owned())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<LogEvent>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| {
        !event.timestamp.is_empty() && !event.level.is_empty() && !event.message.is_empty()
    }));
}
async fn fixture(port: u16, source: &str) -> PathBuf {
    let directory = env::temp_dir().join(format!("betterlite-b003-{}-{port}", process::id()));
    fs::create_dir_all(&directory).await.unwrap();
    let config_path = directory.join("config.yaml");
    fs::write(&config_path, source).await.unwrap();
    config_path
}
async fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    listener.local_addr().unwrap().port()
}
fn binary_path() -> PathBuf {
    let mut test_binary = env::current_exe().unwrap();
    test_binary.pop();
    test_binary.set_file_name("betterlite");
    test_binary
}
fn spawn_server(config_path: &Path, port: u16) -> ServerGuard {
    let mut executable = env::current_exe().unwrap();
    executable.pop();
    executable.set_file_name("betterlite");
    let child = Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--"])
        .arg(executable)
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    ServerGuard { child: Some(child) }
}
async fn send_sigterm(process_id: u32) {
    let status = Command::new("kill")
        .args(["-TERM", &process_id.to_string()])
        .status()
        .await
        .unwrap();
    assert!(status.success());
}
async fn wait_for_port(port: u16, reachable: bool) {
    let started = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).await.is_ok() != reachable {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "port {port} did not become reachable={reachable}"
        );
        sleep(Duration::from_millis(10)).await;
    }
}
struct ServerGuard {
    child: Option<Child>,
}
impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _result = child.start_kill();
        }
    }
}
impl ServerGuard {
    // requests in flight must finish after the server has seen SIGTERM, or they race its shutdown flag
    async fn terminate(&mut self) -> (Vec<String>, Lines<BufReader<ChildStderr>>) {
        let child = self.child.as_mut().unwrap();
        send_sigterm(child.id().unwrap()).await;
        let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
        let mut logs = Vec::new();
        while !logs
            .last()
            .is_some_and(|line: &String| line.contains("betterlite server shutting down"))
        {
            let line = timeout(Duration::from_secs(10), lines.next_line())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            logs.push(line);
        }
        (logs, lines)
    }
    async fn wait(mut self) -> Vec<u8> {
        let output = self.child.take().unwrap().wait_with_output().await.unwrap();
        assert!(output.status.success());
        output.stderr
    }
}
