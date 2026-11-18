use std::collections::BTreeMap;
use std::error::Error;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use secrecy::SecretString;
use tempfile::TempDir;

mod tests;

const START_TIMEOUT: Duration = Duration::from_secs(20);
const RETRY_DELAY: Duration = Duration::from_millis(50);
const DEFAULT_TTL: i64 = 60;

#[derive(Debug, thiserror::Error)]
pub enum RedisFixtureError {
    #[error("cannot create the fixture directory")]
    Directory(#[source] std::io::Error),
    #[error("cannot start redis-server; the gate image must install it")]
    Spawn(#[source] std::io::Error),
    #[error("redis-server exited before answering PING: {0}")]
    Exited(ExitStatus),
    #[error("redis-server did not answer PING within {0:?}")]
    Timeout(Duration),
    #[error("the redis fixture process or socket failed")]
    Io(#[source] std::io::Error),
    #[error("cannot build the fixture cache")]
    Cache(#[source] betterlite_redis::redis_cache::error::RedisCacheError),
}

// A throwaway redis-server on a loopback port and a temporary directory of its own; the URI never leaves this module.
pub struct OracleRedis {
    child: Child,
    address: SocketAddr,
    directory: Option<TempDir>,
}

impl OracleRedis {
    pub fn start() -> Result<Self, RedisFixtureError> {
        let directory = TempDir::new().map_err(RedisFixtureError::Directory)?;
        let address = free_address()?;
        let mut command = Command::new("redis-server");
        command
            .arg("--bind")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(address.port().to_string())
            .arg("--save")
            .arg("")
            .arg("--appendonly")
            .arg("no")
            .arg("--protected-mode")
            .arg("no")
            .arg("--daemonize")
            .arg("no")
            .arg("--dir")
            .arg(directory.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(RedisFixtureError::Spawn)?;
        if let Err(error) = wait_for_ping(&mut child, address) {
            let _killed = child.kill();
            let _waited = child.wait();
            return Err(error);
        }
        Ok(Self {
            child,
            address,
            directory: Some(directory),
        })
    }

    pub async fn cache(
        &self,
        namespace: Option<&str>,
    ) -> Result<Arc<RedisCache>, RedisFixtureError> {
        let overrides = RedisOverrides {
            url: Some(Some(SecretString::from(self.uri()))),
            ..RedisOverrides::default()
        };
        let options = RedisCacheOptions {
            overrides,
            family: RedisCacheFamily::Standalone,
            namespace: namespace.map(str::to_owned),
            default_ttl: DEFAULT_TTL,
            redis_flush_size: None,
            default_major_version: 7,
            factory: Arc::new(LiveClientFactory::new(None)),
            store: Arc::new(RedisClientStore::new()),
            hooks: Arc::new(Allow),
            gate: Arc::new(Allow),
        };
        let env = BTreeMap::<String, String>::new();
        connect(options, &env)
            .await
            .map(Arc::new)
            .map_err(RedisFixtureError::Cache)
    }

    pub fn shutdown(mut self) -> Result<(), RedisFixtureError> {
        self.stop()
    }

    fn uri(&self) -> String {
        format!("redis://{}/0", self.address)
    }

    fn stop(&mut self) -> Result<(), RedisFixtureError> {
        if self
            .child
            .try_wait()
            .map_err(RedisFixtureError::Io)?
            .is_none()
        {
            self.child.kill().map_err(RedisFixtureError::Io)?;
        }
        self.child.wait().map_err(RedisFixtureError::Io)?;
        drop(self.directory.take());
        Ok(())
    }
}

impl Drop for OracleRedis {
    fn drop(&mut self) {
        let _stopped = self.stop();
    }
}

fn free_address() -> Result<SocketAddr, RedisFixtureError> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(RedisFixtureError::Io)?;
    listener.local_addr().map_err(RedisFixtureError::Io)
}

fn wait_for_ping(child: &mut Child, address: SocketAddr) -> Result<(), RedisFixtureError> {
    let deadline = Instant::now()
        .checked_add(START_TIMEOUT)
        .ok_or(RedisFixtureError::Timeout(START_TIMEOUT))?;
    while Instant::now().checked_duration_since(deadline).is_none() {
        if let Some(status) = child.try_wait().map_err(RedisFixtureError::Io)? {
            return Err(RedisFixtureError::Exited(status));
        }
        if answers_ping(address) {
            return Ok(());
        }
        thread::park_timeout(RETRY_DELAY);
    }
    Err(RedisFixtureError::Timeout(START_TIMEOUT))
}

fn answers_ping(address: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, RETRY_DELAY) else {
        return false;
    };
    if stream.set_read_timeout(Some(RETRY_DELAY)).is_err() {
        return false;
    }
    if stream.write_all(b"*1\r\n$4\r\nPING\r\n").is_err() {
        return false;
    }
    let mut reply = [0_u8; 16];
    match stream.read(&mut reply) {
        Ok(bytes) => reply.starts_with(b"+PONG") && bytes >= 5,
        Err(_) => false,
    }
}

struct Allow;

impl ServiceHooks for Allow {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

impl BreakerGate for Allow {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}
