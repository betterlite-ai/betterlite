use std::collections::BTreeMap;
use std::error::Error;
use std::net::{SocketAddr, TcpListener};
use std::process::{Child, ExitStatus};
use std::sync::Arc;

use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::TempDir;
use uuid::Uuid;

mod process;
pub(crate) mod resp;
mod tests;

const DEFAULT_TTL: i64 = 60;

#[derive(Debug, thiserror::Error)]
pub enum RedisFixtureError {
    #[error("cannot create the fixture directory")]
    Directory(#[source] std::io::Error),
    #[error("cannot start redis-server")]
    Start(#[source] std::io::Error),
    #[error("cannot stop redis-server")]
    Stop(#[source] std::io::Error),
    #[error("redis-server exited before it became ready: {0}")]
    Exited(ExitStatus),
    #[error("redis-server did not answer PING before the readiness deadline")]
    ReadinessTimeout,
    #[error("cannot probe redis-server")]
    Probe(#[from] redis::RedisError),
    #[error("cannot build the fixture cache")]
    Cache(#[source] RedisCacheError),
}

// The random password and readiness PID check prevent a port allocation race from selecting another server.
pub struct OracleRedis {
    server: Child,
    address: SocketAddr,
    directory: Option<TempDir>,
    password: SecretString,
}

impl OracleRedis {
    pub fn start() -> Result<Self, RedisFixtureError> {
        let directory = TempDir::new().map_err(RedisFixtureError::Directory)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(RedisFixtureError::Start)?;
        let address = listener.local_addr().map_err(RedisFixtureError::Start)?;
        let password = SecretString::from(Uuid::new_v4().to_string());
        drop(listener);
        let server = process::launch(directory.path(), address, &password)?;
        let mut fixture = Self {
            server,
            address,
            directory: Some(directory),
            password,
        };
        let client = redis::Client::open(format!(
            "redis://:{}@{address}/0",
            fixture.password.expose_secret()
        ))?;
        process::wait_ready(&mut fixture.server, &client)?;
        Ok(fixture)
    }

    pub async fn cache(
        &self,
        namespace: Option<&str>,
    ) -> Result<Arc<RedisCache>, RedisFixtureError> {
        let options = RedisCacheOptions {
            overrides: overrides(self.address, &self.password),
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

    pub fn shutdown(&mut self) -> Result<(), RedisFixtureError> {
        if self
            .server
            .try_wait()
            .map_err(RedisFixtureError::Stop)?
            .is_none()
        {
            self.server.kill().map_err(RedisFixtureError::Stop)?;
        }
        self.server.wait().map_err(RedisFixtureError::Stop)?;
        if let Some(directory) = self.directory.take() {
            directory.close().map_err(RedisFixtureError::Directory)?;
        }
        Ok(())
    }
}

fn overrides(address: SocketAddr, password: &SecretString) -> RedisOverrides {
    RedisOverrides {
        host: None,
        port: None,
        db: None,
        username: None,
        password: None,
        url: Some(Some(SecretString::from(format!(
            "redis://:{}@{address}/0",
            password.expose_secret()
        )))),
        ssl: None,
        socket_timeout: None,
        socket_connect_timeout: None,
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        gcp_service_account: None,
        gcp_ssl_ca_certs: None,
        azure_redis_ad_token: None,
        azure_client_id: None,
        azure_tenant_id: None,
        azure_client_secret: None,
        aws_iam_auth: None,
        aws_iam_user_name: None,
        aws_iam_cache_name: None,
        aws_iam_region: None,
        aws_iam_serverless: None,
        passthrough: BTreeMap::new(),
    }
}

impl Drop for OracleRedis {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            tracing::warn!(%error, "failed to clean up the Redis fixture");
        }
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
