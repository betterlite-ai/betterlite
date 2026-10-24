#![cfg(test)]
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{self, ExitStatus, Stdio};
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};
#[derive(Deserialize)]
struct LogEvent {
    timestamp: String,
    level: String,
    message: String,
}
#[tokio::test]
async fn sigterm_finishes_in_flight_request_and_writes_flat_json_logs() {
    let port = free_port().await;
    let (directory, config_path) =
        fixture(port, "general_settings:\n  master_key: test-master\n").await;
    let server = spawn_server(&config_path, port);
    wait_until_ready(port, 100).await;
    let mut request = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    request
        .write_all(
            b"GET /not-served HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer test-master\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    send_sigterm(server.id()).await;
    let mut response = Vec::new();
    request.read_to_end(&mut response).await.unwrap();
    let (status, stderr) = server.wait().await;
    assert!(status.success());
    assert!(response.starts_with(b"HTTP/1.1 404"));
    assert_json_logs(&stderr);
    fs::remove_dir_all(directory).await.unwrap();
}
#[tokio::test]
async fn incomplete_headers_are_closed_by_the_header_read_deadline() {
    let port = free_port().await;
    let (directory, config_path) =
        fixture(port, "general_settings:\n  master_key: test-master\n").await;
    let mut server = spawn_server(&config_path, port);
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
    server.stop().await;
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
#[tokio::test]
async fn server_guard_drop_kills_and_reaps_the_child() {
    let port = free_port().await;
    let (directory, config_path) =
        fixture(port, "general_settings:\n  master_key: test-master\n").await;
    let server = spawn_server(&config_path, port);
    wait_until_ready(port, 100).await;
    let process_id = server.id();
    drop(server);
    assert!(wait_until_gone(process_id, 100).await);
    fs::remove_dir_all(directory).await.unwrap();
}
fn assert_json_logs(stderr: &[u8]) {
    let text = String::from_utf8(stderr.to_owned()).unwrap();
    for line in text.lines() {
        let event: LogEvent = serde_json::from_str(line).unwrap();
        assert!(!event.timestamp.is_empty(), "{line}");
        assert!(!event.level.is_empty(), "{line}");
        assert!(!event.message.is_empty(), "{line}");
    }
}
fn test_directory(port: u16) -> PathBuf {
    env::temp_dir().join(format!("betterlite-b003-{}-{port}", process::id()))
}
async fn fixture(port: u16, source: &str) -> (PathBuf, PathBuf) {
    let directory = test_directory(port);
    fs::create_dir_all(&directory).await.unwrap();
    let config_path = directory.join("config.yaml");
    fs::write(&config_path, source).await.unwrap();
    (directory, config_path)
}
fn binary_path() -> PathBuf {
    let test_binary = env::current_exe().unwrap();
    test_binary
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("betterlite")
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
        .stdout(Stdio::null())
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
async fn wait_until_gone(process_id: u32, attempts: u16) -> bool {
    for _ in 0..attempts {
        let status = Command::new("kill")
            .args(["-0", &process_id.to_string()])
            .stderr(Stdio::null())
            .status()
            .await
            .unwrap();
        if !status.success() {
            return true;
        }
        sleep(Duration::from_millis(10)).await;
    }
    false
}
struct ServerGuard {
    child: Option<Child>,
}
impl ServerGuard {
    async fn wait(mut self) -> (ExitStatus, Vec<u8>) {
        let mut child = self.child.take().unwrap();
        let status = child.wait().await.unwrap();
        let mut stderr = Vec::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_end(&mut stderr)
            .await
            .unwrap();
        (status, stderr)
    }
    async fn stop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            child.start_kill().unwrap();
        }
        let child = self.child.take().unwrap();
        child.wait_with_output().await.unwrap();
    }
    fn id(&self) -> u32 {
        self.child.as_ref().unwrap().id().unwrap()
    }
}
impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _result = child.start_kill();
        }
    }
}
