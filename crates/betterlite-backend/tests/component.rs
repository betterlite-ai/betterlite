use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde::Deserialize;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

const BINARY: &str = env!("CARGO_BIN_EXE_betterlite-backend");

#[derive(Deserialize)]
struct LogLine {
    timestamp: String,
    level: String,
    message: String,
}

fn config_file() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.yaml");
    std::fs::write(&path, "model_list: []\n").unwrap();
    (directory, path)
}

fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

async fn wait_until_listening(port: u16) {
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the server never started listening on {port}");
}

#[tokio::test]
async fn shutdown_exits_zero_on_sigterm() {
    let (_directory, config) = config_file();
    let port = free_port();
    let mut child = Command::new(BINARY)
        .args([
            "--config",
            config.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until_listening(port).await;
    kill(
        Pid::from_raw(i32::try_from(child.id()).unwrap()),
        Signal::SIGTERM,
    )
    .unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

#[tokio::test]
async fn startup_failure_exits_two_on_a_taken_port() {
    let (_directory, config) = config_file();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let output = Command::new(BINARY)
        .args([
            "--config",
            config.to_str().unwrap(),
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[tokio::test]
async fn stderr_json_lines_on_a_bad_config() {
    let missing = std::env::temp_dir().join("betterlite-backend-no-such-config.yaml");
    let output = Command::new(BINARY)
        .args(["--config", missing.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stderr).unwrap();
    let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    assert!(!lines.is_empty());
    for line in lines {
        let parsed: LogLine = serde_json::from_str(line).unwrap();
        assert!(!parsed.timestamp.is_empty());
        assert!(!parsed.level.is_empty());
        assert!(!parsed.message.is_empty());
    }
}
