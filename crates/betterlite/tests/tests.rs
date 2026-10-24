#![cfg(test)]
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
    let server = spawn_server(&config_path, port);
    wait_until_ready(port, 100).await;
    let mut request = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    request
        .write_all(b"GET /not-served HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .unwrap();
    sleep(Duration::from_millis(10)).await;
    let process_id = server.child.as_ref().unwrap().id().unwrap();
    let status = Command::new("kill")
        .args(["-TERM", &process_id.to_string()])
        .status()
        .await
        .unwrap();
    assert!(status.success());
    request
        .write_all(b"Authorization: Bearer test-master\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    request.read_to_end(&mut response).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 404"));
    assert!(response.ends_with(b"{\"detail\":\"Not Found\"}"));
    let stderr = server.wait().await;
    assert_json_logs(&stderr);
}
#[tokio::test]
async fn dropping_server_guard_kills_server() {
    let port = free_port().await;
    let config_path = fixture(port, STANDARD_CONFIG).await;
    let server = spawn_server(&config_path, port);
    wait_until_ready(port, 100).await;
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
fn assert_json_logs(stderr: &[u8]) {
    let mut listening = false;
    for line in String::from_utf8(stderr.to_owned()).unwrap().lines() {
        let event: LogEvent = serde_json::from_str(line).unwrap();
        listening |= !event.timestamp.is_empty()
            && event.level == "INFO"
            && event.message == "betterlite server listening";
    }
    assert!(listening);
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
    async fn wait(mut self) -> Vec<u8> {
        let output = self.child.take().unwrap().wait_with_output().await.unwrap();
        assert!(output.status.success());
        output.stderr
    }
}
