#![cfg(test)]
use serde::Deserialize;
use std::env;
use std::fs;
use std::future::Future;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{self, Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;
#[derive(Deserialize)]
struct LogEvent {
    timestamp: String,
    level: String,
    message: String,
}
#[tokio::test]
async fn sigterm_finishes_in_flight_request_and_writes_flat_json_logs() {
    let directory = test_directory();
    fs::create_dir_all(&directory).unwrap();
    let config_path = directory.join("config.yaml");
    fs::write(
        &config_path,
        "general_settings:\n  master_key: test-master\n",
    )
    .unwrap();
    let port = free_port();
    let mut child = spawn_server(&config_path, port);
    wait_until_ready(port, 100).await;
    let mut request = TcpStream::connect(("127.0.0.1", port)).unwrap();
    request
        .write_all(
            b"GET /not-served HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer test-master\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    send_sigterm(child.id());
    let mut response = Vec::new();
    request.read_to_end(&mut response).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success());
    assert!(response.starts_with(b"HTTP/1.1 404"));
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    String::from_utf8(stderr).unwrap().lines().for_each(|line| {
        let event: LogEvent = serde_json::from_str(line).unwrap();
        assert!(!event.timestamp.is_empty(), "{line}");
        assert!(!event.level.is_empty(), "{line}");
        assert!(!event.message.is_empty(), "{line}");
    });
    fs::remove_dir_all(directory).unwrap();
}
fn test_directory() -> PathBuf {
    env::temp_dir().join(format!("betterlite-b003-{}", process::id()))
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
fn free_port() -> u16 {
    let socket = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    socket.local_addr().unwrap().port()
}
fn spawn_server(config_path: &Path, port: u16) -> process::Child {
    Command::new(binary_path())
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
        .unwrap()
}
fn wait_until_ready(port: u16, attempts: u16) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            stream
                .write_all(b"GET /health/liveliness HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            return;
        }
        assert!(attempts > 0, "betterlite did not start on port {port}");
        sleep(Duration::from_millis(10)).await;
        let remaining = attempts.checked_sub(1).unwrap();
        wait_until_ready(port, remaining).await;
    })
}
fn send_sigterm(process_id: u32) {
    let status = Command::new("kill")
        .args(["-TERM", &process_id.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}
