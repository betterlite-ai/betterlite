mod tests;

use std::collections::BTreeMap;

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
    pub max_connections: Option<i64>,
    pub health_check_interval: Option<i64>,
    pub socket_keepalive: Option<bool>,
    pub credential_provider: bool,
    pub credential_provider_settings: Option<CredentialProviderSettings>,
    pub passthrough: BTreeMap<String, PassthroughValue>,
    pub isolation: Option<IsolationConfig>,
}

impl From<&RedisConnectionSettings> for ClusterClient {
    fn from(settings: &RedisConnectionSettings) -> Self {
        Self {
            startup_nodes: settings.startup_nodes.clone().unwrap_or_default(),
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
#[must_use]
pub fn build(settings: &RedisConnectionSettings, kind: ClientKind) -> ClusterClient {
    let mut client = ClusterClient::from(settings);
    if matches!(kind, ClientKind::Sync) {
        return client;
    }
    client.passthrough.remove("redis_connect_func");
    client
        .health_check_interval
        .get_or_insert(CLUSTER_HEALTH_INTERVAL_SECS);
    client.socket_keepalive.get_or_insert(true);
    client.isolation = Some(IsolationConfig {
        branch: select_branch(true),
        request_ttl: DEFAULT_REQUEST_TTL,
        reinitialize_steps: passthrough_u32(&client.passthrough, "reinitialize_steps")
            .unwrap_or(DEFAULT_REINITIALIZE_STEPS),
        initialize: true,
        read_from_replicas: passthrough_bool(&client.passthrough, "read_from_replicas")
            .unwrap_or(false),
    });
    client
}

fn passthrough_u32(passthrough: &BTreeMap<String, PassthroughValue>, key: &str) -> Option<u32> {
    match passthrough.get(key)? {
        PassthroughValue::Number(number) => {
            let value = number.as_u64()?;
            u32::try_from(value).ok()
        }
        PassthroughValue::Bool(_)
        | PassthroughValue::Text(_)
        | PassthroughValue::List(_)
        | PassthroughValue::Map(_) => None,
    }
}

fn passthrough_bool(passthrough: &BTreeMap<String, PassthroughValue>, key: &str) -> Option<bool> {
    match passthrough.get(key)? {
        PassthroughValue::Bool(flag) => Some(*flag),
        PassthroughValue::Number(_)
        | PassthroughValue::Text(_)
        | PassthroughValue::List(_)
        | PassthroughValue::Map(_) => None,
    }
}
