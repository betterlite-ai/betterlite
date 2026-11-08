pub mod commands;
pub mod connection_info;
pub mod credentials;
pub mod open;
pub mod pipeline;
pub mod plans;
pub mod sentinel;
pub mod tls;

pub(crate) mod tests;

use std::time::Duration;

use redis::aio::ConnectionManager;
use redis::cluster::ClusterClient;
use redis::cluster_async::ClusterConnection;
use redis::{Client, Cmd, FromRedisValue};
use tokio::task::JoinError;

use self::credentials::{RedisCredentialError, SourceProvider};
use self::sentinel::SentinelConnection;

pub struct LiveClientFactory {
    credentials: Option<SourceProvider>,
}

#[derive(Clone)]
pub enum LiveAsyncClient {
    Standalone(ConnectionManager),
    Cluster(ClusterConnection),
    Sentinel(SentinelConnection),
}

impl LiveAsyncClient {
    pub async fn query<T>(&self, command: &Cmd) -> Result<T, LiveClientError>
    where
        T: FromRedisValue,
    {
        match self {
            Self::Standalone(manager) => {
                let mut connection = manager.clone();
                Ok(command.query_async(&mut connection).await?)
            }
            Self::Cluster(cluster) => {
                let mut connection = cluster.clone();
                Ok(command.query_async(&mut connection).await?)
            }
            Self::Sentinel(sentinel) => sentinel.query(command).await,
        }
    }
}

#[derive(Clone)]
pub struct LiveEndpoint {
    pub target: LiveTarget,
    pub connect_timeout: Duration,
    pub response_timeout: Duration,
}

#[derive(Clone)]
pub enum LiveTarget {
    Standalone(Box<Client>),
    Cluster(Box<ClusterClient>),
}

#[derive(Debug, thiserror::Error)]
pub enum LiveClientError {
    #[error(transparent)]
    Redis(#[from] redis::RedisError),
    #[error("the redis setting '{field}' is invalid")]
    InvalidSetting { field: &'static str },
    #[error("a redis credential provider is configured but no credential source was injected")]
    CredentialSourceMissing,
    #[error(transparent)]
    Credential(#[from] RedisCredentialError),
    #[error("the TLS file for '{key}' could not be read")]
    TlsFile { key: &'static str },
    #[error("no sentinel monitor knows the master for service '{service_name}'")]
    NoSentinelMaster { service_name: String },
    #[error("the redis connection task failed")]
    Join(#[from] JoinError),
}
