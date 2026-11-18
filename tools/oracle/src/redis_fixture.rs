use std::collections::BTreeMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

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
use secrecy::SecretString;
use tempfile::TempDir;

pub(crate) mod lease;
pub(crate) mod resp;
mod tests;

use self::lease::LeaseCommands;
use self::resp::{CommandHandler, Entries, RespServer};

const DEFAULT_TTL: i64 = 60;

#[derive(Debug, thiserror::Error)]
pub enum RedisFixtureError {
    #[error("cannot create the fixture directory")]
    Directory(#[source] std::io::Error),
    #[error("cannot start the in-process Redis fixture")]
    Start(#[source] std::io::Error),
    #[error("cannot build the fixture cache")]
    Cache(#[source] RedisCacheError),
}

// An in-process Redis on a loopback port with a temporary directory of its own; the URI never leaves this module.
pub struct OracleRedis {
    server: Option<RespServer>,
    address: SocketAddr,
    directory: Option<TempDir>,
}

impl OracleRedis {
    pub fn start() -> Result<Self, RedisFixtureError> {
        let directory = TempDir::new().map_err(RedisFixtureError::Directory)?;
        let entries: Entries = Entries::default();
        let handler: Arc<dyn CommandHandler> = Arc::new(LeaseCommands {
            entries: Arc::clone(&entries),
            scripts: Mutex::new(BTreeMap::new()),
        });
        let server = RespServer::start(handler).map_err(RedisFixtureError::Start)?;
        let address = server.addr;
        Ok(Self {
            server: Some(server),
            address,
            directory: Some(directory),
        })
    }

    pub async fn cache(
        &self,
        namespace: Option<&str>,
    ) -> Result<Arc<RedisCache>, RedisFixtureError> {
        let overrides = RedisOverrides {
            host: None,
            port: None,
            db: None,
            username: None,
            password: None,
            url: Some(Some(SecretString::from(format!(
                "redis://{}/0",
                self.address
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

    pub fn shutdown(&mut self) {
        let _server = self.server.take();
        let _directory = self.directory.take();
    }
}

impl Drop for OracleRedis {
    fn drop(&mut self) {
        self.shutdown();
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
