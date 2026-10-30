mod tests;

use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use super::nodes::{ClusterNodeConfig, SentinelNodesInput, StartupNodesInput};
use super::scalars::{BoolOrText, FloatOrText, IntOrText, PassthroughValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Direct,
    Url,
    Cluster,
    Sentinel,
}

// Mirrors litellm._redis._get_redis_client_logic's **env_overrides; numeric and boolean fields also accept a digit string, since Helm and env vars stringify everything.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RedisOverrides {
    pub host: Option<String>,
    pub port: Option<IntOrText>,
    pub db: Option<IntOrText>,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub url: Option<SecretString>,
    pub ssl: Option<BoolOrText>,
    pub socket_timeout: Option<FloatOrText>,
    pub socket_connect_timeout: Option<FloatOrText>,
    pub socket_keepalive: Option<BoolOrText>,
    pub max_connections: Option<IntOrText>,
    pub health_check_interval: Option<IntOrText>,
    pub credential_provider: bool,
    pub startup_nodes: Option<StartupNodesInput>,
    pub sentinel_nodes: Option<SentinelNodesInput>,
    pub sentinel_password: Option<SecretString>,
    pub service_name: Option<String>,
    pub passthrough: BTreeMap<String, PassthroughValue>,
}

// What litellm._redis._get_redis_client_logic returns once every field is typed and coerced.
#[derive(Debug, Clone, Default)]
pub struct RedisConnectionSettings {
    pub host: Option<String>,
    pub port: Option<i64>,
    pub db: Option<i64>,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub url: Option<SecretString>,
    pub ssl: Option<bool>,
    pub socket_timeout: Option<f64>,
    pub socket_connect_timeout: Option<f64>,
    pub socket_keepalive: Option<bool>,
    pub max_connections: Option<i64>,
    pub health_check_interval: Option<i64>,
    pub credential_provider: bool,
    pub startup_nodes: Option<Vec<ClusterNodeConfig>>,
    pub sentinel_nodes: Option<Vec<(String, i64)>>,
    pub sentinel_password: Option<SecretString>,
    pub service_name: Option<String>,
    pub passthrough: BTreeMap<String, PassthroughValue>,
}

impl RedisConnectionSettings {
    #[must_use]
    pub fn uses_startup_nodes(&self) -> bool {
        let nodes = self.startup_nodes.as_ref();
        nodes.is_some_and(|list| !list.is_empty())
    }
}
