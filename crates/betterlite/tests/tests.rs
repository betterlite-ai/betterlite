#![cfg(test)]
use std::env;
use std::path::{Path, PathBuf};
use std::process::{self, Stdio};
use std::time::Duration;
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
#[tokio::test]
async fn sigterm_finishes_in_flight_request_and_writes_flat_json_logs() {
    let port = free_port().await;
    let config_path = fixture(port, STANDARD_CONFIG).await;
    let mut server = spawn_server(&config_path, port);
    wait_until_ready(port, 1000).await;
    let mut request = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut health_request = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut readiness_request = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    request
        .write_all(b"GET /not-served HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    health_request
        .write_all(b"GET /health/liveliness HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    readiness_request
        .write_all(b"GET /health/readiness HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    sleep(Duration::from_millis(10)).await;
    let (mut logs, mut lines) = server.terminate().await;
    assert_shutdown_response(&mut health_request).await;
    assert_shutdown_response(&mut readiness_request).await;
    request
        .write_all(b"Authorization: Bearer test-master\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    request.read_to_end(&mut response).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 404"));
    assert!(response.ends_with(b"{\"detail\":\"Not Found\"}"));
    while let Some(line) = lines.next_line().await.unwrap() {
        logs.push(line);
    }
    assert!(server.wait().await.is_empty());
    assert_json_logs(logs.join("\n").as_bytes());
}
async fn assert_shutdown_response(request: &mut TcpStream) {
    request
        .write_all(b"Connection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    request.read_to_end(&mut response).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 503"));
    assert!(response.ends_with(b"{\"status\":\"shutting_down\"}"));
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
    wait_until_ready(port, 1000).await;
    drop(server);
}
#[tokio::test]
async fn dropping_server_guard_kills_server() {
    let port = free_port().await;
    let config_path = fixture(port, STANDARD_CONFIG).await;
    let server = spawn_server(&config_path, port);
    wait_until_ready(port, 1000).await;
    let connection = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    timeout(Duration::from_secs(3), connection.readable())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(connection.try_read(&mut [0]).unwrap(), 0);
    drop(server);
    sleep(Duration::from_millis(100)).await;
    TcpStream::connect(("127.0.0.1", port)).await.unwrap_err();
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
async fn partial_headers_are_closed_after_the_read_deadline() {
    let port = free_port().await;
    let config_path = fixture(port, STANDARD_CONFIG).await;
    let server = spawn_server(&config_path, port);
    wait_until_ready(port, 1000).await;
    let mut connection = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    connection
        .write_all(b"GET /health/liveliness HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    timeout(
        Duration::from_secs(5),
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
async fn wait_until_ready(port: u16, attempts: u16) {
    for _ in 0..attempts {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("betterlite did not start on port {port}");
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
