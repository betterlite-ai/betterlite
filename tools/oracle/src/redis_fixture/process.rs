use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use redis::{Client, cmd};
use secrecy::{ExposeSecret as _, SecretString};

use super::RedisFixtureError;

const PROBE_TIMEOUT: Duration = Duration::from_millis(100);
const START_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn launch(
    directory: &Path,
    address: SocketAddr,
    password: &SecretString,
) -> Result<Child, RedisFixtureError> {
    Command::new("redis-server")
        .args(["--bind", "127.0.0.1", "--port"])
        .arg(address.port().to_string())
        .arg("--dir")
        .arg(directory)
        .args(["--save", "", "--appendonly", "no", "--daemonize", "no"])
        .args(["--protected-mode", "yes", "--requirepass"])
        .arg(password.expose_secret())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(RedisFixtureError::Start)
}

pub(super) fn wait_ready(server: &mut Child, client: &Client) -> Result<(), RedisFixtureError> {
    let started = Instant::now();
    while started.elapsed() < START_TIMEOUT {
        if let Some(status) = server.try_wait().map_err(RedisFixtureError::Start)? {
            return Err(RedisFixtureError::Exited(status));
        }
        if probe(client, server.id()).unwrap_or(false) {
            return Ok(());
        }
        thread::park_timeout(Duration::from_millis(10));
    }
    Err(RedisFixtureError::ReadinessTimeout)
}

fn probe(client: &Client, pid: u32) -> Result<bool, redis::RedisError> {
    let mut connection = client.get_connection_with_timeout(PROBE_TIMEOUT)?;
    connection.set_read_timeout(Some(PROBE_TIMEOUT))?;
    connection.set_write_timeout(Some(PROBE_TIMEOUT))?;
    let pong = cmd("PING").query::<String>(&mut connection)?;
    let info = cmd("INFO").arg("server").query::<String>(&mut connection)?;
    Ok(pong == "PONG" && info.lines().any(|line| line == format!("process_id:{pid}")))
}
