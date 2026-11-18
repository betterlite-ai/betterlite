use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use redis::Client;
use redis::cmd;
use tempfile::{Builder, TempDir};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

use crate::error::RedisFixtureError;

pub(crate) const REDIS_SERVER: &str = "redis-server";
pub(crate) const STARTUP_LIMIT: Duration = Duration::from_secs(5);
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(2);

#[derive(Debug)]
pub struct RedisFixture {
    // The URL is the whole interface a test gets, so it can never reach a Redis on the machine.
    pub url: String,
    pub(crate) child: Child,
    log_path: PathBuf,
    _folder: TempDir,
}

#[derive(Clone)]
pub(crate) struct StartOptions {
    pub(crate) binary: String,
    pub(crate) startup_limit: Duration,
    pub(crate) extra_args: Vec<String>,
}

impl RedisFixture {
    pub async fn start() -> Result<Self, RedisFixtureError> {
        Self::launch(&StartOptions {
            binary: REDIS_SERVER.to_owned(),
            startup_limit: STARTUP_LIMIT,
            extra_args: Vec::new(),
        })
        .await
    }

    pub(crate) async fn launch(options: &StartOptions) -> Result<Self, RedisFixtureError> {
        let owned = options.clone();
        let mut setup = JoinSet::new();
        let _starting = setup.spawn_blocking(move || -> Result<Self, RedisFixtureError> {
            // A unix socket path stops at 107 bytes, so the folder name stays short.
            let folder = Builder::new()
                .prefix("redis-")
                .tempdir()
                .map_err(|source| RedisFixtureError::Folder { source })?;
            let socket = folder.path().join("redis.sock");
            let url = socket_url(&socket)?;
            let log_path = folder.path().join("redis.log");
            let log =
                File::create(&log_path).map_err(|source| RedisFixtureError::Log { source })?;
            let diagnostics = log
                .try_clone()
                .map_err(|source| RedisFixtureError::Log { source })?;
            let child = server_command(&owned, &socket)
                .stdout(Stdio::from(log))
                .stderr(Stdio::from(diagnostics))
                .spawn()
                .map_err(spawn_error)?;
            Ok(Self {
                url,
                child,
                log_path,
                _folder: folder,
            })
        });
        let fixture = join_blocking(&mut setup).await?;
        fixture.await_ready(options.startup_limit).await
    }

    async fn await_ready(self, limit: Duration) -> Result<Self, RedisFixtureError> {
        if timeout(limit, self.wait_for_ping()).await.is_ok() {
            return Ok(self);
        }
        let mut cleanup = JoinSet::new();
        let _cleaning = cleanup.spawn_blocking(move || -> Result<String, RedisFixtureError> {
            let mut fixture = self;
            fixture.stop();
            fs::read_to_string(&fixture.log_path)
                .map_err(|source| RedisFixtureError::Log { source })
        });
        let log = join_blocking(&mut cleanup).await?;
        Err(RedisFixtureError::NotReady { limit, log })
    }

    async fn wait_for_ping(&self) {
        loop {
            if self.answers_ping().await {
                return;
            }
            sleep(POLL_INTERVAL).await;
        }
    }

    async fn answers_ping(&self) -> bool {
        let Ok(client) = Client::open(self.url.as_str()) else {
            return false;
        };
        let Ok(mut connection) = client.get_multiplexed_async_connection().await else {
            return false;
        };
        matches!(
            cmd("PING").query_async::<String>(&mut connection).await,
            Ok(reply) if reply == "PONG"
        )
    }

    fn stop(&mut self) {
        // A kill after the server already exited fails; the wait below still reaps it.
        let _killed = self.child.kill();
        let _reaped = self.child.wait();
    }
}

impl Drop for RedisFixture {
    fn drop(&mut self) {
        self.stop();
    }
}

// The setup and cleanup file work runs in a JoinSet task, so the runtime thread never waits on it.
async fn join_blocking<T: 'static>(
    set: &mut JoinSet<Result<T, RedisFixtureError>>,
) -> Result<T, RedisFixtureError> {
    let joined = set
        .join_next()
        .await
        .ok_or(RedisFixtureError::BlockingTaskAborted)?;
    match joined {
        Ok(outcome) => outcome,
        Err(source) => Err(RedisFixtureError::BlockingTaskFailed { source }),
    }
}

fn server_command(options: &StartOptions, socket: &Path) -> Command {
    let mut command = Command::new(&options.binary);
    command
        .arg("--port")
        .arg("0")
        .arg("--unixsocket")
        .arg(socket)
        .arg("--save")
        .arg("")
        .arg("--appendonly")
        .arg("no")
        .stdin(Stdio::null());
    for extra in &options.extra_args {
        command.arg(extra);
    }
    command
}

fn spawn_error(source: io::Error) -> RedisFixtureError {
    match source.kind() {
        io::ErrorKind::NotFound => RedisFixtureError::MissingServer { source },
        _ => RedisFixtureError::Spawn { source },
    }
}

pub(crate) fn socket_url(socket: &Path) -> Result<String, RedisFixtureError> {
    let path = socket
        .to_str()
        .ok_or_else(|| RedisFixtureError::NonUtf8Path {
            path: socket.to_path_buf(),
        })?;
    Ok(format!("redis+unix://{path}"))
}
