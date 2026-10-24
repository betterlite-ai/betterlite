#![cfg(test)]
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{self, Stdio};
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};
const STANDARD_CONFIG: &str = "general_settings:\n  master_key: test-master\n";
#[derive(Deserialize)]
struct LogEvent {
    timestamp: String,
    level: String,
    message: String,
}
#[tokio::test]
async fn sigterm_finishes_in_flight_request_and_writes_flat_json_logs() {
    let port = free_port().await;
    let (directory, config_path) = fixture(port, STANDARD_CONFIG).await;
    let server = spawn_server(&config_path, port);
    wait_until_ready(port, 100).await;
    let mut request = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    request
        .write_all(b"GET /not-served HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    send_sigterm(server.child.as_ref().unwrap().id().unwrap()).await;
    sleep(Duration::from_millis(100)).await;
    request
        .write_all(b"Authorization: Bearer test-master\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    request.read_to_end(&mut response).await.unwrap();
    let (status, stderr) = server.wait().await;
    assert!(status);
    assert!(response.starts_with(b"HTTP/1.1 404"));
    assert_json_logs(&stderr);
    fs::remove_dir_all(directory).await.unwrap();
}
#[tokio::test]
async fn incomplete_headers_are_closed_by_the_header_read_deadline() {
    let port = free_port().await;
    let (directory, config_path) = fixture(port, STANDARD_CONFIG).await;
    let server = spawn_server(&config_path, port);
    wait_until_ready(port, 100).await;
    let mut request = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    request
        .write_all(b"GET /not-served HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    timeout(Duration::from_secs(3), request.read_to_end(&mut Vec::new()))
        .await
        .unwrap()
        .unwrap();
    send_sigterm(server.child.as_ref().unwrap().id().unwrap()).await;
    let _result = server.wait().await;
    fs::remove_dir_all(directory).await.unwrap();
}
#[tokio::test]
async fn malformed_config_logs_no_secret() {
    let port = free_port().await;
    let secret = "config-secret-must-not-be-logged";
    let (directory, config_path) = fixture(
        port,
        &format!("general_settings:\n  master_key: {secret}\n  broken: [\n"),
    )
    .await;
    let output = Command::new(binary_path())
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
    fs::remove_dir_all(directory).await.unwrap();
}
fn assert_json_logs(stderr: &[u8]) {
    for line in String::from_utf8(stderr.to_owned()).unwrap().lines() {
        let event: LogEvent = serde_json::from_str(line).unwrap();
        assert!(
            !event.timestamp.is_empty() && !event.level.is_empty() && !event.message.is_empty(),
            "{line}"
        );
    }
}
async fn fixture(port: u16, source: &str) -> (PathBuf, PathBuf) {
    let directory = env::temp_dir().join(format!("betterlite-b003-{}-{port}", process::id()));
    fs::create_dir_all(&directory).await.unwrap();
    let config_path = directory.join("config.yaml");
    fs::write(&config_path, source).await.unwrap();
    (directory, config_path)
}
fn binary_path() -> PathBuf {
    let mut test_binary = env::current_exe().unwrap();
    test_binary.pop();
    test_binary.pop();
    test_binary.push("betterlite");
    test_binary
}
async fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    listener.local_addr().unwrap().port()
}
fn spawn_server(config_path: &Path, port: u16) -> ServerGuard {
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
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    ServerGuard { child: Some(child) }
}
async fn wait_until_ready(port: u16, attempts: u16) {
    for _ in 0..attempts {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)).await {
            stream
                .write_all(
                    b"GET /health/liveliness HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("betterlite did not start on port {port}");
}
async fn send_sigterm(process_id: u32) {
    let status = Command::new("kill")
        .args(["-TERM", &process_id.to_string()])
        .status()
        .await
        .unwrap();
    assert!(status.success());
}
struct ServerGuard {
    child: Option<Child>,
}
impl ServerGuard {
    async fn wait(mut self) -> (bool, Vec<u8>) {
        let child = self.child.take().unwrap();
        let output = child.wait_with_output().await.unwrap();
        (output.status.success(), output.stderr)
    }
}
