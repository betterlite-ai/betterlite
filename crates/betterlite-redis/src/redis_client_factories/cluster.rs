mod tests;

use std::collections::BTreeMap;

use secrecy::SecretString;

use super::error::FactoryError;
use crate::redis_cluster_node_isolation::{IsolationConfig, select_branch};
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::{
    ClientKind, CredentialProviderSettings, RedisConnectionSettings,
};

// litellm._redis.get_redis_async_client's cluster branch: periodic health check plus TCP keepalive.
const CLUSTER_HEALTH_INTERVAL_SECS: i64 = 25;
const DEFAULT_REINITIALIZE_STEPS: u32 = 5;
const DEFAULT_REQUEST_TTL: u32 = 16;

#[derive(Debug, Clone)]
pub struct ClusterClient {
    pub startup_nodes: Vec<ClusterNodeConfig>,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub ssl: Option<bool>,
    pub ssl_password: Option<SecretString>,
    pub socket_timeout: Option<f64>,
    pub socket_connect_timeout: Option<f64>,
    pub max_connections: Option<i64>,
    pub health_check_interval: Option<i64>,
    pub socket_keepalive: Option<bool>,
    pub credential_provider: bool,
    pub credential_provider_settings: Option<CredentialProviderSettings>,
    pub passthrough: BTreeMap<String, PassthroughValue>,
    pub isolation: Option<IsolationConfig>,
}

// litellm._redis._get_redis_cluster_kwargs: the data-node credential and TLS/socket fields RedisCluster's own allowlist keeps.
impl From<&RedisConnectionSettings> for ClusterClient {
    fn from(settings: &RedisConnectionSettings) -> Self {
        Self {
            startup_nodes: settings.startup_nodes.clone().unwrap_or_default(),
            username: settings.username.clone(),
            password: settings.password.clone(),
            ssl: settings.ssl,
            ssl_password: settings.ssl_password.clone(),
            socket_timeout: settings.socket_timeout,
            socket_connect_timeout: settings.socket_connect_timeout,
            max_connections: settings.max_connections,
            health_check_interval: settings.health_check_interval,
            socket_keepalive: settings.socket_keepalive,
            credential_provider: settings.credential_provider,
            credential_provider_settings: settings.credential_provider_settings.clone(),
            passthrough: settings.passthrough.clone(),
            isolation: None,
        }
    }
}

// Every async Cluster client carries an IsolationConfig and drops redis_connect_func, since redis-py cannot run a blocking IAM callback during an async handshake.
pub fn build(
    settings: &RedisConnectionSettings,
    kind: ClientKind,
) -> Result<ClusterClient, FactoryError> {
    let mut client = ClusterClient::from(settings);
    if matches!(kind, ClientKind::Sync) {
        return Ok(client);
    }
    client.passthrough.remove("redis_connect_func");
    client
        .health_check_interval
        .get_or_insert(CLUSTER_HEALTH_INTERVAL_SECS);
    client.socket_keepalive.get_or_insert(true);
    let reinitialize_steps = match client.passthrough.get("reinitialize_steps") {
        Some(PassthroughValue::Number(number)) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| FactoryError::InvalidReinitializeSteps {
                value: number.to_string(),
            })?,
        Some(other) => {
            return Err(FactoryError::InvalidReinitializeSteps {
                value: format!("{other:?}"),
            });
        }
        None => DEFAULT_REINITIALIZE_STEPS,
    };
    let read_from_replicas = matches!(
        client.passthrough.get("read_from_replicas"),
        Some(PassthroughValue::Bool(true))
    );
    client.isolation = Some(IsolationConfig {
        branch: select_branch(true),
        request_ttl: DEFAULT_REQUEST_TTL,
        reinitialize_steps,
        initialize: true,
        read_from_replicas,
    });
    Ok(client)
}
